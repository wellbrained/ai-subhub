//! CLIProxyAPI hub through its management API: `GET auth-files` (the logins) and
//! `POST api-call` (usage reads; the hub substitutes `$TOKEN). Writes, all started by the
//! owner: credential priority (steering), removing a login (`DELETE auth-files`) and adding one
//! (the hub's own OAuth flow: `*-auth-url`, then `get-auth-status`). Never reads the hub's
//! auth files directly.
use std::time::Duration;

use chrono::Utc;
use serde_json::{Value, json};

use crate::model::{ProviderKind, ReadSource, UsageResult, UsageState};
use crate::util::{self, failure};
use crate::{claude, codex};

#[derive(Debug, Clone, PartialEq)]
pub struct HubAccount {
    pub provider: ProviderKind,
    pub email: Option<String>,
    pub file_name: String,
    pub auth_index: serde_json::Value,
    pub disabled: bool,
    /// ChatGPT account id (Codex only); required by `wham/usage` as `Chatgpt-Account-Id`.
    pub account_id: Option<String>,
    /// Credential priority from `auth-files`, when the hub reports one.
    pub priority: Option<i64>,
}

#[derive(Debug)]
pub enum HubError {
    /// Not reachable (not running, wrong URL).
    Unreachable,
    /// 401/403: key missing or wrong.
    Unauthorized,
    Other(String),
}

impl std::fmt::Display for HubError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HubError::Unreachable => f.write_str("The hub is not reachable."),
            HubError::Unauthorized => f.write_str("The hub rejected the management key."),
            HubError::Other(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for HubError {}

pub struct HubClient {
    pub base_url: String,
    key: String,
    http: Option<reqwest::blocking::Client>,
}

/// Result of one `api-call`: the provider's HTTP status, its `Retry-After` and the parsed body.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ApiCall {
    pub status: u16,
    pub retry_after: Option<u64>,
    /// Parsed JSON body; Null when absent or not JSON (raw text is never kept).
    pub body: Value,
}

fn first_str<'a>(v: &'a Value, names: &[&str]) -> Option<&'a str> {
    names
        .iter()
        .find_map(|n| v.get(*n).and_then(Value::as_str))
        .filter(|s| !s.trim().is_empty())
}

fn provider_of(file: &Value) -> Option<ProviderKind> {
    match first_str(file, &["provider", "type"])?
        .to_ascii_lowercase()
        .as_str()
    {
        "claude" | "anthropic" => Some(ProviderKind::Claude),
        "codex" | "openai" => Some(ProviderKind::Codex),
        _ => None,
    }
}

fn clean_id(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty() && s.len() <= 128 && !s.chars().any(|c| c.is_control() || c.is_whitespace()))
        .then(|| s.to_string())
}

fn id_in(v: &Value) -> Option<String> {
    ["chatgpt_account_id", "chatgptAccountId"]
        .iter()
        .find_map(|k| v.get(*k)?.as_str().and_then(clean_id))
}

/// Base64url (padding optional) to bytes; None on any invalid character.
pub(crate) fn base64url_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.trim_end_matches('=').bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// Account id from a JWT string. The payload is decoded only to read this id; the token is
/// neither kept nor verified.
fn jwt_account_id(jwt: &str) -> Option<String> {
    let payload = jwt.split('.').nth(1)?;
    let claims: Value = serde_json::from_slice(&base64url_decode(payload)?).ok()?;
    id_in(&claims).or_else(|| id_in(claims.get("https://api.openai.com/auth")?))
}

/// ChatGPT account id from an auth-files entry: direct fields on the entry, `metadata`,
/// `attributes`; else from `id_token` (object or JWT string) in the same three places.
fn account_id_of(entry: &Value) -> Option<String> {
    let scopes = [Some(entry), entry.get("metadata"), entry.get("attributes")];
    scopes.iter().flatten().find_map(|s| id_in(s)).or_else(|| {
        scopes
            .iter()
            .flatten()
            .find_map(|s| match s.get("id_token")? {
                Value::String(t) => jwt_account_id(t),
                obj @ Value::Object(_) => id_in(obj),
                _ => None,
            })
    })
}

/// Parses the `auth-files` body:`{"files": [...]}` (or a bare array). Claude and Codex logins
/// only. The file name is `name` (CLIProxyAPI always sends it: the auth file's name, else its
/// id; checked against its `buildAuthFileEntry` source, 2026-11-08), with `path` as fallback.
pub(crate) fn parse_auth_files(body: &Value) -> Vec<HubAccount> {
    let files = body
        .get("files")
        .and_then(Value::as_array)
        .or_else(|| body.as_array());
    let Some(files) = files else { return vec![] };
    files
        .iter()
        .filter(|f| f.is_object())
        .filter_map(|f| {
            let provider = provider_of(f)?;
            let file_name = first_str(f, &["name", "path"])
                .map(|n| n.rsplit(['/', '\\']).next().unwrap_or(n).to_string())
                .unwrap_or_default();
            let email = first_str(f, &["email"])
                .or_else(|| first_str(f, &["account", "label"]).filter(|s| s.contains('@')))
                .and_then(util::valid_email);
            let disabled = f.get("disabled").and_then(Value::as_bool).unwrap_or(false)
                || first_str(f, &["status"]).is_some_and(|s| s.eq_ignore_ascii_case("disabled"));
            Some(HubAccount {
                provider,
                email,
                file_name,
                auth_index: f.get("auth_index").cloned().unwrap_or(Value::Null),
                disabled,
                account_id: if provider == ProviderKind::Codex {
                    account_id_of(f)
                } else {
                    None
                },
                priority: f.get("priority").and_then(|p| {
                    p.as_i64()
                        .or_else(|| p.as_f64().filter(|x| x.is_finite()).map(|x| x as i64))
                }),
            })
        })
        .collect()
}

/// JSON body of the one allowed hub write: `{"name": <file>, "priority": <n or null>}`.
pub(crate) fn priority_body(file_name: &str, priority: Option<i64>) -> Value {
    json!({"name": file_name, "priority": priority})
}

fn header_value(headers: &Value, name: &str) -> Option<String> {
    let obj = headers.as_object()?;
    let (_, v) = obj.iter().find(|(k, _)| k.eq_ignore_ascii_case(name))?;
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Array(a) => a.first()?.as_str().map(str::to_string),
        _ => None,
    }
}

/// Parses the `api-call` wrapper `{status_code, header, body}`; body may be a JSON string or an
/// object. None when there is no usable `status_code`.
pub(crate) fn parse_api_call(wrapper: &Value, now: chrono::DateTime<Utc>) -> Option<ApiCall> {
    let status = u16::try_from(wrapper.get("status_code")?.as_u64()?).ok()?;
    let retry_after = wrapper
        .get("header")
        .and_then(|h| header_value(h, "retry-after"))
        .and_then(|v| util::parse_retry_after(&v, now));
    let body = match wrapper.get("body") {
        Some(Value::String(s)) => serde_json::from_str(s).unwrap_or(Value::Null),
        Some(v) => v.clone(),
        None => Value::Null,
    };
    Some(ApiCall {
        status,
        retry_after,
        body,
    })
}

fn hub_down() -> UsageResult {
    failure(
        UsageState::Unavailable,
        "The hub is not reachable.",
        ReadSource::Hub,
    )
}

impl HubClient {
    pub fn new(base_url: impl Into<String>, key: impl Into<String>) -> Self {
        let base_url: String = base_url.into();
        Self {
            base_url: base_url.trim().trim_end_matches('/').to_string(),
            key: crate::secrets::normalize_hub_key(&key.into()),
            http: util::http_client(Duration::from_secs(20)),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/v0/management/{path}", self.base_url)
    }

    /// Lists Claude and Codex logins; other providers are skipped.
    pub fn list_accounts(&self) -> Result<Vec<HubAccount>, HubError> {
        let http = self
            .http
            .as_ref()
            .ok_or_else(|| HubError::Other("Could not start the network client.".into()))?;
        let response = http
            .get(self.url("auth-files"))
            .bearer_auth(&self.key)
            .send()
            .map_err(|e| {
                if e.is_connect() || e.is_timeout() {
                    HubError::Unreachable
                } else {
                    HubError::Other("The hub request failed.".into())
                }
            })?;
        match response.status().as_u16() {
            200..=299 => {}
            401 | 403 => return Err(HubError::Unauthorized),
            404 => {
                return Err(HubError::Other(
                    "The hub has no management API at this address.".into(),
                ));
            }
            n => return Err(HubError::Other(format!("The hub answered with HTTP {n}."))),
        }
        let body: Value = response
            .json()
            .map_err(|_| HubError::Other("The hub returned an unexpected response.".into()))?;
        Ok(parse_auth_files(&body))
    }

    /// The only write SubHub makes to the hub: set (or clear, with None) one login's credential
    /// priority. Tries `credentials/fields`, and `auth-files/fields` for older hub builds on 404.
    pub fn set_priority(&self, file_name: &str, priority: Option<i64>) -> Result<(), HubError> {
        let http = self
            .http
            .as_ref()
            .ok_or_else(|| HubError::Other("Could not start the network client.".into()))?;
        let body = priority_body(file_name, priority);
        let mut status = 404;
        for path in ["credentials/fields", "auth-files/fields"] {
            let response = http
                .patch(self.url(path))
                .bearer_auth(&self.key)
                .json(&body)
                .send()
                .map_err(|e| {
                    if e.is_connect() || e.is_timeout() {
                        HubError::Unreachable
                    } else {
                        HubError::Other("The hub request failed.".into())
                    }
                })?;
            status = response.status().as_u16();
            if status != 404 {
                break;
            }
        }
        match status {
            200..=299 => Ok(()),
            401 | 403 => Err(HubError::Unauthorized),
            n => Err(HubError::Other(format!("The hub answered with HTTP {n}."))),
        }
    }

    /// One provider request through the hub (`$TOKEN$` is substituted by the hub). Err is a
    /// hub-level failure already mapped to a result.
    fn api_call(
        &self,
        auth_index: &Value,
        url: &str,
        headers: Value,
    ) -> Result<ApiCall, Box<UsageResult>> {
        self.api_call_inner(auth_index, url, headers)
            .map_err(Box::new)
    }

    #[allow(clippy::result_large_err)] // private; boxed by `api_call`
    fn api_call_inner(
        &self,
        auth_index: &Value,
        url: &str,
        headers: Value,
    ) -> Result<ApiCall, UsageResult> {
        let http = self.http.as_ref().ok_or_else(hub_down)?;
        let request =
            json!({"auth_index": auth_index, "method": "GET", "url": url, "header": headers});
        let response = http
            .post(self.url("api-call"))
            .bearer_auth(&self.key)
            .json(&request)
            .send()
            .map_err(|_| hub_down())?;
        let src = ReadSource::Hub;
        match response.status().as_u16() {
            200..=299 => {}
            401 | 403 => {
                return Err(failure(
                    UsageState::Unavailable,
                    "The hub rejected the management key.",
                    src,
                ));
            }
            n => {
                return Err(failure(
                    UsageState::Unavailable,
                    &format!("The hub answered with HTTP {n}."),
                    src,
                ));
            }
        }
        let wrapper: Value = response.json().map_err(|_| {
            failure(
                UsageState::Unavailable,
                "The hub returned an unexpected response.",
                src,
            )
        })?;
        parse_api_call(&wrapper, Utc::now()).ok_or_else(|| {
            failure(
                UsageState::Unavailable,
                "The hub returned an unexpected response.",
                src,
            )
        })
    }

    /// Reads usage (and plan/email when available) for one hub login. `source` = Hub.
    pub fn read_usage(&self, account: &HubAccount) -> UsageResult {
        let src = ReadSource::Hub;
        if account.auth_index.is_null() {
            return failure(
                UsageState::Unavailable,
                "This hub login has no index to read usage with.",
                src,
            );
        }
        match account.provider {
            ProviderKind::Claude => self.read_claude(account),
            ProviderKind::Codex => self.read_codex(account),
            // The hub listing keeps only Claude and Codex logins.
            _ => failure(
                UsageState::Unavailable,
                "The hub does not read this provider.",
                src,
            ),
        }
    }

    fn read_claude(&self, account: &HubAccount) -> UsageResult {
        let headers = json!({
            "Authorization": "Bearer $TOKEN$",
            "anthropic-beta": "oauth-2025-04-20",
            "Accept": "application/json",
            "User-Agent": util::USER_AGENT,
        });
        let usage = match self.api_call(&account.auth_index, claude::USAGE_URL, headers.clone()) {
            Ok(c) => c,
            Err(r) => return *r,
        };
        // Plan/email are best effort and only fetched after a good usage answer.
        let profile = if (200..300).contains(&usage.status) {
            self.api_call(&account.auth_index, claude::PROFILE_URL, headers)
                .ok()
                .filter(|c| (200..300).contains(&c.status))
                .map(|c| c.body)
        } else {
            None
        };
        let mut r = claude::usage_result(
            usage.status,
            usage.retry_after,
            &usage.body,
            profile.as_ref(),
            None,
            ReadSource::Hub,
            Utc::now(),
        );
        if r.state == UsageState::Ready && r.email.is_none() {
            r.email = account.email.clone();
        }
        r
    }

    fn read_codex(&self, account: &HubAccount) -> UsageResult {
        // Same request shape as the hub's own management UI; the account id header is required.
        let mut headers = json!({
            "Authorization": "Bearer $TOKEN$",
            "Content-Type": "application/json",
            "User-Agent": "codex-tui/0.149.1 (Windows 10.0; x86_64) (codex-tui; 0.149.1)",
        });
        if let Some(id) = &account.account_id {
            headers["Chatgpt-Account-Id"] = json!(id);
        }
        match self.api_call(&account.auth_index, codex::HUB_USAGE_URL, headers) {
            Ok(c) => codex::wham_result(
                c.status,
                c.retry_after,
                &c.body,
                account.email.clone(),
                ReadSource::Hub,
                Utc::now(),
            ),
            Err(r) => *r,
        }
    }
}

/// A sign-in the hub started: open `url` in the browser, then poll with `state`.
#[derive(Debug, Clone, PartialEq)]
pub struct LoginStart {
    pub url: String,
    pub state: String,
}

/// Where a hub sign-in stands.
#[derive(Debug, Clone, PartialEq)]
pub enum LoginStatus {
    Waiting,
    Done,
    Failed(String),
}

/// The management path that starts a sign-in for `provider`.
fn login_path(provider: ProviderKind) -> Option<&'static str> {
    match provider {
        ProviderKind::Claude => Some("anthropic-auth-url"),
        ProviderKind::Codex => Some("codex-auth-url"),
        _ => None,
    }
}

fn parse_login_start(v: &Value) -> Option<LoginStart> {
    let url = v.get("url")?.as_str()?.trim();
    let state = v.get("state")?.as_str()?.trim();
    (url.starts_with("https://") && !state.is_empty()).then(|| LoginStart {
        url: url.to_string(),
        state: state.to_string(),
    })
}

fn parse_login_status(v: &Value) -> LoginStatus {
    match v.get("status").and_then(Value::as_str) {
        Some("ok") => LoginStatus::Done,
        Some("wait") => LoginStatus::Waiting,
        _ => LoginStatus::Failed(
            v.get("error")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|e| !e.is_empty())
                .unwrap_or("The sign-in did not finish.")
                .to_string(),
        ),
    }
}

fn map_send(e: reqwest::Error) -> HubError {
    if e.is_connect() || e.is_timeout() {
        HubError::Unreachable
    } else {
        HubError::Other("The hub request failed.".into())
    }
}

impl HubClient {
    fn http(&self) -> Result<&reqwest::blocking::Client, HubError> {
        self.http
            .as_ref()
            .ok_or_else(|| HubError::Other("Could not start the network client.".into()))
    }

    /// Status check shared by the login calls; the JSON body on success.
    fn json_of(response: reqwest::blocking::Response) -> Result<Value, HubError> {
        match response.status().as_u16() {
            200..=299 => {}
            401 | 403 => return Err(HubError::Unauthorized),
            404 => {
                return Err(HubError::Other(
                    "This hub version does not offer that through its management API.".into(),
                ));
            }
            n => return Err(HubError::Other(format!("The hub answered with HTTP {n}."))),
        }
        response
            .json()
            .map_err(|_| HubError::Other("The hub returned an unexpected response.".into()))
    }

    /// Removes a login from the hub: deletes its auth file there. Signing in again brings it
    /// back. SubHub's account stays (it only loses its hub link on the next listing).
    pub fn remove_login(&self, file_name: &str) -> Result<(), HubError> {
        let response = self
            .http()?
            .delete(self.url("auth-files"))
            .query(&[("name", file_name)])
            .bearer_auth(&self.key)
            .send()
            .map_err(map_send)?;
        Self::json_of(response).map(|_| ())
    }

    /// Starts the hub's sign-in for a new login (`is_webui`: the hub runs the callback itself).
    pub fn start_login(&self, provider: ProviderKind) -> Result<LoginStart, HubError> {
        let response = self
            .http()?
            .get(self.url(login_path(provider).ok_or_else(|| {
                HubError::Other("SubHub adds only Claude and Codex logins to the hub.".into())
            })?))
            .query(&[("is_webui", "true")])
            .bearer_auth(&self.key)
            .send()
            .map_err(map_send)?;
        parse_login_start(&Self::json_of(response)?)
            .ok_or_else(|| HubError::Other("The hub did not return a sign-in link.".into()))
    }

    pub fn login_status(&self, state: &str) -> Result<LoginStatus, HubError> {
        let response = self
            .http()?
            .get(self.url("get-auth-status"))
            .query(&[("state", state)])
            .bearer_auth(&self.key)
            .send()
            .map_err(map_send)?;
        Ok(parse_login_status(&Self::json_of(response)?))
    }

    /// Cancels a pending sign-in (best effort).
    pub fn cancel_login(&self, state: &str) {
        if let Ok(http) = self.http() {
            let _ = http
                .delete(self.url("oauth-session"))
                .query(&[("state", state)])
                .bearer_auth(&self.key)
                .send();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_start_needs_an_https_link_and_a_state() {
        let ok = json!({"status": "ok", "url": "https://claude.ai/oauth/authorize?x=1", "state": "anth-123"});
        assert_eq!(
            parse_login_start(&ok),
            Some(LoginStart {
                url: "https://claude.ai/oauth/authorize?x=1".into(),
                state: "anth-123".into()
            })
        );
        assert_eq!(
            parse_login_start(&json!({"url": "javascript:x", "state": "s"})),
            None
        );
        assert_eq!(
            parse_login_start(&json!({"url": "https://a.test", "state": ""})),
            None
        );
        assert_eq!(login_path(ProviderKind::Codex), Some("codex-auth-url"));
        assert_eq!(login_path(ProviderKind::Cursor), None);
    }

    #[test]
    fn login_status_maps_wait_ok_and_errors() {
        assert_eq!(
            parse_login_status(&json!({"status": "wait"})),
            LoginStatus::Waiting
        );
        assert_eq!(
            parse_login_status(&json!({"status": "ok"})),
            LoginStatus::Done
        );
        assert_eq!(
            parse_login_status(&json!({"status": "error", "error": "expired"})),
            LoginStatus::Failed("expired".into())
        );
        assert_eq!(
            parse_login_status(&json!({})),
            LoginStatus::Failed("The sign-in did not finish.".into())
        );
    }

    fn now() -> chrono::DateTime<Utc> {
        "2026-11-06T10:00:00Z".parse().unwrap()
    }

    #[test]
    fn auth_files_keep_claude_and_codex_only() {
        let body = json!({"files": [
            {"provider": "claude", "email": "a@example.test", "disabled": false, "auth_index": "abc123", "name": "claude-a.json"},
            {"type": "codex", "email": "b@example.test", "disabled": true, "auth_index": 7, "id": "codex-b.json"},
            {"provider": "gemini", "email": "c@example.test", "auth_index": 1},
            "not an object",
            {"email": "d@example.test"}]});
        let accounts = parse_auth_files(&body);
        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts[0].provider, ProviderKind::Claude);
        assert_eq!(accounts[0].file_name, "claude-a.json");
        assert_eq!(accounts[0].auth_index, json!("abc123"));
        assert!(!accounts[0].disabled);
        assert_eq!(accounts[1].provider, ProviderKind::Codex);
        assert_eq!(accounts[1].auth_index, json!(7));
        assert!(accounts[1].disabled);
    }

    #[test]
    fn auth_files_are_parsed_defensively() {
        let body = json!({"files": [
            {"provider": "Claude", "account": "someone@example.test", "path": "C:\\fake\\dir\\x.json", "status": "disabled"},
            {"provider": "claude", "label": "Not an email", "auth_index": "z"}]});
        let a = parse_auth_files(&body);
        assert_eq!(a[0].email.as_deref(), Some("someone@example.test"));
        assert_eq!(a[0].file_name, "x.json");
        assert!(a[0].disabled);
        assert!(a[0].auth_index.is_null());
        assert_eq!(a[1].email, None);
        assert_eq!(a[1].file_name, "");
        assert!(parse_auth_files(&json!({"files": "nope"})).is_empty());
        assert_eq!(
            parse_auth_files(&json!([{"provider": "codex", "auth_index": 1}])).len(),
            1
        );
    }

    fn fake_jwt(claims: &Value) -> String {
        const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let data = claims.to_string().into_bytes();
        let mut out = String::new();
        for chunk in data.chunks(3) {
            let n = chunk
                .iter()
                .enumerate()
                .fold(0u32, |a, (i, b)| a | (u32::from(*b) << (16 - 8 * i)));
            for i in 0..=chunk.len() {
                out.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
            }
        }
        format!("fakehdr.{out}.fakesig")
    }

    #[test]
    fn codex_account_id_from_direct_fields_in_priority_order() {
        let files = |entry: Value| parse_auth_files(&json!({"files": [entry]}));
        let a = files(
            json!({"type": "codex", "chatgptAccountId": "acc-direct", "metadata": {"chatgpt_account_id": "acc-meta"}}),
        );
        assert_eq!(a[0].account_id.as_deref(), Some("acc-direct"));
        let a = files(
            json!({"type": "codex", "metadata": {"chatgpt_account_id": "acc-meta"}, "attributes": {"chatgpt_account_id": "acc-attr"}}),
        );
        assert_eq!(a[0].account_id.as_deref(), Some("acc-meta"));
        let a = files(json!({"type": "codex", "attributes": {"chatgptAccountId": "acc-attr"}}));
        assert_eq!(a[0].account_id.as_deref(), Some("acc-attr"));
        // Direct ids win over any id_token.
        let jwt = fake_jwt(&json!({"chatgpt_account_id": "acc-jwt"}));
        let a = files(
            json!({"type": "codex", "id_token": jwt, "attributes": {"chatgpt_account_id": "acc-attr"}}),
        );
        assert_eq!(a[0].account_id.as_deref(), Some("acc-attr"));
    }

    #[test]
    fn codex_account_id_from_id_token_object_or_jwt() {
        let files = |entry: Value| parse_auth_files(&json!({"files": [entry]}));
        let a = files(json!({"type": "codex", "id_token": {"chatgpt_account_id": "acc-obj"}}));
        assert_eq!(a[0].account_id.as_deref(), Some("acc-obj"));
        let top = fake_jwt(&json!({"chatgpt_account_id": "acc-top", "email": "x@example.test"}));
        let a = files(json!({"type": "codex", "id_token": top}));
        assert_eq!(a[0].account_id.as_deref(), Some("acc-top"));
        let nested =
            fake_jwt(&json!({"https://api.openai.com/auth": {"chatgpt_account_id": "acc-claim"}}));
        let a = files(json!({"type": "codex", "metadata": {"id_token": nested}}));
        assert_eq!(a[0].account_id.as_deref(), Some("acc-claim"));
        let a = files(
            json!({"type": "codex", "attributes": {"id_token": fake_jwt(&json!({"chatgpt_account_id": "acc-a"}))}}),
        );
        assert_eq!(a[0].account_id.as_deref(), Some("acc-a"));
    }

    #[test]
    fn codex_account_id_bad_tokens_give_none() {
        let files = |entry: Value| parse_auth_files(&json!({"files": [entry]}));
        for tok in [
            json!("not-a-jwt"),
            json!("a.!!!.c"),
            json!("a.e30.c"),
            json!(5),
            json!(null),
        ] {
            assert_eq!(
                files(json!({"type": "codex", "id_token": tok}))[0].account_id,
                None
            );
        }
        // Claude logins never carry one.
        assert_eq!(
            files(json!({"type": "claude", "chatgpt_account_id": "x"}))[0].account_id,
            None
        );
        assert_eq!(base64url_decode("aGk").as_deref(), Some(&b"hi"[..]));
        assert_eq!(base64url_decode("aGk=").as_deref(), Some(&b"hi"[..]));
    }

    #[test]
    fn priority_is_parsed_and_body_is_built() {
        let a = parse_auth_files(&json!({"files": [
            {"type": "claude", "name": "a.json", "priority": 5},
            {"type": "claude", "name": "b.json", "priority": null},
            {"type": "claude", "name": "c.json", "priority": "x"},
            {"type": "claude", "name": "d.json"}]}));
        let p: Vec<_> = a.iter().map(|x| x.priority).collect();
        assert_eq!(p, [Some(5), None, None, None]);
        assert_eq!(
            priority_body("a.json", Some(100)),
            json!({"name": "a.json", "priority": 100})
        );
        assert_eq!(
            priority_body("a.json", None),
            json!({"name": "a.json", "priority": null})
        );
    }

    #[test]
    fn api_call_body_may_be_string_or_object() {
        let w = json!({"status_code": 200, "header": {}, "body": "{\"five_hour\":{\"utilization\":5}}"});
        let c = parse_api_call(&w, now()).unwrap();
        assert_eq!(c.status, 200);
        assert_eq!(c.body["five_hour"]["utilization"], 5);
        let w = json!({"status_code": 200, "body": {"a": 1}});
        assert_eq!(parse_api_call(&w, now()).unwrap().body["a"], 1);
        // Non-JSON text is dropped, never kept.
        let w = json!({"status_code": 200, "body": "<html>secret</html>"});
        assert!(parse_api_call(&w, now()).unwrap().body.is_null());
        assert!(parse_api_call(&json!({"body": {}}), now()).is_none());
        assert!(parse_api_call(&json!({"status_code": "200"}), now()).is_none());
    }

    #[test]
    fn api_call_retry_after_from_headers() {
        for header in [
            json!({"Retry-After": ["120"]}),
            json!({"retry-after": "120"}),
        ] {
            let w = json!({"status_code": 429, "header": header, "body": ""});
            let c = parse_api_call(&w, now()).unwrap();
            assert_eq!((c.status, c.retry_after), (429, Some(120)));
        }
    }

    #[test]
    fn hub_429_flows_into_a_rate_limited_result() {
        let w = json!({"status_code": 429, "header": {"Retry-After": ["300"]}, "body": "{}"});
        let c = parse_api_call(&w, now()).unwrap();
        let r = claude::usage_result(
            c.status,
            c.retry_after,
            &c.body,
            None,
            None,
            ReadSource::Hub,
            now(),
        );
        assert_eq!(
            (r.state, r.retry_after_secs),
            (UsageState::RateLimited, Some(300))
        );
        let r = codex::wham_result(
            c.status,
            c.retry_after,
            &c.body,
            None,
            ReadSource::Hub,
            now(),
        );
        assert_eq!(
            (r.state, r.retry_after_secs),
            (UsageState::RateLimited, Some(300))
        );
    }

    #[test]
    fn client_normalizes_url_and_key_and_rejects_missing_index_without_network() {
        let c = HubClient::new(" http://127.0.0.1:8317/ ", "management-key = fake-k");
        assert_eq!(c.base_url, "http://127.0.0.1:8317");
        assert_eq!(c.key, "fake-k");
        let acc = HubAccount {
            provider: ProviderKind::Claude,
            email: None,
            file_name: String::new(),
            auth_index: Value::Null,
            disabled: false,
            account_id: None,
            priority: None,
        };
        let r = c.read_usage(&acc);
        assert_eq!(
            (r.state, r.source),
            (UsageState::Unavailable, ReadSource::Hub)
        );
    }
}

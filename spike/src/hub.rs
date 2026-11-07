//! One read-only pass against the CLIProxyAPI hub: list accounts, then read Claude usage for
//! the first enabled Claude account through `api-call` (the hub injects the token via $TOKEN$).
//! Never logs the key, tokens or raw response bodies.
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

pub const KEY_SERVICE: &str = "AI SubHub";
pub const KEY_USER: &str = "hub-management-key";
pub const BASE: &str = "http://127.0.0.1:8317";

#[derive(Debug, Clone)]
pub struct HubAccount {
    pub provider: String,
    pub email: String,
    pub disabled: bool,
    pub auth_index: Option<Value>,
    /// (window name, % used) when read.
    pub usage: Vec<(String, f64)>,
    pub usage_error: Option<String>,
}

pub fn key_entry() -> Result<keyring::Entry> {
    Ok(keyring::Entry::new(KEY_SERVICE, KEY_USER)?)
}

/// Accepts a bare key or pasted `name = value` lines; picks the management key line.
pub fn normalize_key(raw: &str) -> String {
    let lines: Vec<&str> = raw.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if lines.len() == 1 && !lines[0].contains(char::is_whitespace) {
        return lines[0].to_string();
    }
    let pick = lines
        .iter()
        .find(|l| l.to_ascii_lowercase().contains("manag"))
        .or(lines.last())
        .copied()
        .unwrap_or_default();
    let value = pick.rsplit(['=', ':']).next().unwrap_or(pick);
    value.trim().trim_matches(['"', '\'', '`']).to_string()
}

pub fn read() -> Result<Vec<HubAccount>> {
    let key = key_entry()?
        .get_password()
        .context("no hub key in Credential Manager (run with --store-hub-key <file>)")?;
    let key = normalize_key(&key);
    let http = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()?;

    let res = http
        .get(format!("{BASE}/v0/management/auth-files"))
        .bearer_auth(&key)
        .send()
        .context("hub unreachable")?;
    if !res.status().is_success() {
        bail!("auth-files: HTTP {}", res.status().as_u16());
    }
    let body: Value = res.json().context("auth-files: not JSON")?;
    let files = body
        .get("files")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut accounts: Vec<HubAccount> = files
        .iter()
        .map(|f| HubAccount {
            provider: str_field(f, &["provider", "type"]).unwrap_or_else(|| "?".into()),
            email: str_field(f, &["email", "account", "label"]).unwrap_or_else(|| "?".into()),
            disabled: f.get("disabled").and_then(Value::as_bool).unwrap_or(false),
            auth_index: f.get("auth_index").cloned(),
            usage: vec![],
            usage_error: None,
        })
        .collect();

    // Only the first enabled Claude account, to keep load on the provider minimal.
    if let Some(acc) = accounts
        .iter_mut()
        .find(|a| a.provider == "claude" && !a.disabled && a.auth_index.is_some())
    {
        match claude_usage(&http, &key, acc.auth_index.clone().unwrap()) {
            Ok(u) => acc.usage = u,
            Err(e) => acc.usage_error = Some(e.to_string()),
        }
    }
    Ok(accounts)
}

fn claude_usage(
    http: &reqwest::blocking::Client,
    key: &str,
    auth_index: Value,
) -> Result<Vec<(String, f64)>> {
    let req = json!({
        "auth_index": auth_index,
        "method": "GET",
        "url": "https://api.anthropic.com/api/oauth/usage",
        "header": {
            "Authorization": "Bearer $TOKEN$",
            "anthropic-beta": "oauth-2025-04-20",
            "Accept": "application/json",
            "User-Agent": "ai-subhub-spike"
        }
    });
    let res = http
        .post(format!("{BASE}/v0/management/api-call"))
        .bearer_auth(key)
        .json(&req)
        .send()?;
    if !res.status().is_success() {
        bail!("api-call: HTTP {}", res.status().as_u16());
    }
    let wrapper: Value = res.json()?;
    let status = wrapper.get("status_code").and_then(Value::as_u64).unwrap_or(0);
    if !(200..300).contains(&status) {
        bail!("usage endpoint: HTTP {status}");
    }
    // body may be a JSON string or an object.
    let body = match wrapper.get("body") {
        Some(Value::String(s)) => serde_json::from_str(s)?,
        Some(v) => v.clone(),
        None => bail!("usage: empty body"),
    };
    let mut out = vec![];
    for name in ["five_hour", "seven_day", "seven_day_opus", "seven_day_sonnet"] {
        if let Some(u) = body.get(name).and_then(|w| w.get("utilization")).and_then(Value::as_f64) {
            out.push((name.to_string(), u));
        }
    }
    Ok(out)
}

fn str_field(v: &Value, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|n| v.get(*n).and_then(Value::as_str).map(str::to_string))
}

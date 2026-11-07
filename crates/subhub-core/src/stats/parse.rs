//! Pure line parsers. Only usage, model, timestamp and session fields are declared, so serde skips
//! everything else (message text included) without keeping it.
use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::Tokens;

/// One assistant message of a Claude Code log.
#[derive(Debug, Clone, PartialEq)]
pub struct ClaudeMessage {
    /// Hash of message id + request id (streamed messages repeat); None when there is no id.
    pub key: Option<u64>,
    pub ts: DateTime<Utc>,
    pub session: Option<String>,
    pub model: String,
    pub tokens: Tokens,
}

#[derive(Deserialize)]
struct CLine {
    #[serde(rename = "type")]
    kind: Option<String>,
    timestamp: Option<String>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    #[serde(rename = "requestId")]
    request_id: Option<String>,
    message: Option<CMessage>,
}

#[derive(Deserialize)]
struct CMessage {
    id: Option<String>,
    model: Option<String>,
    usage: Option<CUsage>,
}

#[derive(Deserialize)]
struct CUsage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
}

/// FNV-1a 64: stable across Rust versions (the keys are persisted in the cache).
fn fnv(parts: &[&str]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for p in parts {
        for b in p.bytes().chain(std::iter::once(0xffu8)) {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    h
}

/// Parses one line of a Claude Code `projects/**/*.jsonl` file. Only `type == "assistant"` lines
/// with `message.usage` and a real model count; synthetic messages and all-zero usage are skipped.
pub fn parse_claude_line(line: &str) -> Option<ClaudeMessage> {
    let l: CLine = serde_json::from_str(line).ok()?;
    if l.kind.as_deref() != Some("assistant") {
        return None;
    }
    let m = l.message?;
    let u = m.usage?;
    let model = m.model.filter(|m| !m.is_empty() && !m.starts_with('<'))?;
    let tokens = Tokens {
        input: u.input_tokens.unwrap_or(0),
        cache_read: u.cache_read_input_tokens.unwrap_or(0),
        cache_write: u.cache_creation_input_tokens.unwrap_or(0),
        output: u.output_tokens.unwrap_or(0),
    };
    if tokens.is_zero() {
        return None;
    }
    let ts = DateTime::parse_from_rfc3339(&l.timestamp?)
        .ok()?
        .with_timezone(&Utc);
    let key =
        m.id.as_deref()
            .map(|id| fnv(&[id, l.request_id.as_deref().unwrap_or("")]));
    Some(ClaudeMessage {
        key,
        ts,
        session: l.session_id,
        model,
        tokens,
    })
}

/// Raw Codex usage counters (OpenAI semantics: `input` includes the cached part).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, Deserialize)]
pub struct RawUsage {
    pub input: u64,
    pub cached: u64,
    pub cache_write: u64,
    pub output: u64,
}

impl RawUsage {
    /// `self - prev`, or None when any counter went down (a new counting epoch).
    pub fn checked_sub(&self, prev: &RawUsage) -> Option<RawUsage> {
        Some(RawUsage {
            input: self.input.checked_sub(prev.input)?,
            cached: self.cached.checked_sub(prev.cached)?,
            cache_write: self.cache_write.checked_sub(prev.cache_write)?,
            output: self.output.checked_sub(prev.output)?,
        })
    }

    pub fn is_zero(&self) -> bool {
        self.input + self.output == 0
    }

    /// Normalizes to uncached input / cache read / cache write / output.
    pub fn tokens(&self) -> Tokens {
        Tokens {
            input: self
                .input
                .saturating_sub(self.cached)
                .saturating_sub(self.cache_write),
            cache_read: self.cached,
            cache_write: self.cache_write,
            output: self.output,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CodexEvent {
    /// A turn started with this model.
    Model(String),
    /// A token count: the last request's usage and the running total of the session.
    Usage {
        ts: DateTime<Utc>,
        last: Option<RawUsage>,
        total: Option<RawUsage>,
    },
}

#[derive(Deserialize)]
struct XLine {
    #[serde(rename = "type")]
    kind: Option<String>,
    timestamp: Option<String>,
    payload: Option<XPayload>,
}

#[derive(Deserialize)]
struct XPayload {
    #[serde(rename = "type")]
    kind: Option<String>,
    model: Option<String>,
    info: Option<XInfo>,
}

#[derive(Deserialize)]
struct XInfo {
    total_token_usage: Option<XUsage>,
    last_token_usage: Option<XUsage>,
}

#[derive(Deserialize)]
struct XUsage {
    input_tokens: Option<u64>,
    cached_input_tokens: Option<u64>,
    cache_write_input_tokens: Option<u64>,
    output_tokens: Option<u64>,
}

impl From<XUsage> for RawUsage {
    fn from(u: XUsage) -> Self {
        RawUsage {
            input: u.input_tokens.unwrap_or(0),
            cached: u.cached_input_tokens.unwrap_or(0),
            cache_write: u.cache_write_input_tokens.unwrap_or(0),
            output: u.output_tokens.unwrap_or(0),
        }
    }
}

/// Parses one line of a Codex `sessions/**/*.jsonl` file: `turn_context` (model) and
/// `event_msg` / `token_count` (usage). Everything else is ignored.
pub fn parse_codex_line(line: &str) -> Option<CodexEvent> {
    let l: XLine = serde_json::from_str(line).ok()?;
    let p = l.payload?;
    match l.kind.as_deref()? {
        "turn_context" => p.model.filter(|m| !m.is_empty()).map(CodexEvent::Model),
        "event_msg" if p.kind.as_deref() == Some("token_count") => {
            let info = p.info?;
            let ts = DateTime::parse_from_rfc3339(&l.timestamp?)
                .ok()?
                .with_timezone(&Utc);
            Some(CodexEvent::Usage {
                ts,
                last: info.last_token_usage.map(Into::into),
                total: info.total_token_usage.map(Into::into),
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_assistant_line_with_usage() {
        let line = r#"{"type":"assistant","timestamp":"2026-11-05T10:00:00.000Z","sessionId":"s1","requestId":"r1","message":{"id":"m1","model":"claude-opus-5-5","content":[{"type":"text","text":"SECRET"}],"usage":{"input_tokens":3,"output_tokens":50,"cache_creation_input_tokens":100,"cache_read_input_tokens":2000}}}"#;
        let m = parse_claude_line(line).unwrap();
        assert_eq!(m.model, "claude-opus-5-5");
        assert_eq!(m.session.as_deref(), Some("s1"));
        assert_eq!(
            m.tokens,
            Tokens {
                input: 3,
                cache_read: 2000,
                cache_write: 100,
                output: 50
            }
        );
        assert!(m.key.is_some());
        // Same ids -> same key; different request -> different key.
        let again = parse_claude_line(line).unwrap();
        assert_eq!(m.key, again.key);
        let other = parse_claude_line(&line.replace("r1", "r2")).unwrap();
        assert_ne!(m.key, other.key);
    }

    #[test]
    fn claude_skips_other_types_synthetic_and_empty_usage() {
        let user = r#"{"type":"user","timestamp":"2026-11-05T10:00:00Z","message":{"role":"user","content":"hi"}}"#;
        assert!(parse_claude_line(user).is_none());
        let synthetic = r#"{"type":"assistant","timestamp":"2026-11-05T10:00:00Z","message":{"id":"m","model":"<synthetic>","usage":{"input_tokens":5,"output_tokens":5}}}"#;
        assert!(parse_claude_line(synthetic).is_none());
        let zero = r#"{"type":"assistant","timestamp":"2026-11-05T10:00:00Z","message":{"id":"m","model":"claude-opus-5-5","usage":{"input_tokens":0,"output_tokens":0,"cache_read_input_tokens":null}}}"#;
        assert!(parse_claude_line(zero).is_none());
        assert!(parse_claude_line("not json").is_none());
    }

    #[test]
    fn codex_model_and_usage_events() {
        let turn = r#"{"timestamp":"2026-11-05T10:00:00Z","type":"turn_context","payload":{"turn_id":"t","model":"gpt-6.1-sol","cwd":"C:\\x"}}"#;
        assert_eq!(
            parse_codex_line(turn),
            Some(CodexEvent::Model("gpt-6.1-sol".into()))
        );
        let tc = r#"{"timestamp":"2026-11-05T10:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":40,"cache_write_input_tokens":0,"output_tokens":10,"reasoning_output_tokens":4,"total_tokens":110},"last_token_usage":{"input_tokens":100,"cached_input_tokens":40,"cache_write_input_tokens":0,"output_tokens":10,"reasoning_output_tokens":4,"total_tokens":110}},"rate_limits":null}}"#;
        let Some(CodexEvent::Usage { last, total, .. }) = parse_codex_line(tc) else {
            panic!("usage expected");
        };
        let last = last.unwrap();
        assert_eq!(total, Some(last));
        assert_eq!(
            last.tokens(),
            Tokens {
                input: 60,
                cache_read: 40,
                cache_write: 0,
                output: 10
            }
        );
        assert_eq!(last.tokens().processed(), 110);
        // Rate-limit-only token_count (info null) and unrelated lines yield nothing.
        let none = r#"{"timestamp":"2026-11-05T10:00:01Z","type":"event_msg","payload":{"type":"token_count","info":null}}"#;
        assert!(parse_codex_line(none).is_none());
        let msg = r#"{"timestamp":"2026-11-05T10:00:01Z","type":"response_item","payload":{"type":"message","content":"token_count"}}"#;
        assert!(parse_codex_line(msg).is_none());
    }

    #[test]
    fn raw_usage_sub_detects_resets() {
        let a = RawUsage {
            input: 10,
            cached: 5,
            cache_write: 0,
            output: 2,
        };
        let b = RawUsage {
            input: 25,
            cached: 9,
            cache_write: 0,
            output: 3,
        };
        assert_eq!(b.checked_sub(&a).unwrap().input, 15);
        assert!(a.checked_sub(&b).is_none());
    }
}

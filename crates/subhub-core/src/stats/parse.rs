//! Pure line parsers. Only usage, model, timestamp, session, `isSidechain`, `cwd`, tool names and
//! thread title fields are declared, so serde skips everything else (message text included)
//! without keeping it. Of a shell tool's input only the command's first word survives
//! ([`super::kind::command_word`]; scan v2, owner 2026-11-07).
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer};

use super::Tokens;
use super::kind::{ToolCall, command_word, is_shell_tool};

/// One assistant message line of a Claude Code log. Claude Code writes one line per content
/// block (thinking, text, each tool use), all with the same ids and usage; the scan merges their
/// `tools` into one step.
#[derive(Debug, Clone, PartialEq)]
pub struct ClaudeMessage {
    /// Hash of message id + request id (streamed messages repeat); None when there is no id.
    pub key: Option<u64>,
    pub ts: DateTime<Utc>,
    pub session: Option<String>,
    pub model: String,
    pub tokens: Tokens,
    /// A subagent's step (`isSidechain`).
    pub sidechain: bool,
    /// Working folder of the step.
    pub cwd: Option<String>,
    /// Tool uses in this line's content blocks.
    pub tools: Vec<ToolCall>,
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
    #[serde(rename = "isSidechain", default)]
    sidechain: Option<bool>,
    cwd: Option<String>,
    message: Option<CMessage>,
}

#[derive(Deserialize)]
struct CMessage {
    id: Option<String>,
    model: Option<String>,
    usage: Option<CUsage>,
    #[serde(default, deserialize_with = "blocks")]
    content: Vec<CBlock>,
}

/// A content block: only its type, a tool's name and its `input.command`.
#[derive(Deserialize)]
struct CBlock {
    #[serde(rename = "type")]
    kind: Option<String>,
    name: Option<String>,
    input: Option<CInput>,
}

#[derive(Deserialize)]
struct CInput {
    command: Option<serde_json::Value>,
}

/// `content` is a list of blocks for assistant lines; anything else (a plain string) gives none.
fn blocks<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<CBlock>, D::Error> {
    struct V;
    impl<'de> serde::de::Visitor<'de> for V {
        type Value = Vec<CBlock>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("content")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> Result<Self::Value, A::Error> {
            let mut out = vec![];
            while let Some(b) = seq.next_element::<Option<CBlock>>()? {
                out.extend(b);
            }
            Ok(out)
        }
        fn visit_str<E>(self, _: &str) -> Result<Self::Value, E> {
            Ok(vec![])
        }
        fn visit_unit<E>(self) -> Result<Self::Value, E> {
            Ok(vec![])
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut map: A,
        ) -> Result<Self::Value, A::Error> {
            while map
                .next_entry::<serde::de::IgnoredAny, serde::de::IgnoredAny>()?
                .is_some()
            {}
            Ok(vec![])
        }
    }
    d.deserialize_any(V)
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

/// The first word of a shell `command` given as a string or as an argv list (`["bash", "-lc",
/// "cargo test"]`: a shell wrapper's script is used).
fn shell_word(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => command_word(s),
        serde_json::Value::Array(a) => {
            let parts: Vec<&str> = a.iter().filter_map(|x| x.as_str()).collect();
            let first = command_word(parts.first()?)?;
            let wrapper = matches!(
                first.as_str(),
                "bash" | "sh" | "zsh" | "pwsh" | "powershell" | "cmd"
            );
            match parts.last() {
                Some(script) if wrapper && parts.len() > 1 => command_word(script),
                _ => Some(first),
            }
        }
        _ => None,
    }
}

fn tool_call(name: &str, command: Option<&serde_json::Value>) -> ToolCall {
    let word = if is_shell_tool(name) {
        command.and_then(shell_word)
    } else {
        None
    };
    ToolCall::new(name, word)
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
    let tools = m
        .content
        .iter()
        .filter(|b| b.kind.as_deref() == Some("tool_use"))
        .filter_map(|b| {
            let name = b.name.as_deref().filter(|n| !n.is_empty())?;
            Some(tool_call(
                name,
                b.input.as_ref().and_then(|i| i.command.as_ref()),
            ))
        })
        .collect();
    Some(ClaudeMessage {
        key,
        ts,
        session: l.session_id,
        model,
        tokens,
        sidechain: l.sidechain.unwrap_or(false),
        cwd: l.cwd.filter(|c| !c.is_empty()),
        tools,
    })
}

#[derive(Deserialize)]
struct TitleLine {
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(rename = "customTitle")]
    custom_title: Option<String>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
}

/// A Claude Code `custom-title` line (the thread name the user gave a session): session id and
/// title. Blank titles give None.
pub fn parse_claude_title(line: &str) -> Option<(String, String)> {
    let l: TitleLine = serde_json::from_str(line).ok()?;
    if l.kind.as_deref().is_some_and(|k| k != "custom-title") {
        return None;
    }
    let title = l.custom_title?.trim().to_string();
    (!title.is_empty()).then_some((l.session_id?, title))
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
    /// A turn started: its model and working folder.
    Turn {
        model: Option<String>,
        cwd: Option<String>,
    },
    /// A tool call of the coming request (`function_call`, `custom_tool_call`,
    /// `local_shell_call`, `web_search_call`).
    Tool(ToolCall),
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
    cwd: Option<String>,
    info: Option<XInfo>,
    name: Option<String>,
    /// `function_call` arguments: a JSON string; only `command` / `cmd` is read from it.
    arguments: Option<String>,
    action: Option<XAction>,
}

#[derive(Deserialize)]
struct XAction {
    command: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct XArgs {
    command: Option<serde_json::Value>,
    cmd: Option<serde_json::Value>,
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

/// Parses one line of a Codex `sessions/**/*.jsonl` file: `turn_context` (model, cwd), tool calls
/// in `response_item`s and `event_msg` / `token_count` (usage). Everything else is ignored.
pub fn parse_codex_line(line: &str) -> Option<CodexEvent> {
    let l: XLine = serde_json::from_str(line).ok()?;
    let p = l.payload?;
    match l.kind.as_deref()? {
        "turn_context" => Some(CodexEvent::Turn {
            model: p.model.filter(|m| !m.is_empty()),
            cwd: p.cwd.filter(|c| !c.is_empty()),
        }),
        "response_item" => {
            let kind = p.kind.as_deref()?;
            let call = match kind {
                "function_call" | "custom_tool_call" => {
                    let name = p.name.as_deref().filter(|n| !n.is_empty())?;
                    let args = p
                        .arguments
                        .as_deref()
                        .filter(|_| is_shell_tool(name))
                        .and_then(|a| serde_json::from_str::<XArgs>(a).ok());
                    let command = args.and_then(|a| a.command.or(a.cmd));
                    tool_call(name, command.as_ref())
                }
                "local_shell_call" => tool_call(
                    "local_shell",
                    p.action.as_ref().and_then(|a| a.command.as_ref()),
                ),
                "web_search_call" => ToolCall::new("web_search", None),
                _ => return None,
            };
            Some(CodexEvent::Tool(call))
        }
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
        // No cwd, no sidechain flag, no tools: defaults.
        assert!(!m.sidechain && m.cwd.is_none() && m.tools.is_empty());
    }

    #[test]
    fn claude_tool_use_keeps_only_names_and_the_first_command_word() {
        let line = r#"{"type":"assistant","isSidechain":true,"cwd":"D:\\Github\\x","timestamp":"2026-11-05T10:00:00Z","sessionId":"s1","requestId":"r1","message":{"id":"m1","model":"claude-opus-5-5","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo test --secret-flag","description":"SECRET"}},{"type":"tool_use","name":"Edit","input":{"file_path":"C:\\private.rs","old_string":"SECRET","new_string":"SECRET"}},{"type":"thinking","thinking":"SECRET"}],"usage":{"input_tokens":1,"output_tokens":5}}}"#;
        let m = parse_claude_line(line).unwrap();
        assert!(m.sidechain);
        assert_eq!(m.cwd.as_deref(), Some(r"D:\Github\x"));
        assert_eq!(
            m.tools,
            vec![
                ToolCall::new("Bash", Some("cargo".into())),
                ToolCall::new("Edit", None)
            ]
        );
        assert!(!format!("{m:?}").contains("SECRET"));
        assert!(!format!("{m:?}").contains("secret-flag"));
        // A plain-string content is fine (no tools).
        let s = line.replace(
            r#""content":[{"type":"tool_use""#,
            r#""content":"x","c":[{"type":"tool_use""#,
        );
        assert!(parse_claude_line(&s).unwrap().tools.is_empty());
    }

    #[test]
    fn claude_titles() {
        let t = r#"{"type":"custom-title","customTitle":"Fix login","sessionId":"s1"}"#;
        assert_eq!(
            parse_claude_title(t),
            Some(("s1".to_string(), "Fix login".to_string()))
        );
        assert_eq!(
            parse_claude_title(r#"{"type":"custom-title","customTitle":"  ","sessionId":"s1"}"#),
            None
        );
        assert_eq!(
            parse_claude_title(r#"{"type":"user","customTitle":"x","sessionId":"s1"}"#),
            None
        );
        assert_eq!(parse_claude_title("nope"), None);
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
            Some(CodexEvent::Turn {
                model: Some("gpt-6.1-sol".into()),
                cwd: Some(r"C:\x".into())
            })
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
    fn codex_tool_calls_keep_only_names_and_the_first_command_word() {
        let tool = |l: &str| match parse_codex_line(l) {
            Some(CodexEvent::Tool(t)) => t,
            other => panic!("{other:?}"),
        };
        let exec = r#"{"type":"response_item","payload":{"type":"function_call","name":"exec_command","arguments":"{\"cmd\":\"git status --secret\",\"workdir\":\"C:\\\\x\"}","call_id":"c"}}"#;
        assert_eq!(
            tool(exec),
            ToolCall::new("exec_command", Some("git".into()))
        );
        let shell = r#"{"type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{\"command\":[\"bash\",\"-lc\",\"cargo build\"]}"}}"#;
        assert_eq!(tool(shell), ToolCall::new("shell", Some("cargo".into())));
        let argv = r#"{"type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{\"command\":[\"rg\",\"SECRET\"]}"}}"#;
        assert_eq!(tool(argv), ToolCall::new("shell", Some("rg".into())));
        let patch = r#"{"type":"response_item","payload":{"type":"custom_tool_call","name":"apply_patch","input":"*** SECRET"}}"#;
        assert_eq!(tool(patch), ToolCall::new("apply_patch", None));
        let other = r#"{"type":"response_item","payload":{"type":"function_call","name":"update_plan","arguments":"{\"command\":\"SECRET\"}"}}"#;
        assert_eq!(tool(other), ToolCall::new("update_plan", None));
        let local = r#"{"type":"response_item","payload":{"type":"local_shell_call","action":{"type":"exec","command":["npm","test"]}}}"#;
        assert_eq!(
            tool(local),
            ToolCall::new("local_shell", Some("npm".into()))
        );
        let web = r#"{"type":"response_item","payload":{"type":"web_search_call","action":{"query":"SECRET"}}}"#;
        assert_eq!(tool(web), ToolCall::new("web_search", None));
        let bad = r#"{"type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"not json"}}"#;
        assert_eq!(tool(bad), ToolCall::new("shell", None));
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

//! Task kinds: what an assistant step spent its tokens on, from the tools it called (plan
//! `docs/features/overview.md`, "Task kinds", owner 2026-11-07). Pure and table-driven.
//!
//! Privacy: for shell tools only the command's first word is ever looked at or kept
//! ([`command_word`]); tool inputs, prompts and message text never leave the parser.
use serde::{Deserialize, Serialize};

/// Kind of work of one assistant step (or the share of a step that one tool call stands for).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TaskKind {
    WritingCode,
    BuildsTests,
    GitPullRequests,
    ReadingSearching,
    Agents,
    WebResearch,
    DesignPreviews,
    OtherTools,
    /// Steps without a tool call: answering, explaining, planning.
    Talking,
}

impl TaskKind {
    pub const ALL: [TaskKind; 9] = [
        TaskKind::WritingCode,
        TaskKind::BuildsTests,
        TaskKind::GitPullRequests,
        TaskKind::ReadingSearching,
        TaskKind::Agents,
        TaskKind::WebResearch,
        TaskKind::DesignPreviews,
        TaskKind::OtherTools,
        TaskKind::Talking,
    ];

    /// Row label in the Overview ("Where usage goes").
    pub fn label(self) -> &'static str {
        match self {
            TaskKind::WritingCode => "Writing code",
            TaskKind::BuildsTests => "Builds & tests",
            TaskKind::GitPullRequests => "Git & pull requests",
            TaskKind::ReadingSearching => "Reading & searching",
            TaskKind::Agents => "Running agents & threads",
            TaskKind::WebResearch => "Web research",
            TaskKind::DesignPreviews => "Design previews",
            TaskKind::OtherTools => "Other tools",
            TaskKind::Talking => "Talking & planning",
        }
    }
}

/// One tool call of a step: the tool name and, for shell tools, the command's first word.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    pub name: String,
    /// Shell tools only: [`command_word`] of the command. Nothing else of the input is kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub word: Option<String>,
}

impl ToolCall {
    pub fn new(name: &str, word: Option<String>) -> Self {
        Self {
            name: name.to_string(),
            word,
        }
    }
}

/// Tools whose input is a shell command (Claude Code `Bash` / `PowerShell`, Codex `shell`,
/// `shell_command`, `exec_command`, `local_shell`, `container.exec`).
pub fn is_shell_tool(name: &str) -> bool {
    matches!(
        base_name(name).to_ascii_lowercase().as_str(),
        "bash"
            | "powershell"
            | "shell"
            | "shell_command"
            | "exec_command"
            | "local_shell"
            | "local_shell_call"
            | "container.exec"
    )
}

/// The tool's own name without an MCP prefix (`mcp__server__name` -> `name`).
fn base_name(name: &str) -> &str {
    match name.strip_prefix("mcp__") {
        Some(rest) => rest.rsplit_once("__").map_or(rest, |(_, n)| n),
        None => name,
    }
}

const WRITE_TOOLS: &[&str] = &[
    "edit",
    "write",
    "multiedit",
    "notebookedit",
    "apply_patch",
    "str_replace_based_edit_tool",
];
const READ_TOOLS: &[&str] = &[
    "read",
    "grep",
    "glob",
    "ls",
    "notebookread",
    "view_image",
    "read_file",
    "list_dir",
    "grep_files",
];
const AGENT_TOOLS: &[&str] = &[
    "agent",
    "task",
    "sendmessage",
    "taskoutput",
    "taskstop",
    "listagents",
    "spawn_agent",
    "send_input",
    "wait_agent",
    "close_agent",
];
const WEB_TOOLS: &[&str] = &["websearch", "webfetch", "web_search", "web_fetch"];

/// First words of build and test tools (owner table, extended with the usual suspects).
const BUILD_WORDS: &[&str] = &[
    "cargo",
    "rustc",
    "rustup",
    "npm",
    "npx",
    "pnpm",
    "yarn",
    "bun",
    "deno",
    "node",
    "tsc",
    "vitest",
    "jest",
    "dotnet",
    "msbuild",
    "nuget",
    "pytest",
    "tox",
    "uv",
    "pip",
    "go",
    "make",
    "cmake",
    "ninja",
    "gradle",
    "gradlew",
    "mvn",
    "xcodebuild",
    "swift",
    "flutter",
    "dart",
];
const GIT_WORDS: &[&str] = &["git", "gh"];
/// Shell commands that only read.
const READ_WORDS: &[&str] = &[
    "cat",
    "type",
    "ls",
    "dir",
    "rg",
    "grep",
    "find",
    "fd",
    "head",
    "tail",
    "less",
    "more",
    "wc",
    "tree",
    "sed",
    "awk",
    "jq",
    "stat",
    "file",
    "du",
    "pwd",
    "which",
    "where",
    "get-content",
    "gc",
    "get-childitem",
    "gci",
    "select-string",
    "sls",
    "test-path",
    "get-item",
    "resolve-path",
    "get-command",
    "measure-object",
    "format-hex",
];

/// Kind of one tool call (table in the plan). Shell tools are classified by their first word.
pub fn classify_tool(call: &ToolCall) -> TaskKind {
    let name = base_name(&call.name).to_ascii_lowercase();
    if is_shell_tool(&call.name) {
        return match call.word.as_deref() {
            Some(w) if BUILD_WORDS.contains(&w) => TaskKind::BuildsTests,
            Some(w) if GIT_WORDS.contains(&w) => TaskKind::GitPullRequests,
            Some(w) if READ_WORDS.contains(&w) => TaskKind::ReadingSearching,
            _ => TaskKind::OtherTools,
        };
    }
    let n = name.as_str();
    if WRITE_TOOLS.contains(&n) {
        TaskKind::WritingCode
    } else if READ_TOOLS.contains(&n) {
        TaskKind::ReadingSearching
    } else if n.contains("pull_request") || n.starts_with("gh_") {
        TaskKind::GitPullRequests
    } else if AGENT_TOOLS.contains(&n) || n.contains("delegate") || n.contains("thread") {
        TaskKind::Agents
    } else if WEB_TOOLS.contains(&n) || n.starts_with("web_search") {
        TaskKind::WebResearch
    } else if n.contains("preview") || n.contains("render") || n.contains("screenshot") {
        TaskKind::DesignPreviews
    } else {
        TaskKind::OtherTools
    }
}

/// How many of a step's tool calls fall into each kind (sorted by kind); a step without tools is
/// one share of Talking. The step's value is split evenly over these shares.
pub fn step_kinds(tools: &[ToolCall]) -> Vec<(TaskKind, u32)> {
    if tools.is_empty() {
        return vec![(TaskKind::Talking, 1)];
    }
    let mut counts: std::collections::BTreeMap<TaskKind, u32> = Default::default();
    for t in tools {
        *counts.entry(classify_tool(t)).or_default() += 1;
    }
    counts.into_iter().collect()
}

/// Words that only change into a folder before the real command (`cd x && cargo test`).
const CD_WORDS: &[&str] = &[
    "cd",
    "chdir",
    "pushd",
    "set-location",
    "sl",
    "push-location",
];
/// Prefixes that run the next word (`sudo`, PowerShell's call operator `&`, ...).
const RUNNER_WORDS: &[&str] = &["&", "sudo", "time", "env", "exec", "nohup", "command"];

/// The first word of a shell command, lowercased, without folder and extension
/// (`C:\x\Cargo.EXE build` -> `cargo`, `Get-Content x` -> `get-content`). A leading
/// `cd <dir> &&` / `;`, environment assignments (`FOO=1`, `$env:X='y';`) and runner prefixes
/// (`sudo`, `&`) are skipped so the word names the tool that actually ran. Only this word is ever
/// kept; None for an empty command.
pub fn command_word(command: &str) -> Option<String> {
    let mut rest = command.trim_start();
    loop {
        let (word, after) = split_word(rest)?;
        let lower = word.to_ascii_lowercase();
        if lower.starts_with("$env:") || (is_assignment(word) && !after.is_empty()) {
            rest = skip_separator(after);
            continue;
        }
        if RUNNER_WORDS.contains(&lower.as_str()) && !after.trim().is_empty() {
            rest = after.trim_start();
            continue;
        }
        if CD_WORDS.contains(&lower.as_str()) {
            // Skip to after the next `&&`, `;` or `||`; a bare `cd x` is the command itself.
            match next_command(after) {
                Some(next) => {
                    rest = next;
                    continue;
                }
                None => return Some(clean(word)),
            }
        }
        let w = clean(word);
        return (!w.is_empty()).then_some(w);
    }
}

/// The first word (quotes kept together) and the rest.
fn split_word(s: &str) -> Option<(&str, &str)> {
    let s = s.trim_start();
    if s.is_empty() {
        return None;
    }
    let bytes = s.as_bytes();
    let end = if bytes[0] == b'"' || bytes[0] == b'\'' {
        let q = bytes[0];
        s[1..].find(q as char).map_or(s.len(), |i| i + 2)
    } else {
        s.find(|c: char| c.is_whitespace() || c == ';')
            .unwrap_or(s.len())
            .max(1)
    };
    Some((&s[..end], &s[end..]))
}

fn is_assignment(word: &str) -> bool {
    match word.split_once('=') {
        Some((k, _)) => !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
        None => false,
    }
}

/// After an assignment: drop a following `;` (PowerShell `$env:X='y'; cargo`).
fn skip_separator(s: &str) -> &str {
    let s = s.trim_start();
    s.strip_prefix(';').unwrap_or(s).trim_start()
}

fn next_command(s: &str) -> Option<&str> {
    let i = ["&&", "||", ";"]
        .iter()
        .filter_map(|sep| s.find(sep).map(|i| (i, sep.len())))
        .min()?;
    let next = s[i.0 + i.1..].trim_start();
    (!next.is_empty()).then_some(next)
}

/// Lowercase, without quotes, folders and a program extension.
fn clean(word: &str) -> String {
    let w = word.trim_matches(|c| c == '"' || c == '\'' || c == '(' || c == ')');
    let w = w
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(w)
        .to_ascii_lowercase();
    for ext in [".exe", ".cmd", ".bat", ".ps1", ".sh", ".com"] {
        if let Some(stem) = w.strip_suffix(ext)
            && !stem.is_empty()
        {
            return stem.to_string();
        }
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(name: &str, word: Option<&str>) -> TaskKind {
        classify_tool(&ToolCall::new(name, word.map(String::from)))
    }

    #[test]
    fn first_word_is_lowercased_without_path_and_extension() {
        for (cmd, word) in [
            ("cargo test --workspace", Some("cargo")),
            (r"C:\Users\x\.cargo\bin\Cargo.EXE build", Some("cargo")),
            ("Get-Content -Path foo.txt", Some("get-content")),
            (r#""C:\Program Files\Git\bin\git.exe" status"#, Some("git")),
            ("cd /d/Github/x && cargo clippy", Some("cargo")),
            ("cd D:\\x; git status", Some("git")),
            ("Set-Location D:\\x; npm run build", Some("npm")),
            ("cd somewhere", Some("cd")),
            ("RUST_LOG=debug cargo run", Some("cargo")),
            ("$env:CARGO_TARGET_DIR='x'; cargo test", Some("cargo")),
            ("& 'C:\\tools\\dotnet.exe' build", Some("dotnet")),
            ("sudo make install", Some("make")),
            ("./scripts/release.ps1 -Version 1", Some("release")),
            ("ls;pwd", Some("ls")),
            ("   ", None),
            ("", None),
        ] {
            assert_eq!(command_word(cmd).as_deref(), word, "{cmd}");
        }
    }

    #[test]
    fn tools_map_to_the_table() {
        assert_eq!(kind("Edit", None), TaskKind::WritingCode);
        assert_eq!(kind("Write", None), TaskKind::WritingCode);
        assert_eq!(kind("MultiEdit", None), TaskKind::WritingCode);
        assert_eq!(kind("NotebookEdit", None), TaskKind::WritingCode);
        assert_eq!(kind("apply_patch", None), TaskKind::WritingCode);
        assert_eq!(kind("Bash", Some("cargo")), TaskKind::BuildsTests);
        assert_eq!(kind("PowerShell", Some("dotnet")), TaskKind::BuildsTests);
        assert_eq!(kind("shell", Some("pytest")), TaskKind::BuildsTests);
        assert_eq!(kind("exec_command", Some("pnpm")), TaskKind::BuildsTests);
        assert_eq!(kind("Bash", Some("git")), TaskKind::GitPullRequests);
        assert_eq!(kind("shell_command", Some("gh")), TaskKind::GitPullRequests);
        assert_eq!(
            kind("mcp__github__create_pull_request", None),
            TaskKind::GitPullRequests
        );
        assert_eq!(kind("Read", None), TaskKind::ReadingSearching);
        assert_eq!(kind("Grep", None), TaskKind::ReadingSearching);
        assert_eq!(kind("Glob", None), TaskKind::ReadingSearching);
        assert_eq!(kind("LS", None), TaskKind::ReadingSearching);
        assert_eq!(kind("Bash", Some("rg")), TaskKind::ReadingSearching);
        assert_eq!(
            kind("PowerShell", Some("get-content")),
            TaskKind::ReadingSearching
        );
        assert_eq!(kind("Agent", None), TaskKind::Agents);
        assert_eq!(kind("Task", None), TaskKind::Agents);
        assert_eq!(kind("SendMessage", None), TaskKind::Agents);
        assert_eq!(kind("mcp__t3-code__delegate_task", None), TaskKind::Agents);
        assert_eq!(
            kind("mcp__t3-code__t3_thread_launch", None),
            TaskKind::Agents
        );
        assert_eq!(kind("WebSearch", None), TaskKind::WebResearch);
        assert_eq!(kind("WebFetch", None), TaskKind::WebResearch);
        assert_eq!(kind("web_search", None), TaskKind::WebResearch);
        assert_eq!(
            kind("mcp__t3-code__preview_snapshot", None),
            TaskKind::DesignPreviews
        );
        assert_eq!(
            kind("mcp__vectorcraft__render_preview", None),
            TaskKind::DesignPreviews
        );
        assert_eq!(
            kind("mcp__x__ui_screenshot", None),
            TaskKind::DesignPreviews
        );
        assert_eq!(kind("TodoWrite", None), TaskKind::OtherTools);
        assert_eq!(kind("update_plan", None), TaskKind::OtherTools);
        // A shell tool with an unknown, missing or non-reading command is Other.
        assert_eq!(kind("Bash", Some("rm")), TaskKind::OtherTools);
        assert_eq!(kind("Bash", None), TaskKind::OtherTools);
    }

    #[test]
    fn steps_split_into_kind_shares() {
        assert_eq!(step_kinds(&[]), vec![(TaskKind::Talking, 1)]);
        let tools = [
            ToolCall::new("Edit", None),
            ToolCall::new("Bash", Some("cargo".into())),
            ToolCall::new("Write", None),
        ];
        assert_eq!(
            step_kinds(&tools),
            vec![(TaskKind::WritingCode, 2), (TaskKind::BuildsTests, 1)]
        );
        assert!(TaskKind::ALL.iter().all(|k| !k.label().is_empty()));
    }
}

//! Sign-in launch (port of the C# `LaunchService` / `LoginCompletion`). SubHub only starts the
//! provider CLI's own login in a terminal for the account's profile; it never reads, stores or
//! passes a token. Everything here is pure command building except `LaunchPlan::spawn` and the
//! completion file.
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::codex;
use crate::model::ProviderKind;
use crate::settings::Terminal;

/// Environment variables that would make the CLI bill tokens or use another login instead of the
/// subscription profile. Cleared for every launch (same list as the C# app).
pub const OVERRIDE_VARS: [&str; 11] = [
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
    "OPENAI_API_KEY",
    "OPENAI_BASE_URL",
    "CODEX_API_KEY",
    "CLAUDE_SECURESTORAGE_CONFIG_DIR",
];

/// The variable that points the CLI at the profile folder.
pub fn profile_env_var(provider: ProviderKind) -> &'static str {
    match provider {
        ProviderKind::Claude => "CLAUDE_CONFIG_DIR",
        ProviderKind::Codex => "CODEX_HOME",
        ProviderKind::Copilot => "COPILOT_HOME",
        ProviderKind::Cursor => "CURSOR_CONFIG_DIR",
        ProviderKind::Antigravity => "GEMINI_CLI_HOME",
    }
}

/// How to sign in an app login (Copilot, Cursor, Antigravity): SubHub does not open those.
pub fn app_sign_in_hint(provider: ProviderKind) -> &'static str {
    match provider {
        ProviderKind::Copilot => {
            "Sign in with the Copilot CLI (`copilot`, then /login) or `gh auth login`; SubHub reads that login."
        }
        ProviderKind::Cursor => {
            "Sign in with the Cursor CLI (`cursor-agent login`); SubHub reads Cursor through that login."
        }
        ProviderKind::Antigravity => {
            "Sign in once in Antigravity's CLI (run `agy`); SubHub asks it for usage."
        }
        _ => "",
    }
}

/// The CLI's login subcommand.
pub fn login_args(provider: ProviderKind) -> &'static [&'static str] {
    match provider {
        ProviderKind::Claude => &["auth", "login", "--claudeai"],
        ProviderKind::Codex => &["login"],
        // App logins sign in through their own app or CLI (the read says how).
        _ => &[],
    }
}

fn cli_name(provider: ProviderKind) -> &'static str {
    match provider {
        ProviderKind::Claude => "claude",
        ProviderKind::Codex => "codex",
        ProviderKind::Copilot => "copilot",
        ProviderKind::Cursor => "cursor-agent",
        ProviderKind::Antigravity => "agy",
    }
}

/// A PowerShell single-quoted literal: data, never code.
pub fn ps_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn ps_path(p: &Path) -> String {
    ps_quote(&p.to_string_lossy())
}

/// Standard base64.
fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// `powershell -EncodedCommand` value: base64 of the UTF-16LE script, so no quoting by the
/// terminal or command line can alter it.
pub fn encode_command(script: &str) -> String {
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64(&bytes)
}

/// Script prefix: drops the override variables and points the CLI at the profile.
pub fn env_script(provider: ProviderKind, profile_dir: &Path) -> String {
    let mut s = String::new();
    for key in OVERRIDE_VARS {
        s.push_str(&format!(
            "Remove-Item -LiteralPath 'Env:{key}' -ErrorAction SilentlyContinue; "
        ));
    }
    s.push_str(&format!(
        "$env:{} = {}; ",
        profile_env_var(provider),
        ps_path(profile_dir)
    ));
    s
}

/// The whole login script.
pub fn login_script(
    provider: ProviderKind,
    profile_dir: &Path,
    exe: &Path,
    cwd: &Path,
    completion: Option<&Path>,
) -> String {
    cli_script(
        provider,
        profile_dir,
        exe,
        cwd,
        login_args(provider),
        completion,
    )
}

/// The script that runs a provider CLI for a profile: drops the override variables, points the
/// CLI at the profile, goes to `cwd` and runs it with `args`. With `completion`, the CLI's exit
/// code is written there when it ends (the terminal may stay open or be detached from its launcher).
pub fn cli_script(
    provider: ProviderKind,
    profile_dir: &Path,
    exe: &Path,
    cwd: &Path,
    args: &[&str],
    completion: Option<&Path>,
) -> String {
    let args: String = args.iter().map(|a| format!(" {}", ps_quote(a))).collect();
    let mut s = env_script(provider, profile_dir);
    s.push_str(&format!(
        "Set-Location -LiteralPath {}; $global:LASTEXITCODE = 1; & {}{args}",
        ps_path(cwd),
        ps_path(exe)
    ));
    if let Some(file) = completion {
        s.push_str(&format!(
            "; [System.IO.File]::WriteAllText({}, [string]$LASTEXITCODE)",
            ps_path(file)
        ));
    }
    s
}

/// A process to start. Built purely so it can be tested; `spawn` only runs it.
#[derive(Debug, Clone, PartialEq)]
pub struct LaunchPlan {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env_remove: Vec<&'static str>,
    /// The profile variable (`CLAUDE_CONFIG_DIR` / `CODEX_HOME`); none for non-CLI launches.
    pub env_set: Option<(&'static str, PathBuf)>,
    pub cwd: Option<PathBuf>,
    /// No console window (background helpers such as the VS Code opener).
    pub hidden: bool,
}

/// Login in the chosen terminal. `windows_terminal` is the `wt` executable when found; without it
/// (or with `Terminal::PowerShell`) a PowerShell window is used.
pub fn plan_login(
    provider: ProviderKind,
    profile_dir: &Path,
    exe: &Path,
    cwd: &Path,
    terminal: Terminal,
    windows_terminal: Option<&Path>,
    completion: Option<&Path>,
) -> LaunchPlan {
    let script = login_script(provider, profile_dir, exe, cwd, completion);
    terminal_plan(
        &script,
        provider,
        profile_dir,
        cwd,
        terminal,
        windows_terminal,
    )
}

/// `powershell.exe` (or `wt new-tab powershell.exe`) running `script` in a new window.
fn terminal_plan(
    script: &str,
    provider: ProviderKind,
    profile_dir: &Path,
    cwd: &Path,
    terminal: Terminal,
    windows_terminal: Option<&Path>,
) -> LaunchPlan {
    let wt = (terminal == Terminal::WindowsTerminal)
        .then_some(windows_terminal)
        .flatten();
    let mut args: Vec<String> = vec![];
    if wt.is_some() {
        args.push("new-tab".into());
        args.push("powershell.exe".into());
    }
    for a in ["-NoLogo", "-NoProfile", "-NoExit", "-EncodedCommand"] {
        args.push(a.into());
    }
    args.push(encode_command(script));
    LaunchPlan {
        program: wt.map_or_else(|| PathBuf::from("powershell.exe"), Path::to_path_buf),
        args,
        env_remove: OVERRIDE_VARS.to_vec(),
        env_set: Some((profile_env_var(provider), profile_dir.to_path_buf())),
        cwd: Some(cwd.to_path_buf()),
        hidden: false,
    }
}

impl LaunchPlan {
    /// Starts the process and lets it go. The child's environment is also set in the script, as
    /// a running Windows Terminal hands new tabs its own environment instead of ours.
    pub fn spawn(&self) -> std::io::Result<()> {
        let mut cmd = std::process::Command::new(&self.program);
        cmd.args(&self.args);
        if let Some(cwd) = &self.cwd {
            cmd.current_dir(cwd);
        }
        for key in &self.env_remove {
            cmd.env_remove(key);
        }
        if let Some((key, value)) = &self.env_set {
            cmd.env(key, value);
        }
        #[cfg(windows)]
        if self.hidden {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        cmd.spawn().map(drop)
    }
}

// ---------- projects and the Launch menu ----------

/// The provider CLI in a project folder, in an explicitly chosen terminal (this overrides the
/// saved sign-in terminal preference). Same environment isolation as the login: the override
/// variables are cleared and the profile variable points at the account's profile folder.
pub fn plan_project_cli(
    provider: ProviderKind,
    profile_dir: &Path,
    exe: &Path,
    project_dir: &Path,
    terminal: Terminal,
    windows_terminal: Option<&Path>,
) -> LaunchPlan {
    let script = cli_script(provider, profile_dir, exe, project_dir, &[], None);
    terminal_plan(
        &script,
        provider,
        profile_dir,
        project_dir,
        terminal,
        windows_terminal,
    )
}

/// VS Code on the folder in a new window: `code --new-window -- <dir>` through a hidden
/// PowerShell with literal quoting (the C# app's way; works for `code.cmd`).
pub fn plan_vscode(code_exe: &Path, project_dir: &Path) -> LaunchPlan {
    let script = format!(
        "& {} '--new-window' '--' {}",
        ps_path(code_exe),
        ps_path(project_dir)
    );
    LaunchPlan {
        program: PathBuf::from("powershell.exe"),
        args: ["-NoLogo", "-NoProfile", "-EncodedCommand"]
            .iter()
            .map(|a| (*a).to_string())
            .chain([encode_command(&script)])
            .collect(),
        env_remove: vec![],
        env_set: None,
        cwd: None,
        hidden: true,
    }
}

/// Percent-encoding of everything but the unreserved characters (the same set as .NET's
/// `Uri.EscapeDataString`), on the UTF-8 bytes.
pub fn escape_data_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() * 2);
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The folder as a full path, without the verbatim `\\?\` prefix.
fn full_path(dir: &Path) -> String {
    let p = std::fs::canonicalize(dir).unwrap_or_else(|_| {
        if dir.is_absolute() {
            dir.to_path_buf()
        } else {
            std::env::current_dir().map_or_else(|_| dir.to_path_buf(), |c| c.join(dir))
        }
    });
    let s = p.to_string_lossy().into_owned();
    match s.strip_prefix(r"\\?\") {
        Some(rest) => rest.to_string(),
        None => s,
    }
}

/// Claude Desktop's documented link that opens a local folder in its Code view
/// (`claude://code/new?folder=<escaped full path>`); Claude asks the user to confirm.
pub fn claude_desktop_uri(project_dir: &Path) -> String {
    format!(
        "claude://code/new?folder={}",
        escape_data_string(&full_path(project_dir))
    )
}

/// Starts an installed desktop app by its Start-menu id (`Family!App`) through Explorer, or
/// by its .exe path.
pub fn plan_desktop_app(app_id: &str) -> LaunchPlan {
    let (program, args) = if app_id.contains('!') && !Path::new(app_id).is_absolute() {
        (
            PathBuf::from("explorer.exe"),
            vec![format!("shell:AppsFolder\\{app_id}")],
        )
    } else {
        (PathBuf::from(app_id), vec![])
    };
    LaunchPlan {
        program,
        args,
        env_remove: vec![],
        env_set: None,
        cwd: None,
        hidden: false,
    }
}

/// Installed desktop apps, by Windows' own Start-menu / package registration.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DesktopApps {
    pub claude: Option<String>,
    pub codex: Option<String>,
}

impl DesktopApps {
    pub fn for_provider(&self, provider: ProviderKind) -> Option<&str> {
        match provider {
            ProviderKind::Claude => self.claude.as_deref(),
            ProviderKind::Codex => self.codex.as_deref(),
            _ => None,
        }
    }
}

/// Queries Windows registration only (Start apps, the Codex app package), never the apps'
/// credentials or databases. Prints `{"Claude": id|null, "Codex": id|null}` (as the C# app).
const DESKTOP_SCRIPT: &str = r#"$ErrorActionPreference = 'SilentlyContinue'
$apps = @(Get-StartApps)
$claude = ($apps | Where-Object Name -eq 'Claude' | Select-Object -First 1).AppID
$codex = ($apps | Where-Object Name -eq 'Codex' | Select-Object -First 1).AppID
if (-not $codex) {
    $package = Get-AppxPackage -Name 'OpenAI.Codex' | Select-Object -First 1
    if ($package) { $codex = $package.PackageFamilyName + '!App' }
}
[pscustomobject]@{ Claude = $claude; Codex = $codex } | ConvertTo-Json -Compress"#;

/// Parses the detection script's output; anything unreadable means "none found".
pub fn parse_desktop_apps(text: &str) -> DesktopApps {
    let text = text.trim().trim_start_matches('\u{feff}');
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
        return DesktopApps::default();
    };
    let get = |name: &str| {
        v.get(name)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    DesktopApps {
        claude: get("Claude"),
        codex: get("Codex"),
    }
}

/// Detects the desktop apps once (called at startup on a background thread). The helper
/// PowerShell is hidden and killed after `timeout`; then nothing is found.
pub fn detect_desktop_apps(timeout: Duration) -> DesktopApps {
    use std::io::Read;
    use std::process::{Command, Stdio};
    if !cfg!(windows) {
        return DesktopApps::default();
    }
    let mut cmd = Command::new("powershell.exe");
    cmd.args(["-NoLogo", "-NoProfile", "-EncodedCommand"])
        .arg(encode_command(DESKTOP_SCRIPT))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let Ok(mut child) = cmd.spawn() else {
        return DesktopApps::default();
    };
    let Some(mut out) = child.stdout.take() else {
        let _ = child.kill();
        return DesktopApps::default();
    };
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut text = String::new();
        let _ = out.read_to_string(&mut text);
        let _ = tx.send(text);
    });
    let result = rx.recv_timeout(timeout);
    let _ = child.kill();
    let _ = child.wait();
    result.map_or_else(|_| DesktopApps::default(), |t| parse_desktop_apps(&t))
}

/// VS Code's `code` launcher (`code.cmd` on Windows), found like the C# app does.
pub fn find_vscode() -> Option<PathBuf> {
    find_on_path("code", false)
}

/// One entry of a project's target submenu in the Launch menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    WindowsTerminal,
    PowerShell,
    VsCode,
    /// The account's provider desktop app (Claude Desktop link / Codex Desktop).
    Desktop,
}

/// What can be launched on this machine (CLI, terminals, editor, desktop app).
#[derive(Debug, Clone, Default)]
pub struct LaunchEnv {
    pub cli: Option<PathBuf>,
    pub windows_terminal: Option<PathBuf>,
    pub vscode: Option<PathBuf>,
    pub desktop_app: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuTarget {
    pub target: Target,
    pub label: &'static str,
    /// Why the item is disabled (shown as its hint); None = launchable.
    pub disabled: Option<&'static str>,
}

pub const HINT_NO_PROFILE: &str = "No CLI profile: sign in through the hub";
pub const HINT_NO_FOLDER: &str = "The folder no longer exists";
pub const HINT_NO_CLI: &str = "CLI not found (set it in Settings → General → Advanced)";
pub const HINT_NO_WT: &str = "Windows Terminal not found";
pub const HINT_NO_CODE: &str = "VS Code not found on PATH";
pub const HINT_NO_DESKTOP: &str = "Not installed";

/// The target submenu of one project for one account, in menu order, with disabled hints. A
/// missing folder disables everything; hub-only accounts (no profile) cannot start a CLI with
/// their login, but VS Code and the desktop app still work.
pub fn menu_targets(
    provider: ProviderKind,
    has_profile: bool,
    folder_exists: bool,
    env: &LaunchEnv,
) -> Vec<MenuTarget> {
    let gate = |reason: Option<&'static str>| (!folder_exists).then_some(HINT_NO_FOLDER).or(reason);
    let cli_reason = if !has_profile {
        Some(HINT_NO_PROFILE)
    } else if env.cli.is_none() {
        Some(HINT_NO_CLI)
    } else {
        None
    };
    vec![
        MenuTarget {
            target: Target::WindowsTerminal,
            label: "Windows Terminal",
            disabled: gate(cli_reason.or(env.windows_terminal.is_none().then_some(HINT_NO_WT))),
        },
        MenuTarget {
            target: Target::PowerShell,
            label: "PowerShell",
            disabled: gate(cli_reason),
        },
        MenuTarget {
            target: Target::VsCode,
            label: "VS Code",
            disabled: gate(env.vscode.is_none().then_some(HINT_NO_CODE)),
        },
        MenuTarget {
            target: Target::Desktop,
            label: match provider {
                ProviderKind::Claude => "Claude Desktop · local folder",
                ProviderKind::Codex => "Codex Desktop · choose folder there",
                _ => "Desktop app",
            },
            disabled: gate(env.desktop_app.is_none().then_some(HINT_NO_DESKTOP)),
        },
    ]
}

/// Where to start the login: the user's home when it exists, else the profile folder.
pub fn login_cwd(profile_dir: &Path) -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| profile_dir.to_path_buf())
}

// ---------- executables ----------

fn extensions() -> &'static [&'static str] {
    if cfg!(windows) {
        &[".exe", ".cmd", ".bat"]
    } else {
        &[""]
    }
}

/// A configured path wins and must be an existing file (None = "not found", no silent fallback,
/// like the C# app). Without one the usual places are searched: `~/.local/bin`, npm global, PATH
/// and, for Codex, the desktop app's bundled CLI.
pub fn find_cli(provider: ProviderKind, configured: Option<&Path>) -> Option<PathBuf> {
    match configured {
        Some(p) => p.is_file().then(|| p.to_path_buf()),
        None => find_on_path(cli_name(provider), provider == ProviderKind::Codex),
    }
}

/// Windows Terminal (`wt.exe`, an app execution alias on PATH).
pub fn find_windows_terminal() -> Option<PathBuf> {
    find_on_path("wt", false)
}

fn find_on_path(name: &str, bundled_codex: bool) -> Option<PathBuf> {
    let dirs = codex::candidate_dirs(
        std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from),
        bundled_codex
            .then(|| std::env::var_os("LOCALAPPDATA").map(PathBuf::from))
            .flatten(),
        std::env::var_os("PATH"),
    );
    codex::find_in(&dirs, name, extensions())
}

/// A configured CLI path as typed: empty clears it; otherwise it must be an existing file, with
/// an executable extension on Windows. Quotes around a pasted path are dropped.
pub fn validate_cli_path(input: &str) -> Result<Option<PathBuf>, &'static str> {
    let t = input.trim().trim_matches('"').trim();
    if t.is_empty() {
        return Ok(None);
    }
    let p = PathBuf::from(t);
    let ext_ok = !cfg!(windows)
        || p.extension().is_some_and(|e| {
            ["exe", "cmd", "bat"]
                .iter()
                .any(|x| e.eq_ignore_ascii_case(x))
        });
    if p.is_file() && ext_ok {
        Ok(Some(p))
    } else {
        Err("Choose an existing CLI executable, or leave empty.")
    }
}

// ---------- start with Windows ----------

/// The Run-key value for an exe path: quoted, so spaces in the path are safe.
pub fn startup_command(exe: &Path) -> String {
    format!("\"{}\"", exe.display())
}

// ---------- login completion ----------

/// How long a started sign-in is watched.
pub const LOGIN_TIMEOUT: Duration = Duration::from_secs(600);

/// A unique, credential-free file the terminal script writes the CLI's exit code into. It reports
/// the result independently of the terminal process (which stays open or detaches).
pub struct LoginCompletion {
    dir: PathBuf,
    file: PathBuf,
    started: Instant,
}

impl LoginCompletion {
    pub fn new() -> std::io::Result<Self> {
        let dir =
            std::env::temp_dir().join(format!("AI-SubHub-login-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir)?;
        Ok(Self {
            file: dir.join("exit-code"),
            dir,
            started: Instant::now(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.file
    }

    /// The exit code once the CLI ended (a file still being written reads as not yet).
    pub fn poll(&self) -> Option<i32> {
        parse_exit_code(&std::fs::read_to_string(&self.file).ok()?)
    }

    pub fn timed_out(&self) -> bool {
        self.started.elapsed() > LOGIN_TIMEOUT
    }
}

pub fn parse_exit_code(text: &str) -> Option<i32> {
    text.trim().parse().ok()
}

impl Drop for LoginCompletion {
    fn drop(&mut self) {
        // Best effort: this session's signal file only.
        let _ = std::fs::remove_file(&self.file);
        let _ = std::fs::remove_dir(&self.dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn quoting_treats_paths_as_data() {
        assert_eq!(ps_quote("C:\\a b\\c"), "'C:\\a b\\c'");
        assert_eq!(ps_quote("O'Brien; calc"), "'O''Brien; calc'");
        assert_eq!(ps_quote("$env:X `x"), "'$env:X `x'");
    }

    #[test]
    fn base64_known_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn encoded_command_is_utf16le_base64() {
        // "Get-Date" in UTF-16LE, as PowerShell documents -EncodedCommand.
        assert_eq!(encode_command("Get-Date"), "RwBlAHQALQBEAGEAdABlAA==");
        // Non-ASCII survives (surrogate pair for the emoji).
        let enc = encode_command("é😀");
        assert_eq!(enc.len() % 4, 0);
    }

    #[test]
    fn script_clears_overrides_and_points_at_the_profile() {
        let s = env_script(ProviderKind::Claude, &p("C:\\Users\\a b\\.claude-it's"));
        for key in OVERRIDE_VARS {
            assert!(
                s.contains(&format!(
                    "Remove-Item -LiteralPath 'Env:{key}' -ErrorAction SilentlyContinue"
                )),
                "{key}"
            );
        }
        assert!(s.ends_with("$env:CLAUDE_CONFIG_DIR = 'C:\\Users\\a b\\.claude-it''s'; "));
        let c = env_script(ProviderKind::Codex, &p("D:\\x"));
        assert!(c.ends_with("$env:CODEX_HOME = 'D:\\x'; "));
    }

    #[test]
    fn login_script_per_provider() {
        let s = login_script(
            ProviderKind::Claude,
            &p("C:\\prof"),
            &p("C:\\bin\\claude.exe"),
            &p("C:\\Users\\me"),
            None,
        );
        assert!(s.contains(
            "Set-Location -LiteralPath 'C:\\Users\\me'; $global:LASTEXITCODE = 1; & 'C:\\bin\\claude.exe' 'auth' 'login' '--claudeai'"
        ));
        assert!(!s.contains("WriteAllText"));
        let s = login_script(
            ProviderKind::Codex,
            &p("C:\\prof"),
            &p("C:\\bin\\codex.cmd"),
            &p("C:\\Users\\me"),
            Some(&p("C:\\Temp\\it's\\exit-code")),
        );
        assert!(s.contains("& 'C:\\bin\\codex.cmd' 'login'; [System.IO.File]::WriteAllText('C:\\Temp\\it''s\\exit-code', [string]$LASTEXITCODE)"));
    }

    #[test]
    fn plan_uses_windows_terminal_when_chosen_and_found() {
        let wt = p("C:\\wt.exe");
        let plan = plan_login(
            ProviderKind::Claude,
            &p("C:\\prof"),
            &p("C:\\claude.exe"),
            &p("C:\\home"),
            Terminal::WindowsTerminal,
            Some(&wt),
            None,
        );
        assert_eq!(plan.program, wt);
        assert_eq!(&plan.args[..2], ["new-tab", "powershell.exe"]);
        assert_eq!(&plan.args[2..5], ["-NoLogo", "-NoProfile", "-NoExit"]);
        assert_eq!(plan.args[5], "-EncodedCommand");
        assert_eq!(plan.args.len(), 7);
        assert_eq!(plan.env_set, Some(("CLAUDE_CONFIG_DIR", p("C:\\prof"))));
        assert!(plan.env_remove.contains(&"ANTHROPIC_API_KEY"));
        assert_eq!(plan.cwd, Some(p("C:\\home")));
        assert!(!plan.hidden);
    }

    #[test]
    fn plan_falls_back_to_powershell() {
        for (terminal, wt) in [
            (Terminal::PowerShell, Some(p("C:\\wt.exe"))),
            (Terminal::WindowsTerminal, None),
        ] {
            let plan = plan_login(
                ProviderKind::Codex,
                &p("C:\\prof"),
                &p("C:\\codex.exe"),
                &p("C:\\home"),
                terminal,
                wt.as_deref(),
                None,
            );
            assert_eq!(plan.program, p("powershell.exe"));
            assert_eq!(plan.args[0], "-NoLogo");
            assert_eq!(plan.args.len(), 5);
            assert_eq!(plan.env_set.as_ref().unwrap().0, "CODEX_HOME");
        }
    }

    // ---------- projects and the Launch menu ----------

    /// The PowerShell script inside a plan's `-EncodedCommand` argument (UTF-16LE base64).
    fn decode_script(plan: &LaunchPlan) -> String {
        let enc = plan.args.last().unwrap();
        let mut bits = 0u32;
        let mut n = 0;
        let mut bytes = vec![];
        for c in enc.bytes().filter(|b| *b != b'=') {
            let v = match c {
                b'A'..=b'Z' => c - b'A',
                b'a'..=b'z' => c - b'a' + 26,
                b'0'..=b'9' => c - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                _ => panic!("not base64"),
            };
            bits = (bits << 6) | u32::from(v);
            n += 6;
            if n >= 8 {
                n -= 8;
                bytes.push((bits >> n) as u8);
                bits &= (1 << n) - 1;
            }
        }
        let units: Vec<u16> = bytes
            .chunks(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16(&units).unwrap()
    }

    #[test]
    fn project_cli_runs_the_cli_in_the_folder_with_isolated_env() {
        let plan = plan_project_cli(
            ProviderKind::Claude,
            &p("C:\\Users\\a b\\.claude-it's"),
            &p("C:\\bin\\claude.exe"),
            &p("D:\\work\\O'Brien & Co (x)"),
            Terminal::PowerShell,
            Some(&p("C:\\wt.exe")),
        );
        assert_eq!(plan.program, p("powershell.exe"));
        assert_eq!(&plan.args[..3], ["-NoLogo", "-NoProfile", "-NoExit"]);
        assert_eq!(plan.cwd, Some(p("D:\\work\\O'Brien & Co (x)")));
        assert_eq!(
            plan.env_set,
            Some(("CLAUDE_CONFIG_DIR", p("C:\\Users\\a b\\.claude-it's")))
        );
        assert_eq!(plan.env_remove.len(), 11);
        let script = decode_script(&plan);
        for key in OVERRIDE_VARS {
            assert!(script.contains(&format!("Remove-Item -LiteralPath 'Env:{key}'")));
        }
        assert!(script.contains("$env:CLAUDE_CONFIG_DIR = 'C:\\Users\\a b\\.claude-it''s'; "));
        // Path with space, apostrophe, ampersand and parentheses stays one literal; plain run
        // of the CLI without sign-in arguments.
        assert!(script.ends_with(
            "Set-Location -LiteralPath 'D:\\work\\O''Brien & Co (x)'; $global:LASTEXITCODE = 1; & 'C:\\bin\\claude.exe'"
        ));
        assert!(!script.contains("WriteAllText"));
    }

    #[test]
    fn project_cli_codex_uses_codex_home_and_the_chosen_terminal() {
        let wt = p("C:\\wt.exe");
        let plan = plan_project_cli(
            ProviderKind::Codex,
            &p("D:\\prof"),
            &p("C:\\bin\\codex.cmd"),
            &p("D:\\proj"),
            Terminal::WindowsTerminal,
            Some(&wt),
        );
        assert_eq!(plan.program, wt);
        assert_eq!(&plan.args[..2], ["new-tab", "powershell.exe"]);
        assert_eq!(plan.env_set, Some(("CODEX_HOME", p("D:\\prof"))));
        assert!(decode_script(&plan).ends_with("& 'C:\\bin\\codex.cmd'"));
        // Windows Terminal asked for but missing: PowerShell.
        let plan = plan_project_cli(
            ProviderKind::Codex,
            &p("D:\\prof"),
            &p("C:\\bin\\codex.cmd"),
            &p("D:\\proj"),
            Terminal::WindowsTerminal,
            None,
        );
        assert_eq!(plan.program, p("powershell.exe"));
    }

    #[test]
    fn vscode_opens_a_new_window_through_a_hidden_powershell() {
        let plan = plan_vscode(
            &p("C:\\Users\\a b\\AppData\\Local\\Programs\\Microsoft VS Code\\bin\\code.cmd"),
            &p("D:\\it's here\\proj"),
        );
        assert_eq!(plan.program, p("powershell.exe"));
        assert_eq!(
            &plan.args[..3],
            ["-NoLogo", "-NoProfile", "-EncodedCommand"]
        );
        assert!(plan.hidden);
        assert!(plan.env_set.is_none() && plan.env_remove.is_empty() && plan.cwd.is_none());
        assert_eq!(
            decode_script(&plan),
            "& 'C:\\Users\\a b\\AppData\\Local\\Programs\\Microsoft VS Code\\bin\\code.cmd' '--new-window' '--' 'D:\\it''s here\\proj'"
        );
    }

    #[test]
    fn data_string_escaping_matches_dotnet() {
        assert_eq!(escape_data_string("AZaz09-._~"), "AZaz09-._~");
        assert_eq!(
            escape_data_string("C:\\a b\\it's&x=1#é"),
            "C%3A%5Ca%20b%5Cit%27s%26x%3D1%23%C3%A9"
        );
    }

    #[test]
    fn claude_desktop_link_escapes_the_full_path() {
        assert_eq!(
            claude_desktop_uri(&p("C:\\My Projects\\app")),
            "claude://code/new?folder=C%3A%5CMy%20Projects%5Capp"
        );
        // An existing folder is resolved to its full path without the verbatim prefix.
        let dir = tempfile::tempdir().unwrap();
        let uri = claude_desktop_uri(dir.path());
        assert!(uri.starts_with("claude://code/new?folder="));
        assert!(!uri.contains("%3F"), "{uri}");
    }

    #[test]
    fn desktop_app_plans() {
        let plan = plan_desktop_app("OpenAI.Codex_2p2nqsd0c76g0!App");
        assert_eq!(plan.program, p("explorer.exe"));
        assert_eq!(
            plan.args,
            ["shell:AppsFolder\\OpenAI.Codex_2p2nqsd0c76g0!App"]
        );
        let plan = plan_desktop_app("C:\\Apps\\Codex.exe");
        assert_eq!(plan.program, p("C:\\Apps\\Codex.exe"));
        assert!(plan.args.is_empty());
    }

    fn full_env() -> LaunchEnv {
        LaunchEnv {
            cli: Some(p("C:\\claude.exe")),
            windows_terminal: Some(p("C:\\wt.exe")),
            vscode: Some(p("C:\\code.cmd")),
            desktop_app: Some("Claude!App".into()),
        }
    }

    fn hints(t: &[MenuTarget]) -> Vec<Option<&'static str>> {
        t.iter().map(|m| m.disabled).collect()
    }

    #[test]
    fn menu_lists_the_four_targets_in_order() {
        let t = menu_targets(ProviderKind::Claude, true, true, &full_env());
        let order: Vec<Target> = t.iter().map(|m| m.target).collect();
        assert_eq!(
            order,
            [
                Target::WindowsTerminal,
                Target::PowerShell,
                Target::VsCode,
                Target::Desktop
            ]
        );
        assert_eq!(hints(&t), [None, None, None, None]);
        assert_eq!(t[3].label, "Claude Desktop · local folder");
        let t = menu_targets(ProviderKind::Codex, true, true, &full_env());
        assert_eq!(t[3].label, "Codex Desktop · choose folder there");
    }

    #[test]
    fn missing_pieces_disable_only_their_items_with_a_hint() {
        let mut env = full_env();
        env.windows_terminal = None;
        env.vscode = None;
        env.desktop_app = None;
        let t = menu_targets(ProviderKind::Claude, true, true, &env);
        assert_eq!(
            hints(&t),
            [
                Some(HINT_NO_WT),
                None,
                Some(HINT_NO_CODE),
                Some(HINT_NO_DESKTOP)
            ]
        );
        let env = LaunchEnv {
            cli: None,
            ..full_env()
        };
        let t = menu_targets(ProviderKind::Codex, true, true, &env);
        assert_eq!(
            hints(&t),
            [Some(HINT_NO_CLI), Some(HINT_NO_CLI), None, None]
        );
    }

    #[test]
    fn hub_only_accounts_cannot_start_a_cli_but_can_use_vscode_and_desktop() {
        let t = menu_targets(ProviderKind::Claude, false, true, &full_env());
        assert_eq!(
            hints(&t),
            [Some(HINT_NO_PROFILE), Some(HINT_NO_PROFILE), None, None]
        );
        assert_eq!(HINT_NO_PROFILE, "No CLI profile: sign in through the hub");
    }

    #[test]
    fn a_missing_folder_disables_everything() {
        let t = menu_targets(ProviderKind::Claude, true, false, &full_env());
        assert!(t.iter().all(|m| m.disabled == Some(HINT_NO_FOLDER)));
    }

    #[test]
    fn desktop_detection_output_parses() {
        let apps = parse_desktop_apps(r#"{"Claude":"Claude_pzs8sxrjxfjjc!Claude","Codex":null}"#);
        assert_eq!(apps.claude.as_deref(), Some("Claude_pzs8sxrjxfjjc!Claude"));
        assert_eq!(apps.codex, None);
        assert_eq!(
            apps.for_provider(ProviderKind::Claude),
            Some("Claude_pzs8sxrjxfjjc!Claude")
        );
        assert_eq!(apps.for_provider(ProviderKind::Codex), None);
        let both = parse_desktop_apps("\u{feff} {\"Claude\":\" \",\"Codex\":\"X!App\"}\r\n");
        assert_eq!((both.claude, both.codex.as_deref()), (None, Some("X!App")));
        for junk in ["", "not json", "[]", "null"] {
            assert_eq!(parse_desktop_apps(junk), DesktopApps::default(), "{junk}");
        }
    }

    #[test]
    fn encoded_script_in_the_plan_matches_the_script() {
        let plan = plan_login(
            ProviderKind::Codex,
            &p("C:\\prof"),
            &p("C:\\codex.exe"),
            &p("C:\\home"),
            Terminal::PowerShell,
            None,
            Some(&p("C:\\t\\exit-code")),
        );
        let script = login_script(
            ProviderKind::Codex,
            &p("C:\\prof"),
            &p("C:\\codex.exe"),
            &p("C:\\home"),
            Some(&p("C:\\t\\exit-code")),
        );
        assert_eq!(plan.args.last().unwrap(), &encode_command(&script));
    }

    #[test]
    fn configured_cli_path_wins_and_must_exist() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("claude.exe");
        std::fs::write(&exe, b"").unwrap();
        assert_eq!(find_cli(ProviderKind::Claude, Some(&exe)), Some(exe));
        assert_eq!(
            find_cli(ProviderKind::Claude, Some(&dir.path().join("nope.exe"))),
            None
        );
    }

    #[test]
    fn cli_path_validation() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("codex.exe");
        std::fs::write(&exe, b"").unwrap();
        let txt = dir.path().join("codex.txt");
        std::fs::write(&txt, b"").unwrap();
        assert_eq!(validate_cli_path("   "), Ok(None));
        assert_eq!(
            validate_cli_path(&format!("\"{}\"", exe.display())),
            Ok(Some(exe))
        );
        assert!(validate_cli_path(&dir.path().join("x.exe").to_string_lossy()).is_err());
        if cfg!(windows) {
            assert!(validate_cli_path(&txt.to_string_lossy()).is_err());
        }
    }

    #[test]
    fn startup_value_is_a_quoted_path() {
        assert_eq!(
            startup_command(&p("C:\\Program Files\\SubHub\\ai-subhub.exe")),
            "\"C:\\Program Files\\SubHub\\ai-subhub.exe\""
        );
    }

    #[test]
    fn completion_file_roundtrip_and_cleanup() {
        let c = LoginCompletion::new().unwrap();
        let path = c.path().to_path_buf();
        assert_eq!(c.poll(), None);
        std::fs::write(&path, "0").unwrap();
        assert_eq!(c.poll(), Some(0));
        std::fs::write(&path, " 1\r\n").unwrap();
        assert_eq!(c.poll(), Some(1));
        std::fs::write(&path, "").unwrap();
        assert_eq!(c.poll(), None);
        assert!(!c.timed_out());
        let dir = path.parent().unwrap().to_path_buf();
        drop(c);
        assert!(!dir.exists());
    }

    #[test]
    fn exit_codes_parse() {
        assert_eq!(parse_exit_code("0"), Some(0));
        assert_eq!(parse_exit_code("-1"), Some(-1));
        assert_eq!(parse_exit_code("abc"), None);
    }
}

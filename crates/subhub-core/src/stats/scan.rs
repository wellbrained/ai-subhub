//! Log discovery, incremental per-file cache and aggregation into hourly buckets per model, task
//! kind, main thread / subagent, working folder and thread.
//!
//! Each file keeps its own aggregated buckets plus how many bytes were read, so a later scan reads
//! only appended bytes. A file that shrank (or whose size and time disagree) is read again from
//! the start; a file that vanished drops out (a Codex session moved to `archived_sessions` is the
//! same data under a new path, so keeping the old entry would count it twice).
//!
//! Scan v2 (owner 2026-11-07): a step's tokens are split evenly over its tool calls by task kind
//! (`kind.rs`), integer parts with the remainder on the first kind, so every total stays exact
//! and the Usage view adds up as before.
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, UNIX_EPOCH};

use anyhow::Context;
use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};

use super::kind::{TaskKind, ToolCall, step_kinds};
use super::parse::{
    ClaudeMessage, CodexEvent, CopilotEvent, RawUsage, parse_claude_line, parse_claude_title,
    parse_codex_line, parse_copilot_line,
};
use super::{Tokens, hour_index};
use crate::model::ProviderKind;
use crate::profile::{ProfileEnv, discover_app_dirs, discover_dirs, home_dir};

/// 2: buckets gained task kind, sidechain, cwd and thread; thread titles (one full re-scan).
const CACHE_VERSION: u32 = 2;

/// Aggregated tokens of one file for one local hour, model, task kind, main/sub thread, working
/// folder and thread. `cwd` and `thread` index the entry's `cwds` / `threads` tables (they repeat
/// in every bucket, so the cache stores each once per file).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileBucket {
    pub hour: i64,
    pub model: String,
    pub tokens: Tokens,
    pub kind: TaskKind,
    #[serde(default)]
    pub sidechain: bool,
    #[serde(default)]
    pub cwd: Option<u32>,
    #[serde(default)]
    pub thread: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionRec {
    pub id: String,
    pub hours: Vec<i64>,
}

/// What the cache remembers per log file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileEntry {
    pub provider: ProviderKind,
    pub size: u64,
    pub mtime_ms: i64,
    /// Bytes consumed so far (up to the last complete line).
    pub offset: u64,
    /// Codex: model of the latest turn.
    #[serde(default)]
    pub model: Option<String>,
    /// Codex: working folder of the latest turn. Copilot: working folder of the session.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Codex: tool calls since the last token count (they belong to the next one).
    #[serde(default)]
    pub pending: Vec<ToolCall>,
    /// Codex: running total after the previous token count, to take per-request deltas.
    #[serde(default)]
    pub prev_total: Option<RawUsage>,
    /// Copilot: cumulative usage per model at the previous `session.shutdown` (a resumed session
    /// appends another shutdown with higher totals; only the difference is new).
    #[serde(default)]
    pub copilot_prev: BTreeMap<String, RawUsage>,
    /// Claude: hashes of message id + request id already counted from this file.
    #[serde(default)]
    pub seen: Vec<u64>,
    /// Working folders the buckets refer to.
    #[serde(default)]
    pub cwds: Vec<String>,
    /// Thread (session) ids the buckets refer to.
    #[serde(default)]
    pub threads: Vec<String>,
    /// Claude: thread titles (`custom-title` lines) by session id; the last one wins.
    #[serde(default)]
    pub titles: BTreeMap<String, String>,
    pub buckets: Vec<FileBucket>,
    pub sessions: Vec<SessionRec>,
}

impl FileEntry {
    fn new(provider: ProviderKind) -> Self {
        Self {
            provider,
            size: 0,
            mtime_ms: 0,
            offset: 0,
            model: None,
            cwd: None,
            pending: vec![],
            prev_total: None,
            copilot_prev: BTreeMap::new(),
            seen: vec![],
            cwds: vec![],
            threads: vec![],
            titles: BTreeMap::new(),
            buckets: vec![],
            sessions: vec![],
        }
    }
}

#[derive(Serialize, Deserialize)]
struct CacheFile {
    version: u32,
    files: BTreeMap<String, FileEntry>,
}

/// One merged bucket: all files, one local hour, provider, model, task kind, main/sub thread,
/// working folder and thread.
#[derive(Debug, Clone, PartialEq)]
pub struct Bucket {
    pub hour: i64,
    pub provider: ProviderKind,
    pub model: String,
    pub tokens: Tokens,
    pub kind: TaskKind,
    /// Subagent work (Claude `isSidechain`; Codex: always false for now).
    pub sidechain: bool,
    /// Working folder of the steps (None when the log has none). Resolved to a project folder
    /// later (`insights::project_of`), so editing projects needs no re-scan.
    pub cwd: Option<Arc<str>>,
    /// Session id (Claude `sessionId`, Codex file name, Copilot session folder).
    pub thread: Arc<str>,
}

/// A session and the local hours it had usage in.
#[derive(Debug, Clone, PartialEq)]
pub struct ScanSession {
    pub provider: ProviderKind,
    pub id: String,
    pub hours: Vec<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct Scan {
    pub buckets: Vec<Bucket>,
    pub sessions: Vec<ScanSession>,
    /// Thread titles by session id (Claude `custom-title`); threads without one are missing.
    pub titles: HashMap<String, String>,
    /// Log files found.
    pub files: usize,
    /// Files read (new or grown) in this scan.
    pub files_read: usize,
    pub elapsed: Duration,
    pub finished_at: Option<DateTime<Utc>>,
}

/// Where logs live: Claude config dirs' `projects` folders, Codex `sessions` folders and the
/// Copilot CLI's `session-state` folder, already canonical and without duplicates (the owner's Claude account folders share one `projects`).
#[derive(Debug, Clone, Default)]
pub struct Roots {
    pub claude: Vec<PathBuf>,
    pub codex: Vec<PathBuf>,
    pub copilot: Vec<PathBuf>,
}

impl Roots {
    fn push(list: &mut Vec<PathBuf>, dir: &Path) {
        if let Ok(c) = fs::canonicalize(dir)
            && c.is_dir()
            && !list.contains(&c)
        {
            list.push(c);
        }
    }

    pub fn add_claude(&mut self, dir: &Path) {
        Self::push(&mut self.claude, dir);
    }

    pub fn add_codex(&mut self, dir: &Path) {
        Self::push(&mut self.codex, dir);
    }

    pub fn add_copilot(&mut self, dir: &Path) {
        Self::push(&mut self.copilot, dir);
    }
}

/// `projects` of every Claude profile folder, `sessions` + `archived_sessions` of every Codex
/// one (`profile::discover_dirs`, the same discovery as profile detection) and `session-state` of
/// the Copilot home (`profile::discover_app_dirs`).
pub fn discover_roots(home: &Path, env: &ProfileEnv) -> Roots {
    let mut roots = Roots::default();
    for (provider, dir) in discover_dirs(home, env) {
        match provider {
            ProviderKind::Claude => roots.add_claude(&dir.join("projects")),
            ProviderKind::Codex => {
                roots.add_codex(&dir.join("sessions"));
                roots.add_codex(&dir.join("archived_sessions"));
            }
            _ => {}
        }
    }
    for (provider, dir) in discover_app_dirs(home, env) {
        if provider == ProviderKind::Copilot {
            roots.add_copilot(&dir.join("session-state"));
        }
    }
    roots
}

pub fn cache_path() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("AI SubHub").join("stats-cache.json")
}

/// Scans the real logs of the current user with the default cache.
pub fn scan_default() -> anyhow::Result<Scan> {
    let home = home_dir().context("No home folder")?;
    scan(
        &discover_roots(&home, &ProfileEnv::from_process()),
        Some(&cache_path()),
    )
}

struct Listed {
    key: String,
    path: PathBuf,
    provider: ProviderKind,
    size: u64,
    mtime_ms: i64,
    compressed: bool,
}

fn walk(dir: &Path, provider: ProviderKind, out: &mut Vec<Listed>, seen: &mut HashSet<String>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let path = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            walk(&path, provider, out, seen);
            continue;
        }
        let name = e.file_name().to_string_lossy().to_lowercase();
        let compressed = name.ends_with(".jsonl.zst");
        let ok = match provider {
            ProviderKind::Claude => name.ends_with(".jsonl"),
            ProviderKind::Codex => name.ends_with(".jsonl") || compressed,
            _ => false,
        };
        if !ok {
            continue;
        }
        list_file(path, provider, compressed, out, seen);
    }
}

fn list_file(
    path: PathBuf,
    provider: ProviderKind,
    compressed: bool,
    out: &mut Vec<Listed>,
    seen: &mut HashSet<String>,
) {
    let key = path.to_string_lossy().into_owned();
    if seen.contains(&key) {
        return;
    }
    let Ok(md) = fs::metadata(&path) else { return };
    seen.insert(key.clone());
    let mtime_ms = md
        .modified()
        .ok()
        .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as i64);
    out.push(Listed {
        key,
        path,
        provider,
        size: md.len(),
        mtime_ms,
        compressed,
    });
}

/// Copilot's `session-state`: `<session>/events.jsonl`, plus the older flat `<uuid>.jsonl`. Other
/// files in a session folder (checkpoints, plans, ...) are not usage logs.
fn walk_copilot(root: &Path, out: &mut Vec<Listed>, seen: &mut HashSet<String>) {
    let Ok(rd) = fs::read_dir(root) else { return };
    for e in rd.flatten() {
        let path = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            let events = path.join("events.jsonl");
            if events.is_file() {
                list_file(events, ProviderKind::Copilot, false, out, seen);
            }
        } else if e
            .file_name()
            .to_string_lossy()
            .to_lowercase()
            .ends_with(".jsonl")
        {
            list_file(path, ProviderKind::Copilot, false, out, seen);
        }
    }
}

fn load_cache(path: &Path) -> CacheFile {
    let empty = || CacheFile {
        version: CACHE_VERSION,
        files: BTreeMap::new(),
    };
    let Ok(bytes) = fs::read(path) else {
        return empty();
    };
    match serde_json::from_slice::<CacheFile>(&bytes) {
        Ok(c) if c.version == CACHE_VERSION => c,
        _ => empty(),
    }
}

fn save_cache(path: &Path, cache: &CacheFile) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    fs::write(&tmp, serde_json::to_vec(cache)?)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Splits `t` over kind shares: each part gets `count / total` of every counter (integer), the
/// first part also the remainder, so the parts add up to `t` exactly.
pub(crate) fn split_tokens(t: &Tokens, shares: &[(TaskKind, u32)]) -> Vec<(TaskKind, Tokens)> {
    let total: u64 = shares.iter().map(|(_, c)| *c as u64).sum();
    if total <= 1 || shares.len() <= 1 {
        return shares.first().map(|(k, _)| (*k, *t)).into_iter().collect();
    }
    let part = |n: u64, c: u32| n * c as u64 / total;
    let mut out: Vec<(TaskKind, Tokens)> = shares
        .iter()
        .map(|(k, c)| {
            (
                *k,
                Tokens {
                    input: part(t.input, *c),
                    cache_read: part(t.cache_read, *c),
                    cache_write: part(t.cache_write, *c),
                    output: part(t.output, *c),
                },
            )
        })
        .collect();
    let mut sum = Tokens::default();
    for (_, p) in &out {
        sum.add(p);
    }
    let first = &mut out[0].1;
    first.input += t.input - sum.input;
    first.cache_read += t.cache_read - sum.cache_read;
    first.cache_write += t.cache_write - sum.cache_write;
    first.output += t.output - sum.output;
    out
}

/// A Claude assistant message being collected: its content blocks arrive as separate lines with
/// the same key, each adding its tool uses.
struct Step {
    key: Option<u64>,
    hour: i64,
    model: String,
    session: String,
    sidechain: bool,
    cwd: Option<String>,
    tokens: Tokens,
    tools: Vec<ToolCall>,
}

type BucketKey = (i64, String, TaskKind, bool, Option<u32>, u32);

/// Per-file accumulation of lines into buckets.
struct FileProc {
    entry: FileEntry,
    buckets: HashMap<BucketKey, Tokens>,
    sessions: HashMap<String, BTreeSet<i64>>,
    cwd_ix: HashMap<String, u32>,
    thread_ix: HashMap<String, u32>,
    /// Claude: the message whose blocks are still coming.
    step: Option<Step>,
    /// Session id when a line has none (the file name).
    default_session: String,
}

fn local_hour(ts: DateTime<Utc>) -> i64 {
    hour_index(ts.with_timezone(&Local).naive_local())
}

fn index_of(table: &mut Vec<String>, ix: &mut HashMap<String, u32>, value: &str) -> u32 {
    if let Some(i) = ix.get(value) {
        return *i;
    }
    let i = table.len() as u32;
    table.push(value.to_string());
    ix.insert(value.to_string(), i);
    i
}

impl FileProc {
    fn new(entry: FileEntry, default_session: String) -> Self {
        let buckets = entry
            .buckets
            .iter()
            .map(|b| {
                (
                    (
                        b.hour,
                        b.model.clone(),
                        b.kind,
                        b.sidechain,
                        b.cwd,
                        b.thread,
                    ),
                    b.tokens,
                )
            })
            .collect();
        let sessions = entry
            .sessions
            .iter()
            .map(|s| (s.id.clone(), s.hours.iter().copied().collect()))
            .collect();
        let table = |t: &[String]| {
            t.iter()
                .enumerate()
                .map(|(i, s)| (s.clone(), i as u32))
                .collect()
        };
        Self {
            cwd_ix: table(&entry.cwds),
            thread_ix: table(&entry.threads),
            entry,
            buckets,
            sessions,
            step: None,
            default_session,
        }
    }

    /// Adds one step: its tokens split over the kinds of its tool calls.
    #[allow(clippy::too_many_arguments)]
    fn add(
        &mut self,
        hour: i64,
        model: &str,
        session: &str,
        sidechain: bool,
        cwd: Option<&str>,
        tools: &[ToolCall],
        t: &Tokens,
    ) {
        let parts = split_tokens(t, &step_kinds(tools));
        self.add_parts(hour, model, session, sidechain, cwd, &parts);
    }

    /// Adds tokens already split by task kind.
    fn add_parts(
        &mut self,
        hour: i64,
        model: &str,
        session: &str,
        sidechain: bool,
        cwd: Option<&str>,
        parts: &[(TaskKind, Tokens)],
    ) {
        let cwd = cwd.map(|c| index_of(&mut self.entry.cwds, &mut self.cwd_ix, c));
        let thread = index_of(&mut self.entry.threads, &mut self.thread_ix, session);
        for (kind, part) in parts {
            self.buckets
                .entry((hour, model.to_string(), *kind, sidechain, cwd, thread))
                .or_default()
                .add(part);
        }
        self.sessions
            .entry(session.to_string())
            .or_default()
            .insert(hour);
    }

    fn flush_step(&mut self) {
        if let Some(s) = self.step.take() {
            self.add(
                s.hour,
                &s.model,
                &s.session,
                s.sidechain,
                s.cwd.as_deref(),
                &s.tools,
                &s.tokens,
            );
        }
    }

    /// One Claude log line. `seen` holds message hashes already counted (streamed messages repeat).
    fn feed_claude(&mut self, line: &[u8], seen: &Mutex<HashSet<u64>>) {
        let assistant = memchr::memmem::find(line, b"\"assistant\"").is_some();
        if !assistant || memchr::memmem::find(line, b"\"usage\"").is_none() {
            if !assistant && memchr::memmem::find(line, b"\"customTitle\"").is_some() {
                self.feed_title(line);
            }
            return;
        }
        let Ok(text) = std::str::from_utf8(line) else {
            return;
        };
        let Some(ClaudeMessage {
            key,
            ts,
            session,
            model,
            tokens,
            sidechain,
            cwd,
            tools,
        }) = parse_claude_line(text)
        else {
            return;
        };
        // Another block of the message being collected: only its tool uses are new.
        if let Some(step) = self.step.as_mut()
            && key.is_some()
            && step.key == key
        {
            step.tools.extend(tools);
            return;
        }
        self.flush_step();
        if let Some(k) = key {
            let fresh = seen.lock().map(|mut s| s.insert(k)).unwrap_or(true);
            if !fresh {
                return;
            }
            self.entry.seen.push(k);
        }
        self.step = Some(Step {
            key,
            hour: local_hour(ts),
            model,
            session: session.unwrap_or_else(|| self.default_session.clone()),
            sidechain,
            cwd,
            tokens,
            tools,
        });
        if key.is_none() {
            self.flush_step();
        }
    }

    fn feed_title(&mut self, line: &[u8]) {
        if let Ok(text) = std::str::from_utf8(line)
            && let Some((session, title)) = parse_claude_title(text)
        {
            self.entry.titles.insert(session, title);
        }
    }

    /// One Codex log line: turns (model, cwd), tool calls and token counts (per-request deltas of
    /// the running total). Tool calls belong to the next token count.
    fn feed_codex(&mut self, line: &[u8]) {
        let wanted: [&[u8]; 6] = [
            b"\"token_count\"",
            b"\"turn_context\"",
            b"\"function_call\"",
            b"\"custom_tool_call\"",
            b"\"local_shell_call\"",
            b"\"web_search_call\"",
        ];
        if !wanted
            .iter()
            .any(|w| memchr::memmem::find(line, w).is_some())
        {
            return;
        }
        let Ok(text) = std::str::from_utf8(line) else {
            return;
        };
        match parse_codex_line(text) {
            Some(CodexEvent::Turn { model, cwd }) => {
                if model.is_some() {
                    self.entry.model = model;
                }
                if cwd.is_some() {
                    self.entry.cwd = cwd;
                }
            }
            Some(CodexEvent::Tool(call)) => self.entry.pending.push(call),
            Some(CodexEvent::Usage { ts, last, total }) => {
                let delta = match (total, self.entry.prev_total) {
                    (Some(t), Some(p)) => t.checked_sub(&p).or(last),
                    (Some(t), None) => last.or(Some(t)),
                    (None, _) => last,
                };
                if total.is_some() {
                    self.entry.prev_total = total;
                }
                let Some(d) = delta.filter(|d| !d.is_zero()) else {
                    return;
                };
                let model = self.entry.model.clone().unwrap_or_else(|| "unknown".into());
                let session = self.default_session.clone();
                let cwd = self.entry.cwd.clone();
                let tools = std::mem::take(&mut self.entry.pending);
                self.add(
                    local_hour(ts),
                    &model,
                    &session,
                    false,
                    cwd.as_deref(),
                    &tools,
                    &d.tokens(),
                );
            }
            None => {}
        }
    }

    /// One Copilot CLI log line: `session.start` (cwd) and `session.shutdown` (cumulative usage
    /// per model; the difference to the previous shutdown of the file is new). The log has no
    /// tool calls at this granularity, so the tokens go to "Other tools", at the shutdown hour.
    fn feed_copilot(&mut self, line: &[u8]) {
        if memchr::memmem::find(line, b"\"session.start\"").is_none()
            && memchr::memmem::find(line, b"\"session.shutdown\"").is_none()
        {
            return;
        }
        let Ok(text) = std::str::from_utf8(line) else {
            return;
        };
        match parse_copilot_line(text) {
            Some(CopilotEvent::Start { cwd }) => {
                if cwd.is_some() {
                    self.entry.cwd = cwd;
                }
            }
            Some(CopilotEvent::Shutdown { ts, models }) => {
                let session = self.default_session.clone();
                let cwd = self.entry.cwd.clone();
                for (model, total) in models {
                    let prev = self.entry.copilot_prev.insert(model.clone(), total);
                    // A counter that went down is a new counting epoch: all of it is new.
                    let delta = prev.and_then(|p| total.checked_sub(&p)).unwrap_or(total);
                    if delta.is_zero() {
                        continue;
                    }
                    self.add_parts(
                        local_hour(ts),
                        &model,
                        &session,
                        false,
                        cwd.as_deref(),
                        &[(TaskKind::OtherTools, delta.tokens())],
                    );
                }
            }
            None => {}
        }
    }

    fn finish(mut self) -> FileEntry {
        // A message cut by the end of what was written so far is counted with the blocks read;
        // its later blocks repeat a seen key and add nothing (a rare, small misattribution).
        self.flush_step();
        let mut buckets: Vec<_> = self
            .buckets
            .into_iter()
            .map(
                |((hour, model, kind, sidechain, cwd, thread), tokens)| FileBucket {
                    hour,
                    model,
                    tokens,
                    kind,
                    sidechain,
                    cwd,
                    thread,
                },
            )
            .collect();
        buckets.sort_by(|a, b| {
            (a.hour, &a.model, a.kind, a.sidechain, a.cwd, a.thread).cmp(&(
                b.hour,
                &b.model,
                b.kind,
                b.sidechain,
                b.cwd,
                b.thread,
            ))
        });
        let mut sessions: Vec<_> = self
            .sessions
            .into_iter()
            .map(|(id, hours)| SessionRec {
                id,
                hours: hours.into_iter().collect(),
            })
            .collect();
        sessions.sort_by(|a, b| a.id.cmp(&b.id));
        self.entry.buckets = buckets;
        self.entry.sessions = sessions;
        self.entry
    }
}

struct Job {
    listed: Listed,
    entry: FileEntry,
}

/// Reads the new part of one file into its entry.
fn process(job: Job, seen: &Mutex<HashSet<u64>>) -> FileEntry {
    let Job { listed, entry } = job;
    let stem = listed
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut stem = stem.split(".jsonl").next().unwrap_or(&stem).to_string();
    if listed.provider == ProviderKind::Copilot && stem == "events" {
        // `session-state/<session id>/events.jsonl`: the folder names the session.
        if let Some(dir) = listed.path.parent().and_then(|d| d.file_name()) {
            stem = dir.to_string_lossy().into_owned();
        }
    }
    let mut proc = FileProc::new(entry, stem);
    let Ok(mut file) = File::open(&listed.path) else {
        return proc.finish();
    };
    let mut reader: Box<dyn BufRead> = if listed.compressed {
        match zstd::Decoder::new(file) {
            Ok(d) => Box::new(BufReader::with_capacity(1 << 20, d)),
            Err(_) => return proc.finish(),
        }
    } else {
        if file.seek(SeekFrom::Start(proc.entry.offset)).is_err() {
            return proc.finish();
        }
        Box::new(BufReader::with_capacity(1 << 20, file))
    };
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let n = match reader.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        if !listed.compressed {
            // A line still being written stays for the next scan.
            if buf.last() != Some(&b'\n') {
                break;
            }
            proc.entry.offset += n as u64;
        }
        match listed.provider {
            ProviderKind::Claude => proc.feed_claude(&buf, seen),
            ProviderKind::Codex => proc.feed_codex(&buf),
            ProviderKind::Copilot => proc.feed_copilot(&buf),
            _ => {}
        }
        if buf.capacity() > 64 << 20 {
            buf = Vec::new();
        }
    }
    proc.entry.size = listed.size;
    proc.entry.mtime_ms = listed.mtime_ms;
    proc.finish()
}

/// Scans all logs under `roots`, reading only what is new since the cache (when given), and
/// returns the aggregate.
pub fn scan(roots: &Roots, cache_file: Option<&Path>) -> anyhow::Result<Scan> {
    let started = Instant::now();
    let mut listed = Vec::new();
    let mut keys = HashSet::new();
    for d in &roots.claude {
        walk(d, ProviderKind::Claude, &mut listed, &mut keys);
    }
    for d in &roots.codex {
        walk(d, ProviderKind::Codex, &mut listed, &mut keys);
    }
    for d in &roots.copilot {
        walk_copilot(d, &mut listed, &mut keys);
    }
    let files = listed.len();
    let mut cache = cache_file.map(load_cache).unwrap_or(CacheFile {
        version: CACHE_VERSION,
        files: BTreeMap::new(),
    });

    // Decide per file: keep as cached, read the appended bytes, or read from the start.
    let mut old = std::mem::take(&mut cache.files);
    let mut kept: BTreeMap<String, FileEntry> = BTreeMap::new();
    let mut jobs = Vec::new();
    for l in listed {
        let entry = old.remove(&l.key).filter(|e| e.provider == l.provider);
        let fresh = || FileEntry::new(l.provider);
        match entry {
            None => {
                let entry = fresh();
                jobs.push(Job { listed: l, entry });
            }
            Some(e) => {
                let unchanged = e.size == l.size && e.mtime_ms == l.mtime_ms;
                if unchanged {
                    kept.insert(l.key.clone(), e);
                } else if l.compressed
                    || l.size < e.offset
                    || (l.mtime_ms < e.mtime_ms && l.size != e.size)
                {
                    let entry = fresh();
                    jobs.push(Job { listed: l, entry });
                } else {
                    jobs.push(Job {
                        listed: l,
                        entry: e,
                    });
                }
            }
        }
    }
    // Hashes of messages already counted (files read again start without theirs).
    let seen: HashSet<u64> = kept
        .values()
        .map(|e| e.seen.as_slice())
        .chain(jobs.iter().map(|j| j.entry.seen.as_slice()))
        .flatten()
        .copied()
        .collect();
    let seen = Mutex::new(seen);
    let files_read = jobs.len();

    let workers = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .clamp(1, 8)
        .min(jobs.len().max(1));
    let queue = Mutex::new(jobs.into_iter());
    let done: Mutex<Vec<(String, FileEntry)>> = Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| {
                loop {
                    let job = queue.lock().ok().and_then(|mut q| q.next());
                    let Some(job) = job else { break };
                    let key = job.listed.key.clone();
                    let entry = process(job, &seen);
                    if let Ok(mut d) = done.lock() {
                        d.push((key, entry));
                    }
                }
            });
        }
    });
    for (k, e) in done.into_inner().unwrap_or_default() {
        kept.insert(k, e);
    }
    cache.files = kept;
    if let Some(p) = cache_file {
        // A cache that cannot be written only costs speed next time.
        let _ = save_cache(p, &cache);
    }

    let mut scan = merge(&cache.files);
    scan.files = files;
    scan.files_read = files_read;
    scan.elapsed = started.elapsed();
    scan.finished_at = Some(Utc::now());
    Ok(scan)
}

type MergeKey = (
    i64,
    ProviderKind,
    String,
    TaskKind,
    bool,
    Option<Arc<str>>,
    Arc<str>,
);

fn merge(files: &BTreeMap<String, FileEntry>) -> Scan {
    let mut buckets: HashMap<MergeKey, Tokens> = HashMap::new();
    let mut sessions: HashMap<(ProviderKind, String), BTreeSet<i64>> = HashMap::new();
    let mut titles: HashMap<String, String> = HashMap::new();
    // One shared string per distinct folder / thread across files.
    let mut strings: HashSet<Arc<str>> = HashSet::new();
    let mut intern = |s: &str| -> Arc<str> {
        if let Some(a) = strings.get(s) {
            return a.clone();
        }
        let a: Arc<str> = Arc::from(s);
        strings.insert(a.clone());
        a
    };
    for e in files.values() {
        let cwds: Vec<Arc<str>> = e.cwds.iter().map(|c| intern(c)).collect();
        let threads: Vec<Arc<str>> = e.threads.iter().map(|t| intern(t)).collect();
        for b in &e.buckets {
            let cwd = b.cwd.and_then(|i| cwds.get(i as usize).cloned());
            let thread = threads
                .get(b.thread as usize)
                .cloned()
                .unwrap_or_else(|| intern(""));
            buckets
                .entry((
                    b.hour,
                    e.provider,
                    b.model.clone(),
                    b.kind,
                    b.sidechain,
                    cwd,
                    thread,
                ))
                .or_default()
                .add(&b.tokens);
        }
        for s in &e.sessions {
            sessions
                .entry((e.provider, s.id.clone()))
                .or_default()
                .extend(s.hours.iter().copied());
        }
        for (k, v) in &e.titles {
            titles.insert(k.clone(), v.clone());
        }
    }
    let mut out_b: Vec<Bucket> = buckets
        .into_iter()
        .map(
            |((hour, provider, model, kind, sidechain, cwd, thread), tokens)| Bucket {
                hour,
                provider,
                model,
                tokens,
                kind,
                sidechain,
                cwd,
                thread,
            },
        )
        .collect();
    out_b.sort_by(|a, b| {
        (a.hour, &a.model, a.kind, a.sidechain, &a.cwd, &a.thread).cmp(&(
            b.hour,
            &b.model,
            b.kind,
            b.sidechain,
            &b.cwd,
            &b.thread,
        ))
    });
    let mut out_s: Vec<ScanSession> = sessions
        .into_iter()
        .map(|((provider, id), hours)| ScanSession {
            provider,
            id,
            hours: hours.into_iter().collect(),
        })
        .collect();
    out_s.sort_by(|a, b| a.id.cmp(&b.id));
    Scan {
        buckets: out_b,
        sessions: out_s,
        titles,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn claude_line(id: &str, req: &str, out: u64, session: &str) -> String {
        format!(
            r#"{{"type":"assistant","timestamp":"2026-11-05T10:00:00Z","sessionId":"{session}","requestId":"{req}","message":{{"id":"{id}","model":"claude-opus-5-5","content":"private text","usage":{{"input_tokens":1,"output_tokens":{out},"cache_creation_input_tokens":10,"cache_read_input_tokens":100}}}}}}"#
        )
    }

    fn codex_lines(totals: &[(u64, u64)]) -> String {
        // (running total input, running total output); cached is half of input.
        let mut s = String::from(
            r#"{"timestamp":"2026-11-05T10:00:00Z","type":"turn_context","payload":{"model":"gpt-6.1-sol"}}"#,
        );
        s.push('\n');
        let mut prev = (0u64, 0u64);
        for (i, (ti, to)) in totals.iter().enumerate() {
            let (li, lo) = (ti - prev.0, to - prev.1);
            prev = (*ti, *to);
            s.push_str(&format!(
                r#"{{"timestamp":"2026-11-05T10:0{i}:00Z","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":{ti},"cached_input_tokens":{},"cache_write_input_tokens":0,"output_tokens":{to}}},"last_token_usage":{{"input_tokens":{li},"cached_input_tokens":{},"cache_write_input_tokens":0,"output_tokens":{lo}}}}}}}}}"#,
                ti / 2,
                li / 2
            ));
            s.push('\n');
        }
        s
    }

    fn totals(scan: &Scan, provider: ProviderKind) -> Tokens {
        let mut t = Tokens::default();
        for b in scan.buckets.iter().filter(|b| b.provider == provider) {
            t.add(&b.tokens);
        }
        t
    }

    #[test]
    fn claude_dedupes_repeats_and_reads_only_appended_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("projects").join("p");
        fs::create_dir_all(&proj).unwrap();
        let file = proj.join("a.jsonl");
        let mut roots = Roots::default();
        roots.add_claude(&dir.path().join("projects"));
        let cache = dir.path().join("cache.json");

        // Two repeats of message m1 (streaming) and one m2.
        let mut text = String::new();
        for l in [
            claude_line("m1", "r1", 5, "s1"),
            claude_line("m1", "r1", 5, "s1"),
            claude_line("m2", "r2", 7, "s1"),
        ] {
            text.push_str(&l);
            text.push('\n');
        }
        fs::write(&file, &text).unwrap();
        let s = scan(&roots, Some(&cache)).unwrap();
        let t = totals(&s, ProviderKind::Claude);
        assert_eq!(
            (t.input, t.output, t.cache_read, t.cache_write),
            (2, 12, 200, 20)
        );
        assert_eq!((s.files, s.files_read), (1, 1));
        assert_eq!(s.sessions.len(), 1);

        // Unchanged file: nothing read, same result from the cache.
        let s2 = scan(&roots, Some(&cache)).unwrap();
        assert_eq!(s2.files_read, 0);
        assert_eq!(totals(&s2, ProviderKind::Claude), t);

        // Append a new message and an unfinished line: only the complete new line counts.
        let mut f = fs::OpenOptions::new().append(true).open(&file).unwrap();
        writeln!(f, "{}", claude_line("m3", "r3", 3, "s2")).unwrap();
        write!(f, "{}", &claude_line("m4", "r4", 99, "s2")[..40]).unwrap();
        drop(f);
        let s3 = scan(&roots, Some(&cache)).unwrap();
        let t3 = totals(&s3, ProviderKind::Claude);
        assert_eq!((t3.input, t3.output), (3, 15));
        assert_eq!(s3.sessions.len(), 2);

        // The old message again in another file is not counted twice.
        fs::write(
            proj.join("b.jsonl"),
            format!("{}\n", claude_line("m1", "r1", 5, "s1")),
        )
        .unwrap();
        let s4 = scan(&roots, Some(&cache)).unwrap();
        assert_eq!(totals(&s4, ProviderKind::Claude).output, 15);

        // A shrunk file is read again from the start: its old messages are gone, the new one
        // counts (b.jsonl's m1 stays uncounted: a.jsonl owned it first; logs only ever grow).
        fs::write(&file, format!("{}\n", claude_line("m9", "r9", 1, "s3"))).unwrap();
        let s5 = scan(&roots, Some(&cache)).unwrap();
        assert_eq!(totals(&s5, ProviderKind::Claude).output, 1);
    }

    #[test]
    fn codex_counts_per_request_deltas_not_running_totals() {
        let dir = tempfile::tempdir().unwrap();
        let sessions = dir.path().join("sessions");
        fs::create_dir_all(&sessions).unwrap();
        let file = sessions.join("rollout-1.jsonl");
        let mut roots = Roots::default();
        roots.add_codex(&sessions);

        // Totals 100, 250 and a repeated 250 (duplicate event), outputs 10, 30, 30.
        let mut text = codex_lines(&[(100, 10), (250, 30), (250, 30)]);
        fs::write(&file, &text).unwrap();
        let s = scan(&roots, None).unwrap();
        let t = totals(&s, ProviderKind::Codex);
        assert_eq!(t.output, 30);
        assert_eq!(t.processed(), 250 + 30);
        assert_eq!(t.cache_read, 125);
        assert_eq!(s.buckets[0].model, "gpt-6.1-sol");

        // A continued session (cache on disk) adds only the delta.
        let cache = dir.path().join("c.json");
        let _ = scan(&roots, Some(&cache)).unwrap();
        text.push_str(
            r#"{"timestamp":"2026-11-05T10:09:00Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":400,"cached_input_tokens":200,"cache_write_input_tokens":0,"output_tokens":50},"last_token_usage":{"input_tokens":150,"cached_input_tokens":75,"cache_write_input_tokens":0,"output_tokens":20}}}}"#,
        );
        text.push('\n');
        fs::write(&file, &text).unwrap();
        let s2 = scan(&roots, Some(&cache)).unwrap();
        let t2 = totals(&s2, ProviderKind::Codex);
        assert_eq!(t2.output, 50);
        assert_eq!(t2.processed(), 400 + 50);
    }

    #[test]
    fn codex_reads_zstd_files() {
        let dir = tempfile::tempdir().unwrap();
        let sessions = dir.path().join("archived_sessions");
        fs::create_dir_all(&sessions).unwrap();
        let raw = codex_lines(&[(100, 10)]);
        let packed = zstd::encode_all(raw.as_bytes(), 3).unwrap();
        fs::write(sessions.join("rollout-2.jsonl.zst"), packed).unwrap();
        let mut roots = Roots::default();
        roots.add_codex(&sessions);
        let s = scan(&roots, None).unwrap();
        assert_eq!(totals(&s, ProviderKind::Codex).processed(), 110);
        assert_eq!(s.sessions[0].id, "rollout-2");
    }

    fn copilot_start(cwd: &str) -> String {
        format!(
            r#"{{"type":"session.start","timestamp":"2026-11-05T09:00:00Z","id":"a","data":{{"context":{{"cwd":{}}}}}}}"#,
            serde_json::to_string(cwd).unwrap()
        )
    }

    /// A shutdown snapshot: (model, input incl. cache, output, cache read, cache write).
    fn copilot_shutdown(hour: u32, models: &[(&str, u64, u64, u64, u64)]) -> String {
        let m: Vec<String> = models
            .iter()
            .map(|(name, i, o, r, w)| {
                format!(
                    r#""{name}":{{"requests":{{"count":1}},"usage":{{"inputTokens":{i},"outputTokens":{o},"cacheReadTokens":{r},"cacheWriteTokens":{w},"reasoningTokens":1}}}}"#
                )
            })
            .collect();
        format!(
            r#"{{"type":"session.shutdown","timestamp":"2026-11-05T{hour:02}:00:00Z","id":"b","data":{{"modelMetrics":{{{}}}}}}}"#,
            m.join(",")
        )
    }

    #[test]
    fn copilot_counts_shutdown_differences_and_reads_only_appended_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("session-state");
        let sess = state.join("sess-1");
        fs::create_dir_all(&sess).unwrap();
        fs::create_dir_all(state.join("sess-2")).unwrap();
        // Not a usage log: another file in a session folder.
        fs::write(sess.join("checkpoint.jsonl"), "{}\n").unwrap();
        let file = sess.join("events.jsonl");
        let mut roots = Roots::default();
        roots.add_copilot(&state);
        let cache = dir.path().join("cache.json");

        // Input 1000 = 600 cache read + 100 cache write + 300 uncached. Garbage lines are skipped.
        let first = [
            copilot_start(r"D:\Work\x"),
            "not json".to_string(),
            r#"{"type":"assistant.message","data":{"content":"SECRET"}}"#.to_string(),
            copilot_shutdown(10, &[("claude-sonnet-4.5", 1000, 50, 600, 100)]),
        ];
        fs::write(&file, first.join("\n") + "\n").unwrap();
        let s = scan(&roots, Some(&cache)).unwrap();
        assert_eq!(s.files, 1);
        assert_eq!(
            totals(&s, ProviderKind::Copilot),
            Tokens {
                input: 300,
                cache_read: 600,
                cache_write: 100,
                output: 50
            }
        );
        let b = &s.buckets[0];
        assert_eq!(&*b.thread, "sess-1");
        assert_eq!(b.model, "claude-sonnet-4.5");
        assert_eq!(b.kind, TaskKind::OtherTools);
        assert_eq!(b.cwd.as_deref(), Some(r"D:\Work\x"));
        assert_eq!(s.sessions.len(), 1);
        assert_eq!(s.sessions[0].provider, ProviderKind::Copilot);

        // Resumed: a second shutdown with higher totals, a second model, and an empty one. Only
        // the differences count; a re-scan reads just the appended bytes.
        let more = [
            copilot_shutdown(
                12,
                &[
                    ("claude-sonnet-4.5", 1500, 80, 900, 100),
                    ("gpt-5.1-codex", 200, 20, 0, 0),
                ],
            ),
            copilot_shutdown(13, &[]),
        ];
        let mut f = fs::OpenOptions::new().append(true).open(&file).unwrap();
        writeln!(f, "{}", more.join("\n")).unwrap();
        drop(f);
        let s = scan(&roots, Some(&cache)).unwrap();
        assert_eq!(s.files_read, 1);
        assert_eq!(
            totals(&s, ProviderKind::Copilot),
            Tokens {
                input: 300 + 200 + 200,
                cache_read: 900,
                cache_write: 100,
                output: 80 + 20
            }
        );
        let hours: BTreeSet<i64> = s.buckets.iter().map(|b| b.hour).collect();
        assert_eq!(hours.len(), 2);
        // Unchanged files are not read again and the total stays.
        let again = scan(&roots, Some(&cache)).unwrap();
        assert_eq!(again.files_read, 0);
        assert_eq!(
            totals(&again, ProviderKind::Copilot),
            totals(&s, ProviderKind::Copilot)
        );
    }

    #[test]
    fn copilot_counter_going_down_counts_the_new_snapshot_whole() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("session-state");
        fs::create_dir_all(&state).unwrap();
        // Older flat layout: `<uuid>.jsonl` next to the folders. Input smaller than its cache
        // buckets clamps the uncached part at 0.
        let lines = [
            copilot_shutdown(10, &[("m", 500, 10, 0, 0)]),
            copilot_shutdown(11, &[("m", 100, 5, 150, 0)]),
        ];
        fs::write(state.join("abc.jsonl"), lines.join("\n") + "\n").unwrap();
        let mut roots = Roots::default();
        roots.add_copilot(&state);
        let s = scan(&roots, None).unwrap();
        assert_eq!(
            totals(&s, ProviderKind::Copilot),
            Tokens {
                input: 500,
                cache_read: 150,
                cache_write: 0,
                output: 15
            }
        );
        assert_eq!(&*s.buckets[0].thread, "abc");
        assert!(s.buckets[0].cwd.is_none());
    }

    #[test]
    fn copilot_root_comes_from_the_copilot_home() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        fs::create_dir_all(home.join(".copilot").join("session-state")).unwrap();
        let r = discover_roots(home, &ProfileEnv::default());
        assert_eq!(r.copilot.len(), 1);
        // Without a session-state folder there is nothing to scan.
        let empty = tempfile::tempdir().unwrap();
        fs::create_dir_all(empty.path().join(".copilot")).unwrap();
        assert!(
            discover_roots(empty.path(), &ProfileEnv::default())
                .copilot
                .is_empty()
        );
    }

    #[test]
    fn roots_are_canonical_and_deduplicated() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        fs::create_dir_all(home.join(".claude").join("projects")).unwrap();
        fs::create_dir_all(home.join(".claude-other")).unwrap(); // no projects folder
        fs::create_dir_all(home.join(".codex").join("sessions")).unwrap();
        fs::create_dir_all(home.join(".codex").join("archived_sessions")).unwrap();
        fs::write(home.join(".claude.json"), "{}").unwrap();
        let r = discover_roots(home, &ProfileEnv::default());
        assert_eq!((r.claude.len(), r.codex.len()), (1, 2));
        let mut r2 = Roots::default();
        r2.add_claude(&home.join(".claude").join("projects"));
        r2.add_claude(
            &home
                .join(".claude")
                .join("projects")
                .join("..")
                .join("projects"),
        );
        assert_eq!(r2.claude.len(), 1);
    }

    fn claude_block(id: &str, block: &str, session: &str, extra: &str) -> String {
        format!(
            r#"{{"type":"assistant","timestamp":"2026-11-05T10:00:00Z","sessionId":"{session}","requestId":"r-{id}"{extra},"message":{{"id":"{id}","model":"claude-opus-5-5","content":[{block}],"usage":{{"input_tokens":3,"output_tokens":9,"cache_creation_input_tokens":0,"cache_read_input_tokens":0}}}}}}"#
        )
    }

    fn by_kind(scan: &Scan) -> BTreeMap<TaskKind, Tokens> {
        let mut m: BTreeMap<TaskKind, Tokens> = BTreeMap::new();
        for b in &scan.buckets {
            m.entry(b.kind).or_default().add(&b.tokens);
        }
        m
    }

    #[test]
    fn claude_steps_merge_blocks_split_by_kind_and_keep_dims() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("projects").join("p");
        fs::create_dir_all(proj.join("s1").join("subagents")).unwrap();
        let mut roots = Roots::default();
        roots.add_claude(&dir.path().join("projects"));
        let cwd = r#","cwd":"D:\\Github\\app\\src","isSidechain":false"#;
        let lines = [
            // m1: thinking, then Edit and Bash cargo in later lines with the same ids.
            claude_block(
                "m1",
                r#"{"type":"thinking","thinking":"SECRET"}"#,
                "s1",
                cwd,
            ),
            claude_block(
                "m1",
                r#"{"type":"tool_use","name":"Edit","input":{"file_path":"x"}}"#,
                "s1",
                cwd,
            ),
            claude_block(
                "m1",
                r#"{"type":"tool_use","name":"Bash","input":{"command":"cargo test"}}"#,
                "s1",
                cwd,
            ),
            // m2: text only (Talking), no cwd.
            claude_block("m2", r#"{"type":"text","text":"SECRET"}"#, "s1", ""),
            r#"{"type":"custom-title","customTitle":"Fix the login","sessionId":"s1"}"#.to_string(),
        ];
        fs::write(proj.join("s1.jsonl"), lines.join("\n") + "\n").unwrap();
        // A subagent file of the same thread.
        let side = claude_block(
            "m3",
            r#"{"type":"tool_use","name":"Grep","input":{"pattern":"SECRET"}}"#,
            "s1",
            r#","isSidechain":true,"cwd":"D:\\Github\\app""#,
        );
        fs::write(
            proj.join("s1").join("subagents").join("agent-a.jsonl"),
            side + "\n",
        )
        .unwrap();

        let cache = dir.path().join("c.json");
        let s = scan(&roots, Some(&cache)).unwrap();
        // Totals unchanged by the split: 3 messages x (3 in, 9 out).
        let t = totals(&s, ProviderKind::Claude);
        assert_eq!((t.input, t.output), (9, 27));
        let k = by_kind(&s);
        // m1's 3 in / 9 out split over Edit and Bash cargo: remainder on the first kind.
        assert_eq!(
            (
                k[&TaskKind::WritingCode].input,
                k[&TaskKind::WritingCode].output
            ),
            (2, 5)
        );
        assert_eq!(
            (
                k[&TaskKind::BuildsTests].input,
                k[&TaskKind::BuildsTests].output
            ),
            (1, 4)
        );
        assert_eq!(k[&TaskKind::Talking].output, 9);
        assert_eq!(k[&TaskKind::ReadingSearching].output, 9);
        let sub: Vec<_> = s.buckets.iter().filter(|b| b.sidechain).collect();
        assert_eq!(sub.len(), 1);
        assert_eq!(sub[0].kind, TaskKind::ReadingSearching);
        assert_eq!(sub[0].cwd.as_deref(), Some(r"D:\Github\app"));
        assert!(s.buckets.iter().all(|b| &*b.thread == "s1"));
        let talking = s
            .buckets
            .iter()
            .find(|b| b.kind == TaskKind::Talking)
            .unwrap();
        assert_eq!(talking.cwd, None);
        assert_eq!(
            s.titles.get("s1").map(String::as_str),
            Some("Fix the login")
        );
        // One thread (the subagent file shares the session id).
        assert_eq!(s.sessions.len(), 1);

        // From the cache: the same buckets and titles; nothing kept of the text.
        let s2 = scan(&roots, Some(&cache)).unwrap();
        assert_eq!(s2.files_read, 0);
        assert_eq!(s2.buckets, s.buckets);
        assert_eq!(s2.titles, s.titles);
        let raw = fs::read_to_string(&cache).unwrap();
        assert!(!raw.contains("SECRET") && !raw.contains("cargo test"));
        assert!(raw.contains("\"version\":2"));
    }

    #[test]
    fn old_cache_versions_are_read_again() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("projects").join("p");
        fs::create_dir_all(&proj).unwrap();
        fs::write(
            proj.join("a.jsonl"),
            format!("{}\n", claude_line("m1", "r1", 5, "s1")),
        )
        .unwrap();
        let mut roots = Roots::default();
        roots.add_claude(&dir.path().join("projects"));
        let cache = dir.path().join("c.json");
        fs::write(&cache, r#"{"version":1,"files":{}}"#).unwrap();
        let s = scan(&roots, Some(&cache)).unwrap();
        assert_eq!(s.files_read, 1);
        assert_eq!(totals(&s, ProviderKind::Claude).output, 5);
    }

    #[test]
    fn codex_tool_calls_and_cwd_attach_to_the_next_token_count() {
        let dir = tempfile::tempdir().unwrap();
        let sessions = dir.path().join("sessions");
        fs::create_dir_all(&sessions).unwrap();
        let file = sessions.join("rollout-9.jsonl");
        let mut roots = Roots::default();
        roots.add_codex(&sessions);
        let count = |i: u64, total: u64| {
            format!(
                r#"{{"timestamp":"2026-11-05T10:0{i}:00Z","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":{total},"cached_input_tokens":0,"output_tokens":{total}}},"last_token_usage":{{"input_tokens":10,"cached_input_tokens":0,"output_tokens":10}}}}}}}}"#
            )
        };
        let text = [
            r#"{"timestamp":"2026-11-05T10:00:00Z","type":"turn_context","payload":{"model":"gpt-6.1-sol","cwd":"C:\\work\\api"}}"#.to_string(),
            r#"{"type":"response_item","payload":{"type":"function_call","name":"exec_command","arguments":"{\"cmd\":\"cargo build --SECRET\"}"}}"#.to_string(),
            r#"{"type":"response_item","payload":{"type":"custom_tool_call","name":"apply_patch","input":"SECRET"}}"#.to_string(),
            count(1, 10),
            count(2, 20),
        ]
        .join("\n")
            + "\n";
        let cache = dir.path().join("c.json");
        fs::write(&file, &text).unwrap();
        let s = scan(&roots, Some(&cache)).unwrap();
        let k = by_kind(&s);
        assert_eq!(k[&TaskKind::BuildsTests].output, 5);
        assert_eq!(k[&TaskKind::WritingCode].output, 5);
        assert_eq!(k[&TaskKind::Talking].output, 10);
        assert!(
            s.buckets
                .iter()
                .all(|b| b.cwd.as_deref() == Some(r"C:\work\api") && &*b.thread == "rollout-9")
        );

        // A tool call read in one scan belongs to the token count appended later.
        let more = r#"{"type":"response_item","payload":{"type":"web_search_call"}}"#;
        fs::write(&file, format!("{text}{more}\n")).unwrap();
        let _ = scan(&roots, Some(&cache)).unwrap();
        fs::write(&file, format!("{text}{more}\n{}\n", count(3, 30))).unwrap();
        let s2 = scan(&roots, Some(&cache)).unwrap();
        assert_eq!(by_kind(&s2)[&TaskKind::WebResearch].output, 10);
        assert!(!fs::read_to_string(&cache).unwrap().contains("SECRET"));
    }

    #[test]
    fn split_keeps_totals_exact() {
        let t = Tokens {
            input: 10,
            cache_read: 7,
            cache_write: 1,
            output: 100,
        };
        let parts = split_tokens(
            &t,
            &[
                (TaskKind::WritingCode, 1),
                (TaskKind::BuildsTests, 1),
                (TaskKind::OtherTools, 1),
            ],
        );
        let mut sum = Tokens::default();
        for (_, p) in &parts {
            sum.add(p);
        }
        assert_eq!(sum, t);
        assert_eq!(parts[0].1.output, 34);
        assert_eq!(parts[1].1.output, 33);
        assert_eq!(
            split_tokens(&t, &[(TaskKind::Talking, 1)]),
            vec![(TaskKind::Talking, t)]
        );
    }
}

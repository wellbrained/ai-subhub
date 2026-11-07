//! Log discovery, incremental per-file cache and aggregation into hourly per-model buckets.
//!
//! Each file keeps its own aggregated buckets plus how many bytes were read, so a later scan reads
//! only appended bytes. A file that shrank (or whose size and time disagree) is read again from
//! the start; a file that vanished drops out (a Codex session moved to `archived_sessions` is the
//! same data under a new path, so keeping the old entry would count it twice).
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, UNIX_EPOCH};

use anyhow::Context;
use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};

use super::parse::{ClaudeMessage, CodexEvent, RawUsage, parse_claude_line, parse_codex_line};
use super::{Tokens, hour_index};
use crate::model::ProviderKind;

const CACHE_VERSION: u32 = 1;

/// Aggregated tokens of one file for one local hour and model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileBucket {
    pub hour: i64,
    pub model: String,
    pub tokens: Tokens,
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
    /// Codex: running total after the previous token count, to take per-request deltas.
    #[serde(default)]
    pub prev_total: Option<RawUsage>,
    /// Claude: hashes of message id + request id already counted from this file.
    #[serde(default)]
    pub seen: Vec<u64>,
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
            prev_total: None,
            seen: vec![],
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

/// One merged bucket: all files, one local hour, one provider and model.
#[derive(Debug, Clone, PartialEq)]
pub struct Bucket {
    pub hour: i64,
    pub provider: ProviderKind,
    pub model: String,
    pub tokens: Tokens,
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
    /// Log files found.
    pub files: usize,
    /// Files read (new or grown) in this scan.
    pub files_read: usize,
    pub elapsed: Duration,
    pub finished_at: Option<DateTime<Utc>>,
}

/// Where logs live: Claude config dirs' `projects` folders and Codex `sessions` folders, already
/// canonical and without duplicates (the owner's Claude account folders share one `projects`).
#[derive(Debug, Clone, Default)]
pub struct Roots {
    pub claude: Vec<PathBuf>,
    pub codex: Vec<PathBuf>,
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
}

/// `<home>\.claude*\projects` and `<home>\.codex*\sessions` + `archived_sessions`.
pub fn discover_roots(home: &Path) -> Roots {
    let mut roots = Roots::default();
    let mut names: Vec<_> = fs::read_dir(home)
        .map(|rd| rd.flatten().map(|e| e.file_name()).collect())
        .unwrap_or_default();
    names.sort();
    for n in names {
        let name = n.to_string_lossy();
        let dir = home.join(&n);
        if name.starts_with(".claude") {
            roots.add_claude(&dir.join("projects"));
        } else if name.starts_with(".codex") {
            roots.add_codex(&dir.join("sessions"));
            roots.add_codex(&dir.join("archived_sessions"));
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
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .context("No home folder")?;
    scan(&discover_roots(&home), Some(&cache_path()))
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
        };
        if !ok {
            continue;
        }
        let key = path.to_string_lossy().into_owned();
        if !seen.insert(key.clone()) {
            continue;
        }
        let Ok(md) = e.metadata() else { continue };
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

/// Per-file accumulation of lines into buckets.
struct FileProc {
    entry: FileEntry,
    buckets: HashMap<(i64, String), Tokens>,
    sessions: HashMap<String, BTreeSet<i64>>,
    /// Session id when a line has none (the file name).
    default_session: String,
}

fn local_hour(ts: DateTime<Utc>) -> i64 {
    hour_index(ts.with_timezone(&Local).naive_local())
}

impl FileProc {
    fn new(entry: FileEntry, default_session: String) -> Self {
        let buckets = entry
            .buckets
            .iter()
            .map(|b| ((b.hour, b.model.clone()), b.tokens))
            .collect();
        let sessions = entry
            .sessions
            .iter()
            .map(|s| (s.id.clone(), s.hours.iter().copied().collect()))
            .collect();
        Self {
            entry,
            buckets,
            sessions,
            default_session,
        }
    }

    fn add(&mut self, hour: i64, model: &str, session: &str, t: &Tokens) {
        self.buckets
            .entry((hour, model.to_string()))
            .or_default()
            .add(t);
        self.sessions
            .entry(session.to_string())
            .or_default()
            .insert(hour);
    }

    /// One Claude log line. `seen` holds message hashes already counted (streamed messages repeat).
    fn feed_claude(&mut self, line: &[u8], seen: &Mutex<HashSet<u64>>) {
        if memchr::memmem::find(line, b"\"usage\"").is_none()
            || memchr::memmem::find(line, b"\"assistant\"").is_none()
        {
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
        }) = parse_claude_line(text)
        else {
            return;
        };
        if let Some(k) = key {
            let fresh = seen.lock().map(|mut s| s.insert(k)).unwrap_or(true);
            if !fresh {
                return;
            }
            self.entry.seen.push(k);
        }
        let session = session.unwrap_or_else(|| self.default_session.clone());
        self.add(local_hour(ts), &model, &session, &tokens);
    }

    /// One Codex log line: model changes and token counts (per-request deltas of the running total).
    fn feed_codex(&mut self, line: &[u8]) {
        if memchr::memmem::find(line, b"\"token_count\"").is_none()
            && memchr::memmem::find(line, b"\"turn_context\"").is_none()
        {
            return;
        }
        let Ok(text) = std::str::from_utf8(line) else {
            return;
        };
        match parse_codex_line(text) {
            Some(CodexEvent::Model(m)) => self.entry.model = Some(m),
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
                self.add(local_hour(ts), &model, &session, &d.tokens());
            }
            None => {}
        }
    }

    fn finish(mut self) -> FileEntry {
        let mut buckets: Vec<_> = self
            .buckets
            .into_iter()
            .map(|((hour, model), tokens)| FileBucket {
                hour,
                model,
                tokens,
            })
            .collect();
        buckets.sort_by(|a, b| (a.hour, &a.model).cmp(&(b.hour, &b.model)));
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
    let stem = stem.split(".jsonl").next().unwrap_or(&stem).to_string();
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

fn merge(files: &BTreeMap<String, FileEntry>) -> Scan {
    let mut buckets: HashMap<(i64, bool, String), Tokens> = HashMap::new();
    let mut sessions: HashMap<(bool, String), BTreeSet<i64>> = HashMap::new();
    let is_claude = |p: ProviderKind| p == ProviderKind::Claude;
    for e in files.values() {
        for b in &e.buckets {
            buckets
                .entry((b.hour, is_claude(e.provider), b.model.clone()))
                .or_default()
                .add(&b.tokens);
        }
        for s in &e.sessions {
            sessions
                .entry((is_claude(e.provider), s.id.clone()))
                .or_default()
                .extend(s.hours.iter().copied());
        }
    }
    let provider = |claude: bool| {
        if claude {
            ProviderKind::Claude
        } else {
            ProviderKind::Codex
        }
    };
    let mut out_b: Vec<Bucket> = buckets
        .into_iter()
        .map(|((hour, c, model), tokens)| Bucket {
            hour,
            provider: provider(c),
            model,
            tokens,
        })
        .collect();
    out_b.sort_by(|a, b| (a.hour, &a.model).cmp(&(b.hour, &b.model)));
    let mut out_s: Vec<ScanSession> = sessions
        .into_iter()
        .map(|((c, id), hours)| ScanSession {
            provider: provider(c),
            id,
            hours: hours.into_iter().collect(),
        })
        .collect();
    out_s.sort_by(|a, b| a.id.cmp(&b.id));
    Scan {
        buckets: out_b,
        sessions: out_s,
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

    #[test]
    fn roots_are_canonical_and_deduplicated() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        fs::create_dir_all(home.join(".claude").join("projects")).unwrap();
        fs::create_dir_all(home.join(".claude-other")).unwrap(); // no projects folder
        fs::create_dir_all(home.join(".codex").join("sessions")).unwrap();
        fs::create_dir_all(home.join(".codex").join("archived_sessions")).unwrap();
        fs::write(home.join(".claude.json"), "{}").unwrap();
        let r = discover_roots(home);
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
}

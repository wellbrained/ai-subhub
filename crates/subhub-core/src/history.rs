//! Reading history: every successful reading is appended to
//! `%LOCALAPPDATA%\AI SubHub\readings-history.jsonl` (one line per account, window and
//! fetch) so the Limits tab can chart it later. Append-only; the oldest lines are trimmed past a
//! cap. Holds only ids, times and percentages, never secrets.
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::model::Reading;

/// Lines kept after a trim; a trim happens when a tenth more has accumulated.
pub const DEFAULT_CAP: usize = 100_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryLine {
    pub account: Uuid,
    pub fetched_at: DateTime<Utc>,
    pub window: String,
    pub used_percent: f64,
    pub resets_at: Option<DateTime<Utc>>,
}

pub fn default_path() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("AI SubHub").join("readings-history.jsonl")
}

pub struct History {
    path: PathBuf,
    cap: usize,
    /// Lines in the file (counted once, then tracked).
    lines: usize,
    /// Newest `fetched_at` already written per account (dedupe).
    last: HashMap<Uuid, DateTime<Utc>>,
}

impl History {
    pub fn open(path: PathBuf) -> Self {
        Self::with_cap(path, DEFAULT_CAP)
    }

    pub fn with_cap(path: PathBuf, cap: usize) -> Self {
        let lines = count_lines(&path);
        let last = tail_last_fetches(&path);
        Self {
            path,
            cap: cap.max(1),
            lines,
            last,
        }
    }

    /// Readings stored.
    pub fn len(&self) -> usize {
        self.lines
    }

    pub fn is_empty(&self) -> bool {
        self.lines == 0
    }

    /// Appends one line per window of `reading` unless this account's reading from that time was
    /// already recorded. Returns whether anything was written.
    pub fn record(&mut self, account: Uuid, reading: &Reading) -> bool {
        if self
            .last
            .get(&account)
            .is_some_and(|t| *t >= reading.fetched_at)
            || reading.windows.is_empty()
        {
            return false;
        }
        let mut text = String::new();
        for w in &reading.windows {
            let line = HistoryLine {
                account,
                fetched_at: reading.fetched_at,
                window: w.key.clone(),
                used_percent: w.used_percent,
                resets_at: w.resets_at,
            };
            if let Ok(s) = serde_json::to_string(&line) {
                text.push_str(&s);
                text.push('\n');
            }
        }
        if let Some(dir) = self.path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let ok = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .and_then(|mut f| f.write_all(text.as_bytes()))
            .is_ok();
        if !ok {
            return false;
        }
        self.last.insert(account, reading.fetched_at);
        self.lines += reading.windows.len();
        if self.lines > self.cap + self.cap / 10 {
            self.trim();
        }
        true
    }

    /// Keeps the newest `cap` lines (atomic rewrite).
    fn trim(&mut self) {
        let Ok(file) = File::open(&self.path) else {
            return;
        };
        let all: Vec<String> = BufReader::new(file).lines().map_while(Result::ok).collect();
        let keep = &all[all.len().saturating_sub(self.cap)..];
        let mut tmp = self.path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        let mut out = keep.join("\n");
        out.push('\n');
        if fs::write(&tmp, out).is_ok() && fs::rename(&tmp, &self.path).is_ok() {
            self.lines = keep.len();
        }
    }
}

fn count_lines(path: &Path) -> usize {
    let Ok(mut f) = File::open(path) else {
        return 0;
    };
    let mut buf = vec![0u8; 1 << 16];
    let mut n = 0;
    while let Ok(k) = f.read(&mut buf) {
        if k == 0 {
            break;
        }
        n += buf[..k].iter().filter(|b| **b == b'\n').count();
    }
    n
}

/// Newest fetch per account within the last 256 KiB (enough for dedupe).
fn tail_last_fetches(path: &Path) -> HashMap<Uuid, DateTime<Utc>> {
    let mut map = HashMap::new();
    let Ok(mut f) = File::open(path) else {
        return map;
    };
    let len = f.metadata().map_or(0, |m| m.len());
    let from = len.saturating_sub(256 * 1024);
    if f.seek(SeekFrom::Start(from)).is_err() {
        return map;
    }
    let mut text = String::new();
    let mut bytes = Vec::new();
    if f.read_to_end(&mut bytes).is_err() {
        return map;
    }
    text.push_str(&String::from_utf8_lossy(&bytes));
    for line in text.lines() {
        if let Ok(l) = serde_json::from_str::<HistoryLine>(line) {
            let e = map.entry(l.account).or_insert(l.fetched_at);
            if l.fetched_at > *e {
                *e = l.fetched_at;
            }
        }
    }
    map
}

// ---------- reading side: series for the Limits chart ----------

/// One charted reading: when it was fetched and the % used.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SeriesPoint {
    pub at: DateTime<Utc>,
    pub used: f64,
}

/// One account's weekly (all-models) and 5-hour readings in a range, oldest first.
#[derive(Debug, Clone, PartialEq)]
pub struct AccountSeries {
    pub account: Uuid,
    pub weekly: Vec<SeriesPoint>,
    pub five_hour: Vec<SeriesPoint>,
}

/// Builds the series from history lines, keeping `since <= fetched_at <= until`. Lines that do not
/// parse and windows other than the weekly / 5-hour ones (`insights::readings::kind_of`) are
/// ignored. Accounts come out in id order, points in time order.
pub fn build_series<'a>(
    lines: impl IntoIterator<Item = &'a str>,
    since: DateTime<Utc>,
    until: DateTime<Utc>,
) -> Vec<AccountSeries> {
    let kept: Vec<HistoryLine> = lines
        .into_iter()
        .filter_map(|line| serde_json::from_str::<HistoryLine>(line).ok())
        .filter(|l| l.fetched_at >= since && l.fetched_at <= until && l.used_percent.is_finite())
        .collect();
    // Kind per account and window key (Codex keys get theirs from how far ahead they reset).
    let mut grouped: std::collections::BTreeMap<(Uuid, &str), Vec<&HistoryLine>> =
        Default::default();
    for l in &kept {
        grouped.entry((l.account, &l.window)).or_default().push(l);
    }
    let kinds: std::collections::HashMap<(Uuid, &str), bool> = grouped
        .iter()
        .filter_map(|(k, ls)| Some((*k, crate::insights::readings::kind_of(k.1, ls)?)))
        .collect();
    let mut map: std::collections::BTreeMap<Uuid, AccountSeries> = Default::default();
    for l in &kept {
        let Some(&session) = kinds.get(&(l.account, l.window.as_str())) else {
            continue;
        };
        let weekly = !session;
        let s = map.entry(l.account).or_insert_with(|| AccountSeries {
            account: l.account,
            weekly: Vec::new(),
            five_hour: Vec::new(),
        });
        let p = SeriesPoint {
            at: l.fetched_at,
            used: l.used_percent.clamp(0.0, 100.0),
        };
        if weekly {
            &mut s.weekly
        } else {
            &mut s.five_hour
        }
        .push(p);
    }
    let mut out: Vec<AccountSeries> = map.into_values().collect();
    for s in &mut out {
        s.weekly.sort_by_key(|p| p.at);
        s.five_hour.sort_by_key(|p| p.at);
    }
    out
}

/// Reads the history file at `path` (streamed line by line) and builds the series for the range.
/// A missing file gives no series. Blocking: call it from a background executor.
pub fn load_series(path: &Path, since: DateTime<Utc>, until: DateTime<Utc>) -> Vec<AccountSeries> {
    let Ok(file) = File::open(path) else {
        return Vec::new();
    };
    let lines: Vec<String> = BufReader::new(file).lines().map_while(Result::ok).collect();
    build_series(lines.iter().map(String::as_str), since, until)
}

/// Every line fetched at or after `since`, in file order (the Overview analysis input). Lines
/// that do not parse are skipped; a missing file gives none. Blocking: call it from a background
/// executor.
pub fn load_lines(path: &Path, since: DateTime<Utc>) -> Vec<HistoryLine> {
    let Ok(file) = File::open(path) else {
        return Vec::new();
    };
    BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter_map(|l| serde_json::from_str::<HistoryLine>(&l).ok())
        .filter(|l| l.fetched_at >= since)
        .collect()
}

/// Splits points into runs wherever two neighbours are more than `max_gap` apart, so a chart
/// does not draw a line across time the app was not running.
pub fn split_gaps(points: &[SeriesPoint], max_gap: chrono::Duration) -> Vec<&[SeriesPoint]> {
    let mut runs = Vec::new();
    let mut start = 0;
    for i in 1..points.len() {
        if points[i].at - points[i - 1].at > max_gap {
            runs.push(&points[start..i]);
            start = i;
        }
    }
    if !points.is_empty() {
        runs.push(&points[start..]);
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AllowanceWindow;

    fn line(acc: Uuid, at: &str, window: &str, used: f64) -> String {
        serde_json::to_string(&HistoryLine {
            account: acc,
            fetched_at: at.parse().unwrap(),
            window: window.into(),
            used_percent: used,
            resets_at: None,
        })
        .unwrap()
    }

    #[test]
    fn series_filter_range_group_and_order() {
        let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let lines = [
            line(b, "2026-11-07T10:00:00Z", "weekly_all", 40.0),
            line(a, "2026-11-07T11:00:00Z", "seven_day", 20.0),
            line(a, "2026-11-07T09:00:00Z", "seven_day", 10.0),
            line(a, "2026-11-07T09:00:00Z", "five_hour", 90.0),
            line(a, "2026-11-07T09:00:00Z", "seven_day_opus", 5.0),
            line(a, "2026-11-01T09:00:00Z", "seven_day", 1.0),
            "not json".to_string(),
            String::new(),
            line(a, "2026-11-07T12:00:00Z", "session", 130.0),
        ];
        let since = "2026-11-07T00:00:00Z".parse().unwrap();
        let until = "2026-11-07T23:00:00Z".parse().unwrap();
        let s = build_series(lines.iter().map(String::as_str), since, until);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].account, a);
        let weekly: Vec<f64> = s[0].weekly.iter().map(|p| p.used).collect();
        assert_eq!(weekly, [10.0, 20.0]);
        let five: Vec<f64> = s[0].five_hour.iter().map(|p| p.used).collect();
        assert_eq!(five, [90.0, 100.0]);
        assert_eq!(s[1].account, b);
        assert_eq!(s[1].weekly.len(), 1);
        assert!(s[1].five_hour.is_empty());
    }

    #[test]
    fn codex_series_by_how_far_ahead_they_reset() {
        let a = Uuid::from_u128(1);
        let at = |t: &str, window: &str, ahead_h: i64| {
            let fetched: DateTime<Utc> = t.parse().unwrap();
            serde_json::to_string(&HistoryLine {
                account: a,
                fetched_at: fetched,
                window: window.into(),
                used_percent: 30.0,
                resets_at: Some(fetched + chrono::Duration::hours(ahead_h)),
            })
            .unwrap()
        };
        let lines = [
            at("2026-11-07T09:00:00Z", "codex:primary", 4),
            at("2026-11-07T09:00:00Z", "codex:secondary", 100),
            at("2026-11-07T10:00:00Z", "codex:primary", 3),
        ];
        let since = "2026-11-07T00:00:00Z".parse().unwrap();
        let until = "2026-11-07T23:00:00Z".parse().unwrap();
        let s = build_series(lines.iter().map(String::as_str), since, until);
        assert_eq!((s[0].five_hour.len(), s[0].weekly.len()), (2, 1));
    }

    #[test]
    fn load_series_reads_the_file_and_tolerates_a_missing_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("h.jsonl");
        let (since, until) = (
            "2026-11-07T00:00:00Z".parse().unwrap(),
            "2026-11-08T00:00:00Z".parse().unwrap(),
        );
        assert!(load_series(&path, since, until).is_empty());
        assert!(load_lines(&path, since).is_empty());
        let a = Uuid::new_v4();
        let mut h = History::open(path.clone());
        h.record(a, &reading("2026-11-07T10:00:00Z", 10.0));
        h.record(a, &reading("2026-11-07T10:05:00Z", 12.0));
        assert_eq!(load_lines(&path, since).len(), 4);
        assert_eq!(
            load_lines(&path, "2026-11-07T10:01:00Z".parse().unwrap()).len(),
            2
        );
        let s = load_series(&path, since, until);
        assert_eq!(
            (s.len(), s[0].weekly.len(), s[0].five_hour.len()),
            (1, 2, 2)
        );
    }

    #[test]
    fn gaps_split_runs_but_drops_stay_in_one_run() {
        let p = |t: &str, used| SeriesPoint {
            at: t.parse().unwrap(),
            used,
        };
        let pts = [
            p("2026-11-07T10:00:00Z", 90.0),
            p("2026-11-07T10:05:00Z", 2.0), // a reset: stays connected
            p("2026-11-07T10:10:00Z", 3.0),
            p("2026-11-07T20:00:00Z", 8.0), // app was off
        ];
        let runs = split_gaps(&pts, chrono::Duration::hours(3));
        assert_eq!(runs.iter().map(|r| r.len()).collect::<Vec<_>>(), [3, 1]);
        assert!(split_gaps(&[], chrono::Duration::hours(3)).is_empty());
    }

    fn reading(at: &str, used: f64) -> Reading {
        Reading {
            fetched_at: at.parse().unwrap(),
            windows: vec![
                AllowanceWindow {
                    key: "five_hour".into(),
                    label: "Session".into(),
                    used_percent: used,
                    resets_at: Some("2026-11-07T15:00:00Z".parse().unwrap()),
                },
                AllowanceWindow {
                    key: "seven_day".into(),
                    label: "Weekly".into(),
                    used_percent: used / 2.0,
                    resets_at: None,
                },
            ],
        }
    }

    #[test]
    fn appends_dedupes_per_account_and_fetch_time() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("h.jsonl");
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let mut h = History::open(path.clone());
        assert!(h.is_empty());
        assert!(h.record(a, &reading("2026-11-07T10:00:00Z", 10.0)));
        assert!(!h.record(a, &reading("2026-11-07T10:00:00Z", 10.0)));
        assert!(h.record(b, &reading("2026-11-07T10:00:00Z", 20.0)));
        assert!(h.record(a, &reading("2026-11-07T10:05:00Z", 12.0)));
        assert_eq!(h.len(), 6);

        // A new process picks up the count and the dedupe state from the file.
        let mut again = History::open(path.clone());
        assert_eq!(again.len(), 6);
        assert!(!again.record(a, &reading("2026-11-07T10:05:00Z", 12.0)));
        let first = fs::read_to_string(&path).unwrap();
        let line: HistoryLine = serde_json::from_str(first.lines().next().unwrap()).unwrap();
        assert_eq!((line.account, line.window.as_str()), (a, "five_hour"));
    }

    #[test]
    fn trims_oldest_lines_past_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("h.jsonl");
        let a = Uuid::new_v4();
        let mut h = History::with_cap(path.clone(), 10);
        for i in 0..20 {
            let at = format!("2026-11-07T10:{i:02}:00Z");
            h.record(a, &reading(&at, i as f64));
        }
        // Each reading is 2 lines; trimming kept the newest <= cap + 10%.
        assert!(h.len() <= 12, "{}", h.len());
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), h.len());
        // The newest reading survived, the oldest did not.
        assert!(text.contains("10:19:00"));
        assert!(!text.contains("10:00:00"));
    }
}

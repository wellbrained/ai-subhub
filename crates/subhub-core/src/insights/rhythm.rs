//! Work rhythm and the warm-up suggestion (plan "Analysis", owner 2026-11-07): when the day's
//! first 5-hour window usually starts and runs out, and which start time for a warm-up window
//! would make its reset land at that moment.
//!
//! Workdays are Monday to Friday (local) with a window start or log activity; weekend use does
//! not shape the suggestion.
use std::collections::{BTreeMap, HashMap};

use chrono::{
    DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Timelike, Utc, Weekday,
};
use uuid::Uuid;

use super::readings::{SESSION_LEN_H, SessionWindow, WindowEnd};
use super::zone::Zone;

/// Workdays the usual start and first hit are taken over.
pub const RHYTHM_DAYS: usize = 10;
/// Recent workdays whose first window must hit ...
pub const RECENT_DAYS: usize = 5;
/// ... at least this often for a warm-up suggestion.
pub const RECENT_HITS: usize = 2;
/// Suggested times are rounded down to this many minutes.
pub const ROUND_MIN: i64 = 15;
/// A warm-up never starts more than this long before the usual start.
pub const MAX_BEFORE_START_H: i64 = 4;

pub fn is_workday(d: NaiveDate) -> bool {
    !matches!(d.weekday(), Weekday::Sat | Weekday::Sun)
}

/// The first 5-hour window of a workday.
#[derive(Debug, Clone, PartialEq)]
pub struct FirstWindow {
    pub account: Uuid,
    pub start: DateTime<Utc>,
    pub resets_at: DateTime<Utc>,
    /// Hit - start, when it hit.
    pub hit_after: Option<Duration>,
    pub hit_at: Option<DateTime<Utc>>,
    pub waited: Option<Duration>,
}

/// When a workday started: the earliest window start, else the first hour with log activity.
#[derive(Debug, Clone, PartialEq)]
pub struct DayStart {
    pub date: NaiveDate,
    /// Local time.
    pub start: NaiveTime,
    pub from_logs: bool,
    pub first: Option<FirstWindow>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rhythm {
    /// The last [`RHYTHM_DAYS`] workdays, oldest first.
    pub days: Vec<DayStart>,
    /// Median start (local).
    pub usual_start: Option<NaiveTime>,
    /// Median time from the first window's start to its hit, over the days it hit.
    pub usual_first_hit: Option<Duration>,
    /// Of the last [`RECENT_DAYS`] workdays, on how many the first window hit.
    pub recent_first_hits: usize,
    pub recent_days: usize,
}

fn minutes(t: NaiveTime) -> i64 {
    (t.hour() * 60 + t.minute()) as i64
}

fn time_of(min: i64) -> NaiveTime {
    let m = min.clamp(0, 24 * 60 - 1);
    NaiveTime::from_hms_opt((m / 60) as u32, (m % 60) as u32, 0).unwrap_or(NaiveTime::MIN)
}

/// Median, the mean of the middle two for an even count.
fn median(mut v: Vec<i64>) -> Option<i64> {
    if v.is_empty() {
        return None;
    }
    v.sort();
    let n = v.len();
    Some(if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2
    })
}

/// The rhythm from the windows of the counted accounts and the first log hour per local day
/// (`log_starts`), up to `today`.
pub fn rhythm(
    windows: &[&SessionWindow],
    log_starts: &BTreeMap<NaiveDate, NaiveTime>,
    zone: &Zone,
    today: NaiveDate,
) -> Rhythm {
    let mut firsts: BTreeMap<NaiveDate, &SessionWindow> = BTreeMap::new();
    for w in windows {
        let date = zone.local(w.start).date();
        if !is_workday(date) || date > today {
            continue;
        }
        let e = firsts.entry(date).or_insert(w);
        let better =
            (w.start, w.hit_at.is_none(), w.account) < (e.start, e.hit_at.is_none(), e.account);
        if better {
            *e = w;
        }
    }
    let mut days: BTreeMap<NaiveDate, DayStart> = BTreeMap::new();
    for (date, w) in &firsts {
        days.insert(
            *date,
            DayStart {
                date: *date,
                start: zone.local(w.start).time(),
                from_logs: false,
                first: Some(FirstWindow {
                    account: w.account,
                    start: w.start,
                    resets_at: w.resets_at,
                    hit_after: w.hit_at.map(|h| h - w.start),
                    hit_at: w.hit_at,
                    waited: w.waited,
                }),
            },
        );
    }
    for (date, start) in log_starts {
        if is_workday(*date) && *date <= today {
            days.entry(*date).or_insert(DayStart {
                date: *date,
                start: *start,
                from_logs: true,
                first: None,
            });
        }
    }
    let all: Vec<DayStart> = days.into_values().collect();
    let days: Vec<DayStart> = all[all.len().saturating_sub(RHYTHM_DAYS)..].to_vec();
    let usual_start = median(days.iter().map(|d| minutes(d.start)).collect()).map(time_of);
    let usual_first_hit = median(
        days.iter()
            .filter_map(|d| d.first.as_ref()?.hit_after)
            .map(|d| d.num_minutes())
            .collect(),
    )
    .map(Duration::minutes);
    let recent = &days[days.len().saturating_sub(RECENT_DAYS)..];
    Rhythm {
        recent_first_hits: recent
            .iter()
            .filter(|d| d.first.as_ref().is_some_and(|f| f.hit_after.is_some()))
            .count(),
        recent_days: recent.len(),
        days,
        usual_start,
        usual_first_hit,
    }
}

/// The warm-up time: when the first window hit on at least 2 of the last 5 workdays, the time
/// T with T + 5 h ~ usual start + usual first hit, rounded down to 15 minutes and never more
/// than 4 hours before the usual start (rounded up to the grid). None when it would not start
/// before the usual start.
pub fn suggest_time(r: &Rhythm) -> Option<NaiveTime> {
    if r.recent_first_hits < RECENT_HITS {
        return None;
    }
    let start = minutes(r.usual_start?);
    let hit = r.usual_first_hit?.num_minutes();
    let raw = (start + hit - SESSION_LEN_H * 60).div_euclid(ROUND_MIN) * ROUND_MIN;
    let lowest =
        (start - MAX_BEFORE_START_H * 60 + ROUND_MIN - 1).div_euclid(ROUND_MIN) * ROUND_MIN;
    let t = raw.max(lowest).max(0);
    (t < start).then(|| time_of(t))
}

/// Accounts whose window was the day's first on the days it hit, most often first.
pub fn first_accounts(r: &Rhythm) -> Vec<Uuid> {
    let mut count: HashMap<Uuid, usize> = HashMap::new();
    for f in r.days.iter().filter_map(|d| d.first.as_ref()) {
        if f.hit_at.is_some() {
            *count.entry(f.account).or_default() += 1;
        }
    }
    let mut v: Vec<(Uuid, usize)> = count.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v.into_iter().map(|(a, _)| a).collect()
}

/// One past workday replayed with the warm-up.
#[derive(Debug, Clone, PartialEq)]
pub struct SimDay {
    pub date: NaiveDate,
    pub waited_now: Duration,
    pub waited_with: Duration,
    /// The warm-up would have started a window that day.
    pub ran: bool,
    /// Skipped: a window of the account was already running at the warm-up time.
    pub skipped_running: bool,
}

/// The warm-up replayed over the rhythm's workdays (first window only; an estimate).
#[derive(Debug, Clone, PartialEq)]
pub struct Simulation {
    pub days: Vec<SimDay>,
    pub waited_now: Duration,
    pub waited_with: Duration,
    /// Warm-up windows started (extra windows).
    pub runs: usize,
}

impl Simulation {
    pub fn saved(&self) -> Duration {
        self.waited_now - self.waited_with
    }
}

/// For each workday with a first window: the reset moved to T + 5 h (T in local time of that
/// day, so DST days keep the wall time) and the wait after its hit recomputed. A day where a
/// window of that account was already running at T keeps its wait (the warm-up skips).
pub fn simulate(
    at: NaiveTime,
    r: &Rhythm,
    windows_of: &dyn Fn(Uuid) -> Vec<(DateTime<Utc>, DateTime<Utc>)>,
    zone: &Zone,
) -> Simulation {
    let mut days = vec![];
    for d in &r.days {
        let Some(f) = &d.first else { continue };
        let t = zone.utc_of(d.date.and_time(at));
        let waited_now = f.waited.unwrap_or_else(Duration::zero);
        let running = windows_of(f.account)
            .iter()
            .any(|(start, reset)| *start <= t && t < *reset);
        let waited_with = if running {
            waited_now
        } else {
            let reset = t + Duration::hours(SESSION_LEN_H);
            match f.hit_at {
                Some(h) if h < reset => reset - h,
                _ => Duration::zero(),
            }
        };
        days.push(SimDay {
            date: d.date,
            waited_now,
            waited_with: waited_with.min(waited_now),
            ran: !running,
            skipped_running: running,
        });
    }
    Simulation {
        waited_now: days.iter().map(|d| d.waited_now).sum(),
        waited_with: days.iter().map(|d| d.waited_with).sum(),
        runs: days.iter().filter(|d| d.ran).count(),
        days,
    }
}

/// One window on a timeline, in local wall time.
#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub start: NaiveDateTime,
    pub reset: NaiveDateTime,
    /// When it ran out (then the rest until the reset is waiting).
    pub hit: Option<NaiveDateTime>,
    /// Started by the warm-up (no work in it before the day starts).
    pub warm: bool,
    /// No reading in its last hours (PC off): drawn faint, never as a hit.
    pub unknown: bool,
}

/// One past workday drawn twice: as it went, and with the warm-up (an estimate).
#[derive(Debug, Clone, PartialEq)]
pub struct ExampleDay {
    pub date: NaiveDate,
    pub account: Uuid,
    /// When work started that day (local).
    pub start: NaiveTime,
    /// The account's windows that started that day.
    pub real: Vec<Span>,
    /// The warm-up window, then back-to-back windows while work went on; the first one's wait
    /// comes from the real first hit, later hits are not guessed.
    pub with: Vec<Span>,
    pub waited_now: Duration,
    pub waited_with: Duration,
}

/// The day the timeline shows: the latest simulated workday the warm-up helped most recently
/// (it ran and saved time), else the latest one it ran on.
pub fn example_day(
    at: NaiveTime,
    r: &Rhythm,
    sim: &Simulation,
    windows: &dyn Fn(Uuid) -> Vec<SessionWindow>,
    zone: &Zone,
) -> Option<ExampleDay> {
    let pick = sim
        .days
        .iter()
        .rev()
        .find(|d| d.ran && d.waited_with < d.waited_now)
        .or_else(|| sim.days.iter().rev().find(|d| d.ran))?;
    let day = r.days.iter().find(|d| d.date == pick.date)?;
    let first = day.first.as_ref()?;
    let local = |t: DateTime<Utc>| zone.local(t);
    let mut real: Vec<Span> = windows(first.account)
        .into_iter()
        .filter(|w| zone.local(w.start).date() == pick.date)
        .map(|w| Span {
            start: local(w.start),
            reset: local(w.resets_at),
            hit: w.hit_at.map(local),
            warm: false,
            unknown: w.end == WindowEnd::Unknown,
        })
        .collect();
    real.sort_by_key(|s| s.start);
    let last_reset = real.iter().map(|s| s.reset).max()?;
    let t = zone.utc_of(pick.date.and_time(at));
    let first_reset = t + Duration::hours(SESSION_LEN_H);
    let mut with = vec![Span {
        start: local(t),
        reset: local(first_reset),
        hit: first.hit_at.filter(|h| *h < first_reset).map(local),
        warm: true,
        unknown: false,
    }];
    let mut start = first_reset;
    while local(start) < last_reset {
        let reset = start + Duration::hours(SESSION_LEN_H);
        with.push(Span {
            start: local(start),
            reset: local(reset),
            hit: None,
            warm: false,
            unknown: false,
        });
        start = reset;
    }
    Some(ExampleDay {
        date: pick.date,
        account: first.account,
        start: day.start,
        real,
        with,
        waited_now: pick.waited_now,
        waited_with: pick.waited_with,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insights::zone::tests::spring;

    fn win(account: u128, start: &str, hit_after_min: Option<i64>) -> SessionWindow {
        let start: DateTime<Utc> = start.parse().unwrap();
        let reset = start + Duration::hours(5);
        let hit = hit_after_min.map(|m| start + Duration::minutes(m));
        SessionWindow {
            account: Uuid::from_u128(account),
            start,
            resets_at: reset,
            peak: if hit.is_some() { 100.0 } else { 40.0 },
            hit_at: hit,
            waited: hit.map(|h| reset - h),
            end: WindowEnd::Seen,
            free_elsewhere: vec![],
        }
    }

    fn date(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    #[test]
    fn usual_start_and_first_hit_are_medians_of_workdays() {
        // Mon Nov 2 .. Fri Nov 6 2026, UTC; plus a Saturday that must not count.
        let ws = [
            win(1, "2026-11-02T08:00:00Z", Some(180)),
            win(2, "2026-11-02T10:00:00Z", None),
            win(1, "2026-11-03T09:00:00Z", Some(150)),
            win(1, "2026-11-04T08:00:00Z", None),
            win(1, "2026-11-05T07:00:00Z", Some(200)),
            win(1, "2026-11-07T05:00:00Z", Some(10)),
        ];
        let refs: Vec<&SessionWindow> = ws.iter().collect();
        // Friday only has log activity at 08:00.
        let logs = BTreeMap::from([(
            date("2026-11-06"),
            NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
        )]);
        let r = rhythm(&refs, &logs, &Zone::utc(), date("2026-11-07"));
        assert_eq!(r.days.len(), 5);
        assert!(r.days[4].from_logs);
        assert_eq!(r.usual_start, NaiveTime::from_hms_opt(8, 0, 0));
        assert_eq!(r.usual_first_hit, Some(Duration::minutes(180)));
        assert_eq!((r.recent_first_hits, r.recent_days), (3, 5));
        assert_eq!(first_accounts(&r), vec![Uuid::from_u128(1)]);
        // 8:00 + 3:00 - 5:00 = 6:00.
        assert_eq!(suggest_time(&r), NaiveTime::from_hms_opt(6, 0, 0));
    }

    #[test]
    fn suggestion_rounds_down_and_stays_within_four_hours() {
        let mut r = Rhythm {
            days: vec![],
            usual_start: NaiveTime::from_hms_opt(8, 10, 0),
            usual_first_hit: Some(Duration::minutes(170)),
            recent_first_hits: 2,
            recent_days: 5,
        };
        // 8:10 + 2:50 - 5:00 = 6:00 exactly; 8:10 + 2:57 - 5 = 6:07 -> 6:00.
        assert_eq!(suggest_time(&r), NaiveTime::from_hms_opt(6, 0, 0));
        r.usual_first_hit = Some(Duration::minutes(177));
        assert_eq!(suggest_time(&r), NaiveTime::from_hms_opt(6, 0, 0));
        // A hit 20 min in would mean 3:30; capped at 4 h before 8:10 -> 4:15.
        r.usual_first_hit = Some(Duration::minutes(20));
        assert_eq!(suggest_time(&r), NaiveTime::from_hms_opt(4, 15, 0));
        // Fewer than 2 recent first-window hits: no suggestion.
        r.recent_first_hits = 1;
        assert_eq!(suggest_time(&r), None);
    }

    #[test]
    fn simulation_moves_the_reset_and_skips_running_windows() {
        let ws = [
            win(1, "2026-11-02T08:00:00Z", Some(180)), // waits 2 h
            win(1, "2026-11-03T08:00:00Z", Some(120)), // waits 3 h
            win(1, "2026-11-04T08:00:00Z", None),
        ];
        // On Nov 3 a late window from the night before still runs at 06:00.
        let mut all: Vec<SessionWindow> = ws.to_vec();
        all.push(win(1, "2026-11-03T02:00:00Z", None));
        let refs: Vec<&SessionWindow> = ws.iter().collect();
        let r = rhythm(&refs, &BTreeMap::new(), &Zone::utc(), date("2026-11-07"));
        let of = |a: Uuid| {
            all.iter()
                .filter(|w| w.account == a)
                .map(|w| (w.start, w.resets_at))
                .collect()
        };
        let s = simulate(
            NaiveTime::from_hms_opt(6, 0, 0).unwrap(),
            &r,
            &of,
            &Zone::utc(),
        );
        assert_eq!(s.days.len(), 3);
        // Nov 2: hit at 11:00, reset now 13:00 (2 h), with warm-up 11:00 (0).
        assert_eq!(
            (s.days[0].waited_now, s.days[0].waited_with),
            (Duration::hours(2), Duration::zero())
        );
        // Nov 3: skipped, keeps 3 h.
        assert!(s.days[1].skipped_running && !s.days[1].ran);
        assert_eq!(s.days[1].waited_with, Duration::hours(3));
        assert_eq!(s.waited_now, Duration::hours(5));
        assert_eq!(s.saved(), Duration::hours(2));
        assert_eq!(s.runs, 2);
    }

    #[test]
    fn dst_day_keeps_the_local_wall_time() {
        // 08:00 local both days: 07:00 UTC on Fri Mar 26 (+1), 06:00 UTC on Mon Mar 29 (+2).
        let zone = spring();
        let ws = [
            win(1, "2027-03-26T07:00:00Z", Some(180)),
            win(1, "2027-03-29T06:00:00Z", Some(180)),
        ];
        let refs: Vec<&SessionWindow> = ws.iter().collect();
        let r = rhythm(&refs, &BTreeMap::new(), &zone, date("2027-03-30"));
        assert_eq!(r.usual_start, NaiveTime::from_hms_opt(8, 0, 0));
        let at = suggest_time(&r).unwrap();
        assert_eq!(at, NaiveTime::from_hms_opt(6, 0, 0).unwrap());
        let s = simulate(at, &r, &|_| vec![], &zone);
        // Both days: warm-up reset at 11:00 local = the hit, nothing left to wait.
        assert!(s.days.iter().all(|d| d.waited_with == Duration::zero()));
        assert_eq!(s.saved(), Duration::hours(4));
    }

    #[test]
    fn example_day_draws_the_real_day_and_the_warm_up_chain() {
        // Mon: start 09:00, hit 12:30 (waits 1:30), then 14:00-19:00; Tue: no hit.
        let ws = vec![
            win(1, "2026-11-02T09:00:00Z", Some(210)),
            win(1, "2026-11-02T14:00:00Z", None),
            win(1, "2026-11-03T09:00:00Z", None),
        ];
        let firsts: Vec<&SessionWindow> = vec![&ws[0], &ws[2]];
        let r = rhythm(&firsts, &BTreeMap::new(), &Zone::utc(), date("2026-11-04"));
        let at = NaiveTime::from_hms_opt(7, 0, 0).unwrap();
        let of = |a: Uuid| {
            ws.iter()
                .filter(|w| w.account == a)
                .map(|w| (w.start, w.resets_at))
                .collect()
        };
        let sim = simulate(at, &r, &of, &Zone::utc());
        let all = |_: Uuid| ws.clone();
        let e = example_day(at, &r, &sim, &all, &Zone::utc()).unwrap();
        // Monday: the warm-up saved time there (Tuesday ran but saved nothing).
        assert_eq!(e.date, date("2026-11-02"));
        assert_eq!(e.real.len(), 2);
        assert!(e.real[0].hit.is_some() && !e.real[0].warm);
        let hm = |s: &str| date("2026-11-02").and_time(s.parse().unwrap());
        // Warm-up 07:00-12:00 resets before the 12:30 hit: no wait, then 12:00-17:00, 17:00-22:00
        // while the real day went on until 19:00.
        let spans: Vec<_> = e.with.iter().map(|s| (s.start, s.reset, s.warm)).collect();
        assert_eq!(
            spans,
            vec![
                (hm("07:00:00"), hm("12:00:00"), true),
                (hm("12:00:00"), hm("17:00:00"), false),
                (hm("17:00:00"), hm("22:00:00"), false),
            ]
        );
        assert_eq!(e.with[0].hit, None);
        assert_eq!(
            (e.waited_now, e.waited_with),
            (Duration::minutes(90), Duration::zero())
        );
    }
}

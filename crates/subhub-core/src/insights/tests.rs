//! `analyze` over generated reading histories: the plan's rules end to end and its edge cases
//! (no history, PC-off gaps, an account added mid-cycle, unknown plan, benched hub login, resets
//! moving by seconds, Codex without a 5-hour window, logs without cwd or title), and the app
//! logins' own plan fit (monthly Copilot, Cursor's weekly ladder, Antigravity chosen by hand).
use std::path::PathBuf;
use std::sync::Arc;

use chrono::{Datelike, Weekday};

use super::*;
use crate::billing::MonthlyBilling;
use crate::model::{AllowanceWindow, HubLink, Reading};
use crate::projects::Project;
use crate::stats::{Bucket, TaskKind, Tokens};

const A: Uuid = Uuid::from_u128(0xa); // Claude Pro, hits on Mon/Wed/Fri
const B: Uuid = Uuid::from_u128(0xb); // Claude Max 20x, light: a downgrade
const C: Uuid = Uuid::from_u128(0xc); // Claude Pro, benched hub login, idle
const D: Uuid = Uuid::from_u128(0xd); // Codex Plus with only a weekly window
const E: Uuid = Uuid::from_u128(0xe); // Claude, unknown plan
const F: Uuid = Uuid::from_u128(0xf); // Claude Pro added on Wed Nov 4
const G: Uuid = Uuid::from_u128(0x10); // Copilot Pro, monthly credits
const H: Uuid = Uuid::from_u128(0x11); // Cursor Premium, weekly plan
const I: Uuid = Uuid::from_u128(0x12); // Antigravity, Ultra 5x chosen by hand

fn t(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}

/// Saturday, Nov 7 2026, 12:00 UTC.
fn now() -> DateTime<Utc> {
    t("2026-11-07T12:00:00Z")
}

fn account(id: Uuid, provider: ProviderKind, hub: Option<bool>) -> Account {
    Account {
        id,
        provider,
        label: format!("acc-{id}"),
        email: None,
        profile_dir: hub.is_none().then(|| PathBuf::from(r"C:\p")),
        hub: hub.map(|benched| HubLink {
            auth_index: serde_json::json!(1),
            file_name: "x.json".into(),
            benched,
            account_id: None,
            priority: None,
        }),
        group: None,
        hidden: false,
        hidden_windows: vec![],
        pinned_windows: vec![],
    }
}

/// One account's readings: every 15 minutes 07:00-19:00 UTC on workdays (09:00-11:00 on
/// Saturdays) from `from` until `now`. `windows(date)` gives (start hour, % per hour) of the
/// 5-hour windows that day; weekly use climbs to `weekly(cycle reset)` by Friday. Resets carry a
/// few seconds of jitter, like the providers' do.
struct Gen<'a> {
    id: Uuid,
    session_key: Option<&'a str>,
    weekly_key: &'a str,
    from: DateTime<Utc>,
    windows: &'a dyn Fn(NaiveDate) -> Vec<(u32, f64)>,
    weekly: &'a dyn Fn(DateTime<Utc>) -> f64,
    /// Extra (start, reset, % per hour) windows outside the reading hours.
    extra: Vec<(DateTime<Utc>, f64)>,
}

fn weekly_reset(at: DateTime<Utc>) -> DateTime<Utc> {
    // Mondays 06:00 UTC.
    let d = at.date_naive();
    let mon = d - Duration::days(d.weekday().num_days_from_monday() as i64);
    let r = mon.and_hms_opt(6, 0, 0).unwrap().and_utc();
    if r > at { r } else { r + Duration::days(7) }
}

impl Gen<'_> {
    fn lines(&self) -> Vec<HistoryLine> {
        let mut out = vec![];
        let mut day = self.from.date_naive();
        let mut n = 0i64;
        while day <= now().date_naive() {
            let (lo, hi) = match day.weekday() {
                Weekday::Sun => (0, 0),
                Weekday::Sat => (9 * 4, 11 * 4),
                _ => (7 * 4, 19 * 4),
            };
            let mut wins: Vec<(DateTime<Utc>, f64)> = (self.windows)(day)
                .into_iter()
                .map(|(h, rate)| (day.and_hms_opt(h, 0, 0).unwrap().and_utc(), rate))
                .collect();
            wins.extend(self.extra.iter().copied());
            for q in lo..=hi {
                if hi == 0 {
                    break;
                }
                let at = day.and_hms_opt(0, 0, 0).unwrap().and_utc() + Duration::minutes(q * 15);
                if at < self.from || at > now() {
                    continue;
                }
                n += 1;
                let jitter = Duration::seconds(n % 7 - 3);
                if let Some(key) = self.session_key {
                    let w = wins
                        .iter()
                        .find(|(s, _)| *s <= at && at < *s + Duration::hours(5));
                    let (used, reset) = match w {
                        Some((s, rate)) => {
                            let h = (at - *s).num_minutes() as f64 / 60.0;
                            (
                                (rate * h).min(100.0),
                                Some(*s + Duration::hours(5) + jitter),
                            )
                        }
                        None => (0.0, None),
                    };
                    out.push(HistoryLine {
                        account: self.id,
                        fetched_at: at,
                        window: key.into(),
                        used_percent: used.round(),
                        resets_at: reset,
                    });
                }
                let reset = weekly_reset(at);
                let peak = (self.weekly)(reset);
                let elapsed = (at - (reset - Duration::days(7))).num_minutes() as f64;
                let used = (peak * elapsed / (4.5 * 24.0 * 60.0)).min(peak);
                out.push(HistoryLine {
                    account: self.id,
                    fetched_at: at,
                    window: self.weekly_key.into(),
                    used_percent: used.round(),
                    resets_at: Some(reset + jitter),
                });
            }
            day = day.succ_opt().unwrap();
        }
        out
    }
}

fn weekday(d: NaiveDate) -> bool {
    !matches!(d.weekday(), Weekday::Sat | Weekday::Sun)
}

/// Peaks per cycle by its reset: Oct 19, Oct 26, Nov 2, then the running one.
fn by_cycle(peaks: [f64; 4]) -> impl Fn(DateTime<Utc>) -> f64 {
    move |reset| {
        if reset <= t("2026-10-19T06:01:00Z") {
            peaks[0]
        } else if reset <= t("2026-10-26T06:01:00Z") {
            peaks[1]
        } else if reset <= t("2026-11-02T06:01:00Z") {
            peaks[2]
        } else {
            peaks[3]
        }
    }
}

struct Fixture {
    settings: Settings,
    history: Vec<HistoryLine>,
    plans: HashMap<Uuid, String>,
}

fn fixture() -> Fixture {
    // A day before the 4-week range, so the range is fully covered.
    let start = now() - Duration::days(29);
    let a_windows = |d: NaiveDate| {
        if !weekday(d) {
            return vec![];
        }
        // Mon/Wed/Fri: runs out 3 h in (34 %/h); Tue/Thu: 20 %/h, never quite.
        let hit = matches!(d.weekday(), Weekday::Mon | Weekday::Wed | Weekday::Fri);
        if d == date("2026-11-03") {
            // The late window (extra) instead of the afternoon one.
            return vec![(8, 20.0)];
        }
        vec![(8, if hit { 34.0 } else { 20.0 }), (14, 15.0)]
    };
    let b_windows = |d: NaiveDate| {
        if weekday(d) { vec![(9, 8.0)] } else { vec![] }
    };
    let none = |_: NaiveDate| vec![];
    let a_week = by_cycle([75.0, 80.0, 85.0, 60.0]);
    let b_week = by_cycle([20.0, 21.0, 22.0, 15.0]);
    let c_week = by_cycle([3.0, 2.0, 4.0, 1.0]);
    let d_week = by_cycle([30.0, 40.0, 50.0, 20.0]);
    let e_week = by_cycle([50.0, 50.0, 50.0, 30.0]);
    let f_week = by_cycle([0.0, 0.0, 0.0, 40.0]);
    let mut history = vec![];
    let gens = [
        Gen {
            id: A,
            session_key: Some("five_hour"),
            weekly_key: "seven_day",
            from: start,
            windows: &a_windows,
            weekly: &a_week,
            // PC off: a late window on Tue Nov 3 (17:00-22:00), last read at 19:00.
            extra: vec![(t("2026-11-03T17:00:00Z"), 40.0)],
        },
        Gen {
            id: B,
            session_key: Some("five_hour"),
            weekly_key: "seven_day",
            from: start,
            windows: &b_windows,
            weekly: &b_week,
            extra: vec![],
        },
        Gen {
            id: C,
            session_key: Some("five_hour"),
            weekly_key: "seven_day",
            from: start,
            windows: &none,
            weekly: &c_week,
            extra: vec![],
        },
        Gen {
            id: D,
            session_key: None,
            weekly_key: "codex:primary",
            from: start,
            windows: &none,
            weekly: &d_week,
            extra: vec![],
        },
        Gen {
            id: E,
            session_key: Some("five_hour"),
            weekly_key: "seven_day",
            from: start,
            windows: &b_windows,
            weekly: &e_week,
            extra: vec![],
        },
        Gen {
            id: F,
            session_key: Some("five_hour"),
            weekly_key: "seven_day",
            from: t("2026-11-04T07:00:00Z"),
            windows: &b_windows,
            weekly: &f_week,
            extra: vec![],
        },
    ];
    for g in &gens {
        history.extend(g.lines());
    }
    let mut settings = Settings {
        accounts: vec![
            account(A, ProviderKind::Claude, None),
            account(B, ProviderKind::Claude, Some(false)),
            account(C, ProviderKind::Claude, Some(true)),
            account(D, ProviderKind::Codex, None),
            account(E, ProviderKind::Claude, None),
            account(F, ProviderKind::Claude, None),
        ],
        ..Settings::default()
    };
    settings.hub.enabled = true;
    let plans = HashMap::from([
        (A, "Pro".to_string()),
        (B, "Max 20x".to_string()),
        (C, "Pro".to_string()),
        (D, "Plus".to_string()),
        (E, "Team".to_string()),
        (F, "Pro".to_string()),
    ]);
    Fixture {
        settings,
        history,
        plans,
    }
}

fn run(f: &Fixture, scan: Option<&Scan>, range: InsightRange) -> Overview {
    analyze(&Input {
        history: &f.history,
        scan,
        settings: &f.settings,
        plans: &f.plans,
        prices: &PriceBook::default(),
        range,
        now: now(),
        zone: &Zone::utc(),
    })
}

fn date(s: &str) -> NaiveDate {
    s.parse().unwrap()
}

#[test]
fn no_history_collects_instead_of_guessing() {
    let f = Fixture {
        history: vec![],
        ..fixture()
    };
    let o = run(&f, None, InsightRange::FourWeeks);
    // Plan fit: two cycles from now; 5-hour analysis: five workdays from Monday.
    for a in &o.accounts {
        assert_eq!(a.fit.ready_on(), Some(date("2026-11-21")), "{}", a.label);
        assert!(a.windows.is_empty() && a.cycles.is_empty());
    }
    assert_eq!(o.windows.ready_on(), Some(date("2026-11-13")));
    assert_eq!(o.header.verdict.collecting_until, Some(date("2026-11-21")));
    assert_eq!((o.header.weekly_avg, o.header.hits_per_week), (None, 0.0));
    assert!(o.tips.is_empty());
    assert_eq!(o.usage, UsageReport::default());
}

#[test]
fn plan_fit_verdicts_for_each_kind_of_account() {
    let o = run(&fixture(), None, InsightRange::FourWeeks);
    let fit = |id| o.account(id).unwrap().fit.ready().cloned();
    // A: three hits a week on Pro -> at the limit, Max 5x for $80 more.
    let a = fit(A).unwrap();
    assert_eq!(a.verdict, FitVerdict::AtLimit);
    assert_eq!(a.cycles, 3);
    assert_eq!(a.hits_per_week, 3.0);
    assert_eq!(
        (a.suggested, a.price_change),
        (Some(PlanTier::ClaudeMax5x), Some(80.0))
    );
    // B: peak 22 % of Max 20x is 88 % of Max 5x, no hits -> downgrade, $100 a month less.
    let b = fit(B).unwrap();
    assert_eq!(b.verdict, FitVerdict::Downgrade);
    assert_eq!(b.price_change, Some(-100.0));
    let w = o.account(B).unwrap().what_if.as_ref().unwrap();
    assert_eq!(w.change, WhatIfChange::Tier(PlanTier::ClaudeMax5x));
    assert!((w.own_after.unwrap() - 84.0).abs() < 0.5);
    // C: benched and idle -> Unused, but it never counts as capacity.
    assert_eq!(fit(C).unwrap().verdict, FitVerdict::Unused);
    assert!(o.account(C).unwrap().benched);
    // D: Codex with only a weekly window.
    let d = o.account(D).unwrap();
    assert!(!d.has_session_window && d.windows.is_empty());
    assert_eq!(fit(D).unwrap().verdict, FitVerdict::Fits);
    // E: unknown plan: a verdict without tier advice.
    let e = fit(E).unwrap();
    assert_eq!(
        (e.tier, e.suggested, e.verdict),
        (None, None, FitVerdict::Fits)
    );
    // F: added Wednesday: the running cycle counts on Nov 9, the second on Nov 16.
    assert_eq!(
        o.account(F).unwrap().fit.ready_on(),
        Some(date("2026-11-16"))
    );
    assert_eq!(o.header.verdict.at_limit, vec![A]);
    assert_eq!(o.header.verdict.downgrade, vec![B]);
    assert_eq!(o.header.verdict.unused, vec![C]);
    assert_eq!(o.header.verdict.collecting_until, None);
}

#[test]
fn windows_hits_gaps_and_free_accounts() {
    let o = run(&fixture(), None, InsightRange::FourWeeks);
    let a = o.account(A).unwrap();
    // 4 weeks x Mon/Wed/Fri.
    assert_eq!(a.hits, 12);
    let hit = a
        .windows
        .iter()
        .find(|w| w.start == t("2026-11-02T08:00:00Z"))
        .unwrap();
    assert_eq!(hit.hit_at, Some(t("2026-11-02T11:00:00Z")));
    assert!(
        (hit.waited.unwrap() - Duration::hours(2))
            .num_seconds()
            .abs()
            <= 5
    );
    // B had room; the benched C never counts.
    assert_eq!(hit.free_elsewhere, vec![B, E]);
    // The late window on Nov 3 was never read in its last 2 hours: a gap, not a hit.
    let late = a
        .windows
        .iter()
        .find(|w| w.start == t("2026-11-03T17:00:00Z"))
        .unwrap();
    assert_eq!((late.end, late.hit_at), (WindowEnd::Unknown, None));
    // Resets that moved by seconds did not split windows: one per start.
    let mut starts: Vec<_> = a.windows.iter().map(|w| w.start.timestamp() / 60).collect();
    let n = starts.len();
    starts.dedup();
    assert_eq!(starts.len(), n);

    let r = o.windows.ready().unwrap();
    assert_eq!(r.hits, 12);
    assert_eq!(r.hits_with_free_account, 12);
    assert!((r.hits_per_week - 3.0).abs() < 1e-9);
    assert_eq!(o.header.hits_per_week, r.hits_per_week);
    assert!((o.header.waited - Duration::hours(24)).num_seconds().abs() < 120);
}

#[test]
fn rhythm_and_warm_up() {
    let o = run(&fixture(), None, InsightRange::FourWeeks);
    let r = o.windows.ready().unwrap();
    assert_eq!(r.rhythm.usual_start, NaiveTime::from_hms_opt(8, 0, 0));
    assert_eq!(r.rhythm.usual_first_hit, Some(Duration::hours(3)));
    assert_eq!(r.rhythm.recent_first_hits, 3);
    let w = r.warmup.as_ref().unwrap();
    // 8:00 + 3:00 - 5:00.
    assert_eq!(w.at, NaiveTime::from_hms_opt(6, 0, 0).unwrap());
    assert_eq!(w.accounts, vec![A]);
    // Last 10 workdays: 6 hit days, 2 h each, all saved.
    assert_eq!(w.simulation.days.len(), 10);
    assert!(
        (w.simulation.waited_now - Duration::hours(12))
            .num_seconds()
            .abs()
            < 60
    );
    assert_eq!(w.simulation.waited_with, Duration::zero());
    assert_eq!(w.simulation.runs, 10);
}

#[test]
fn tips_are_ranked_with_stable_ids() {
    let o = run(&fixture(), None, InsightRange::FourWeeks);
    let ids: Vec<&str> = o.tips.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            format!("upgrade:{A}"),
            format!("switch:{A}"),
            "warm-up".to_string(),
            format!("downgrade:{B}"),
            format!("unused:{C}"),
        ]
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
    );
    let switch = &o.tips[1];
    assert!(matches!(
        switch.kind,
        TipKind::SwitchInsteadOfWaiting { to, times: 12, .. } if to == B
    ));
    // A has a local profile only (no hub link): the tip opens the 5-hour row, not steering.
    assert_eq!(switch.action, Some(TipAction::OpenRow(TipRow::Windows)));
    // Same input, same tips.
    assert_eq!(run(&fixture(), None, InsightRange::FourWeeks).tips, o.tips);
}

#[test]
fn ending_plan_moves_its_use_and_leads_the_tips() {
    let mut f = fixture();
    f.settings.billing.insert(
        A,
        MonthlyBilling {
            auto_renew: false,
            ..MonthlyBilling::new(date("2026-11-12"))
        },
    );
    f.settings
        .billing
        .insert(B, MonthlyBilling::new(date("2026-11-20")));
    let o = run(&f, None, InsightRange::FourWeeks);
    assert_eq!(o.renewals.len(), 2);
    assert_eq!(o.header.next_renewal.as_ref().unwrap().account, A);
    assert_eq!(o.header.next_renewal.as_ref().unwrap().days, 5);
    let w = o.account(A).unwrap().what_if.clone().unwrap();
    // A's 80 Pro units go to B (the only counted Claude account with a known tier).
    assert_eq!(w.others.len(), 1);
    assert!((w.others[0].after - (21.0 + 80.0 / 20.0)).abs() < 0.5);
    assert_eq!(o.tips[0].id, format!("ending:{A}"));
    assert!(matches!(
        &o.tips[0].kind,
        TipKind::EndingPlan {
            days: 5,
            what_if: Some(_),
            ..
        }
    ));
}

#[test]
fn expiring_weekly_room_gets_a_tip() {
    let mut f = fixture();
    f.settings.readings.insert(
        B,
        Reading {
            fetched_at: now() - Duration::minutes(5),
            windows: vec![AllowanceWindow {
                key: "seven_day".into(),
                label: "Weekly".into(),
                used_percent: 20.0,
                resets_at: Some(now() + Duration::hours(10)),
            }],
        },
    );
    let o = run(&f, None, InsightRange::FourWeeks);
    let b = o.account(B).unwrap();
    assert_eq!(b.now_weekly.unwrap().used, 20.0);
    assert!(b.now_session.is_none());
    let tip = o
        .tips
        .iter()
        .find(|t| t.id == format!("expiring:{B}"))
        .unwrap();
    assert!(matches!(tip.kind, TipKind::ExpiringWeekly { left, .. } if left == 80.0));
}

#[test]
fn ranges_cut_cycles_and_windows() {
    let f = fixture();
    let week = run(&f, None, InsightRange::Week);
    let a = week.account(A).unwrap();
    // The cycle that ended Nov 2 and the running one.
    assert_eq!(a.cycles.len(), 2);
    assert_eq!(a.weekly_avg, Some(85.0));
    assert_eq!(a.hits, 3);
    // Plan fit still looks back 4 weeks.
    assert!(a.fit.ready().is_some());
    // The quarter also holds the hit on Fri Oct 9; per week over the 29 days of history.
    let quarter = run(&f, None, InsightRange::Quarter);
    assert_eq!(quarter.account(A).unwrap().hits, 13);
    assert!((quarter.header.hits_per_week - 13.0 * 7.0 / 29.0).abs() < 0.05);
}

#[test]
fn where_usage_goes_from_the_scan() {
    let f = Fixture {
        settings: Settings {
            projects: vec![Project {
                id: Uuid::from_u128(77),
                label: "App".into(),
                directory: PathBuf::from(r"D:\Github\app"),
                hidden: false,
            }],
            ..fixture().settings
        },
        ..fixture()
    };
    let hour = hour_index(now().naive_utc()) - 2;
    let b = |kind, cwd: Option<&str>, thread: &str, out| Bucket {
        hour,
        provider: ProviderKind::Claude,
        model: "claude-opus-5-5".into(),
        tokens: Tokens {
            output: out,
            ..Default::default()
        },
        kind,
        sidechain: false,
        cwd: cwd.map(Arc::from),
        thread: Arc::from(thread),
    };
    let scan = Scan {
        buckets: vec![
            b(TaskKind::BuildsTests, Some(r"D:\Github\app"), "t1", 300),
            b(TaskKind::WritingCode, Some(r"D:\Github\app\src"), "t1", 500),
            // A log line without cwd or title.
            b(TaskKind::Talking, None, "t2", 200),
        ],
        ..Default::default()
    };
    let o = run(&f, Some(&scan), InsightRange::Week);
    assert!((o.usage.kind_share(TaskKind::BuildsTests) - 0.3).abs() < 1e-9);
    assert_eq!(
        o.usage.projects[0].key,
        ProjectKey::Saved(Uuid::from_u128(77))
    );
    assert_eq!(o.usage.projects[1].key, ProjectKey::Unknown);
    assert!(o.usage.threads.iter().all(|t| t.title.is_none()));
    // Builds & tests over 20 %: an insight tip.
    assert!(o.tips.iter().any(|t| t.id == "task:builds-tests"));
}

/// Copilot's monthly credits: a reading at 10:00 UTC on weekdays from `from`, climbing to
/// `peak(reset)` by the end of each month; resets on the 1st, 00:00 UTC.
fn copilot_lines(id: Uuid, from: &str, peak: impl Fn(DateTime<Utc>) -> f64) -> Vec<HistoryLine> {
    let mut out = vec![];
    let mut day = date(from);
    while day < now().date_naive() {
        if weekday(day) {
            let at = day.and_hms_opt(10, 0, 0).unwrap().and_utc();
            let first = day.with_day(1).unwrap();
            let reset = first
                .checked_add_months(chrono::Months::new(1))
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_utc();
            out.push(HistoryLine {
                account: id,
                fetched_at: at,
                window: "copilot:premium".into(),
                used_percent: (peak(reset) * day.day() as f64 / 31.0).round(),
                resets_at: Some(reset),
            });
        }
        day = day.succ_opt().unwrap();
    }
    out
}

/// The fixture plus a Copilot Pro at 97 % of its October credits by its last weekday, a Cursor Premium around 20 %
/// of its week and an Antigravity account set to Ultra 5x by hand around 15 % of its week.
fn with_app_logins(f: &Fixture) -> Fixture {
    let mut f = Fixture {
        settings: f.settings.clone(),
        history: f.history.clone(),
        plans: f.plans.clone(),
    };
    let start = now() - Duration::days(29);
    let none = |_: NaiveDate| vec![];
    let light = |d: NaiveDate| {
        if weekday(d) { vec![(9, 6.0)] } else { vec![] }
    };
    let cursor_week = by_cycle([18.0, 20.0, 19.0, 10.0]);
    let agy_week = by_cycle([12.0, 15.0, 14.0, 5.0]);
    for g in [
        Gen {
            id: H,
            session_key: None,
            weekly_key: "cursor:plan",
            from: start,
            windows: &none,
            weekly: &cursor_week,
            extra: vec![],
        },
        Gen {
            id: I,
            session_key: Some("agy:5h"),
            weekly_key: "agy:weekly",
            from: start,
            windows: &light,
            weekly: &agy_week,
            extra: vec![],
        },
    ] {
        f.history.extend(g.lines());
    }
    f.history.extend(copilot_lines(G, "2026-09-25", |_| 100.0));
    f.settings.accounts.extend([
        account(G, ProviderKind::Copilot, None),
        account(H, ProviderKind::Cursor, None),
        account(I, ProviderKind::Antigravity, None),
    ]);
    f.plans.insert(G, "Pro".into());
    f.plans.insert(H, "Premium".into());
    f.settings.manual_plans.insert(I, "Ultra 5x".into());
    f
}

#[test]
fn app_logins_get_their_own_plan_fit() {
    let f = with_app_logins(&fixture());
    let o = run(&f, None, InsightRange::FourWeeks);
    let fit = |id| o.account(id).unwrap().fit.ready().cloned().unwrap();
    // Copilot: one ended month (October, read on its 22 weekdays) is enough; 97 % is at the
    // limit, Pro+ for $29 more. No 5-hour window, so no hits.
    let g = o.account(G).unwrap();
    assert_eq!(
        (g.cycle, g.tier),
        (CycleLen::Month, Some(PlanTier::CopilotPro))
    );
    let gf = fit(G);
    assert_eq!(
        (gf.verdict, gf.cycles, gf.hits_per_week),
        (FitVerdict::AtLimit, 1, 0.0)
    );
    assert_eq!(
        (gf.suggested, gf.price_change),
        (Some(PlanTier::CopilotProPlus), Some(29.0))
    );
    // Cursor: weekly resets, so "Premium" is the current lineup; 20 % fits Plus at 80 %.
    let h = o.account(H).unwrap();
    assert_eq!(
        (h.cycle, h.tier),
        (CycleLen::Week, Some(PlanTier::CursorPremium))
    );
    let hf = fit(H);
    assert_eq!(
        (hf.verdict, hf.suggested, hf.price_change),
        (
            FitVerdict::Downgrade,
            Some(PlanTier::CursorPlus),
            Some(-22.0)
        )
    );
    // Antigravity: the plan chosen by hand is its label and tier; the advice has no price.
    let i = o.account(I).unwrap();
    assert_eq!(i.plan.as_deref(), Some("Ultra 5x"));
    assert!(i.has_session_window && !i.plan_conflict);
    let x = fit(I);
    assert_eq!(
        (x.verdict, x.suggested, x.price_change),
        (FitVerdict::Downgrade, Some(PlanTier::AgyPro), None)
    );
    // No what-if: one login per PC.
    assert!(g.what_if.is_none() && h.what_if.is_none() && i.what_if.is_none());
}

#[test]
fn app_logins_stay_out_of_the_totals() {
    let base = run(&fixture(), None, InsightRange::FourWeeks);
    let o = run(&with_app_logins(&fixture()), None, InsightRange::FourWeeks);
    // Same headline, numbers, 5-hour analysis, warm-up and tips as without them.
    assert_eq!(o.header, base.header);
    assert_eq!(o.windows, base.windows);
    assert_eq!(o.tips, base.tips);
    for id in [G, H, I] {
        assert!(!o.tips.iter().any(|t| t.account == Some(id)));
        let v = &o.header.verdict;
        assert!(
            ![&v.unused, &v.at_limit, &v.downgrade]
                .iter()
                .any(|l| l.contains(&id))
        );
    }
    // Alone, they leave the header collecting (no Claude or Codex verdict to give).
    let alone = Fixture {
        settings: Settings {
            accounts: o
                .accounts
                .iter()
                .filter(|a| !in_totals(a.provider))
                .map(|a| account(a.account, a.provider, None))
                .collect(),
            ..with_app_logins(&fixture()).settings
        },
        ..with_app_logins(&fixture())
    };
    let o = run(&alone, None, InsightRange::FourWeeks);
    assert_eq!(o.header.weekly_avg, None);
    assert_eq!(o.header.verdict, Verdict::default());
    assert!(o.windows.ready_on().is_some() && o.tips.is_empty());
    assert!(o.account(G).unwrap().fit.ready().is_some());
}

#[test]
fn monthly_plan_fit_waits_for_one_ended_month() {
    let mut f = with_app_logins(&fixture());
    // Added on Oct 20: October was read on 9 days only, so it does not count; November can
    // still be read on enough days, so the first verdict comes with its reset on Dec 1.
    f.history.retain(|l| l.account != G);
    f.history.extend(copilot_lines(G, "2026-10-20", |_| 50.0));
    let o = run(&f, None, InsightRange::FourWeeks);
    let g = o.account(G).unwrap();
    assert!(g.cycles.iter().all(|c| !c.counted));
    assert_eq!(g.fit.ready_on(), Some(date("2026-12-01")));
    // Without any reading: a month from now.
    f.history.retain(|l| l.account != G);
    let o = run(&f, None, InsightRange::FourWeeks);
    assert_eq!(
        o.account(G).unwrap().fit.ready_on(),
        Some(date("2026-12-07"))
    );
}

#[test]
fn a_paid_antigravity_choice_on_a_free_reading_gives_no_verdict() {
    let mut f = with_app_logins(&fixture());
    // The latest reading has only the weekly window (the Free plan's shape).
    f.settings.readings.insert(
        I,
        Reading {
            fetched_at: now() - Duration::minutes(5),
            windows: vec![AllowanceWindow {
                key: "agy:weekly".into(),
                label: "Weekly".into(),
                used_percent: 10.0,
                resets_at: Some(now() + Duration::days(2)),
            }],
        },
    );
    f.plans.insert(I, "Free".into());
    let o = run(&f, None, InsightRange::FourWeeks);
    let i = o.account(I).unwrap();
    // The choice still names the plan, but there is no verdict.
    assert_eq!(i.plan.as_deref(), Some("Ultra 5x"));
    assert!(i.plan_conflict && i.fit.ready().is_none());
    // Not set: the reported plan, no tier, a verdict without advice.
    f.settings.manual_plans.clear();
    let o = run(&f, None, InsightRange::FourWeeks);
    let i = o.account(I).unwrap();
    assert_eq!(
        (i.plan.as_deref(), i.tier, i.plan_conflict),
        (Some("Free"), None, false)
    );
    assert!(i.fit.ready().is_some_and(|x| x.suggested.is_none()));
}

#[test]
fn cursor_names_are_read_in_the_ladder_of_their_cycle() {
    let mut f = with_app_logins(&fixture());
    // The same history labelled monthly by the latest reading: "Premium" is no legacy plan.
    f.settings.readings.insert(
        H,
        Reading {
            fetched_at: now() - Duration::minutes(5),
            windows: vec![AllowanceWindow {
                key: "cursor:plan".into(),
                label: "Monthly".into(),
                used_percent: 10.0,
                resets_at: Some(now() + Duration::days(20)),
            }],
        },
    );
    let o = run(&f, None, InsightRange::FourWeeks);
    assert_eq!(o.account(H).unwrap().cycle, CycleLen::Month);
    assert_eq!(o.account(H).unwrap().tier, None);
    f.plans.insert(H, "Ultra".into());
    let o = run(&f, None, InsightRange::FourWeeks);
    assert_eq!(
        o.account(H).unwrap().tier,
        Some(PlanTier::CursorLegacyUltra)
    );
}

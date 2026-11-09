//! Tips: concrete things to do, each with a stable id (kind + account, for "Not for me"), a
//! priority, structured data and an optional action. The UI words them.
use std::collections::HashMap;

use chrono::{DateTime, Duration, NaiveDate, NaiveTime, Utc};
use uuid::Uuid;

use super::fit::{FitVerdict, PlanTier, WhatIf};
use super::readings::CycleLen;
use super::usage::UsageReport;
use super::{AccountInsight, Renewal, WindowsReport, in_totals};
use crate::settings::Settings;
use crate::stats::TaskKind;

/// An access end this many days ahead or fewer gets a tip.
pub const ENDING_DAYS: i64 = 14;
/// A weekly window resetting within this many hours ...
pub const EXPIRING_HOURS: i64 = 24;
/// ... with at least this % left gets a "use it" tip.
pub const EXPIRING_LEFT: f64 = 30.0;
/// A task kind above this share of the value gets an insight (Builds & tests).
pub const TASK_INSIGHT_SHARE: f64 = 0.20;
/// A warm-up must save at least this much waiting over the replayed days.
pub const WARMUP_MIN_SAVED_MIN: i64 = 15;

/// The Overview row a tip opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TipRow {
    Plans,
    Windows,
    Renewals,
    Usage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TipAction {
    OpenRow(TipRow),
    /// Hub steering (the account card's menu / Settings -> Hub).
    Steering,
    /// The warm-up setup dialog, filled in with the suggestion.
    SetUpWarmUp,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TipKind {
    /// Start the first window earlier so it resets when the user usually runs out.
    WarmUp {
        at: NaiveTime,
        accounts: Vec<Uuid>,
        /// Waiting saved over the replayed days, and the waiting without warm-up.
        saved: Duration,
        waited_now: Duration,
        days: usize,
    },
    /// Hit the 5-hour limit while another account had room.
    SwitchInsteadOfWaiting {
        account: Uuid,
        /// The account that was free most often.
        to: Uuid,
        times: usize,
        waited: Duration,
    },
    Downgrade {
        account: Uuid,
        to: PlanTier,
        /// USD per month (positive).
        saving: f64,
        peak_on_smaller: f64,
    },
    Upgrade {
        account: Uuid,
        to: PlanTier,
        /// USD per month more.
        extra: f64,
        peak: f64,
        hits_per_week: f64,
    },
    /// At the limit on the largest tier.
    SpreadWork {
        account: Uuid,
        peak: f64,
        hits_per_week: f64,
    },
    EndingPlan {
        account: Uuid,
        on: NaiveDate,
        days: i64,
        what_if: Option<WhatIf>,
    },
    UnusedPlan {
        account: Uuid,
        avg: f64,
        /// USD per month when the plan is known.
        saving: Option<f64>,
    },
    TaskInsight {
        kind: TaskKind,
        share: f64,
    },
    /// Weekly room that resets soon unused.
    ExpiringWeekly {
        account: Uuid,
        left: f64,
        resets_at: DateTime<Utc>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tip {
    /// Stable across runs: `<kind>:<account>` (or `<kind>` without one).
    pub id: String,
    /// Higher first.
    pub priority: u8,
    pub account: Option<Uuid>,
    pub action: Option<TipAction>,
    pub kind: TipKind,
}

pub(super) struct TipInput<'a> {
    pub accounts: &'a [AccountInsight],
    pub settings: &'a Settings,
    pub windows: Option<&'a WindowsReport>,
    pub renewals: &'a [Renewal],
    pub usage: &'a UsageReport,
    pub now: DateTime<Utc>,
}

fn tip(slug: &str, account: Option<Uuid>, priority: u8, action: TipRow, kind: TipKind) -> Tip {
    Tip {
        id: match account {
            Some(a) => format!("{slug}:{a}"),
            None => slug.to_string(),
        },
        priority,
        account,
        action: Some(TipAction::OpenRow(action)),
        kind,
    }
}

pub(super) fn tips(i: &TipInput) -> Vec<Tip> {
    let mut out = vec![];
    for r in i.renewals {
        if !r.auto_renew && (0..=ENDING_DAYS).contains(&r.days) {
            let what_if = i
                .accounts
                .iter()
                .find(|a| a.account == r.account)
                .and_then(|a| a.what_if.clone());
            out.push(tip(
                "ending",
                Some(r.account),
                90,
                TipRow::Renewals,
                TipKind::EndingPlan {
                    account: r.account,
                    on: r.date,
                    days: r.days,
                    what_if,
                },
            ));
        }
    }
    for a in i.accounts {
        let id = Some(a.account);
        // Copilot, Cursor and Antigravity: their verdict stays in their own Plans row.
        if let Some(f) = a.fit.ready().filter(|_| in_totals(a.provider)) {
            match f.verdict {
                FitVerdict::AtLimit if !a.benched => out.push(match f.suggested {
                    Some(to) => tip(
                        "upgrade",
                        id,
                        80,
                        TipRow::Plans,
                        TipKind::Upgrade {
                            account: a.account,
                            to,
                            extra: f.price_change.unwrap_or(0.0),
                            peak: f.peak,
                            hits_per_week: f.hits_per_week,
                        },
                    ),
                    None => tip(
                        "spread",
                        id,
                        78,
                        TipRow::Plans,
                        TipKind::SpreadWork {
                            account: a.account,
                            peak: f.peak,
                            hits_per_week: f.hits_per_week,
                        },
                    ),
                }),
                FitVerdict::Downgrade => out.push(tip(
                    "downgrade",
                    id,
                    50,
                    TipRow::Plans,
                    TipKind::Downgrade {
                        account: a.account,
                        to: f.suggested.expect("a downgrade names its tier"),
                        saving: -f.price_change.unwrap_or(0.0),
                        peak_on_smaller: f.peak_on_suggested.unwrap_or(f.peak),
                    },
                )),
                FitVerdict::Unused => out.push(tip(
                    "unused",
                    id,
                    45,
                    TipRow::Plans,
                    TipKind::UnusedPlan {
                        account: a.account,
                        avg: f.avg,
                        saving: f.price_change.map(|p| -p),
                    },
                )),
                _ => {}
            }
        }
        if a.benched {
            continue;
        }
        // Waited although another account had room.
        let free: Vec<&super::SessionWindow> = a
            .windows
            .iter()
            .filter(|w| w.hit_at.is_some() && !w.free_elsewhere.is_empty())
            .collect();
        if !free.is_empty() {
            let mut count: HashMap<Uuid, usize> = HashMap::new();
            for w in &free {
                for b in &w.free_elsewhere {
                    *count.entry(*b).or_default() += 1;
                }
            }
            let to = count
                .into_iter()
                .max_by(|x, y| x.1.cmp(&y.1).then(y.0.cmp(&x.0)))
                .map(|(b, _)| b)
                .expect("non-empty");
            let steerable = i.settings.hub.enabled
                && i.settings
                    .accounts
                    .iter()
                    .find(|x| x.id == a.account)
                    .is_some_and(|x| x.hub.is_some());
            let mut t = tip(
                "switch",
                id,
                75,
                TipRow::Windows,
                TipKind::SwitchInsteadOfWaiting {
                    account: a.account,
                    to,
                    times: free.len(),
                    waited: free.iter().filter_map(|w| w.waited).sum(),
                },
            );
            if steerable {
                t.action = Some(TipAction::Steering);
            }
            out.push(t);
        }
        // Weekly room that is about to reset unused.
        if a.cycle == CycleLen::Week
            && let Some(w) = a.now_weekly
            && let Some(reset) = w.resets_at
            && reset > i.now
            && reset - i.now <= Duration::hours(EXPIRING_HOURS)
            && 100.0 - w.used >= EXPIRING_LEFT
            && i.now - w.fetched_at <= Duration::hours(6)
        {
            out.push(Tip {
                action: None,
                ..tip(
                    "expiring",
                    id,
                    55,
                    TipRow::Plans,
                    TipKind::ExpiringWeekly {
                        account: a.account,
                        left: 100.0 - w.used,
                        resets_at: reset,
                    },
                )
            });
        }
    }
    if let Some(w) = i.windows
        && let Some(s) = &w.warmup
        && s.simulation.saved() >= Duration::minutes(WARMUP_MIN_SAVED_MIN)
    {
        out.push(tip(
            "warm-up",
            None,
            70,
            TipRow::Windows,
            TipKind::WarmUp {
                at: s.at,
                accounts: s.accounts.clone(),
                saved: s.simulation.saved(),
                waited_now: s.simulation.waited_now,
                days: s.simulation.days.len(),
            },
        ));
    }
    let builds = i.usage.kind_share(TaskKind::BuildsTests);
    if builds > TASK_INSIGHT_SHARE {
        out.push(tip(
            "task:builds-tests",
            None,
            20,
            TipRow::Usage,
            TipKind::TaskInsight {
                kind: TaskKind::BuildsTests,
                share: builds,
            },
        ));
    }
    out.sort_by(|a, b| b.priority.cmp(&a.priority).then(a.id.cmp(&b.id)));
    out
}

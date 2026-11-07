//! Plan fit and what-if (plan "Analysis", owner 2026-11-07): does an account's plan match its
//! weekly use, and where would its use go if it ended or got smaller.
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::readings::{SessionWindow, WeeklyCycle};
use crate::model::ProviderKind;

/// Subscription tiers with a known size. Factors are relative to the provider's smallest paid
/// plan; Codex's are rough ("about").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PlanTier {
    ClaudePro,
    ClaudeMax5x,
    ClaudeMax20x,
    CodexPlus,
    CodexPro,
}

/// Date of the price table below (USD per month, list prices).
pub const PLAN_PRICES_AS_OF: &str = "2026-11-07";

impl PlanTier {
    /// From a plan label as shown on the card ("Pro", "Max 5x", "Max 20x", "Plus"). "Max" without
    /// a size, Team, Free and others are unknown.
    pub fn from_plan(provider: ProviderKind, plan: &str) -> Option<Self> {
        let p = plan.trim().to_ascii_lowercase();
        match (provider, p.as_str()) {
            (ProviderKind::Claude, "pro") => Some(PlanTier::ClaudePro),
            (ProviderKind::Claude, "max 5x") => Some(PlanTier::ClaudeMax5x),
            (ProviderKind::Claude, "max 20x") => Some(PlanTier::ClaudeMax20x),
            (ProviderKind::Codex, "plus") => Some(PlanTier::CodexPlus),
            (ProviderKind::Codex, "pro") => Some(PlanTier::CodexPro),
            _ => None,
        }
    }

    pub fn provider(self) -> ProviderKind {
        match self {
            PlanTier::ClaudePro | PlanTier::ClaudeMax5x | PlanTier::ClaudeMax20x => {
                ProviderKind::Claude
            }
            PlanTier::CodexPlus | PlanTier::CodexPro => ProviderKind::Codex,
        }
    }

    /// Capacity relative to the provider's smallest paid tier.
    pub fn factor(self) -> f64 {
        match self {
            PlanTier::ClaudePro | PlanTier::CodexPlus => 1.0,
            PlanTier::ClaudeMax5x => 5.0,
            PlanTier::ClaudeMax20x => 20.0,
            PlanTier::CodexPro => 10.0,
        }
    }

    /// The factor is a rough guess (Codex): label it "about".
    pub fn approximate(self) -> bool {
        self.provider() == ProviderKind::Codex
    }

    pub fn usd_per_month(self) -> f64 {
        match self {
            PlanTier::ClaudePro | PlanTier::CodexPlus => 20.0,
            PlanTier::ClaudeMax5x => 100.0,
            PlanTier::ClaudeMax20x | PlanTier::CodexPro => 200.0,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PlanTier::ClaudePro | PlanTier::CodexPro => "Pro",
            PlanTier::ClaudeMax5x => "Max 5x",
            PlanTier::ClaudeMax20x => "Max 20x",
            PlanTier::CodexPlus => "Plus",
        }
    }

    pub fn smaller(self) -> Option<Self> {
        match self {
            PlanTier::ClaudeMax20x => Some(PlanTier::ClaudeMax5x),
            PlanTier::ClaudeMax5x => Some(PlanTier::ClaudePro),
            PlanTier::CodexPro => Some(PlanTier::CodexPlus),
            PlanTier::ClaudePro | PlanTier::CodexPlus => None,
        }
    }

    pub fn larger(self) -> Option<Self> {
        match self {
            PlanTier::ClaudePro => Some(PlanTier::ClaudeMax5x),
            PlanTier::ClaudeMax5x => Some(PlanTier::ClaudeMax20x),
            PlanTier::CodexPlus => Some(PlanTier::CodexPro),
            PlanTier::ClaudeMax20x | PlanTier::CodexPro => None,
        }
    }
}

/// Counted cycles needed for a verdict.
pub const FIT_MIN_CYCLES: usize = 2;
/// Average weekly use below this: Unused.
pub const UNUSED_AVG: f64 = 5.0;
/// A smaller tier must keep the peak at or under this.
pub const DOWNGRADE_PEAK: f64 = 90.0;
/// ... with at most this many 5-hour hits a week.
pub const DOWNGRADE_HITS: f64 = 1.0;
/// At the limit: a peak this high ...
pub const AT_LIMIT_PEAK: f64 = 95.0;
/// ... or this many 5-hour hits a week.
pub const AT_LIMIT_HITS: f64 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FitVerdict {
    Unused,
    Downgrade,
    AtLimit,
    Fits,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlanFit {
    pub verdict: FitVerdict,
    /// Average and peak weekly use of the counted cycles (% of the own plan).
    pub avg: f64,
    pub peak: f64,
    /// 5-hour hits per counted cycle.
    pub hits_per_week: f64,
    pub cycles: usize,
    /// None: the plan is unknown (no tier advice, no prices).
    pub tier: Option<PlanTier>,
    /// Downgrade: the next smaller tier; At the limit: the next larger one (None at the top:
    /// "spread work").
    pub suggested: Option<PlanTier>,
    /// The peak on the suggested tier (% of that tier).
    pub peak_on_suggested: Option<f64>,
    /// Monthly price change (USD, negative = saving). Unused: minus the whole price (cancel).
    pub price_change: Option<f64>,
}

/// The verdict of one account from its counted weekly cycles and 5-hour windows; None with
/// fewer than [`FIT_MIN_CYCLES`] counted cycles. Hits count while a counted cycle ran.
pub fn plan_fit(
    tier: Option<PlanTier>,
    cycles: &[&WeeklyCycle],
    windows: &[SessionWindow],
) -> Option<PlanFit> {
    let counted: Vec<&&WeeklyCycle> = cycles.iter().filter(|c| c.counted).collect();
    if counted.len() < FIT_MIN_CYCLES {
        return None;
    }
    let n = counted.len() as f64;
    let avg = counted.iter().map(|c| c.peak).sum::<f64>() / n;
    let peak = counted.iter().map(|c| c.peak).fold(0.0, f64::max);
    let hits = windows
        .iter()
        .filter_map(|w| w.hit_at)
        .filter(|h| counted.iter().any(|c| *h >= c.start && *h < c.resets_at))
        .count();
    let hits_per_week = hits as f64 / n;
    let on = |t: PlanTier, own: PlanTier| peak * own.factor() / t.factor();
    let mut fit = PlanFit {
        verdict: FitVerdict::Fits,
        avg,
        peak,
        hits_per_week,
        cycles: counted.len(),
        tier,
        suggested: None,
        peak_on_suggested: None,
        price_change: None,
    };
    if avg < UNUSED_AVG {
        fit.verdict = FitVerdict::Unused;
        fit.price_change = tier.map(|t| -t.usd_per_month());
    } else if peak >= AT_LIMIT_PEAK || hits_per_week >= AT_LIMIT_HITS {
        fit.verdict = FitVerdict::AtLimit;
        if let Some(t) = tier
            && let Some(up) = t.larger()
        {
            fit.suggested = Some(up);
            fit.peak_on_suggested = Some(on(up, t));
            fit.price_change = Some(up.usd_per_month() - t.usd_per_month());
        }
    } else if let Some(t) = tier
        && let Some(down) = t.smaller()
        && on(down, t) <= DOWNGRADE_PEAK
        && hits_per_week <= DOWNGRADE_HITS
    {
        fit.verdict = FitVerdict::Downgrade;
        fit.suggested = Some(down);
        fit.peak_on_suggested = Some(on(down, t));
        fit.price_change = Some(down.usd_per_month() - t.usd_per_month());
    }
    Some(fit)
}

/// What the what-if changes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WhatIfChange {
    /// The account's access ends (Access end date) on this day.
    Ends(DateTime<Utc>),
    /// The account moves to a smaller tier (a suggested downgrade).
    Tier(PlanTier),
}

/// One account in the what-if: its average weekly use now and after (% of its own plan).
#[derive(Debug, Clone, PartialEq)]
pub struct WhatIfShare {
    pub account: Uuid,
    pub before: f64,
    pub after: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WhatIf {
    pub account: Uuid,
    pub change: WhatIfChange,
    /// Average use moved off the account, in Pro units (avg % x tier factor).
    pub moved: f64,
    /// The changed account after (a smaller tier); None when it ends.
    pub own_after: Option<f64>,
    /// The provider's other accounts, in the order given.
    pub others: Vec<WhatIfShare>,
    /// Use the others had no room for (Pro units).
    pub unplaced: f64,
}

/// An account in a what-if: id, tier and average weekly use (%).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Member {
    pub account: Uuid,
    pub tier: PlanTier,
    pub avg: f64,
}

/// Moves the changed account's use onto `others` in proportion to their free room
/// ((100 - avg) x factor). An ending account moves all of it; a smaller tier keeps what fits in
/// 100 % of the new tier and moves only the rest (a suggested downgrade keeps the peak under
/// 90 %, so usually nothing moves).
pub fn what_if(changed: Member, change: WhatIfChange, others: &[Member]) -> WhatIf {
    let units = changed.avg * changed.tier.factor();
    let (moved, own_after) = match change {
        WhatIfChange::Ends(_) => (units, None),
        WhatIfChange::Tier(t) => {
            let cap = 100.0 * t.factor();
            ((units - cap).max(0.0), Some(units.min(cap) / t.factor()))
        }
    };
    let room = |m: &Member| (100.0 - m.avg).max(0.0) * m.tier.factor();
    let total_room: f64 = others.iter().map(room).sum();
    let placed = moved.min(total_room);
    let others = others
        .iter()
        .map(|m| {
            let share = if total_room > 0.0 {
                placed * room(m) / total_room
            } else {
                0.0
            };
            WhatIfShare {
                account: m.account,
                before: m.avg,
                after: m.avg + share / m.tier.factor(),
            }
        })
        .collect();
    WhatIf {
        account: changed.account,
        change,
        moved,
        own_after,
        others,
        unplaced: moved - placed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insights::readings::WindowEnd;
    use chrono::Duration;

    fn t0() -> DateTime<Utc> {
        "2026-10-05T08:00:00Z".parse().unwrap()
    }

    fn cycle(week: i64, peak: f64, counted: bool) -> WeeklyCycle {
        let start = t0() + Duration::weeks(week);
        WeeklyCycle {
            start,
            resets_at: start + Duration::weeks(1),
            peak,
            days: if counted { 5 } else { 1 },
            ended: true,
            counted,
        }
    }

    fn hits(n: usize, week: i64) -> Vec<SessionWindow> {
        (0..n)
            .map(|i| {
                let start = t0() + Duration::weeks(week) + Duration::days(i as i64);
                SessionWindow {
                    account: Uuid::nil(),
                    start,
                    resets_at: start + Duration::hours(5),
                    peak: 100.0,
                    hit_at: Some(start + Duration::hours(3)),
                    waited: Some(Duration::hours(2)),
                    end: WindowEnd::Seen,
                    free_elsewhere: vec![],
                }
            })
            .collect()
    }

    fn fit(tier: Option<PlanTier>, peaks: &[f64], w: &[SessionWindow]) -> Option<PlanFit> {
        let cs: Vec<WeeklyCycle> = peaks
            .iter()
            .enumerate()
            .map(|(i, p)| cycle(i as i64, *p, true))
            .collect();
        let refs: Vec<&WeeklyCycle> = cs.iter().collect();
        plan_fit(tier, &refs, w)
    }

    #[test]
    fn needs_two_counted_cycles() {
        let cs = [cycle(0, 50.0, true), cycle(1, 50.0, false)];
        let refs: Vec<&WeeklyCycle> = cs.iter().collect();
        assert!(plan_fit(Some(PlanTier::ClaudePro), &refs, &[]).is_none());
    }

    #[test]
    fn verdicts_follow_the_cut_offs() {
        let max20 = Some(PlanTier::ClaudeMax20x);
        // Unused: avg under 5 %; saving the whole price.
        let f = fit(max20, &[3.0, 4.0], &[]).unwrap();
        assert_eq!(
            (f.verdict, f.price_change),
            (FitVerdict::Unused, Some(-200.0))
        );
        // Downgrade: 22 % of Max 20x is 88 % of Max 5x (<= 90), no hits.
        let f = fit(max20, &[20.0, 22.0], &[]).unwrap();
        assert_eq!(f.verdict, FitVerdict::Downgrade);
        assert_eq!(f.suggested, Some(PlanTier::ClaudeMax5x));
        assert_eq!(f.peak_on_suggested, Some(88.0));
        assert_eq!(f.price_change, Some(-100.0));
        // 23 % would be 92 % on Max 5x: Fits.
        assert_eq!(
            fit(max20, &[20.0, 23.0], &[]).unwrap().verdict,
            FitVerdict::Fits
        );
        // Downgrade blocked by 2 hits a week.
        let mut w = hits(2, 0);
        w.extend(hits(2, 1));
        assert_eq!(
            fit(max20, &[20.0, 22.0], &w).unwrap().verdict,
            FitVerdict::Fits
        );
        // At the limit by peak: suggests the next tier with its extra cost.
        let pro = Some(PlanTier::ClaudePro);
        let f = fit(pro, &[80.0, 96.0], &[]).unwrap();
        assert_eq!(
            (f.verdict, f.suggested),
            (FitVerdict::AtLimit, Some(PlanTier::ClaudeMax5x))
        );
        assert_eq!(f.price_change, Some(80.0));
        // At the limit by 3 hits a week.
        let mut w = hits(3, 0);
        w.extend(hits(3, 1));
        let f = fit(pro, &[60.0, 70.0], &w).unwrap();
        assert_eq!((f.verdict, f.hits_per_week), (FitVerdict::AtLimit, 3.0));
        // At the top tier: no suggestion ("spread work").
        let f = fit(max20, &[97.0, 99.0], &[]).unwrap();
        assert_eq!((f.verdict, f.suggested), (FitVerdict::AtLimit, None));
        // Unknown plan: verdicts without tier advice.
        let f = fit(None, &[20.0, 22.0], &[]).unwrap();
        assert_eq!(
            (f.verdict, f.suggested, f.price_change),
            (FitVerdict::Fits, None, None)
        );
        // Hits outside counted cycles do not count.
        let f = fit(pro, &[60.0, 70.0], &hits(5, 9)).unwrap();
        assert_eq!(f.hits_per_week, 0.0);
    }

    #[test]
    fn tiers_from_plan_labels() {
        use ProviderKind::*;
        assert_eq!(
            PlanTier::from_plan(Claude, "Max 20x"),
            Some(PlanTier::ClaudeMax20x)
        );
        assert_eq!(
            PlanTier::from_plan(Claude, "pro"),
            Some(PlanTier::ClaudePro)
        );
        assert_eq!(PlanTier::from_plan(Codex, "Pro"), Some(PlanTier::CodexPro));
        assert_eq!(PlanTier::from_plan(Claude, "Max"), None);
        assert_eq!(PlanTier::from_plan(Codex, "Max 5x"), None);
        assert!(PlanTier::CodexPro.approximate() && !PlanTier::ClaudePro.approximate());
        assert_eq!(PlanTier::CodexPro.smaller(), Some(PlanTier::CodexPlus));
    }

    #[test]
    fn what_if_moves_use_in_proportion_to_free_room() {
        let id = Uuid::from_u128;
        let ending = Member {
            account: id(1),
            tier: PlanTier::ClaudePro,
            avg: 60.0,
        };
        // 60 Pro units onto a Max 5x at 50 % (room 250) and a Pro at 50 % (room 50).
        let others = [
            Member {
                account: id(2),
                tier: PlanTier::ClaudeMax5x,
                avg: 50.0,
            },
            Member {
                account: id(3),
                tier: PlanTier::ClaudePro,
                avg: 50.0,
            },
        ];
        let w = what_if(ending, WhatIfChange::Ends(t0()), &others);
        assert_eq!(w.moved, 60.0);
        assert!((w.others[0].after - 60.0).abs() < 1e-9); // +50 units / 5
        assert!((w.others[1].after - 60.0).abs() < 1e-9); // +10 units
        assert_eq!(w.unplaced, 0.0);
        assert_eq!(w.own_after, None);

        // Not enough room: the rest is unplaced.
        let tight = [Member {
            account: id(3),
            tier: PlanTier::ClaudePro,
            avg: 80.0,
        }];
        let w = what_if(ending, WhatIfChange::Ends(t0()), &tight);
        assert!((w.others[0].after - 100.0).abs() < 1e-9);
        assert!((w.unplaced - 40.0).abs() < 1e-9);

        // A downgrade keeps what fits: 18 % of Max 20x = 72 % of Max 5x, nothing moves.
        let big = Member {
            account: id(4),
            tier: PlanTier::ClaudeMax20x,
            avg: 18.0,
        };
        let w = what_if(big, WhatIfChange::Tier(PlanTier::ClaudeMax5x), &others);
        assert_eq!((w.moved, w.own_after), (0.0, Some(72.0)));
        assert_eq!(w.others[0].after, 50.0);
    }
}

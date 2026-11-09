//! Plan fit and what-if (plan "Analysis", owner 2026-11-07): does an account's plan match its
//! weekly use, and where would its use go if it ended or got smaller.
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::readings::{CycleLen, SessionWindow, WeeklyCycle};
use crate::model::ProviderKind;

/// Subscription tiers with a known size. Factors are relative to the smallest paid tier of the
/// same ladder; rough ones are labelled "about" ([`PlanTier::approximate`]). Sources and dates:
/// `docs/research/plan-prices.md` (Copilot, Cursor, Antigravity, owner 2026-11-09).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PlanTier {
    ClaudePro,
    ClaudeMax5x,
    ClaudeMax20x,
    CodexPlus,
    CodexPro,
    /// Copilot, by its monthly AI-credit entitlement (1,500 / 7,000 / 20,000).
    CopilotPro,
    CopilotProPlus,
    CopilotMax,
    /// Cursor's current lineup (weekly pool, "Nx Plus").
    CursorPlus,
    CursorPremium,
    CursorSuper,
    CursorUltra,
    /// Cursor's legacy lineup (monthly pools); never mixed with the current one, since a switch
    /// to the new lineup is permanent.
    CursorLegacyPro,
    CursorLegacyProPlus,
    CursorLegacyUltra,
    /// Google AI plans behind Antigravity, chosen by hand (no reading names them).
    AgyPro,
    AgyUltra5x,
    AgyUltra20x,
}

/// Date of the price table below (USD per month, list prices).
pub const PLAN_PRICES_AS_OF: &str = "2026-11-09";

/// The Antigravity plans the account page offers, as saved and shown.
pub const AGY_PLAN_CHOICES: [&str; 3] = ["Google AI Pro", "Ultra 5x", "Ultra 20x"];

/// A plan name folded for matching: lowercase words, "plus" after "pro" as "+", without a
/// "cursor" / "google ai" / "unified" prefix ("Pro Plus", "pro_plus", "PRO+" -> "pro+").
fn fold(plan: &str) -> String {
    let lower = plan.trim().to_lowercase();
    let mut words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric() && c != '+')
        .filter(|w| !w.is_empty())
        .collect();
    while let Some(first) = words.first() {
        if matches!(
            *first,
            "cursor" | "google" | "ai" | "unified" | "github" | "copilot"
        ) {
            words.remove(0);
        } else {
            break;
        }
    }
    words
        .join(" ")
        .replace("pro plus", "pro+")
        .replace("pro +", "pro+")
}

impl PlanTier {
    /// From a plan label as shown on the card ("Pro", "Max 5x", "Max 20x", "Plus", ...) and the
    /// length of the account's main long window (`cycle`; None before any reading). "Max"
    /// without a size, Team, Free and others are unknown. Copilot's "Pro" / "Pro+" / "Max" are
    /// only ever set from the credit entitlement (`copilot::parse_plan`). Cursor's names are
    /// read within the ladder its cycle says (weekly: current, monthly: legacy), so both
    /// "Ultra"s are told apart and a name from the other ladder is unknown.
    pub fn from_plan(provider: ProviderKind, plan: &str, cycle: Option<CycleLen>) -> Option<Self> {
        let p = fold(plan);
        use PlanTier::*;
        match (provider, p.as_str()) {
            (ProviderKind::Claude, "pro") => Some(ClaudePro),
            (ProviderKind::Claude, "max 5x") => Some(ClaudeMax5x),
            (ProviderKind::Claude, "max 20x") => Some(ClaudeMax20x),
            (ProviderKind::Codex, "plus") => Some(CodexPlus),
            (ProviderKind::Codex, "pro") => Some(CodexPro),
            (ProviderKind::Copilot, "pro") => Some(CopilotPro),
            (ProviderKind::Copilot, "pro+") => Some(CopilotProPlus),
            (ProviderKind::Copilot, "max") => Some(CopilotMax),
            (ProviderKind::Cursor, name) => match (cycle?, name) {
                (CycleLen::Week, "plus") => Some(CursorPlus),
                (CycleLen::Week, "premium") => Some(CursorPremium),
                (CycleLen::Week, "super") => Some(CursorSuper),
                (CycleLen::Week, "ultra") => Some(CursorUltra),
                (CycleLen::Month, "pro") => Some(CursorLegacyPro),
                (CycleLen::Month, "pro+" | "proplus") => Some(CursorLegacyProPlus),
                (CycleLen::Month, "ultra") => Some(CursorLegacyUltra),
                _ => None,
            },
            (ProviderKind::Antigravity, "pro") => Some(AgyPro),
            (ProviderKind::Antigravity, "ultra 5x") => Some(AgyUltra5x),
            (ProviderKind::Antigravity, "ultra 20x") => Some(AgyUltra20x),
            _ => None,
        }
    }

    pub fn provider(self) -> ProviderKind {
        use PlanTier::*;
        match self {
            ClaudePro | ClaudeMax5x | ClaudeMax20x => ProviderKind::Claude,
            CodexPlus | CodexPro => ProviderKind::Codex,
            CopilotPro | CopilotProPlus | CopilotMax => ProviderKind::Copilot,
            CursorPlus | CursorPremium | CursorSuper | CursorUltra | CursorLegacyPro
            | CursorLegacyProPlus | CursorLegacyUltra => ProviderKind::Cursor,
            AgyPro | AgyUltra5x | AgyUltra20x => ProviderKind::Antigravity,
        }
    }

    /// Capacity relative to the smallest paid tier of its ladder.
    pub fn factor(self) -> f64 {
        use PlanTier::*;
        match self {
            ClaudePro | CodexPlus | CopilotPro | CursorPlus | CursorLegacyPro | AgyPro => 1.0,
            ClaudeMax5x | AgyUltra5x => 5.0,
            ClaudeMax20x | AgyUltra20x | CursorLegacyUltra => 20.0,
            CodexPro => 10.0,
            // Credits: 7,000 and 20,000 against Pro's 1,500.
            CopilotProPlus => 7_000.0 / 1_500.0,
            CopilotMax => 20_000.0 / 1_500.0,
            CursorPremium => 4.0,
            CursorSuper => 15.0,
            CursorUltra => 50.0,
            CursorLegacyProPlus => 3.0,
        }
    }

    /// The factor is a rough guess (Codex, Cursor's legacy Pro+ and Ultra, Antigravity's
    /// Ultras): label it "about".
    pub fn approximate(self) -> bool {
        use PlanTier::*;
        matches!(
            self,
            CodexPlus
                | CodexPro
                | CursorLegacyProPlus
                | CursorLegacyUltra
                | AgyUltra5x
                | AgyUltra20x
        )
    }

    /// Advice by capacity only, never by price: Google AI plans bundle storage and other perks,
    /// so a smaller plan's price difference is not what the AI part costs (owner, 2026-11-09).
    pub fn capacity_only(self) -> bool {
        self.provider() == ProviderKind::Antigravity
    }

    pub fn usd_per_month(self) -> f64 {
        use PlanTier::*;
        match self {
            ClaudePro | CodexPlus | CursorLegacyPro => 20.0,
            ClaudeMax5x | CopilotMax | CursorSuper => 100.0,
            ClaudeMax20x | CodexPro | CursorUltra | CursorLegacyUltra => 200.0,
            CopilotPro => 10.0,
            CopilotProPlus => 39.0,
            CursorPlus => 8.0,
            CursorPremium => 30.0,
            CursorLegacyProPlus => 60.0,
            AgyPro => 19.99,
            AgyUltra5x => 99.99,
            AgyUltra20x => 199.99,
        }
    }

    pub fn label(self) -> &'static str {
        use PlanTier::*;
        match self {
            ClaudePro | CodexPro | CopilotPro | CursorLegacyPro => "Pro",
            ClaudeMax5x => "Max 5x",
            ClaudeMax20x => "Max 20x",
            CodexPlus | CursorPlus => "Plus",
            CopilotProPlus | CursorLegacyProPlus => "Pro+",
            CopilotMax => "Max",
            CursorPremium => "Premium",
            CursorSuper => "Super",
            CursorUltra | CursorLegacyUltra => "Ultra",
            AgyPro => AGY_PLAN_CHOICES[0],
            AgyUltra5x => AGY_PLAN_CHOICES[1],
            AgyUltra20x => AGY_PLAN_CHOICES[2],
        }
    }

    pub fn smaller(self) -> Option<Self> {
        use PlanTier::*;
        match self {
            ClaudeMax20x => Some(ClaudeMax5x),
            ClaudeMax5x => Some(ClaudePro),
            CodexPro => Some(CodexPlus),
            CopilotMax => Some(CopilotProPlus),
            CopilotProPlus => Some(CopilotPro),
            CursorUltra => Some(CursorSuper),
            CursorSuper => Some(CursorPremium),
            CursorPremium => Some(CursorPlus),
            CursorLegacyUltra => Some(CursorLegacyProPlus),
            CursorLegacyProPlus => Some(CursorLegacyPro),
            AgyUltra20x => Some(AgyUltra5x),
            AgyUltra5x => Some(AgyPro),
            ClaudePro | CodexPlus | CopilotPro | CursorPlus | CursorLegacyPro | AgyPro => None,
        }
    }

    pub fn larger(self) -> Option<Self> {
        use PlanTier::*;
        match self {
            ClaudePro => Some(ClaudeMax5x),
            ClaudeMax5x => Some(ClaudeMax20x),
            CodexPlus => Some(CodexPro),
            CopilotPro => Some(CopilotProPlus),
            CopilotProPlus => Some(CopilotMax),
            CursorPlus => Some(CursorPremium),
            CursorPremium => Some(CursorSuper),
            CursorSuper => Some(CursorUltra),
            CursorLegacyPro => Some(CursorLegacyProPlus),
            CursorLegacyProPlus => Some(CursorLegacyUltra),
            AgyPro => Some(AgyUltra5x),
            AgyUltra5x => Some(AgyUltra20x),
            ClaudeMax20x | CodexPro | CopilotMax | CursorUltra | CursorLegacyUltra
            | AgyUltra20x => None,
        }
    }
}

/// Counted weekly cycles needed for a verdict.
pub const FIT_MIN_CYCLES: usize = 2;
/// Counted monthly cycles needed: plan fit looks back 4 weeks, which holds one ended month.
pub const FIT_MIN_MONTHS: usize = 1;

/// Counted cycles a verdict needs for cycles of this length.
pub fn min_cycles(len: CycleLen) -> usize {
    match len {
        CycleLen::Week => FIT_MIN_CYCLES,
        CycleLen::Month => FIT_MIN_MONTHS,
    }
}
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
    /// Average and peak use per counted cycle (week or month; % of the own plan).
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
    /// None for an unknown plan and for capacity-only tiers (Antigravity).
    pub price_change: Option<f64>,
}

/// The verdict of one account from its counted cycles (`len` long) and 5-hour windows; None with
/// fewer than [`min_cycles`] counted cycles. Hits count while a counted cycle ran.
pub fn plan_fit(
    tier: Option<PlanTier>,
    len: CycleLen,
    cycles: &[&WeeklyCycle],
    windows: &[SessionWindow],
) -> Option<PlanFit> {
    let counted: Vec<&&WeeklyCycle> = cycles.iter().filter(|c| c.counted).collect();
    if counted.len() < min_cycles(len) {
        return None;
    }
    // Antigravity: what to change, never what it would save.
    let price = |change: f64| (!tier.is_some_and(PlanTier::capacity_only)).then_some(change);
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
        fit.price_change = tier.and_then(|t| price(-t.usd_per_month()));
    } else if peak >= AT_LIMIT_PEAK || hits_per_week >= AT_LIMIT_HITS {
        fit.verdict = FitVerdict::AtLimit;
        if let Some(t) = tier
            && let Some(up) = t.larger()
        {
            fit.suggested = Some(up);
            fit.peak_on_suggested = Some(on(up, t));
            fit.price_change = price(up.usd_per_month() - t.usd_per_month());
        }
    } else if let Some(t) = tier
        && let Some(down) = t.smaller()
        && on(down, t) <= DOWNGRADE_PEAK
        && hits_per_week <= DOWNGRADE_HITS
    {
        fit.verdict = FitVerdict::Downgrade;
        fit.suggested = Some(down);
        fit.peak_on_suggested = Some(on(down, t));
        fit.price_change = price(down.usd_per_month() - t.usd_per_month());
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
        plan_fit(tier, CycleLen::Week, &refs, w)
    }

    #[test]
    fn needs_two_counted_cycles() {
        let cs = [cycle(0, 50.0, true), cycle(1, 50.0, false)];
        let refs: Vec<&WeeklyCycle> = cs.iter().collect();
        assert!(plan_fit(Some(PlanTier::ClaudePro), CycleLen::Week, &refs, &[]).is_none());
        // One counted month is enough for a monthly plan.
        let f = plan_fit(Some(PlanTier::CopilotPro), CycleLen::Month, &refs, &[]).unwrap();
        assert_eq!((f.cycles, f.verdict), (1, FitVerdict::Fits));
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
        let week = Some(CycleLen::Week);
        let tier = |p, plan| PlanTier::from_plan(p, plan, week);
        assert_eq!(tier(Claude, "Max 20x"), Some(PlanTier::ClaudeMax20x));
        assert_eq!(tier(Claude, "pro"), Some(PlanTier::ClaudePro));
        assert_eq!(tier(Codex, "Pro"), Some(PlanTier::CodexPro));
        assert_eq!(tier(Claude, "Max"), None);
        assert_eq!(tier(Codex, "Max 5x"), None);
        assert!(PlanTier::CodexPro.approximate() && !PlanTier::ClaudePro.approximate());
        assert_eq!(PlanTier::CodexPro.smaller(), Some(PlanTier::CodexPlus));
    }

    #[test]
    fn copilot_and_antigravity_tiers() {
        use ProviderKind::*;
        let month = Some(CycleLen::Month);
        assert_eq!(
            PlanTier::from_plan(Copilot, "Pro+", month),
            Some(PlanTier::CopilotProPlus)
        );
        assert_eq!(
            PlanTier::from_plan(Copilot, "Max", month),
            Some(PlanTier::CopilotMax)
        );
        // Labels without a confirmed entitlement carry no tier.
        for plan in [
            "Free",
            "Education",
            "Pro (legacy)",
            "Pro trial",
            "Individual",
            "Business",
        ] {
            assert_eq!(PlanTier::from_plan(Copilot, plan, month), None, "{plan}");
        }
        assert!((PlanTier::CopilotMax.factor() - 13.33).abs() < 0.01);
        assert_eq!(
            PlanTier::CopilotPro.larger(),
            Some(PlanTier::CopilotProPlus)
        );
        // The manual Google AI choices, with or without the "Google AI" prefix.
        for (plan, t) in [
            ("Google AI Pro", PlanTier::AgyPro),
            ("Ultra 5x", PlanTier::AgyUltra5x),
            ("google ai ultra 20x", PlanTier::AgyUltra20x),
        ] {
            assert_eq!(PlanTier::from_plan(Antigravity, plan, None), Some(t));
            assert_eq!(PlanTier::from_plan(Antigravity, t.label(), None), Some(t));
        }
        assert_eq!(PlanTier::from_plan(Antigravity, "Free", None), None);
        assert!(PlanTier::AgyUltra5x.capacity_only() && !PlanTier::CopilotPro.capacity_only());
    }

    #[test]
    fn cursor_ladders_follow_the_cycle() {
        use ProviderKind::Cursor;
        let (week, month) = (Some(CycleLen::Week), Some(CycleLen::Month));
        let tier = |plan, c| PlanTier::from_plan(Cursor, plan, c);
        assert_eq!(tier("Premium", week), Some(PlanTier::CursorPremium));
        assert_eq!(tier("Super", week), Some(PlanTier::CursorSuper));
        assert_eq!(tier("Plus", week), Some(PlanTier::CursorPlus));
        // Both Ultras, told apart by the cycle.
        assert_eq!(tier("Ultra", week), Some(PlanTier::CursorUltra));
        assert_eq!(tier("ultra", month), Some(PlanTier::CursorLegacyUltra));
        assert_eq!(tier("Ultra", None), None);
        // Legacy names, however they are spelled; never across ladders.
        for plan in ["Pro+", "pro_plus", "Pro Plus", "PRO +"] {
            assert_eq!(
                tier(plan, month),
                Some(PlanTier::CursorLegacyProPlus),
                "{plan}"
            );
        }
        assert_eq!(tier("Pro", month), Some(PlanTier::CursorLegacyPro));
        assert_eq!(tier("Pro+", week), None);
        assert_eq!(tier("Premium", month), None);
        assert_eq!(tier("Hobby", month), None);
        assert_eq!(PlanTier::CursorPlus.smaller(), None);
        assert_eq!(
            PlanTier::CursorLegacyPro.larger(),
            Some(PlanTier::CursorLegacyProPlus)
        );
        assert_eq!(
            PlanTier::CursorLegacyUltra.smaller(),
            Some(PlanTier::CursorLegacyProPlus)
        );
        assert!(PlanTier::CursorLegacyUltra.approximate() && !PlanTier::CursorUltra.approximate());
        assert_eq!(
            (
                PlanTier::CursorUltra.factor(),
                PlanTier::CursorUltra.usd_per_month()
            ),
            (50.0, 200.0)
        );
    }

    #[test]
    fn antigravity_advice_never_has_a_price() {
        let ultra = Some(PlanTier::AgyUltra5x);
        // 15 % of Ultra 5x is 75 % of Google AI Pro: a downgrade, by capacity only.
        let f = fit(ultra, &[12.0, 15.0], &[]).unwrap();
        assert_eq!(
            (f.verdict, f.suggested, f.price_change),
            (FitVerdict::Downgrade, Some(PlanTier::AgyPro), None)
        );
        assert!((f.peak_on_suggested.unwrap() - 75.0).abs() < 1e-9);
        // Unused: no "cancel and save".
        let f = fit(ultra, &[1.0, 2.0], &[]).unwrap();
        assert_eq!((f.verdict, f.price_change), (FitVerdict::Unused, None));
        // At the limit: the next tier by name.
        let f = fit(Some(PlanTier::AgyPro), &[90.0, 97.0], &[]).unwrap();
        assert_eq!(
            (f.verdict, f.suggested, f.price_change),
            (FitVerdict::AtLimit, Some(PlanTier::AgyUltra5x), None)
        );
        // Cursor keeps its prices: Premium at 20 % fits Plus at 80 %, $22 a month less.
        let f = fit(Some(PlanTier::CursorPremium), &[18.0, 20.0], &[]).unwrap();
        assert_eq!(f.suggested, Some(PlanTier::CursorPlus));
        assert_eq!(f.price_change, Some(-22.0));
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

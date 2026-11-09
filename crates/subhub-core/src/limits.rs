//! Pure math for the Limits board (Statistics window): card titles, window lengths, the average
//! "% left" over accounts, what that average regains at the soonest reset, the pace marker and
//! compact countdowns. No UI, no clock: callers pass `now`.
use chrono::{DateTime, Duration, Utc};

/// Card title for a window label: the Claude "Session" window is the 5-hour one
/// ("Session" -> "5-hour", "Session · X" -> "5-hour · X"); everything else is kept.
pub fn window_title(label: &str) -> String {
    match label.strip_prefix("Session") {
        Some(rest) if rest.is_empty() || rest.starts_with(" ·") => format!("5-hour{rest}"),
        _ => label.to_string(),
    }
}

/// Sort rank of a card: 5-hour first, Weekly second, everything else after (stable).
pub fn title_rank(title: &str) -> u8 {
    match title {
        "5-hour" => 0,
        "Weekly" => 1,
        _ => 2,
    }
}

/// Length of a window from its title: "5-hour", "Weekly" / "Weekly · Fable", "2-week", "1-day",
/// "30-minute". Unknown titles (a bare slot name) have no known length.
pub fn window_duration(title: &str) -> Option<Duration> {
    let base = title.split(" ·").next().unwrap_or(title).trim();
    if base == "Weekly" {
        return Some(Duration::days(7));
    }
    // Copilot and legacy Cursor plans (a month is taken as 30 days for the pace marker).
    if base == "Monthly" {
        return Some(Duration::days(30));
    }
    let (n, unit) = base.split_once('-')?;
    let n: i64 = n.parse().ok().filter(|n| *n > 0)?;
    match unit {
        "minute" => Some(Duration::minutes(n)),
        "hour" => Some(Duration::hours(n)),
        "day" => Some(Duration::days(n)),
        "week" => Some(Duration::weeks(n)),
        _ => None,
    }
}

/// One account's fresh reading of a window kind.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Slot {
    /// Percent left, 0..=100.
    pub left: f64,
    pub resets_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aggregate {
    /// Average percent left over the slots.
    pub left: f64,
    /// Percentage points the average gains at the soonest upcoming reset (0 when none).
    pub regain: f64,
}

/// Average % left over `slots`, and how much that average regains when the account(s) with the
/// soonest upcoming reset (strictly after `now`) reset: their used % divided by the slot count.
/// A reset that already passed never counts (resets never refill quotas locally). `None` for no
/// slots.
pub fn aggregate(slots: &[Slot], now: DateTime<Utc>) -> Option<Aggregate> {
    if slots.is_empty() {
        return None;
    }
    let n = slots.len() as f64;
    let left = slots.iter().map(|s| s.left.clamp(0.0, 100.0)).sum::<f64>() / n;
    let soonest = slots
        .iter()
        .filter_map(|s| s.resets_at)
        .filter(|at| *at > now)
        .min();
    let regain = soonest.map_or(0.0, |at| {
        slots
            .iter()
            .filter(|s| s.resets_at == Some(at))
            .map(|s| 100.0 - s.left.clamp(0.0, 100.0))
            .sum::<f64>()
            / n
    });
    Some(Aggregate { left, regain })
}

/// Fraction of a window's time still ahead (remaining / total), clamped to 0..=1; 0 for an
/// empty window.
pub fn pace_fraction(remaining: Duration, total: Duration) -> f32 {
    let total = total.num_seconds();
    if total <= 0 {
        return 0.0;
    }
    (remaining.num_seconds() as f32 / total as f32).clamp(0.0, 1.0)
}

/// "1d 7h", "4h 13m", "12m" (at least "1m"; a passed time is "0m").
pub fn compact_duration(d: Duration) -> String {
    let secs = d.num_seconds();
    if secs <= 0 {
        return "0m".into();
    }
    let (days, h, m) = (secs / 86_400, (secs % 86_400) / 3600, (secs % 3600) / 60);
    if days > 0 {
        format!("{days}d {h}h")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{}m", m.max(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        "2026-11-07T12:00:00Z".parse().unwrap()
    }

    fn slot(left: f64, resets_in: Option<Duration>) -> Slot {
        Slot {
            left,
            resets_at: resets_in.map(|d| now() + d),
        }
    }

    #[test]
    fn five_hour_board_matches_the_screenshot() {
        // Work 73% (4h 13m), Side 96% (a minute later), Personal 100% (not started).
        let slots = [
            slot(73.0, Some(Duration::minutes(253))),
            slot(96.0, Some(Duration::minutes(254))),
            slot(100.0, None),
        ];
        let a = aggregate(&slots, now()).unwrap();
        assert_eq!(a.left.round(), 90.0);
        assert_eq!(a.regain.round(), 9.0);
    }

    #[test]
    fn weekly_board_matches_the_screenshot() {
        // Work 39% (4d 3h), Side 20% (1d 7h), Personal 99% (4d 4h).
        let slots = [
            slot(39.0, Some(Duration::hours(99))),
            slot(20.0, Some(Duration::hours(31))),
            slot(99.0, Some(Duration::hours(100))),
        ];
        let a = aggregate(&slots, now()).unwrap();
        assert_eq!(a.left.round(), 53.0);
        assert_eq!(a.regain.round(), 27.0);
    }

    #[test]
    fn nothing_regains_without_an_upcoming_reset() {
        let slots = [
            slot(100.0, Some(Duration::hours(2))),
            slot(100.0, Some(Duration::hours(3))),
        ];
        assert_eq!(aggregate(&slots, now()).unwrap().regain, 0.0);
        // A reset in the past never counts.
        let past = [slot(40.0, Some(Duration::minutes(-5))), slot(60.0, None)];
        let a = aggregate(&past, now()).unwrap();
        assert_eq!((a.left, a.regain), (50.0, 0.0));
        assert_eq!(aggregate(&[], now()), None);
    }

    #[test]
    fn accounts_resetting_at_the_same_time_regain_together() {
        let at = Some(Duration::hours(1));
        let a = aggregate(&[slot(50.0, at), slot(70.0, at), slot(100.0, None)], now()).unwrap();
        assert!((a.regain - 80.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn pace_is_the_time_still_ahead() {
        let week = Duration::days(7);
        assert_eq!(pace_fraction(Duration::days(7), week), 1.0);
        assert!((pace_fraction(Duration::hours(31), week) - 31.0 / 168.0).abs() < 1e-6);
        assert_eq!(pace_fraction(Duration::hours(-1), week), 0.0);
        assert_eq!(pace_fraction(Duration::days(9), week), 1.0);
        assert_eq!(pace_fraction(Duration::hours(1), Duration::zero()), 0.0);
    }

    #[test]
    fn compact_durations() {
        assert_eq!(compact_duration(Duration::minutes(253)), "4h 13m");
        assert_eq!(compact_duration(Duration::hours(31)), "1d 7h");
        assert_eq!(compact_duration(Duration::hours(99)), "4d 3h");
        assert_eq!(compact_duration(Duration::minutes(12)), "12m");
        assert_eq!(compact_duration(Duration::seconds(20)), "1m");
        assert_eq!(compact_duration(Duration::seconds(-5)), "0m");
    }

    #[test]
    fn titles_and_lengths() {
        assert_eq!(window_title("Session"), "5-hour");
        assert_eq!(window_title("5-hour"), "5-hour");
        assert_eq!(window_title("Weekly · Fable"), "Weekly · Fable");
        assert_eq!(window_title("Sessions"), "Sessions");
        assert_eq!(window_duration("5-hour"), Some(Duration::hours(5)));
        assert_eq!(window_duration("Weekly"), Some(Duration::days(7)));
        assert_eq!(window_duration("Weekly · Fable"), Some(Duration::days(7)));
        assert_eq!(window_duration("2-week"), Some(Duration::weeks(2)));
        assert_eq!(window_duration("30-minute"), Some(Duration::minutes(30)));
        assert_eq!(window_duration("secondary"), None);
        assert!(title_rank("5-hour") < title_rank("Weekly"));
        assert!(title_rank("Weekly") < title_rank("Weekly · Fable"));
    }
}

//! Text formatting for usage values and reset times.
use chrono::{DateTime, Local, Utc};
use subhub_core::model::AllowanceWindow;
use subhub_core::settings::{DisplaySettings, UsageStyle};

/// "58% left" / "42% used".
pub fn usage(w: &AllowanceWindow, d: &DisplaySettings) -> String {
    match d.usage_style {
        UsageStyle::Remaining => format!("{:.0}% left", w.remaining_percent()),
        UsageStyle::Used => format!("{:.0}% used", w.used_percent.clamp(0.0, 100.0)),
    }
}

/// Short number for bars: "58%".
pub fn percent(w: &AllowanceWindow, d: &DisplaySettings) -> String {
    match d.usage_style {
        UsageStyle::Remaining => format!("{:.0}%", w.remaining_percent()),
        UsageStyle::Used => format!("{:.0}%", w.used_percent.clamp(0.0, 100.0)),
    }
}

/// Fraction for gauges, following the usage style.
pub fn gauge_fraction(w: &AllowanceWindow, d: &DisplaySettings) -> f32 {
    let v = match d.usage_style {
        UsageStyle::Remaining => w.remaining_percent(),
        UsageStyle::Used => w.used_percent.clamp(0.0, 100.0),
    };
    (v / 100.0) as f32
}

/// "2d 4h", "1h 13m", "12m"; a passed reset shows "reset due" (never refills locally).
pub fn duration_until(at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let secs = (at - now).num_seconds();
    if secs <= 0 {
        return "reset due".into();
    }
    subhub_core::limits::compact_duration(at - now)
}

/// The reset as a countdown ("2d 4h"). Always relative since 2026-11-07 (owner); the exact time
/// is in `reset_exact`, shown as a tooltip where one exists.
pub fn reset(w: &AllowanceWindow, now: DateTime<Utc>) -> Option<String> {
    Some(duration_until(w.resets_at?, now))
}

/// "Resets Sat, Nov 8, 14:30" in local time, for the tooltip on a reset countdown.
pub fn reset_exact(at: DateTime<Utc>) -> String {
    format!(
        "Resets {}",
        at.with_timezone(&Local).format("%a, %b %-d, %H:%M")
    )
}

pub fn ago(at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let secs = (now - at).num_seconds().max(0);
    if secs < 60 {
        "just now".into()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn durations() {
        let now = Utc::now();
        assert_eq!(duration_until(now + Duration::minutes(73), now), "1h 13m");
        assert_eq!(duration_until(now + Duration::hours(52), now), "2d 4h");
        assert_eq!(duration_until(now + Duration::seconds(20), now), "1m");
        assert_eq!(duration_until(now - Duration::minutes(1), now), "reset due");
    }
}

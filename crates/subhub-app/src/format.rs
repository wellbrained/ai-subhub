//! Text formatting for usage values and reset times.
use chrono::{DateTime, Local, Utc};
use subhub_core::model::AllowanceWindow;
use subhub_core::settings::{DisplaySettings, ResetFormat, UsageStyle};

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

pub fn reset(w: &AllowanceWindow, d: &DisplaySettings, now: DateTime<Utc>) -> Option<String> {
    let at = w.resets_at?;
    Some(match d.reset_format {
        ResetFormat::Relative => duration_until(at, now),
        ResetFormat::Exact => {
            let local = at.with_timezone(&Local);
            if (at - now).num_hours() < 20 {
                local.format("%H:%M").to_string()
            } else {
                local.format("%a %H:%M").to_string()
            }
        }
    })
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

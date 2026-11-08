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

/// Text that may not fit: cut off with "…", the full text in a tooltip on hover (owner,
/// 2026-11-08: names that are cut off show in full on hover). `key` keeps the element id
/// unique next to other clipped texts (pass an index where the same text can repeat).
pub trait Clip: gpui::Styled + gpui::InteractiveElement + gpui::ParentElement + Sized {
    fn clip(
        self,
        key: impl std::fmt::Display,
        text: impl Into<gpui::SharedString>,
    ) -> gpui::Stateful<Self>;
}

impl Clip for gpui::Div {
    fn clip(
        self,
        key: impl std::fmt::Display,
        text: impl Into<gpui::SharedString>,
    ) -> gpui::Stateful<Self> {
        use gpui::{
            InteractiveElement as _, ParentElement as _, StatefulInteractiveElement as _,
            Styled as _,
        };
        let text: gpui::SharedString = text.into();
        let tip = text.clone();
        self.id(gpui::SharedString::from(format!("clip-{key}-{text}")))
            .truncate()
            .child(text)
            .tooltip(move |window, cx| {
                gpui_component::tooltip::Tooltip::new(tip.clone()).build(window, cx)
            })
    }
}

/// Lists that can grow without end (one row per account, project, model…) show about this many
/// rows and scroll the rest (owner, 2026-11-08).
pub const VISIBLE_ROWS: usize = 5;

/// `list` as is up to `VISIBLE_ROWS` rows; beyond that in a scroll area about that many rows
/// high (`row_h`: a typical row with its gap). `key` keeps the scroll area's id unique.
pub fn capped(key: &str, count: usize, row_h: f32, list: gpui::Div) -> gpui::AnyElement {
    use gpui::{InteractiveElement as _, IntoElement as _, ParentElement as _, Styled as _};
    use gpui_component::scroll::ScrollableElement as _;
    if count <= VISIBLE_ROWS {
        return list.into_any_element();
    }
    gpui::div()
        .id(gpui::SharedString::from(format!("capped-{key}")))
        .max_h(gpui::px(row_h * VISIBLE_ROWS as f32))
        .overflow_y_scrollbar()
        // Room for the overlay scrollbar.
        .child(list.pr(gpui::px(14.0)))
        .into_any_element()
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

//! Monthly billing (port of the C# `MonthlyBilling.cs`). The date is entered by hand: neither the
//! Claude profile nor Codex's app-server exposes a reliable billing date. Monthly recurrence only;
//! later dates are estimates that keep the original day of month (Jan 31 -> Feb 28 -> Mar 31).
//! SubHub never buys, cancels or changes a subscription.
use chrono::{Datelike, Months, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::model::ProviderKind;

fn yes() -> bool {
    true
}

/// An account's tracked monthly date. `auto_renew == false` means a fixed access end date.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonthlyBilling {
    /// The date as entered (the monthly anchor), local calendar.
    pub date: NaiveDate,
    /// true = Monthly renewal; false = Access end date (never recurs).
    #[serde(default = "yes")]
    pub auto_renew: bool,
    /// Remind 7 and 3 days before.
    #[serde(default = "yes")]
    pub reminders: bool,
}

impl MonthlyBilling {
    pub fn new(date: NaiveDate) -> Self {
        Self {
            date,
            auto_renew: true,
            reminders: true,
        }
    }
}

/// The last reminder shown for an account: the cycle date and the threshold (7 or 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReminderMark {
    pub date: NaiveDate,
    pub days: i32,
}

/// A reminder that is due now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reminder {
    pub date: NaiveDate,
    pub days_remaining: i64,
    /// 7 or 3: the most urgent threshold reached.
    pub threshold: i32,
}

/// The date the next cycle falls on. Renewals roll forward monthly from the anchor; an access end
/// date stays where it is (also when it is in the past).
pub fn next_date(billing: &MonthlyBilling, today: NaiveDate) -> NaiveDate {
    if !billing.auto_renew || billing.date >= today {
        return billing.date;
    }
    let months = (today.year() - billing.date.year()) * 12 + today.month() as i32
        - billing.date.month() as i32;
    let months = months.max(0) as u32;
    // Always count from the anchor, so a 31st never drifts to the 28th for good.
    let at = |m: u32| billing.date.checked_add_months(Months::new(m));
    match at(months) {
        Some(d) if d >= today => d,
        Some(_) => at(months + 1).unwrap_or(billing.date),
        None => billing.date,
    }
}

/// Days from `today` to the next date (negative = ago).
pub fn days_until(billing: &MonthlyBilling, today: NaiveDate) -> i64 {
    (next_date(billing, today) - today).num_days()
}

/// True when the shown date is a projection, not the date that was entered.
pub fn is_estimate(billing: &MonthlyBilling, today: NaiveDate) -> bool {
    next_date(billing, today) != billing.date
}

/// "Renews Dec 1, 2026 · in 25 days · estimated"
pub fn description(billing: &MonthlyBilling, today: NaiveDate) -> String {
    let date = next_date(billing, today);
    let days = (date - today).num_days();
    let verb = if !billing.auto_renew && days < 0 {
        "Ended"
    } else if billing.auto_renew {
        "Renews"
    } else {
        "Ends"
    };
    let relative = match days {
        0 => "today".to_string(),
        1 => "tomorrow".to_string(),
        d if d > 0 => format!("in {d} days"),
        d => format!("{} days ago", -d),
    };
    let source = if date == billing.date {
        "manual"
    } else {
        "estimated"
    };
    format!("{verb} {} · {relative} · {source}", format_date(date))
}

/// "Dec 1, 2026"
pub fn format_date(d: NaiveDate) -> String {
    d.format("%b %-d, %Y").to_string()
}

/// The reminder due today, if any: 7 days (or fewer) before, escalating to 3. Only the most
/// urgent threshold is delivered when several were missed, and a (cycle, threshold) pair that was
/// already shown is not repeated.
pub fn reminder(
    billing: &MonthlyBilling,
    shown: Option<&ReminderMark>,
    today: NaiveDate,
) -> Option<Reminder> {
    if !billing.reminders {
        return None;
    }
    let date = next_date(billing, today);
    let days = (date - today).num_days();
    if !(0..=7).contains(&days) {
        return None;
    }
    let threshold = if days <= 3 { 3 } else { 7 };
    if shown.is_some_and(|m| m.date == date && m.days <= threshold) {
        return None;
    }
    Some(Reminder {
        date,
        days_remaining: days,
        threshold,
    })
}

/// The browser page for the provider's billing (opens the browser's own signed-in account, which
/// may differ from the CLI profile).
pub fn billing_url(provider: ProviderKind) -> &'static str {
    match provider {
        ProviderKind::Claude => "https://claude.ai/settings/billing",
        ProviderKind::Codex => "https://chatgpt.com/#settings/Account",
        ProviderKind::Copilot => "https://github.com/settings/copilot",
        ProviderKind::Cursor => "https://cursor.com/dashboard",
        ProviderKind::Antigravity => "https://one.google.com/settings",
    }
}

/// Strict `YYYY-MM-DD`.
pub fn parse_date(text: &str) -> Option<NaiveDate> {
    let t = text.trim();
    (t.len() == 10).then(|| NaiveDate::parse_from_str(t, "%Y-%m-%d").ok())?
}

/// Result of saving an edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub billing: MonthlyBilling,
    /// The reminder history still applies (same cycle and kind).
    pub keep_history: bool,
}

/// The editor's save rule: a renewal date must be today or later; entering the date that is
/// already shown (an estimate) keeps the original monthly anchor and the reminder history.
pub fn apply_edit(
    existing: Option<&MonthlyBilling>,
    entered: NaiveDate,
    auto_renew: bool,
    today: NaiveDate,
) -> Result<Edit, &'static str> {
    if auto_renew && entered < today {
        return Err("Use the next renewal date, today or later.");
    }
    let same =
        existing.is_some_and(|b| next_date(b, today) == entered && b.auto_renew == auto_renew);
    let reminders = existing.is_none_or(|b| b.reminders);
    let date = match existing {
        Some(b) if same => b.date,
        _ => entered,
    };
    Ok(Edit {
        billing: MonthlyBilling {
            date,
            auto_renew,
            reminders,
        },
        keep_history: same,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    #[test]
    fn monthly_dates_keep_the_original_day_without_drift() {
        for (anchor, today, expected) in [
            ("2026-01-31", "2026-02-01", "2026-02-28"),
            ("2028-01-31", "2028-02-01", "2028-02-29"),
            ("2026-01-31", "2026-03-01", "2026-03-31"),
            ("2026-01-31", "2026-04-01", "2026-04-30"),
            ("2026-12-31", "2027-01-01", "2027-01-31"),
            ("2026-10-24", "2026-10-24", "2026-10-24"),
            ("2026-10-24", "2026-10-23", "2026-10-24"),
            ("2026-10-24", "2026-10-25", "2026-11-24"),
        ] {
            assert_eq!(
                next_date(&MonthlyBilling::new(d(anchor)), d(today)),
                d(expected),
                "{anchor} seen on {today}"
            );
        }
    }

    #[test]
    fn canceled_access_ends_without_rolling_into_another_month() {
        let b = MonthlyBilling {
            auto_renew: false,
            ..MonthlyBilling::new(d("2026-10-24"))
        };
        let today = d("2026-10-25");
        assert_eq!(next_date(&b, today), b.date);
        assert!(description(&b, today).starts_with("Ended"));
        assert!(reminder(&b, None, today).is_none());
    }

    #[test]
    fn catch_up_chooses_only_the_most_urgent_reminder() {
        let today = d("2026-10-24");
        for (days, threshold) in [
            (8, None),
            (7, Some(7)),
            (5, Some(7)),
            (3, Some(3)),
            (1, Some(3)),
            (0, Some(3)),
        ] {
            let b = MonthlyBilling::new(today + chrono::Duration::days(days));
            assert_eq!(
                reminder(&b, None, today).map(|r| r.threshold),
                threshold,
                "{days} days"
            );
        }
    }

    #[test]
    fn reminders_are_deduplicated_per_cycle_and_threshold() {
        let due = d("2026-10-31");
        let b = MonthlyBilling::new(due);
        let mut mark = ReminderMark { date: due, days: 7 };
        assert!(reminder(&b, Some(&mark), due - chrono::Duration::days(5)).is_none());
        assert_eq!(
            reminder(&b, Some(&mark), due - chrono::Duration::days(3)).map(|r| r.threshold),
            Some(3)
        );
        mark.days = 3;
        assert!(reminder(&b, Some(&mark), due).is_none());
        // The next monthly cycle is a new date, so the 7-day reminder fires again.
        assert_eq!(
            reminder(&b, Some(&mark), d("2026-11-23")).map(|r| r.threshold),
            Some(7)
        );
    }

    #[test]
    fn tracking_is_optional_and_projections_are_marked() {
        let date = d("2026-10-24");
        let off = MonthlyBilling {
            reminders: false,
            ..MonthlyBilling::new(date)
        };
        assert!(reminder(&off, None, date).is_none());
        let b = MonthlyBilling::new(date);
        assert!(description(&b, date).contains("manual"));
        assert!(!is_estimate(&b, date));
        let later = date + chrono::Duration::days(1);
        assert!(description(&b, later).contains("estimated"));
        assert!(is_estimate(&b, later));
        let end = MonthlyBilling {
            auto_renew: false,
            ..b
        };
        assert!(description(&end, date).starts_with("Ends"));
    }

    #[test]
    fn description_reads_like_the_popover_line() {
        let b = MonthlyBilling::new(d("2026-12-01"));
        assert_eq!(
            description(&b, d("2026-11-06")),
            "Renews Dec 1, 2026 · in 25 days · manual"
        );
        assert_eq!(
            description(&b, d("2026-11-30")),
            "Renews Dec 1, 2026 · tomorrow · manual"
        );
        assert_eq!(
            description(&b, d("2026-12-01")),
            "Renews Dec 1, 2026 · today · manual"
        );
        assert_eq!(
            description(&b, d("2027-01-01")),
            "Renews Jan 1, 2027 · today · estimated"
        );
    }

    #[test]
    fn parse_date_is_strict() {
        assert_eq!(parse_date(" 2026-12-01 "), Some(d("2026-12-01")));
        for bad in [
            "",
            "2026-2-1",
            "01.12.2026",
            "2026-13-01",
            "2026-02-30",
            "x",
        ] {
            assert_eq!(parse_date(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn edit_rules() {
        let today = d("2026-11-06");
        // A renewal in the past is refused; an access end in the past is fine.
        assert!(apply_edit(None, d("2026-11-05"), true, today).is_err());
        assert!(apply_edit(None, d("2026-11-05"), false, today).is_ok());
        // Entering the shown (estimated) date keeps the monthly anchor and the history.
        let existing = MonthlyBilling::new(d("2026-01-31"));
        let shown = next_date(&existing, today);
        assert_eq!(shown, d("2026-11-30"));
        let e = apply_edit(Some(&existing), shown, true, today).unwrap();
        assert_eq!(e.billing.date, d("2026-01-31"));
        assert!(e.keep_history);
        // A different date, or another kind, starts a fresh cycle.
        let e = apply_edit(Some(&existing), d("2026-12-05"), true, today).unwrap();
        assert_eq!(e.billing.date, d("2026-12-05"));
        assert!(!e.keep_history);
        let e = apply_edit(Some(&existing), shown, false, today).unwrap();
        assert!(!e.billing.auto_renew && !e.keep_history);
        // Reminder preference survives an edit.
        let off = MonthlyBilling {
            reminders: false,
            ..existing
        };
        assert!(
            !apply_edit(Some(&off), d("2026-12-05"), true, today)
                .unwrap()
                .billing
                .reminders
        );
    }

    #[test]
    fn urls_per_provider() {
        assert_eq!(
            billing_url(ProviderKind::Claude),
            "https://claude.ai/settings/billing"
        );
        assert_eq!(
            billing_url(ProviderKind::Codex),
            "https://chatgpt.com/#settings/Account"
        );
    }

    #[test]
    fn roundtrip_and_defaults() {
        let b: MonthlyBilling = serde_json::from_str(r#"{"date":"2026-10-31"}"#).unwrap();
        assert!(b.auto_renew && b.reminders);
        let json = serde_json::to_string(&b).unwrap();
        assert_eq!(serde_json::from_str::<MonthlyBilling>(&json).unwrap(), b);
    }
}

//! Hover card content (pure): what the card over the tray icon or a taskbar badge says, and the
//! plain-text tip Windows keeps for screen readers. The app draws the card; formatting of
//! % and reset times for display stays with the app (it follows the Display settings).
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::model::{Account, ProviderKind, Reading, UsageState};

/// The steering role of an account (hub steering, ADR 0001).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Steering {
    None,
    /// The user picked it for new hub threads.
    Preferred,
    /// Auto steering's current target.
    Next,
}

/// How far the card can trust the numbers. Missing data is never shown as zero or full.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Fresh,
    /// Last known usage: the refresh failed or is overdue.
    Stale,
    CoolingDown,
    SignInNeeded,
    CliMissing,
    /// Disabled in the hub: shown, not counted as capacity.
    Benched,
    /// No reading yet this session or saved.
    NoReading,
}

impl Health {
    /// Short label for the card and the text tip; None when fresh.
    pub fn label(self) -> Option<&'static str> {
        match self {
            Health::Fresh => None,
            Health::Stale => Some("Stale"),
            Health::CoolingDown => Some("Cooling down"),
            Health::SignInNeeded => Some("Sign in needed"),
            Health::CliMissing => Some("CLI missing"),
            Health::Benched => Some("Benched"),
            Health::NoReading => Some("No reading yet"),
        }
    }

    /// The numbers are last known values, not current ones (the card dims them).
    pub fn dimmed(self) -> bool {
        !matches!(self, Health::Fresh | Health::NoReading)
    }
}

/// What the app knows about one account.
pub struct AccountView<'a> {
    pub account: &'a Account,
    pub reading: Option<&'a Reading>,
    /// State of the last usage check; None = none yet.
    pub last_state: Option<UsageState>,
    /// Last check failed or the reading is overdue (`AppState::is_stale`).
    pub stale: bool,
    pub plan: Option<String>,
    pub steering: Steering,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CardWindow {
    /// "5h", "W", or the provider's own label for other windows.
    pub label: String,
    pub remaining_percent: f64,
    pub used_percent: f64,
    pub resets_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CardAccount {
    pub id: Uuid,
    pub name: String,
    pub provider: ProviderKind,
    pub plan: Option<String>,
    pub health: Health,
    pub steering: Steering,
    pub fetched_at: Option<DateTime<Utc>>,
    /// 5-hour then weekly (as on the badges), then the account's other windows (a model's own
    /// weekly limit such as "Fable", owner 2026-11-08) by their short name.
    pub windows: Vec<CardWindow>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub accounts: Vec<CardAccount>,
    /// Visible accounts left out to keep the card small ("+N more").
    pub more: usize,
}

pub fn health(v: &AccountView) -> Health {
    if v.account.hub.as_ref().is_some_and(|h| h.benched) {
        return Health::Benched;
    }
    match v.last_state {
        Some(UsageState::RateLimited) => return Health::CoolingDown,
        Some(UsageState::SignInRequired) => return Health::SignInNeeded,
        Some(UsageState::CliMissing) => return Health::CliMissing,
        _ => {}
    }
    if v.reading.is_none() {
        Health::NoReading
    } else if v.stale {
        Health::Stale
    } else {
        Health::Fresh
    }
}

pub fn card_account(v: &AccountView) -> CardAccount {
    let a = v.account;
    let visible: Vec<_> = v
        .reading
        .map(|r| {
            r.windows
                .iter()
                .filter(|w| !a.hidden_windows.contains(&w.key))
                .collect()
        })
        .unwrap_or_default();
    let window = |w: &crate::model::AllowanceWindow, label: &str| CardWindow {
        label: label.to_string(),
        remaining_percent: w.remaining_percent(),
        used_percent: w.used_percent.clamp(0.0, 100.0),
        resets_at: w.resets_at,
    };
    let mut windows: Vec<CardWindow> = Vec::new();
    if let Some(w) = visible.iter().find(|w| w.is_session()) {
        windows.push(window(w, "5h"));
    }
    if let Some(w) = visible.iter().find(|w| w.is_weekly()) {
        windows.push(window(w, "W"));
    }
    let main = |w: &&&crate::model::AllowanceWindow| w.is_session() || w.is_weekly();
    if windows.is_empty() {
        windows = visible
            .iter()
            .take(2)
            .map(|w| window(w, &w.label))
            .collect();
    } else {
        // Other windows after 5h and W: "Weekly · Fable" shows as "Fable".
        for w in visible.iter().filter(|w| !main(w)) {
            let short = w.label.rsplit(" · ").next().unwrap_or(&w.label);
            windows.push(window(w, short));
        }
    }
    CardAccount {
        id: a.id,
        name: a.label.clone(),
        provider: a.provider,
        plan: v.plan.clone(),
        health: health(v),
        steering: v.steering,
        fetched_at: v.reading.map(|r| r.fetched_at),
        windows,
    }
}

/// The tray card: every visible account in settings order, at most `max` (then "+N more").
pub fn tray_card(views: &[AccountView], max: usize) -> Card {
    let visible: Vec<&AccountView> = views.iter().filter(|v| !v.account.hidden).collect();
    let accounts = visible.iter().take(max).map(|v| card_account(v)).collect();
    Card {
        accounts,
        more: visible.len().saturating_sub(max),
    }
}

/// Windows' tray tip holds 128 UTF-16 units including the terminating NUL.
pub const TIP_MAX: usize = 127;

/// Plain-text tip for the tray icon (shown by Windows when hover cards are off, and always read
/// by screen readers): "AI SubHub · % left", then "Name 5h 37% W 64%" per account plus its
/// state. Cut at a line end with "…" to stay within `TIP_MAX`.
pub fn tip_text(card: &Card) -> String {
    let mut lines = vec!["AI SubHub · % left".to_string()];
    for a in &card.accounts {
        let mut l = a.name.clone();
        for w in &a.windows {
            l.push_str(&format!(" {} {:.0}%", w.label, w.remaining_percent));
        }
        if let Some(h) = a.health.label() {
            l.push_str(&format!(" ({})", h.to_lowercase()));
        }
        lines.push(l);
    }
    if card.more > 0 {
        lines.push(format!("+{} more", card.more));
    }
    let units = |s: &str| s.encode_utf16().count();
    let mut out = String::new();
    for (i, l) in lines.iter().enumerate() {
        let sep = usize::from(i > 0);
        // Room for this line, and for "\n…" if more lines follow.
        let tail = if i + 1 < lines.len() { 2 } else { 0 };
        if units(&out) + sep + units(l) + tail > TIP_MAX {
            if i == 0 {
                return l.chars().take(TIP_MAX).collect();
            }
            out.push_str("\n…");
            return out;
        }
        if sep == 1 {
            out.push('\n');
        }
        out.push_str(l);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AllowanceWindow, HubLink};

    fn account(label: &str) -> Account {
        Account {
            id: Uuid::new_v4(),
            provider: ProviderKind::Claude,
            label: label.into(),
            email: None,
            profile_dir: None,
            hub: None,
            group: None,
            hidden: false,
            hidden_windows: vec![],
            pinned_windows: vec![],
        }
    }

    fn reading(windows: &[(&str, f64)]) -> Reading {
        Reading {
            fetched_at: Utc::now(),
            windows: windows
                .iter()
                .map(|(k, used)| AllowanceWindow {
                    key: k.to_string(),
                    label: k.to_string(),
                    used_percent: *used,
                    resets_at: None,
                })
                .collect(),
        }
    }

    fn view<'a>(a: &'a Account, r: Option<&'a Reading>) -> AccountView<'a> {
        AccountView {
            account: a,
            reading: r,
            last_state: Some(UsageState::Ready),
            stale: false,
            plan: None,
            steering: Steering::None,
        }
    }

    #[test]
    fn session_then_weekly_and_hidden_windows_left_out() {
        let mut a = account("Hotmail");
        let r = reading(&[
            ("seven_day", 36.0),
            ("seven_day_opus", 10.0),
            ("five_hour", 63.0),
        ]);
        let c = card_account(&view(&a, Some(&r)));
        let got: Vec<_> = c
            .windows
            .iter()
            .map(|w| (w.label.as_str(), w.remaining_percent))
            .collect();
        // 5h and W first, then the model window (Opus) after them.
        assert_eq!(got[..2], [("5h", 37.0), ("W", 64.0)]);
        assert_eq!(got.len(), 3);
        assert_eq!(got[2].1, 90.0);
        a.hidden_windows = vec!["five_hour".into(), "seven_day_opus".into()];
        assert_eq!(card_account(&view(&a, Some(&r))).windows.len(), 1);
    }

    #[test]
    fn model_windows_follow_session_and_weekly() {
        let a = account("X");
        let mut r = reading(&[("session", 10.0), ("weekly_all", 20.0)]);
        r.windows.push(AllowanceWindow {
            key: "weekly_scoped:Fable".into(),
            label: "Weekly · Fable".into(),
            used_percent: 30.0,
            resets_at: None,
        });
        let c = card_account(&view(&a, Some(&r)));
        let labels: Vec<&str> = c.windows.iter().map(|w| w.label.as_str()).collect();
        assert_eq!(labels, ["5h", "W", "Fable"]);
        assert_eq!(c.windows[2].remaining_percent, 70.0);
    }

    #[test]
    fn other_windows_only_when_no_session_or_weekly() {
        let a = account("X");
        let r = reading(&[("codex:other", 20.0)]);
        let c = card_account(&view(&a, Some(&r)));
        assert_eq!(c.windows[0].label, "codex:other");
    }

    #[test]
    fn health_follows_the_last_check() {
        let mut a = account("A");
        let r = reading(&[("five_hour", 0.0)]);
        let mut v = view(&a, Some(&r));
        assert_eq!(health(&v), Health::Fresh);
        v.stale = true;
        assert_eq!(health(&v), Health::Stale);
        v.last_state = Some(UsageState::RateLimited);
        assert_eq!(health(&v), Health::CoolingDown);
        v.last_state = Some(UsageState::SignInRequired);
        assert_eq!(health(&v), Health::SignInNeeded);
        assert_eq!(health(&view(&a, None)), Health::NoReading);
        a.hub = Some(HubLink {
            auth_index: serde_json::json!(1),
            file_name: "x.json".into(),
            benched: true,
            account_id: None,
            priority: None,
        });
        assert_eq!(health(&view(&a, Some(&r))), Health::Benched);
    }

    #[test]
    fn missing_data_is_not_zero_or_full() {
        let a = account("A");
        let c = card_account(&view(&a, None));
        assert!(c.windows.is_empty());
        assert_eq!(c.health.label(), Some("No reading yet"));
        assert!(!c.health.dimmed());
    }

    #[test]
    fn tray_card_skips_hidden_accounts_and_counts_the_rest() {
        let accounts: Vec<Account> = (0..5)
            .map(|i| {
                let mut a = account(&format!("A{i}"));
                a.hidden = i == 1;
                a
            })
            .collect();
        let views: Vec<_> = accounts.iter().map(|a| view(a, None)).collect();
        let card = tray_card(&views, 3);
        let names: Vec<_> = card.accounts.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["A0", "A2", "A3"]);
        assert_eq!(card.more, 1);
    }

    #[test]
    fn tip_text_lists_both_windows_and_states() {
        let a = account("Hotmail");
        let b = account("Tavora");
        let r = reading(&[("five_hour", 63.0), ("seven_day", 36.0)]);
        let mut vb = view(&b, Some(&r));
        vb.stale = true;
        let card = tray_card(&[view(&a, Some(&r)), vb], 6);
        assert_eq!(
            tip_text(&card),
            "AI SubHub · % left\nHotmail 5h 37% W 64%\nTavora 5h 37% W 64% (stale)"
        );
    }

    #[test]
    fn tip_text_stays_within_the_windows_limit() {
        let accounts: Vec<Account> = (0..12)
            .map(|i| account(&format!("A long account name {i}")))
            .collect();
        let r = reading(&[("five_hour", 1.0), ("seven_day", 2.0)]);
        let views: Vec<_> = accounts.iter().map(|a| view(a, Some(&r))).collect();
        let tip = tip_text(&tray_card(&views, 12));
        assert!(tip.encode_utf16().count() <= TIP_MAX, "{}", tip.len());
        assert!(tip.ends_with("\n…"));
    }
}

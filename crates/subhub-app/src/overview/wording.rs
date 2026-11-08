use super::*;

// ---------- wording ----------

/// "Nov 24".
pub(super) fn date_text(d: NaiveDate) -> String {
    d.format("%b %-d").to_string()
}

/// "7:00".
pub(super) fn time_text(t: NaiveTime) -> String {
    t.format("%-H:%M").to_string()
}

/// "40 min", "3 h", "3 h 05".
pub fn dur_text(d: Duration) -> String {
    let mins = d.num_minutes().max(0);
    if mins < 60 {
        return format!("{mins} min");
    }
    let (h, m) = (mins / 60, mins % 60);
    if m == 0 {
        format!("{h} h")
    } else {
        format!("{h} h {m:02}")
    }
}

/// "5", "1.3" (one decimal unless whole).
pub fn num(x: f64) -> String {
    if !x.is_finite() {
        return "—".into();
    }
    if (x - x.round()).abs() < 0.05 {
        format!("{:.0}", x.round())
    } else {
        format!("{x:.1}")
    }
}

pub(super) fn pct(v: f64) -> String {
    format!("{:.0}%", v.round())
}

/// "$100".
pub(super) fn usd(v: f64) -> String {
    format!("${:.0}", v.abs().round())
}

/// "A", "A and B", "A, B and C".
pub fn names(v: &[String]) -> String {
    match v {
        [] => String::new(),
        [a] => a.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// "today", "tomorrow", "in 4 days", "2 days ago".
pub fn days_text(days: i64) -> String {
    match days {
        0 => "today".into(),
        1 => "tomorrow".into(),
        -1 => "yesterday".into(),
        d if d > 1 => format!("in {d} days"),
        d => format!("{} days ago", -d),
    }
}

pub(super) fn collecting_text(d: NaiveDate) -> String {
    format!("Collecting data · first result around {}", date_text(d))
}

/// A share of usage in words for the "about … of what you pay for" sentence.
pub fn share_words(share: f64) -> Option<&'static str> {
    if share >= 0.9 {
        return Some("nearly all");
    }
    if share < 0.07 {
        return None;
    }
    let steps = [
        (0.1, "a tenth"),
        (0.2, "a fifth"),
        (0.25, "a quarter"),
        (1.0 / 3.0, "a third"),
        (0.5, "half"),
        (2.0 / 3.0, "two thirds"),
        (0.75, "three quarters"),
    ];
    steps
        .iter()
        .min_by(|a, b| (a.0 - share).abs().total_cmp(&(b.0 - share).abs()))
        .map(|s| s.1)
}

/// An account's name; when another account has the same name, with its provider ("Personal
/// (Codex)"), so sentences stay unambiguous.
pub(super) fn label_of(o: &Overview, id: Uuid) -> String {
    let Some(a) = o.account(id) else {
        return "An account".into();
    };
    let twin = o
        .accounts
        .iter()
        .any(|b| b.account != id && b.label.trim().eq_ignore_ascii_case(a.label.trim()));
    if twin {
        format!("{} ({})", a.label, provider_name(a.provider))
    } else {
        a.label.clone()
    }
}

/// Accounts that count in sentences: benched hub logins never do (they are not capacity).
pub(super) fn counting(o: &Overview, ids: &[Uuid]) -> Vec<Uuid> {
    ids.iter()
        .copied()
        .filter(|id| o.account(*id).is_some_and(|a| !a.benched))
        .collect()
}

pub(super) fn lower_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_lowercase().chain(c).collect(),
        None => String::new(),
    }
}

pub(super) fn kind_lower(k: TaskKind) -> String {
    lower_first(k.label())
}

/// What a task kind covers (muted subline in "Where usage goes").
pub(super) fn kind_covers(k: TaskKind) -> &'static str {
    match k {
        TaskKind::WritingCode => "Edit, Write, patches",
        TaskKind::BuildsTests => "cargo, npm, test runs",
        TaskKind::GitPullRequests => "git, gh",
        TaskKind::ReadingSearching => "Read, Grep, file listings",
        TaskKind::Agents => "subagents, threads",
        TaskKind::WebResearch => "search, fetch",
        TaskKind::DesignPreviews => "previews, screenshots",
        TaskKind::OtherTools => "other tool calls",
        TaskKind::Talking => "answers without tools",
    }
}

pub(super) fn project_label(k: &ProjectKey, label: &str) -> String {
    match k {
        ProjectKey::Unknown => "No folder in the logs".into(),
        _ if label.is_empty() => "Removed project".into(),
        _ => label.to_string(),
    }
}

/// The verdict sentence: a bold first clause and a muted rest.
pub fn verdict(o: &Overview) -> (String, String) {
    let v = &o.header.verdict;
    let named = |ids: &[Uuid]| names(&ids.iter().map(|i| label_of(o, *i)).collect::<Vec<_>>());
    let first = match (v.used_share, v.collecting_until) {
        (None, Some(d)) => {
            return (
                "Collecting data for your plans.".into(),
                format!(
                    "The first verdict comes around {}, after two full weeks.",
                    date_text(d)
                ),
            );
        }
        (None, None) => {
            return (
                "No weekly readings yet.".into(),
                "Keep SubHub running; the Overview fills in as readings come in.".into(),
            );
        }
        (Some(s), _) => match share_words(s) {
            Some(w) => format!("You use about {w} of what you pay for."),
            None => "You use very little of what you pay for.".into(),
        },
    };
    let downgrade = counting(o, &v.downgrade);
    let unused = counting(o, &v.unused);
    let at_limit = counting(o, &v.at_limit);
    let idle: Vec<Uuid> = downgrade.iter().chain(&unused).copied().collect();
    let one = |ids: &[Uuid], single: &str, many: &str| {
        if ids.len() == 1 {
            single.to_string()
        } else {
            many.to_string()
        }
    };
    let rest = match (idle.is_empty(), at_limit.is_empty()) {
        (false, false) => format!(
            "{} mostly {} idle while {} {} into {} limits.",
            named(&idle),
            one(&idle, "sits", "sit"),
            named(&at_limit),
            one(&at_limit, "runs", "run"),
            one(&at_limit, "its", "their"),
        ),
        (true, false) => format!(
            "{} {} into {} limits.",
            named(&at_limit),
            one(&at_limit, "runs", "run"),
            one(&at_limit, "its", "their"),
        ),
        (false, true) if unused.is_empty() => {
            format!("{} could do with a smaller plan.", named(&downgrade))
        }
        (false, true) => format!(
            "{} mostly {} idle.",
            named(&idle),
            one(&idle, "sits", "sit")
        ),
        (true, true) => "Every plan fits how you use it.".into(),
    };
    (first, rest)
}

/// The type chip of a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Msg {
    Tip,
    Clock,
    Danger,
    Good,
    Info,
    Saving,
}

/// A tip in words.
pub struct TipText {
    pub msg: Msg,
    pub title: String,
    pub rest: String,
    pub action: Option<(&'static str, TipAction)>,
}

pub fn tip_text(tip: &Tip, o: &Overview, steering_on: bool) -> TipText {
    let open = |label, row| Some((label, TipAction::OpenRow(row)));
    let row_action = |label| match tip.action {
        Some(TipAction::OpenRow(r)) => open(label, r),
        Some(TipAction::Steering) => Some((
            if steering_on {
                "Check hub steering"
            } else {
                "Turn on hub steering"
            },
            TipAction::Steering,
        )),
        Some(TipAction::SetUpWarmUp) => Some(("Set it up…", TipAction::SetUpWarmUp)),
        None => None,
    };
    match &tip.kind {
        TipKind::WarmUp {
            at,
            saved,
            waited_now,
            days,
            ..
        } => {
            let runout = o.windows.ready().and_then(|w| {
                Some(w.rhythm.usual_start? + w.rhythm.usual_first_hit?.to_std().ok()?)
            });
            let reset = *at + Duration::hours(5);
            let rest = match runout {
                Some(r) if (r - reset).num_minutes().abs() <= 20 => format!(
                    "Your first reset then comes at {}, right when you usually run out.",
                    time_text(reset)
                ),
                Some(r) => format!(
                    "Your first reset then comes at {}; you usually run out around {}.",
                    time_text(reset),
                    time_text(r)
                ),
                None => format!(
                    "Over the last {days} workdays that would have cut about {} of the {} you waited.",
                    dur_text(*saved),
                    dur_text(*waited_now)
                ),
            };
            TipText {
                msg: Msg::Clock,
                title: format!("Start your first window at {}.", time_text(*at)),
                rest,
                // Opens the setup dialog with the suggestion filled in; nothing runs before the
                // user confirms it (owner, 2026-11-08).
                action: Some(("Set it up…", TipAction::SetUpWarmUp)),
            }
        }
        TipKind::SwitchInsteadOfWaiting {
            account,
            to,
            times,
            waited,
        } => TipText {
            msg: Msg::Tip,
            title: "Switch instead of waiting.".into(),
            rest: format!(
                "{} {} hit its 5-hour limit while {} had room: {} waited.",
                if *times == 1 {
                    "Once".to_string()
                } else {
                    format!("{times} times")
                },
                label_of(o, *account),
                label_of(o, *to),
                dur_text(*waited)
            ),
            action: row_action("See 5-hour windows"),
        },
        TipKind::Downgrade {
            account,
            to,
            saving,
            peak_on_smaller,
        } => {
            let decide = o
                .renewals
                .iter()
                .find(|r| r.account == *account && r.auto_renew && r.days >= 0)
                .map(|r| format!(" Decide before {}.", date_text(r.date)))
                .unwrap_or_default();
            TipText {
                msg: Msg::Saving,
                title: format!("{} fits {}.", label_of(o, *account), to.label()),
                rest: format!(
                    "Its busiest week would use about {} of {}. Switching saves ~{} a month.{decide}",
                    pct(*peak_on_smaller),
                    to.label(),
                    usd(*saving)
                ),
                action: row_action("See plans"),
            }
        }
        TipKind::Upgrade {
            account,
            to,
            extra,
            peak,
            hits_per_week,
        } => TipText {
            msg: Msg::Danger,
            title: format!("{} runs into its limits.", label_of(o, *account)),
            rest: format!(
                "Its busiest week used {} and it hits the 5-hour limit {} times a week. {} would give it room for ~{} more a month.",
                pct(*peak),
                num(*hits_per_week),
                to.label(),
                usd(*extra)
            ),
            action: row_action("See plans"),
        },
        TipKind::SpreadWork {
            account,
            hits_per_week,
            ..
        } => TipText {
            msg: Msg::Danger,
            title: format!("{} is at its limit.", label_of(o, *account)),
            rest: format!(
                "It is on the biggest plan and hits the 5-hour limit {} times a week: move some work to another account.",
                num(*hits_per_week)
            ),
            action: row_action("See plans"),
        },
        TipKind::EndingPlan {
            account,
            on,
            days,
            what_if,
        } => {
            let when = if *days == 0 {
                "today".to_string()
            } else {
                date_text(*on)
            };
            let provider = o.account(*account).map(|a| a.provider);
            TipText {
                msg: Msg::Danger,
                title: format!("{} ends {when}.", label_of(o, *account)),
                rest: what_if
                    .as_ref()
                    .map(|w| what_if_text(w, o, provider))
                    .unwrap_or_else(|| "Plan where its work goes next.".into()),
                action: row_action("See renewals"),
            }
        }
        TipKind::UnusedPlan {
            account,
            avg,
            saving,
        } => TipText {
            msg: Msg::Saving,
            title: format!("{} sits unused.", label_of(o, *account)),
            rest: format!(
                "It used {} of its weekly limit on average.{}",
                pct(*avg),
                saving
                    .map(|s| format!(" Cancelling saves {} a month.", usd(s)))
                    .unwrap_or_default()
            ),
            action: row_action("See plans"),
        },
        TipKind::TaskInsight { kind, share } => TipText {
            msg: Msg::Info,
            title: format!("{} take {}.", kind.label(), pct(share * 100.0)),
            rest: match kind {
                TaskKind::BuildsTests => "Mostly re-reading long build output. Quieter runs (only the failing test, less output) would cut a good part of it.".into(),
                _ => String::new(),
            },
            action: row_action("See where usage goes"),
        },
        TipKind::ExpiringWeekly {
            account,
            left,
            resets_at,
        } => TipText {
            msg: Msg::Tip,
            title: format!("{} has {} of its week left.", label_of(o, *account), pct(*left)),
            rest: format!(
                "It resets in {}; a good time for a big job.",
                crate::format::duration_until(*resets_at, o.now)
            ),
            action: row_action("See plans"),
        },
    }
}

/// "Its work would move to Codex Work (~84 %)." for an ending plan; the smaller tier's load for
/// a downgrade.
pub(super) fn what_if_text(w: &WhatIf, o: &Overview, provider: Option<ProviderKind>) -> String {
    match w.change {
        WhatIfChange::Ends(_) => {
            if w.others.is_empty() {
                return format!(
                    "No other {} account would take over its work.",
                    provider.map_or("", provider_name)
                );
            }
            let parts: Vec<String> = w
                .others
                .iter()
                .map(|x| {
                    format!(
                        "{} (~{}, now {})",
                        label_of(o, x.account),
                        pct(x.after),
                        pct(x.before)
                    )
                })
                .collect();
            let mut s = format!("Its work would move to {}.", names(&parts));
            if w.unplaced > 0.5 {
                s.push_str(" Some of it would not fit.");
            }
            s
        }
        WhatIfChange::Tier(t) => match w.own_after {
            Some(after) => format!(
                "On {} it would be at ~{} of its week.",
                t.label(),
                pct(after)
            ),
            None => format!("On {} it would be full.", t.label()),
        },
    }
}

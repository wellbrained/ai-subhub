//! Statistics -> Overview tab (plan `docs/features/overview.md`, build step 2; owner-approved
//! mockups `overview2.html` for the page and `overview.html` for the Plans details, 2026-11-07).
//!
//! One calm page: a verdict sentence, four numbers, the top tip ("N more tips"), then one line per
//! topic (Plans, 5-hour windows, Renewals, Where usage goes) that states its answer in words and
//! opens below itself (one at a time). No cards: space, small muted headings and hairlines only.
//! Messages carry a tinted type chip (tip, clock, danger, good, info, saving).
//!
//! The analysis (`subhub_core::insights::analyze`) runs on the background executor whenever its
//! inputs change (readings, history, scan, settings, range) or the result is 10 minutes old; the
//! page shows the last result meanwhile and a quiet line before the first one.
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration as StdDuration, Instant};

use chrono::{Duration, NaiveDate, NaiveDateTime, NaiveTime, Timelike, Utc};
use gpui::{
    AnyElement, App, Context, Entity, FontWeight, HighlightStyle, Hsla, IntoElement, ParentElement,
    SharedString, Styled, StyledText, div, prelude::*, px, relative,
};
use gpui_component::Disableable as _;
use gpui_component::switch::Switch;
use subhub_core::insights::{
    self, AT_LIMIT_HITS, AccountInsight, Collect, ExampleDay, FitVerdict, InsightRange, Overview,
    PlanFit, ProjectKey, Renewal, Span, Tip, TipAction, TipKind, TipRow, WhatIf, WhatIfChange,
    WindowsReport, Zone,
};
use subhub_core::model::ProviderKind;
use subhub_core::settings::UsageStyle;
use subhub_core::stats::TaskKind;
use subhub_core::warmup::WarmupSettings;
use uuid::Uuid;

use crate::icons::{Icon, icon};
use crate::state::{AppState, SettingsTarget, provider_name};
use crate::stats_window::StatsWindow;
use crate::taskbar::account_tag;
use crate::theme::Tokens;
use crate::title_bar::seg;

/// A result older than this is recomputed (today, running windows and resets move on).
const RECOMPUTE: StdDuration = StdDuration::from_secs(10 * 60);
/// History read beyond the plan-fit lookback, so the oldest cycle in it starts inside.
const HISTORY_EXTRA_DAYS: i64 = 14;

/// Font sizes (px): body lines, small lines, tiny sublines.
const BODY: f32 = 13.0;
const SMALL: f32 = 12.0;
const TINY: f32 = 11.5;
/// Label column of the "Where usage goes" bars (wider in the Thread view, which has no side
/// column).
const LABEL_W: f32 = 190.0;
const THREAD_LABEL_W: f32 = 340.0;
/// Side padding inside the content column (the mockup's 48 px body padding).
const SIDE: f32 = 28.0;

/// "Where usage goes" switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dim {
    Task,
    Project,
    Thread,
    Model,
}

/// The Overview's UI state inside the Statistics window.
pub struct OverviewState {
    pub range: InsightRange,
    /// Inputs signature, when it was computed, the result.
    result: Option<(u64, Instant, Arc<Overview>)>,
    /// The signature a background run is computing.
    computing: Option<u64>,
    /// The open row (one at a time).
    pub open: Option<TipRow>,
    /// The tip list under the top tip is expanded.
    pub tips_open: bool,
    pub dim: Dim,
    /// The warm-up setup dialog is open with this draft.
    pub warm_dialog: Option<WarmupSettings>,
    /// Demo only (`--demo-open-row warmup-dialog`): open the dialog once the first result is in.
    demo_dialog: bool,
}

impl OverviewState {
    pub fn new() -> Self {
        let (open, tips_open) = match crate::demo::open_row() {
            Some("plans") => (Some(TipRow::Plans), false),
            Some("windows") => (Some(TipRow::Windows), false),
            Some("renewals") => (Some(TipRow::Renewals), false),
            Some("usage") => (Some(TipRow::Usage), false),
            Some("tips") => (None, true),
            Some("warmup-dialog") => (Some(TipRow::Windows), false),
            _ => (None, false),
        };
        Self {
            range: InsightRange::FourWeeks,
            result: None,
            computing: None,
            open,
            tips_open,
            dim: Dim::Task,
            warm_dialog: None,
            demo_dialog: crate::demo::open_row() == Some("warmup-dialog"),
        }
    }

    /// The last analysis result, if any.
    pub fn latest(&self) -> Option<Arc<Overview>> {
        self.result.as_ref().map(|(_, _, o)| o.clone())
    }

    /// The result is old enough to recompute (the window's 30 s timer re-renders then).
    pub fn stale(&self) -> bool {
        self.result
            .as_ref()
            .is_some_and(|(_, at, _)| at.elapsed() >= RECOMPUTE)
    }

    /// Starts a background analysis when the inputs changed or the result is old. Never blocks:
    /// the page keeps the last result until the new one arrives.
    pub fn ensure(&mut self, state: &Entity<AppState>, cx: &mut Context<StatsWindow>) {
        let s = state.read(cx);
        let sig = signature(s, self.range);
        let fresh = self
            .result
            .as_ref()
            .is_some_and(|(k, at, _)| *k == sig && at.elapsed() < RECOMPUTE);
        if fresh || self.computing == Some(sig) {
            return;
        }
        self.computing = Some(sig);
        let settings = s.settings.clone();
        let scan = s.stats.clone();
        let plans = s.settings.plans.clone();
        let prices = s.prices.clone();
        let range = self.range;
        let state = state.clone();
        cx.spawn(async move |this, cx| {
            let overview = cx
                .background_executor()
                .spawn(async move {
                    let now = Utc::now();
                    let since = now
                        - Duration::days(
                            range.days().max(insights::FIT_LOOKBACK_DAYS) + HISTORY_EXTRA_DAYS,
                        );
                    let history = subhub_core::history::load_lines(
                        &subhub_core::history::default_path(),
                        since,
                    );
                    insights::analyze(&insights::Input {
                        history: &history,
                        scan: scan.as_deref(),
                        settings: &settings,
                        plans: &plans,
                        prices: &prices,
                        range,
                        now,
                        zone: &Zone::Local,
                    })
                })
                .await;
            let _ = this.update(cx, |w, cx| {
                let ov = w.overview_mut();
                if ov.computing == Some(sig) {
                    ov.computing = None;
                }
                let overview = Arc::new(overview);
                ov.result = Some((sig, Instant::now(), overview.clone()));
                if ov.demo_dialog {
                    ov.demo_dialog = false;
                    ov.warm_dialog = Some(suggested_draft(&overview, state.read(cx)));
                }
                cx.notify();
            });
        })
        .detach();
    }
}

/// What the analysis reads from the state: a change here means a new run.
fn signature(s: &AppState, range: InsightRange) -> u64 {
    let mut h = DefaultHasher::new();
    range.days().hash(&mut h);
    s.stats_rev.hash(&mut h);
    s.history.len().hash(&mut h);
    for a in &s.settings.accounts {
        a.id.hash(&mut h);
        a.label.hash(&mut h);
        a.provider.hash(&mut h);
        a.profile_dir.is_some().hash(&mut h);
        a.hub.as_ref().map(|x| x.benched).hash(&mut h);
        s.settings
            .readings
            .get(&a.id)
            .map(|r| r.fetched_at)
            .hash(&mut h);
        s.settings.plans.get(&a.id).hash(&mut h);
        format!("{:?}", s.settings.billing.get(&a.id)).hash(&mut h);
    }
    for p in &s.settings.projects {
        p.id.hash(&mut h);
        p.directory.hash(&mut h);
        p.label.hash(&mut h);
    }
    s.settings.hub.enabled.hash(&mut h);
    h.finish()
}

// ---------- wording ----------

/// "Nov 24".
fn date_text(d: NaiveDate) -> String {
    d.format("%b %-d").to_string()
}

/// "7:00".
fn time_text(t: NaiveTime) -> String {
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

fn pct(v: f64) -> String {
    format!("{:.0}%", v.round())
}

/// "$100".
fn usd(v: f64) -> String {
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

fn collecting_text(d: NaiveDate) -> String {
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
fn label_of(o: &Overview, id: Uuid) -> String {
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
fn counting(o: &Overview, ids: &[Uuid]) -> Vec<Uuid> {
    ids.iter()
        .copied()
        .filter(|id| o.account(*id).is_some_and(|a| !a.benched))
        .collect()
}

fn lower_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_lowercase().chain(c).collect(),
        None => String::new(),
    }
}

fn kind_lower(k: TaskKind) -> String {
    lower_first(k.label())
}

/// What a task kind covers (muted subline in "Where usage goes").
fn kind_covers(k: TaskKind) -> &'static str {
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

fn project_label(k: &ProjectKey, label: &str) -> String {
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
fn what_if_text(w: &WhatIf, o: &Overview, provider: Option<ProviderKind>) -> String {
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

// ---------- building blocks ----------

fn b(s: impl Into<String>) -> (String, bool) {
    (s.into(), true)
}

fn m(s: impl Into<String>) -> (String, bool) {
    (s.into(), false)
}

/// Text with bold parts in the text colour; the rest takes the parent's (muted) colour.
fn rich(parts: Vec<(String, bool)>, t: &Tokens) -> StyledText {
    let mut text = String::new();
    let mut hl = vec![];
    for (p, bold) in parts {
        let start = text.len();
        text.push_str(&p);
        if bold {
            hl.push((
                start..text.len(),
                HighlightStyle {
                    color: Some(t.text),
                    font_weight: Some(FontWeight::SEMIBOLD),
                    ..Default::default()
                },
            ));
        }
    }
    StyledText::new(text).with_highlights(hl)
}

fn msg_style(k: Msg, t: &Tokens) -> (Icon, Hsla) {
    match k {
        Msg::Tip => (Icon::Lightbulb, t.accent_text()),
        Msg::Clock => (Icon::Clock, t.accent_text()),
        Msg::Danger => (Icon::Warning, t.danger),
        Msg::Good => (Icon::CircleCheck, t.ok),
        Msg::Info => (Icon::Info, t.info),
        Msg::Saving => (Icon::DollarSign, t.ok),
    }
}

/// The tinted type chip of a message.
fn chip(k: Msg, size: f32, t: &Tokens) -> gpui::Div {
    let (glyph, color) = msg_style(k, t);
    div()
        .size(px(size))
        .flex_none()
        .rounded(px((size * 0.3).round()))
        .flex()
        .items_center()
        .justify_center()
        .bg(color.opacity(if t.dark { 0.16 } else { 0.12 }))
        .child(icon(glyph, (size * 0.58).round(), color))
}

fn pill(text: impl Into<SharedString>, fg: Hsla, bg: Hsla) -> gpui::Div {
    div()
        .flex_none()
        .whitespace_nowrap()
        .px(px(8.0))
        .py(px(1.0))
        .rounded_full()
        .bg(bg)
        .text_size(px(11.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(fg)
        .child(text.into())
}

fn fit_pill(v: FitVerdict, t: &Tokens) -> gpui::Div {
    let tint = |c: Hsla| c.opacity(if t.dark { 0.16 } else { 0.12 });
    match v {
        FitVerdict::Fits => pill("Fits", t.ok, tint(t.ok)),
        FitVerdict::Downgrade => pill("Downgrade", t.info, tint(t.info)),
        FitVerdict::AtLimit => pill("At limit", t.danger, tint(t.danger)),
        FitVerdict::Unused => pill("Unused", t.muted, t.surface),
    }
}

fn dot(color: Hsla) -> gpui::Div {
    div().size(px(8.0)).flex_none().rounded_full().bg(color)
}

/// A small text link in the accent colour (`faint`: a quiet one, like "Not for me").
fn link(
    id: impl Into<gpui::ElementId>,
    text: impl Into<SharedString>,
    faint: bool,
    t: &Tokens,
    on_click: impl Fn(&mut App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    let (color, hover) = if faint {
        (t.faint, t.muted)
    } else {
        (t.accent_text(), t.text)
    };
    div()
        .id(id.into())
        .flex_none()
        .whitespace_nowrap()
        .cursor_pointer()
        .text_size(px(12.5))
        .when(!faint, |d| d.font_weight(FontWeight::SEMIBOLD))
        .text_color(color)
        .hover(move |s| s.text_color(hover))
        .child(text.into())
        .on_click(move |_, _, cx| on_click(cx))
}

/// A horizontal mini bar of `frac` (0..1) on the track.
fn mini_bar(frac: f64, color: Hsla, h: f32, t: &Tokens) -> gpui::Div {
    let f = frac.clamp(0.0, 1.0) as f32;
    div()
        .relative()
        .flex_1()
        .h(px(h))
        .rounded(px(h / 2.0))
        .bg(t.track)
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .h_full()
                .w(relative(if f > 0.0 { f.max(0.02) } else { 0.0 }))
                .rounded(px(h / 2.0))
                .bg(color),
        )
}

/// Opens the Settings window on a page.
pub type OpenSettings = Rc<dyn Fn(SettingsTarget, &mut App)>;

/// Handlers and data the page needs.
pub struct Ctx<'a> {
    pub t: &'a Tokens,
    pub s: &'a AppState,
    pub me: Entity<StatsWindow>,
    pub state: Entity<AppState>,
    pub open_settings: OpenSettings,
}

impl Ctx<'_> {
    /// A click handler that changes the Overview state.
    fn upd(&self, f: impl Fn(&mut OverviewState) + 'static) -> impl Fn(&mut App) + 'static {
        let me = self.me.clone();
        move |cx: &mut App| {
            me.update(cx, |w, cx| {
                f(w.overview_mut());
                cx.notify();
            })
        }
    }

    fn act(&self, a: TipAction) -> impl Fn(&mut App) + 'static {
        let upd = self.upd(move |ov| {
            if let TipAction::OpenRow(r) = a {
                ov.open = Some(r);
            }
        });
        let settings = self.open_settings.clone();
        let setup = open_dialog(self.me.clone(), self.state.clone());
        move |cx: &mut App| match a {
            TipAction::OpenRow(_) => upd(cx),
            TipAction::Steering => settings(SettingsTarget::Hub, cx),
            TipAction::SetUpWarmUp => setup(cx),
        }
    }

    fn hidden(&self, id: Uuid) -> bool {
        self.s.account(id).is_some_and(|a| a.hidden)
    }

    fn color(&self, id: Uuid) -> Hsla {
        self.s
            .account(id)
            .map_or(self.t.faint, |a| account_tag(self.s, a).1)
    }
}

/// The range switch for the title bar.
pub fn range_switch(ov: &OverviewState, me: Entity<StatsWindow>, t: &Tokens) -> AnyElement {
    let ranges = InsightRange::ALL.map(|r| (r, r.label()));
    seg("ov-range", &ranges, &ov.range, t, move |r, cx| {
        me.update(cx, |w, cx| {
            w.overview_mut().range = r;
            cx.notify();
        })
    })
}

// ---------- the page ----------

pub fn body(ov: &OverviewState, cx: &Ctx) -> AnyElement {
    let t = cx.t;
    let Some((_, _, o)) = &ov.result else {
        return div()
            .px(px(SIDE))
            .py_10()
            .text_size(px(BODY))
            .text_color(t.muted)
            .child("Reading your usage history…")
            .into_any_element();
    };
    let o = o.as_ref();
    let (first, rest) = verdict(o);
    let verdict_line = div()
        .text_size(px(20.0))
        .line_height(relative(1.35))
        .text_color(t.muted)
        .child(
            StyledText::new(format!("{first} {rest}")).with_highlights([(
                0..first.len(),
                HighlightStyle {
                    color: Some(t.text),
                    font_weight: Some(FontWeight::SEMIBOLD),
                    ..Default::default()
                },
            )]),
        );

    div()
        .flex()
        .flex_col()
        .px(px(SIDE))
        .pt(px(12.0))
        .child(verdict_line)
        .child(kpis(o, t).mt(px(18.0)))
        .child(tips_block(ov, o, cx).mt(px(24.0)))
        .child(rows(ov, o, cx).mt(px(8.0)))
        .into_any_element()
}

fn kpi(value: String, unit: &str, label: &str, t: &Tokens) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .items_baseline()
                .gap(px(3.0))
                .child(
                    div()
                        .text_size(px(20.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(value),
                )
                .when(!unit.is_empty(), |d| {
                    d.child(
                        div()
                            .text_size(px(SMALL))
                            .text_color(t.muted)
                            .child(unit.to_string()),
                    )
                }),
        )
        .child(
            div()
                .text_size(px(SMALL))
                .text_color(t.muted)
                .child(label.to_string()),
        )
}

fn kpis(o: &Overview, t: &Tokens) -> gpui::Div {
    let h = &o.header;
    let weekly = match h.weekly_avg {
        Some(a) => kpi(pct(a), "avg", "of weekly limits used", t),
        None => kpi("—".into(), "", "of weekly limits used", t),
    };
    let renewal = match &h.next_renewal {
        Some(r) if r.days == 0 => kpi("Today".into(), "", "next renewal", t),
        Some(r) => kpi(
            r.days.to_string(),
            if r.days == 1 { "day" } else { "days" },
            "to the next renewal",
            t,
        ),
        None => kpi("—".into(), "", "no renewal dates set", t),
    };
    div()
        .flex()
        .gap(px(40.0))
        .child(weekly)
        .child(kpi(num(h.hits_per_week), "/ week", "5-hour limit hits", t))
        .child(kpi(dur_text(h.waited), "", "waited for resets", t))
        .child(renewal)
}

/// The top tip line with its action, "Not for me" and "N more tips"; the other tips below it
/// while expanded.
fn tips_block(ov: &OverviewState, o: &Overview, cx: &Ctx) -> gpui::Div {
    let t = cx.t;
    let dismissed = &cx.s.settings.dismissed_tips;
    // The warm-up tip is done once the warm-up is on.
    let warm_on = cx.s.settings.warmup.enabled;
    let all: Vec<&Tip> = o
        .tips
        .iter()
        .filter(|tp| !(warm_on && matches!(tp.kind, TipKind::WarmUp { .. })))
        .collect();
    let visible: Vec<&Tip> = all
        .iter()
        .copied()
        .filter(|tp| !dismissed.contains(&tp.id))
        .collect();
    let hidden = all.len() - visible.len();
    let steering_on = cx.s.settings.steering.enabled;
    let restore = {
        let state = cx.state.clone();
        move |cx: &mut App| {
            state.update(cx, |s, cx| {
                s.settings.dismissed_tips.clear();
                s.save();
                cx.notify();
            })
        }
    };
    let dismiss = |id: String| {
        let state = cx.state.clone();
        move |cx: &mut App| {
            state.update(cx, |s, cx| {
                if !s.settings.dismissed_tips.contains(&id) {
                    s.settings.dismissed_tips.push(id.clone());
                    s.save();
                }
                cx.notify();
            })
        }
    };

    let wrap = div()
        .flex()
        .flex_col()
        .border_t_1()
        .border_b_1()
        .border_color(t.border);
    let Some(top) = visible.first() else {
        return wrap.child(
            div()
                .flex()
                .items_center()
                .gap(px(12.0))
                .py(px(12.0))
                .child(chip(Msg::Good, 26.0, t))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(BODY))
                        .text_color(t.muted)
                        .child(rich(
                            vec![b("No tips right now."), m(" Everything looks in order.")],
                            t,
                        )),
                )
                .when(hidden > 0, |d| {
                    d.child(link(
                        "tips-restore",
                        format!("Show hidden tips ({hidden})"),
                        true,
                        t,
                        restore.clone(),
                    ))
                }),
        );
    };

    let text = tip_text(top, o, steering_on);
    let more = visible.len() - 1;
    let mut line = div()
        .flex()
        .items_center()
        .gap(px(12.0))
        .py(px(12.0))
        .child(chip(text.msg, 26.0, t))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(BODY))
                .text_color(t.muted)
                .child(rich(vec![b(text.title), m(format!(" {}", text.rest))], t)),
        );
    if let Some((label, a)) = text.action {
        line = line.child(link("tip-act-0", label, false, t, cx.act(a)));
    }
    line = line.child(link(
        "tip-dismiss-0",
        "Not for me",
        true,
        t,
        dismiss(top.id.clone()),
    ));
    if more > 0 || hidden > 0 {
        let label = if ov.tips_open {
            "· Fewer tips".to_string()
        } else if more > 0 {
            format!("· {more} more tip{}", if more == 1 { "" } else { "s" })
        } else {
            format!("· {hidden} hidden")
        };
        line = line.child(link(
            "tips-more",
            label,
            true,
            t,
            cx.upd(|ov| ov.tips_open = !ov.tips_open),
        ));
    }
    let mut out = wrap.child(line);
    if ov.tips_open {
        let mut list = div().flex().flex_col().pl(px(38.0)).pb(px(8.0));
        for (i, tp) in visible.iter().enumerate().skip(1) {
            let text = tip_text(tp, o, steering_on);
            let mut links = div().flex().items_center().gap(px(14.0)).mt(px(3.0));
            if let Some((label, a)) = text.action {
                links = links.child(link(("tip-act", i), label, false, t, cx.act(a)));
            }
            links = links.child(link(
                ("tip-dismiss", i),
                "Not for me",
                true,
                t,
                dismiss(tp.id.clone()),
            ));
            list = list.child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(10.0))
                    .py(px(8.0))
                    .border_t_1()
                    .border_color(t.border.opacity(0.7))
                    .child(chip(text.msg, 24.0, t))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(12.5))
                                    .line_height(relative(1.45))
                                    .text_color(t.muted)
                                    .child(rich(
                                        vec![b(text.title), m(format!(" {}", text.rest))],
                                        t,
                                    )),
                            )
                            .child(links),
                    ),
            );
        }
        if hidden > 0 {
            list = list.child(
                div()
                    .flex()
                    .pt(px(8.0))
                    .border_t_1()
                    .border_color(t.border.opacity(0.7))
                    .child(link(
                        "tips-restore",
                        format!("Show hidden tips ({hidden})"),
                        true,
                        t,
                        restore,
                    )),
            );
        }
        out = out.child(list);
    }
    out
}

// ---------- rows ----------

/// One topic row: name, the answer in words, a short right-hand note and the chevron; its
/// details below while open.
#[allow(clippy::too_many_arguments)]
fn row(
    id: &'static str,
    name: &str,
    answer: AnyElement,
    right: Option<AnyElement>,
    open: bool,
    content: impl FnOnce() -> AnyElement,
    on_toggle: impl Fn(&mut App) + 'static,
    t: &Tokens,
) -> gpui::Div {
    let (chev, chev_hover) = (t.faint, t.text);
    let head = div()
        .id(id)
        .group(id)
        .flex()
        .items_center()
        .gap(px(16.0))
        .py(px(14.0))
        .cursor_pointer()
        .child(
            div()
                .w(px(190.0))
                .flex_none()
                .text_size(px(BODY))
                .font_weight(FontWeight::SEMIBOLD)
                .child(name.to_string()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(BODY))
                .text_color(t.muted)
                .child(answer),
        )
        .child(
            div()
                .w(px(150.0))
                .flex_none()
                .flex()
                .justify_end()
                .items_center()
                .gap(px(6.0))
                .children(right),
        )
        .child(
            div().w(px(18.0)).flex_none().flex().justify_end().child(
                icon(
                    if open {
                        Icon::ChevronDown
                    } else {
                        Icon::ChevronRight
                    },
                    14.0,
                    chev,
                )
                .group_hover(id, move |s| s.text_color(chev_hover)),
            ),
        )
        .on_click(move |_, _, cx| on_toggle(cx));
    div()
        .flex()
        .flex_col()
        .border_b_1()
        .border_color(t.border)
        .child(head)
        .when(open, |d| d.child(div().pb(px(22.0)).child(content())))
}

fn rows(ov: &OverviewState, o: &Overview, cx: &Ctx) -> gpui::Div {
    let t = cx.t;
    let toggle = |r: TipRow| {
        cx.upd(move |ov| {
            ov.open = if ov.open == Some(r) { None } else { Some(r) };
        })
    };
    let is = |r: TipRow| ov.open == Some(r);
    let (plans_answer, plans_right) = plans_line(o, cx);
    let (win_answer, win_right) = windows_line(o, &cx.s.settings.warmup, t);
    let (ren_answer, ren_right) = renewals_line(o, t);
    let usage_answer = usage_line(o, cx);
    div()
        .flex()
        .flex_col()
        .child(row(
            "row-plans",
            "Plans",
            plans_answer,
            plans_right,
            is(TipRow::Plans),
            || plans_content(o, cx),
            toggle(TipRow::Plans),
            t,
        ))
        .child(row(
            "row-windows",
            "5-hour windows",
            win_answer,
            win_right,
            is(TipRow::Windows),
            || windows_content(o, cx),
            toggle(TipRow::Windows),
            t,
        ))
        .child(row(
            "row-renewals",
            "Renewals",
            ren_answer,
            ren_right,
            is(TipRow::Renewals),
            || renewals_content(o, cx),
            toggle(TipRow::Renewals),
            t,
        ))
        .child(row(
            "row-usage",
            "Where usage goes",
            usage_answer,
            None,
            is(TipRow::Usage),
            || usage_content(ov, o, cx),
            toggle(TipRow::Usage),
            t,
        ))
}

fn plain(text: impl Into<SharedString>) -> AnyElement {
    div().child(text.into()).into_any_element()
}

// ----- Plans -----

fn plans_line(o: &Overview, cx: &Ctx) -> (AnyElement, Option<AnyElement>) {
    let t = cx.t;
    let shown: Vec<&AccountInsight> = o
        .accounts
        .iter()
        .filter(|a| !cx.hidden(a.account) && !a.benched)
        .collect();
    let mut parts: Vec<Vec<(String, bool)>> = vec![];
    let mut verdicts: Vec<FitVerdict> = vec![];
    for want in [
        FitVerdict::Downgrade,
        FitVerdict::AtLimit,
        FitVerdict::Unused,
    ] {
        for a in &shown {
            let Some(f) = a.fit.ready().filter(|f| f.verdict == want) else {
                continue;
            };
            if !verdicts.contains(&want) {
                verdicts.push(want);
            }
            let p = match want {
                FitVerdict::Downgrade => {
                    let to = f.suggested.map_or("a smaller plan", |s| s.label());
                    let mut v = vec![b(format!("{} could drop to {to}", label_of(o, a.account)))];
                    if let Some(c) = f.price_change.filter(|c| *c < 0.0) {
                        v.push(m(format!(" and save ~{} a month", usd(c))));
                    }
                    v
                }
                FitVerdict::AtLimit => vec![b(label_of(o, a.account)), m(" is at its limit")],
                _ => vec![b(label_of(o, a.account)), m(" sits unused")],
            };
            parts.push(p);
        }
    }
    let answer = if parts.is_empty() {
        let ready_on = shown.iter().filter_map(|a| a.fit.ready_on()).min();
        if shown.iter().any(|a| a.fit.ready().is_some()) {
            plain("Every plan fits how you use it")
        } else if let Some(d) = ready_on {
            plain(collecting_text(d))
        } else {
            plain("No accounts yet")
        }
    } else {
        let mut all = vec![];
        for (i, p) in parts.into_iter().take(2).enumerate() {
            if i > 0 {
                all.push(m(" · "));
            }
            all.extend(p);
        }
        rich(all, t).into_any_element()
    };
    let right = (!verdicts.is_empty()).then(|| {
        div()
            .flex()
            .gap(px(6.0))
            .children(verdicts.into_iter().map(|v| fit_pill(v, t)))
            .into_any_element()
    });
    (answer, right)
}

const C_NAME: f32 = 190.0;
const C_NOW: f32 = 118.0;
const C_WEEK: f32 = 124.0;
const C_HITS: f32 = 62.0;
const C_GAP: f32 = 16.0;

fn plans_content(o: &Overview, cx: &Ctx) -> AnyElement {
    let t = cx.t;
    let small = |s: String| div().text_size(px(SMALL)).text_color(t.muted).child(s);
    let header = div()
        .flex()
        .gap(px(C_GAP))
        .pb(px(6.0))
        .child(div().w(px(C_NAME)).flex_none())
        .child(
            small(
                if cx.s.settings.display.usage_style == UsageStyle::Remaining {
                    "Now · left"
                } else {
                    "Now · used"
                }
                .into(),
            )
            .w(px(C_NOW))
            .flex_none(),
        )
        .child(
            small(format!("Weekly used · {}", o.range.label()))
                .w(px(C_WEEK))
                .flex_none(),
        )
        .child(small("5-h hits".into()).w(px(C_HITS)).flex_none())
        .child(small("Plan fit".into()).flex_1());
    let mut list = div().flex().flex_col().child(header);
    for a in o.accounts.iter().filter(|a| !cx.hidden(a.account)) {
        list = list.child(account_row(a, o, cx));
    }
    list.into_any_element()
}

fn renewal_of(o: &Overview, id: Uuid) -> Option<&Renewal> {
    o.renewals.iter().find(|r| r.account == id)
}

fn account_row(a: &AccountInsight, o: &Overview, cx: &Ctx) -> gpui::Div {
    let t = cx.t;
    let sub = if a.benched {
        "Benched in the hub".to_string()
    } else {
        match renewal_of(o, a.account) {
            Some(r) if r.days >= 0 => format!(
                "{} {} · {} d",
                if r.auto_renew { "Renews" } else { "Ends" },
                date_text(r.date),
                r.days
            ),
            Some(r) => format!("Ended {}", date_text(r.date)),
            None => "No renewal date".into(),
        }
    };
    let name = div()
        .w(px(C_NAME))
        .flex_none()
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(7.0))
                .child(dot(cx.color(a.account)))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(px(BODY))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(label_of(o, a.account)),
                )
                .children(a.plan.clone().map(|p| {
                    div()
                        .flex_none()
                        .px(px(6.0))
                        .rounded(px(5.0))
                        .bg(t.surface)
                        .text_size(px(11.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.muted)
                        .child(p)
                })),
        )
        .child(
            div()
                .pl(px(15.0))
                .mt(px(2.0))
                .text_size(px(SMALL))
                .text_color(t.faint)
                .child(sub),
        );

    // Follows "Show usage as" like the popover (% left by default).
    let left = cx.s.settings.display.usage_style == UsageStyle::Remaining;
    let now_bar = |label: &str, w: Option<insights::NowWindow>| {
        let (frac, text, color) = match w {
            Some(w) => {
                let v = if left { 100.0 - w.used } else { w.used };
                (v / 100.0, pct(v), t.level(100.0 - w.used))
            }
            None => (0.0, "—".into(), t.accent),
        };
        div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .text_size(px(TINY))
            .text_color(t.muted)
            .child(div().w(px(22.0)).flex_none().child(label.to_string()))
            .child(mini_bar(frac, color, 5.0, t))
            .child(
                div()
                    .w(px(32.0))
                    .flex_none()
                    .text_right()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.text)
                    .child(text),
            )
    };
    let now = div()
        .w(px(C_NOW))
        .flex_none()
        .flex()
        .flex_col()
        .gap(px(5.0))
        .when(a.has_session_window || a.now_session.is_some(), |d| {
            d.child(now_bar("5 h", a.now_session))
        })
        .child(now_bar("Wk", a.now_weekly));

    // Weekly use per cycle: the last few cycles as columns under a dashed 100 % line.
    let cycles: Vec<&insights::WeeklyCycle> = a.cycles.iter().rev().take(6).rev().collect();
    let n = cycles.len();
    let week = div()
        .w(px(C_WEEK))
        .flex_none()
        .flex()
        .flex_col()
        .child(
            div()
                .relative()
                .h(px(30.0))
                .flex()
                .items_end()
                .gap(px(4.0))
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .w(px((n.max(1) as f32) * 19.0 - 4.0))
                        .h(px(1.0))
                        .bg(t.faint.opacity(0.45)),
                )
                .children(cycles.iter().enumerate().map(|(i, c)| {
                    let last = i + 1 == n;
                    div()
                        .w(px(15.0))
                        .h(px((c.peak.clamp(0.0, 100.0) as f32 * 0.3).max(1.0)))
                        .rounded_t(px(2.0))
                        .bg(t.accent.opacity(if last { 0.95 } else { 0.5 }))
                })),
        )
        .child(
            div()
                .mt(px(3.0))
                .text_size(px(TINY))
                .text_color(t.muted)
                .child(match (a.weekly_avg, a.weekly_peak) {
                    (Some(avg), Some(peak)) => format!("avg {} · peak {}", pct(avg), pct(peak)),
                    _ if n > 0 => "no full week yet".into(),
                    _ => "no readings yet".into(),
                }),
        );

    let hits = div()
        .w(px(C_HITS))
        .flex_none()
        .flex()
        .flex_col()
        .child(
            div()
                .text_size(px(BODY))
                .font_weight(FontWeight::SEMIBOLD)
                .when(a.hits_per_week >= AT_LIMIT_HITS && !a.benched, |d| {
                    d.text_color(t.danger)
                })
                .child(if a.has_session_window {
                    num(a.hits_per_week)
                } else {
                    "—".into()
                }),
        )
        .child(
            div()
                .text_size(px(TINY))
                .text_color(t.muted)
                .child(if a.has_session_window {
                    "per week"
                } else {
                    "no 5-h window"
                }),
        );

    let fit_cell = div().flex_1().flex().items_center().child(if a.benched {
        pill("Benched", t.muted, t.surface)
    } else {
        match &a.fit {
            Collect::Ready(f) => fit_pill(f.verdict, t),
            Collect::Collecting { .. } => pill("Collecting", t.muted, t.surface),
        }
    });

    let explain = fit_line(a, o);
    let what_if = a
        .what_if
        .as_ref()
        .map(|w| what_if_text(w, o, Some(a.provider)));
    div()
        .flex()
        .flex_col()
        .pt(px(10.0))
        .pb(px(10.0))
        .border_t_1()
        .border_color(t.border.opacity(0.7))
        .when(a.benched, |d| d.opacity(0.7))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(C_GAP))
                .child(name)
                .child(now)
                .child(week)
                .child(hits)
                .child(fit_cell),
        )
        .child(
            div()
                .pl(px(C_NAME + C_GAP))
                .pt(px(6.0))
                .text_size(px(SMALL))
                .text_color(t.muted)
                .child(explain),
        )
        .children(what_if.map(|w| {
            div()
                .pl(px(C_NAME + C_GAP))
                .pt(px(2.0))
                .text_size(px(SMALL))
                .text_color(t.muted)
                .child(rich(
                    vec![
                        b(match a.what_if.as_ref().map(|x| x.change) {
                            Some(WhatIfChange::Tier(_)) => "What if it downgrades: ",
                            _ => "What if it ends: ",
                        }),
                        m(lower_first(&w)),
                    ],
                    t,
                ))
        }))
}

/// The plan fit in one line.
fn fit_line(a: &AccountInsight, o: &Overview) -> String {
    let line = fit_words(a, o);
    if !a.benched {
        return line;
    }
    let stop = if line.ends_with('.') { "" } else { "." };
    format!("{line}{stop} Benched in the hub, so it never counts as capacity.")
}

fn fit_words(a: &AccountInsight, o: &Overview) -> String {
    let f: &PlanFit = match &a.fit {
        Collect::Ready(f) => f,
        Collect::Collecting { ready_on } => return collecting_text(*ready_on),
    };
    let unknown = if f.tier.is_none() {
        " Plan unknown, so no price advice."
    } else {
        ""
    };
    let line = match f.verdict {
        FitVerdict::Fits => format!(
            "Well used: avg {}, peak {} of its week.",
            pct(f.avg),
            pct(f.peak)
        ),
        FitVerdict::Downgrade => match (f.suggested, f.peak_on_suggested) {
            (Some(to), Some(p)) => format!(
                "Your busiest week would fit {} ({}).{}",
                to.label(),
                pct(p),
                f.price_change
                    .filter(|c| *c < 0.0)
                    .map(|c| format!(" Saves ~{} a month.", usd(c)))
                    .unwrap_or_default()
            ),
            _ => "A smaller plan would do.".into(),
        },
        FitVerdict::AtLimit => match f.suggested {
            Some(to) => format!(
                "Peak {} and {} 5-hour hits a week. {} would give it room{}.",
                pct(f.peak),
                num(f.hits_per_week),
                to.label(),
                f.price_change
                    .filter(|c| *c > 0.0)
                    .map(|c| format!(" (~{} more a month)", usd(c)))
                    .unwrap_or_default()
            ),
            None => {
                "Already the biggest plan and it hits its limits: spread work to other accounts."
                    .into()
            }
        },
        FitVerdict::Unused => format!(
            "Used {} on average in {}.{}",
            pct(f.avg),
            o.range.label(),
            f.price_change
                .map(|c| format!(" Not needed? Cancelling saves {} a month.", usd(c)))
                .unwrap_or_default()
        ),
    };
    format!("{line}{unknown}")
}

// ----- 5-hour windows -----

fn windows_line(
    o: &Overview,
    warm: &WarmupSettings,
    t: &Tokens,
) -> (AnyElement, Option<AnyElement>) {
    // With the warm-up on, the note says so instead of the waited time.
    let warm_note = warm.enabled.then(|| {
        div()
            .flex()
            .items_center()
            .gap(px(5.0))
            .text_size(px(SMALL))
            .text_color(t.ok)
            .child(icon(Icon::Clock, 12.0, t.ok))
            .child(format!(
                "Warm-up {} {}",
                time_text(warm.time),
                warm.days_text()
            ))
            .into_any_element()
    });
    let w: &WindowsReport = match &o.windows {
        Collect::Ready(w) => w,
        Collect::Collecting { ready_on } => return (plain(collecting_text(*ready_on)), warm_note),
    };
    let answer = if w.hits_with_free_account > 0 {
        let n = w.hits_with_free_account;
        rich(
            vec![
                b(if n == 1 {
                    "Once".to_string()
                } else {
                    format!("{n} times")
                }),
                m(" you waited while another account had a fresh window"),
            ],
            t,
        )
        .into_any_element()
    } else if w.hits > 0 {
        rich(
            vec![
                b(format!("{} hits a week", num(w.hits_per_week))),
                m(", no other account had room then"),
            ],
            t,
        )
        .into_any_element()
    } else {
        plain("No 5-hour limit hits in this range")
    };
    let right = warm_note.or_else(|| {
        (w.waited > Duration::zero()).then(|| {
            div()
                .text_size(px(SMALL))
                .text_color(t.faint)
                .child(format!("{} waited", dur_text(w.waited)))
                .into_any_element()
        })
    });
    (answer, right)
}

fn stat(value: String, label: &str, first: bool, t: &Tokens) -> gpui::Div {
    div()
        .flex_1()
        .flex()
        .flex_col()
        .when(!first, |d| {
            d.pl(px(14.0)).border_l_1().border_color(t.border)
        })
        .child(
            div()
                .text_size(px(17.0))
                .font_weight(FontWeight::SEMIBOLD)
                .child(value),
        )
        .child(
            div()
                .text_size(px(SMALL))
                .text_color(t.muted)
                .child(label.to_string()),
        )
}

fn windows_content(o: &Overview, cx: &Ctx) -> AnyElement {
    let t = cx.t;
    let w = match &o.windows {
        Collect::Ready(w) => w,
        Collect::Collecting { ready_on } => {
            return div()
                .flex()
                .gap(px(28.0))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(10.0))
                        .child(explainer(t))
                        .child(
                            div()
                                .text_size(px(SMALL))
                                .text_color(t.muted)
                                .child(format!(
                                    "The 5-hour analysis needs windows on 5 workdays. {}.",
                                    collecting_text(*ready_on)
                                )),
                        ),
                )
                .child(warmup_column(o, cx))
                .into_any_element();
        }
    };

    // Left: how it works, one day drawn twice, the warm-up result.
    let mut left = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .child(explainer(t));
    if let Some(s) = &w.warmup {
        if let Some(e) = &s.example {
            left = left.child(day_timeline(e, s.at, cx.color(e.account), t).mt(px(14.0)));
        }
        let n = s.simulation.days.len();
        left = left.child(
            div()
                .flex()
                .mt(px(16.0))
                .child(stat(
                    dur_text(s.simulation.waited_now),
                    &format!("waited on the last {n} workdays"),
                    true,
                    t,
                ))
                .child(
                    stat(
                        dur_text(s.simulation.waited_with),
                        &format!("with a {} warm-up, same work", time_text(s.at)),
                        false,
                        t,
                    )
                    .text_color(t.ok),
                )
                .child(stat(
                    format!("+{}", s.simulation.runs),
                    "extra windows, one a day",
                    false,
                    t,
                )),
        );
    }

    // Choosing the start by hand, in words.
    left = left.child(
        div()
            .mt(px(18.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(h4("Set your own start", t))
            .children(
                own_start_lines(w, o, &cx.s.settings.warmup)
                    .into_iter()
                    .map(|parts| {
                        div()
                            .text_size(px(SMALL))
                            .line_height(relative(1.5))
                            .text_color(t.muted)
                            .child(rich(parts, t))
                    }),
            ),
    );
    let right = warmup_column(o, cx);

    let numbers = div()
        .flex()
        .child(stat(w.hits.to_string(), "5-hour limit hits", true, t))
        .child(stat(num(w.hits_per_week), "hits a week", false, t))
        .child(stat(dur_text(w.waited), "waited for resets", false, t))
        .child(stat(
            format!("{}×", w.hits_with_free_account),
            "while another account was free",
            false,
            t,
        ));

    div()
        .flex()
        .flex_col()
        .gap(px(24.0))
        .child(div().flex().gap(px(28.0)).child(left).child(right))
        .child(week_timeline(w, o, cx))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(h4(&format!("In the range · {}", o.range.label()), t))
                .child(numbers),
        )
        .into_any_element()
}

fn h4(text: &str, t: &Tokens) -> gpui::Div {
    div()
        .text_size(px(SMALL))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.muted)
        .child(text.to_string())
}

/// How windows and resets work (owner 2026-11-08: explain it shortly).
fn explainer(t: &Tokens) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(h4("How the 5-hour window works", t))
        .child(
            div()
                .text_size(px(SMALL))
                .line_height(relative(1.5))
                .text_color(t.muted)
                .child(rich(
                    vec![
                        m("Your "),
                        b("first message starts a window"),
                        m(", and it resets "),
                        b("5 hours later"),
                        m(
                            ", whether you used all of it or not. The next message after a \
                           reset starts the next window. So the time of your first message \
                           decides all resets of the day.",
                        ),
                    ],
                    t,
                )),
        )
}

/// The "Set your own start" text: the user's rhythm and what to do about it.
fn own_start_lines(
    w: &WindowsReport,
    o: &Overview,
    warm: &WarmupSettings,
) -> Vec<Vec<(String, bool)>> {
    let r = &w.rhythm;
    let mut out = vec![];
    match (r.usual_start, r.usual_first_hit) {
        (Some(start), Some(hit)) => out.push(vec![
            m("You usually start around "),
            b(time_text(start)),
            m(" and run out of the first window around "),
            b(time_text(start + hit)),
            m("."),
        ]),
        (Some(start), None) => out.push(vec![
            m("You usually start around "),
            b(time_text(start)),
            m(" and your first window rarely runs out."),
        ]),
        _ => {}
    }
    if warm.enabled {
        out.push(vec![
            m("The warm-up starts your windows at "),
            b(time_text(warm.time)),
            m(format!(
                " on {}, so the first reset comes at ",
                warm.days_text()
            )),
            b(time_text(warm.time + Duration::hours(5))),
            m(". To move it, change the time; any message before that starts the window too."),
        ]);
        return out;
    }
    match &w.warmup {
        Some(s) => {
            let who = names(
                &s.accounts
                    .iter()
                    .map(|id| label_of(o, *id))
                    .collect::<Vec<_>>(),
            );
            let mut line = vec![m("Send any short message at "), b(time_text(s.at))];
            if !who.is_empty() {
                line.push(m(" with "));
                line.push(b(who));
            }
            line.push(m(". Its window then resets at "));
            line.push(b(time_text(s.at + Duration::hours(5))));
            line.push(m(", close to when you run out, and the next one is fresh."));
            out.push(line);
            out.push(vec![m(format!(
                "Over your last {} workdays that would have saved about {} of waiting \
                 (an estimate from your readings).",
                s.simulation.days.len(),
                dur_text(s.simulation.saved())
            ))]);
        }
        None if r.recent_first_hits < insights::RECENT_HITS => out.push(vec![m(
            "Your first window seldom runs out, so an earlier start would not save time \
             right now.",
        )]),
        None => out.push(vec![m(
            "An earlier start would not move the first reset closer to when you run out.",
        )]),
    }
    out
}

/// Where `t` lies between `from` and `to`, 0..1.
fn frac_of(t: NaiveDateTime, from: NaiveDateTime, to: NaiveDateTime) -> f32 {
    let all = (to - from).num_seconds().max(1) as f32;
    ((t - from).num_seconds() as f32 / all).clamp(0.0, 1.0)
}

/// One lane of windows between `from` and `to` (cut at `now` when given): work in `color`
/// (solid until a hit, lighter in a window without one), the wait after a hit tinted in the
/// danger colour, a warm-up window outlined, an unknown end (PC off) on the track colour.
/// `labels` puts each reset's time above it.
#[allow(clippy::too_many_arguments)]
fn lane(
    spans: &[Span],
    from: NaiveDateTime,
    to: NaiveDateTime,
    now: Option<NaiveDateTime>,
    color: Hsla,
    h: f32,
    labels: bool,
    t: &Tokens,
) -> gpui::Div {
    let top = if labels { 15.0 } else { 0.0 };
    let mut d = div().relative().w_full().h(px(h + top));
    let piece = |a: f32, b: f32| {
        div()
            .absolute()
            .top(px(top))
            .h(px(h))
            .left(relative(a))
            .w(relative((b - a).max(0.0)))
    };
    // The wait after a hit: a thin line in the danger colour, so it never blends with a red or
    // pink account colour.
    let wait = |a: f32, b: f32| {
        div()
            .absolute()
            .top(px(top + h / 2.0 - 1.0))
            .h(px(2.0))
            .left(relative(a))
            .w(relative((b - a).max(0.0)))
            .bg(t.danger)
    };
    for s in spans {
        let end = now.map_or(s.reset, |n| s.reset.min(n));
        if end <= from || s.start >= to {
            continue;
        }
        let x0 = frac_of(s.start, from, to);
        let x1 = frac_of(end, from, to);
        let hit = s.hit.filter(|h| *h < end).map(|h| frac_of(h, from, to));
        if s.warm {
            d = d.child(
                piece(x0, x1)
                    .rounded(px(3.0))
                    .border_1()
                    .border_color(t.accent.opacity(0.8))
                    .bg(t.accent.opacity(0.14))
                    .when(labels, |p| {
                        p.overflow_hidden().pl(px(6.0)).flex().items_center().child(
                            div()
                                .text_size(px(10.5))
                                .text_color(t.accent_text())
                                .whitespace_nowrap()
                                .child(format!("warm-up {}", time_text(s.start.time()))),
                        )
                    }),
            );
            if let Some(xh) = hit {
                d = d.child(wait(xh, x1));
            }
        } else if s.unknown {
            d = d.child(piece(x0, x1).rounded(px(3.0)).bg(t.track));
        } else if let Some(xh) = hit {
            d = d
                .child(piece(x0, xh).rounded_l(px(3.0)).bg(color.opacity(0.85)))
                .child(wait(xh, x1));
        } else {
            d = d.child(piece(x0, x1).rounded(px(3.0)).bg(color.opacity(0.35)));
        }
        if labels && s.reset < to {
            let xr = frac_of(s.reset, from, to);
            d = d
                .child(
                    div()
                        .absolute()
                        .top(px(top - 3.0))
                        .h(px(h + 6.0))
                        .left(relative(xr))
                        .w(px(1.0))
                        .bg(t.text.opacity(0.6)),
                )
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left(relative(xr))
                        .ml(px(-18.0))
                        .w(px(36.0))
                        .flex()
                        .justify_center()
                        .text_size(px(10.5))
                        .text_color(t.muted)
                        .child(time_text(s.reset.time())),
                );
        }
    }
    d
}

/// Hour labels every 3 hours under a day timeline.
fn hour_axis(from: NaiveDateTime, to: NaiveDateTime, t: &Tokens) -> gpui::Div {
    let mut d = div().relative().w_full().h(px(14.0));
    let mut h = from;
    while h <= to {
        let x = frac_of(h, from, to);
        let (first, last) = (h == from, h == to);
        d = d.child(
            div()
                .absolute()
                .top_0()
                .left(relative(x))
                .ml(px(if last {
                    -36.0
                } else if first {
                    0.0
                } else {
                    -18.0
                }))
                .w(px(36.0))
                .flex()
                .when(last, |d| d.justify_end())
                .when(!last && !first, |d| d.justify_center())
                .text_size(px(10.5))
                .text_color(t.faint)
                .child(if h.time() == NaiveTime::MIN && h > from {
                    "24:00".to_string()
                } else {
                    time_text(h.time())
                }),
        );
        h += Duration::hours(3);
    }
    d
}

fn legend(color: Hsla, text: &str) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(5.0))
        .child(div().w(px(12.0)).h(px(8.0)).rounded(px(2.0)).bg(color))
        .child(text.to_string())
}

/// Legend entry for the wait line.
fn legend_line(color: Hsla, text: &str) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(5.0))
        .child(div().w(px(12.0)).h(px(2.0)).bg(color))
        .child(text.to_string())
}

/// One past day as it went and with the warm-up.
fn day_timeline(e: &ExampleDay, at: NaiveTime, color: Hsla, t: &Tokens) -> gpui::Div {
    const LABEL: f32 = 96.0;
    let first_h = e
        .with
        .iter()
        .chain(&e.real)
        .map(|s| s.start.time().hour())
        .min()
        .unwrap_or(6);
    let from_h = (first_h / 3) * 3;
    let from = e.date.and_hms_opt(from_h, 0, 0).unwrap_or_default();
    let midnight = e.date.and_time(NaiveTime::MIN) + Duration::days(1);
    let last = e
        .real
        .iter()
        .chain(&e.with)
        .map(|s| s.reset)
        .max()
        .unwrap_or(from)
        .min(midnight);
    let mut to = from + Duration::hours(3);
    while to < last {
        to += Duration::hours(3);
    }
    let row = |name: &str, spans: &[Span]| {
        div()
            .flex()
            .items_end()
            .child(
                div()
                    .w(px(LABEL))
                    .flex_none()
                    .pb(px(2.0))
                    .text_size(px(SMALL))
                    .text_color(t.muted)
                    .child(name.to_string()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(lane(spans, from, to, None, color, 18.0, true, t)),
            )
    };
    let start_x = frac_of(e.date.and_time(e.start), from, to);
    let plot = div()
        .relative()
        .flex()
        .flex_col()
        .gap(px(6.0))
        // "You started" marker across both lanes (first, so the lanes paint over it).
        .child(
            div()
                .absolute()
                .top(px(12.0))
                .bottom_0()
                .left(px(LABEL))
                .right_0()
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(relative(start_x))
                        .w(px(1.0))
                        .bg(t.text.opacity(0.4)),
                ),
        )
        .child(row("That day", &e.real))
        .child(row("With warm-up", &e.with));
    let label = format!(
        "{} {}: you started at {}. With a {} warm-up the resets come earlier (estimate).",
        e.date.format("%A"),
        date_text(e.date),
        time_text(e.start),
        time_text(at)
    );
    div()
        .flex()
        .flex_col()
        .gap(px(4.0))
        .child(div().text_size(px(TINY)).text_color(t.faint).child(label))
        .child(plot)
        .child(div().pl(px(LABEL)).child(hour_axis(from, to, t)))
        .child(
            div()
                .flex()
                .gap(px(14.0))
                .pl(px(LABEL))
                .pt(px(2.0))
                .text_size(px(TINY))
                .text_color(t.muted)
                .child(legend(color.opacity(0.85), "working"))
                .child(legend_line(t.danger, "waiting for the reset"))
                .child(legend(color.opacity(0.35), "window without a hit")),
        )
}

/// The last 7 days: one lane per account with 5-hour windows.
fn week_timeline(w: &WindowsReport, o: &Overview, cx: &Ctx) -> gpui::Div {
    const LABEL: f32 = 130.0;
    let t = cx.t;
    let from = w.week_from;
    let to = from + Duration::days(insights::WEEK_DAYS);
    let lanes: Vec<&insights::WeekLane> = w.week.iter().filter(|l| !cx.hidden(l.account)).collect();
    let day_w = 1.0 / insights::WEEK_DAYS as f32;
    let mut days = div().relative().w_full().h(px(14.0));
    let mut grid = div()
        .absolute()
        .top_0()
        .bottom_0()
        .left(px(LABEL))
        .right_0();
    for i in 0..insights::WEEK_DAYS {
        let d0 = from + Duration::days(i);
        let x = frac_of(d0, from, to);
        let today = d0.date() == o.today;
        days = days.child(
            div()
                .absolute()
                .top_0()
                .left(relative(x))
                .w(relative(day_w))
                .flex()
                .justify_center()
                .text_size(px(10.5))
                .text_color(if today { t.text } else { t.faint })
                .child(if today {
                    "Today".to_string()
                } else {
                    d0.format("%a %-d").to_string()
                }),
        );
        if i > 0 {
            grid = grid.child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(relative(x))
                    .w(px(1.0))
                    .bg(t.border),
            );
        }
    }
    let mut list = div().relative().flex().flex_col().gap(px(7.0)).child(grid);
    for l in &lanes {
        list = list.child(
            div()
                .flex()
                .items_center()
                .child(
                    div()
                        .w(px(LABEL))
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(7.0))
                        .pr(px(8.0))
                        .child(dot(cx.color(l.account)))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(px(SMALL))
                                .child(label_of(o, l.account)),
                        ),
                )
                .child(div().flex_1().min_w_0().child(lane(
                    &l.spans,
                    from,
                    to,
                    Some(w.week_to),
                    cx.color(l.account),
                    12.0,
                    false,
                    t,
                ))),
        );
    }
    let hits = lanes
        .iter()
        .flat_map(|l| &l.spans)
        .filter(|s| s.hit.is_some_and(|h| h >= from))
        .count();
    div()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(
            div()
                .flex()
                .items_baseline()
                .gap(px(10.0))
                .child(h4("Last 7 days, by account", t))
                .child(
                    div()
                        .text_size(px(TINY))
                        .text_color(t.faint)
                        .child(match hits {
                            0 => "no limit hits".to_string(),
                            1 => "1 limit hit".to_string(),
                            n => format!("{n} limit hits"),
                        }),
                ),
        )
        .child(div().pl(px(LABEL)).child(days))
        .child(if lanes.is_empty() {
            div()
                .text_size(px(SMALL))
                .text_color(t.muted)
                .child("No account with a 5-hour window.")
        } else {
            list
        })
        .child(
            div()
                .flex()
                .gap(px(14.0))
                .pl(px(LABEL))
                .pt(px(2.0))
                .text_size(px(TINY))
                .text_color(t.muted)
                .child(legend(t.muted.opacity(0.85), "until the limit"))
                .child(legend_line(t.danger, "waiting"))
                .child(legend(t.muted.opacity(0.35), "window without a hit"))
                .child(legend(t.track, "PC off, end unknown")),
        )
}

// ----- warm-up controls (build step 4; mockup `warmup.html`) -----

/// A line of the controls column: label (and a small hint) left, the control right.
fn control_line(label: &str, hint: Option<String>, control: AnyElement, t: &Tokens) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(10.0))
        .py(px(8.0))
        .border_t_1()
        .border_color(t.border)
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .child(div().text_size(px(BODY)).child(label.to_string()))
                .children(hint.map(|h| {
                    div()
                        .mt(px(1.0))
                        .text_size(px(TINY))
                        .text_color(t.faint)
                        .child(h)
                })),
        )
        .child(control)
}

/// A small toggle chip.
fn toggle_chip(
    id: impl Into<gpui::ElementId>,
    text: impl Into<SharedString>,
    on: bool,
    enabled: bool,
    t: &Tokens,
    on_click: impl Fn(&mut App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id.into())
        .flex_none()
        .px(px(7.0))
        .py(px(2.0))
        .rounded(px(6.0))
        .text_size(px(TINY))
        .border_1()
        .when(on, |d| {
            d.bg(t.accent.opacity(if t.dark { 0.2 } else { 0.14 }))
                .border_color(gpui::transparent_black())
                .text_color(t.text)
        })
        .when(!on, |d| d.border_color(t.border).text_color(t.muted))
        .when(enabled, |d| {
            d.cursor_pointer()
                .hover(|s| s.text_color(t.text))
                .on_click(move |_, _, cx| on_click(cx))
        })
        .when(!enabled, |d| d.opacity(0.5))
        .child(text.into())
}

/// A small square button with one character ("−", "+").
fn step_button(
    id: &'static str,
    text: &'static str,
    t: &Tokens,
    on_click: impl Fn(&mut App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .size(px(24.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .border_1()
        .border_color(t.border)
        .text_size(px(BODY))
        .text_color(t.muted)
        .cursor_pointer()
        .hover(|s| s.text_color(t.text).bg(t.surface_hover))
        .child(text)
        .on_click(move |_, _, cx| on_click(cx))
}

/// "today 7:00", "yesterday 7:02", "Nov 6, 7:00".
fn when_text(at: chrono::DateTime<Utc>, today: NaiveDate) -> String {
    let local = at.with_timezone(&chrono::Local).naive_local();
    let time = time_text(local.time());
    match (today - local.date()).num_days() {
        0 => format!("today {time}"),
        1 => format!("yesterday {time}"),
        _ => format!("{}, {time}", date_text(local.date())),
    }
}

/// The accounts the warm-up can be chosen for: visible ones with a 5-hour window.
fn warm_candidates<'a>(o: &'a Overview, s: &AppState) -> Vec<&'a AccountInsight> {
    o.accounts
        .iter()
        .filter(|a| a.has_session_window && !s.account(a.account).is_some_and(|x| x.hidden))
        .collect()
}

fn can_warm_id(s: &AppState, id: Uuid) -> bool {
    s.account(id).is_some_and(subhub_core::warmup::can_warm)
}

/// What the setup dialog opens with: the current settings when the warm-up is on; otherwise
/// the suggestion (time and accounts) over the last choices, with Wake the PC off. The user
/// confirms or changes it; nothing runs before (owner, 2026-11-08).
pub fn suggested_draft(o: &Overview, s: &AppState) -> WarmupSettings {
    let cur = &s.settings.warmup;
    if cur.enabled {
        return cur.clone();
    }
    let mut d = cur.clone();
    d.wake_pc = false;
    if let Some(x) = o.windows.ready().and_then(|r| r.warmup.as_ref()) {
        d.time = x.at;
        let picked: Vec<Uuid> = x
            .accounts
            .iter()
            .copied()
            .filter(|id| can_warm_id(s, *id))
            .collect();
        if !picked.is_empty() {
            d.accounts = picked;
        }
    }
    d.accounts.retain(|id| can_warm_id(s, *id));
    if d.days.is_empty() {
        d.days = subhub_core::warmup::WORKDAYS.to_vec();
    }
    d
}

/// Opens the setup dialog (and the 5-hour row) with [`suggested_draft`].
fn open_dialog(me: Entity<StatsWindow>, state: Entity<AppState>) -> impl Fn(&mut App) + 'static {
    move |app: &mut App| {
        let draft = {
            let o = me.read(app).overview_ref().latest();
            o.map(|o| suggested_draft(&o, state.read(app)))
        };
        me.update(app, |w, cx| {
            let ov = w.overview_mut();
            ov.open = Some(TipRow::Windows);
            ov.warm_dialog = Some(draft.unwrap_or_default());
            cx.notify();
        });
    }
}

/// "Personal (Claude), Work (Codex)".
fn account_names(o: &Overview, ids: &[Uuid]) -> String {
    names(&ids.iter().map(|id| label_of(o, *id)).collect::<Vec<_>>())
}

fn small_text(text: impl Into<SharedString>, t: &Tokens) -> gpui::Div {
    div()
        .text_size(px(TINY))
        .line_height(relative(1.5))
        .text_color(t.faint)
        .child(text.into())
}

/// A text button (secondary, or the accent for the main action).
fn text_button(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    primary: bool,
    enabled: bool,
    t: &Tokens,
    on_click: impl Fn(&mut App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id.into())
        .flex_none()
        .px(px(12.0))
        .py(px(5.0))
        .rounded(px(8.0))
        .text_size(px(12.5))
        .font_weight(FontWeight::SEMIBOLD)
        .when(primary, |d| d.bg(t.accent).text_color(gpui::white()))
        .when(!primary, |d| {
            d.border_1().border_color(t.border).bg(t.surface)
        })
        .when(!enabled, |d| d.opacity(0.5))
        .when(enabled, |d| {
            d.cursor_pointer()
                .hover(move |s| {
                    if primary {
                        s.opacity(0.9)
                    } else {
                        s.bg(t.surface_hover)
                    }
                })
                .on_click(move |_, _, cx| on_click(cx))
        })
        .child(label.into())
}

/// The Warm-up column of the 5-hour row. Off: what is suggested and "Set up warm-up…". On:
/// the settings in words, the last runs, "Change…", "Turn off" and "Start a window now".
fn warmup_column(o: &Overview, cx: &Ctx) -> gpui::Div {
    let t = cx.t;
    let s = cx.s;
    let w = &s.settings.warmup;
    let col = div()
        .w(px(300.0))
        .flex_none()
        .pl(px(28.0))
        .border_l_1()
        .border_color(t.border)
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(h4("Warm-up", t));
    let setup = open_dialog(cx.me.clone(), cx.state.clone());
    let para = |parts: Vec<(String, bool)>| {
        div()
            .text_size(px(SMALL))
            .line_height(relative(1.5))
            .text_color(t.muted)
            .child(rich(parts, t))
    };

    if !w.enabled {
        let suggestion = o.windows.ready().and_then(|r| r.warmup.as_ref());
        let mut parts = vec![m(
            "SubHub can send a one-word prompt at a time you choose, so the first window starts \
             early. It only runs once you turn it on.",
        )];
        if let Some(x) = suggestion {
            parts = vec![m("Suggested: "), b(time_text(x.at)), m(" on weekdays")];
            let who: Vec<Uuid> = x
                .accounts
                .iter()
                .copied()
                .filter(|id| can_warm_id(s, *id))
                .collect();
            if !who.is_empty() {
                parts.push(m(" for "));
                parts.push(b(account_names(o, &who)));
            }
            parts.push(m(". It only runs once you turn it on."));
        }
        return col
            .child(para(parts))
            .child(div().mt(px(4.0)).flex().child(text_button(
                "warmup-setup",
                "Set up warm-up…",
                true,
                true,
                t,
                setup,
            )));
    }

    // On.
    let chosen: Vec<Uuid> = w
        .accounts
        .iter()
        .copied()
        .filter(|id| can_warm_id(s, *id))
        .collect();
    let mut summary = vec![
        m("On at "),
        b(time_text(w.time)),
        m(format!(" {}", w.days_text())),
    ];
    if chosen.is_empty() {
        summary.push(m(", no account chosen"));
    } else {
        summary.push(m(" for "));
        summary.push(b(account_names(o, &chosen)));
    }
    summary.push(m(if w.wake_pc {
        ". Wakes the PC for it."
    } else {
        ". Runs while SubHub is running."
    }));
    let runs: Vec<String> = chosen
        .iter()
        .filter_map(|id| {
            let r = s.warmup_log.last.get(id)?;
            Some(format!(
                "{} · {} · {}",
                label_of(o, *id),
                when_text(r.at, o.today),
                r.outcome.text()
            ))
        })
        .collect();
    let running = !s.warmup_running.is_empty();
    let start_now = {
        let state = cx.state.clone();
        let ids = chosen.clone();
        text_button(
            "warmup-now",
            if running {
                "Starting…"
            } else if chosen.len() > 1 {
                "Start windows now"
            } else {
                "Start a window now"
            },
            false,
            !chosen.is_empty() && !running,
            t,
            move |app| {
                let ids = ids.clone();
                state.update(app, |s, cx| s.warmup_now(ids, cx));
            },
        )
    };
    let turn_off = {
        let state = cx.state.clone();
        link("warmup-off", "Turn off", true, t, move |app| {
            state.update(app, |s, cx| s.set_warmup(|w| w.enabled = false, cx));
        })
    };
    col.child(para(summary))
        .children(
            s.warmup_task_error
                .clone()
                .map(|e| div().text_size(px(TINY)).text_color(t.danger).child(e)),
        )
        .when(w.wake_pc && s.dev_build(), |d| {
            d.child(small_text(
                "Dev build: the installed app sets up the wake task.",
                t,
            ))
        })
        .when(!runs.is_empty(), |d| {
            d.child(
                div()
                    .mt(px(4.0))
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(h4("Last runs", t))
                    .children(
                        runs.into_iter()
                            .map(|r| div().text_size(px(TINY)).text_color(t.muted).child(r)),
                    ),
            )
        })
        .child(
            div()
                .mt(px(6.0))
                .flex()
                .items_center()
                .gap(px(12.0))
                .child(start_now)
                .child(link("warmup-change", "Change…", false, t, setup))
                .child(turn_off),
        )
}

/// The setup dialog over the Statistics window, editing a draft; "Turn on" / "Save" applies it,
/// Cancel drops it.
pub fn dialog(ov: &OverviewState, cx: &Ctx) -> Option<AnyElement> {
    let draft = ov.warm_dialog.as_ref()?;
    let o = ov.latest()?;
    let o = o.as_ref();
    let t = cx.t;
    let s = cx.s;
    let editing = s.settings.warmup.enabled;
    let suggestion = o.windows.ready().and_then(|r| r.warmup.as_ref());
    let candidates = warm_candidates(o, s);
    // Every control edits the draft.
    let edit = |f: Rc<dyn Fn(&mut WarmupSettings)>| {
        cx.upd(move |ov| {
            if let Some(d) = ov.warm_dialog.as_mut() {
                f(d);
            }
        })
    };

    let step = |mins: i64| -> Rc<dyn Fn(&mut WarmupSettings)> {
        Rc::new(move |w: &mut WarmupSettings| {
            let m = (w.time.hour() as i64 * 60 + w.time.minute() as i64 + mins).rem_euclid(24 * 60);
            let m = m - m % 15;
            w.time = NaiveTime::from_hms_opt((m / 60) as u32, (m % 60) as u32, 0).unwrap_or(w.time);
        })
    };
    let usual = o.windows.ready().and_then(|r| r.rhythm.usual_start);
    let time_hint = match (suggestion, usual) {
        (Some(x), Some(start)) if x.at == draft.time => Some(format!(
            "Suggested from your usual start, {}",
            time_text(start)
        )),
        (Some(x), Some(start)) => Some(format!(
            "Suggested {} from your usual start, {}",
            time_text(x.at),
            time_text(start)
        )),
        _ => None,
    };
    let use_suggested = suggestion.filter(|x| x.at != draft.time).map(|x| {
        let at = x.at;
        link(
            "warmup-use-suggested",
            format!("Use {}", time_text(at)),
            false,
            t,
            edit(Rc::new(move |w: &mut WarmupSettings| w.time = at)),
        )
    });
    let time = div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .children(use_suggested)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(4.0))
                .child(step_button("warmup-earlier", "−", t, edit(step(-15))))
                .child(
                    div()
                        .w(px(48.0))
                        .flex()
                        .justify_center()
                        .py(px(2.0))
                        .rounded(px(6.0))
                        .bg(t.surface)
                        .text_size(px(BODY))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(time_text(draft.time)),
                )
                .child(step_button("warmup-later", "+", t, edit(step(15)))),
        );

    let mut days = div().flex().gap(px(3.0));
    for (i, d) in subhub_core::warmup::WEEK.iter().enumerate() {
        let d = *d;
        days = days.child(toggle_chip(
            ("warmup-day", i),
            subhub_core::warmup::short_day(d),
            draft.runs_on(d),
            true,
            t,
            edit(Rc::new(move |w: &mut WarmupSettings| {
                if w.runs_on(d) {
                    w.days.retain(|x| *x != d);
                } else {
                    w.days.push(d);
                }
            })),
        ));
    }

    let mut accounts = div().flex().flex_wrap().gap(px(4.0));
    let mut hub_only: Vec<String> = vec![];
    for (i, a) in candidates.iter().enumerate() {
        let id = a.account;
        let ok = can_warm_id(s, id);
        if !ok {
            hub_only.push(label_of(o, id));
        }
        accounts = accounts.child(toggle_chip(
            ("warmup-acc", i),
            label_of(o, id),
            ok && draft.accounts.contains(&id),
            ok,
            t,
            edit(Rc::new(move |w: &mut WarmupSettings| {
                if w.accounts.contains(&id) {
                    w.accounts.retain(|x| *x != id);
                } else {
                    w.accounts.push(id);
                }
            })),
        ));
    }

    let wake = {
        let toggle = edit(Rc::new(|w: &mut WarmupSettings| w.wake_pc = !w.wake_pc));
        Switch::new("warmup-wake")
            .checked(draft.wake_pc)
            .disabled(crate::demo::on())
            .on_click(move |_, _, app| toggle(app))
    };
    let wake_hint = if crate::demo::on() {
        "Not in demo mode".to_string()
    } else {
        "A Windows task wakes the PC from sleep or hibernation (not when shut down)".to_string()
    };

    let valid = !draft.days.is_empty() && draft.accounts.iter().any(|id| can_warm_id(s, *id));
    let apply = {
        let state = cx.state.clone();
        let d = draft.clone();
        let close = cx.upd(|ov| ov.warm_dialog = None);
        move |app: &mut App| {
            let d = d.clone();
            state.update(app, |s, cx| {
                s.set_warmup(
                    |w| {
                        *w = d;
                        w.enabled = true;
                    },
                    cx,
                )
            });
            close(app);
        }
    };
    let cancel = cx.upd(|ov| ov.warm_dialog = None);

    let reset = draft.time + Duration::hours(5);
    let panel = div()
        .id("warmup-dialog")
        .w(px(460.0))
        .flex()
        .flex_col()
        .p(px(22.0))
        .rounded(px(12.0))
        .border_1()
        .border_color(t.border)
        .bg(t.bg)
        .shadow_lg()
        // Clicks inside never reach the backdrop.
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(
            div()
                .text_size(px(16.0))
                .font_weight(FontWeight::SEMIBOLD)
                .child(if editing {
                    "Warm-up"
                } else {
                    "Set up warm-up"
                }),
        )
        .child(
            div()
                .mt(px(6.0))
                .mb(px(8.0))
                .text_size(px(SMALL))
                .line_height(relative(1.5))
                .text_color(t.muted)
                .child(rich(
                    vec![
                        m("At "),
                        b(time_text(draft.time)),
                        m(" SubHub sends the one-word prompt \"ok\" with the smallest model to each \
                           chosen account. Its 5-hour window then resets at "),
                        b(time_text(reset)),
                        m(". Accounts whose window is already running are skipped."),
                    ],
                    t,
                )),
        )
        .child(control_line("Time", time_hint, time.into_any_element(), t))
        .child(control_line("Days", None, days.into_any_element(), t))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .py(px(8.0))
                .border_t_1()
                .border_color(t.border)
                .child(div().text_size(px(BODY)).child("Accounts"))
                .child(if candidates.is_empty() {
                    small_text("No account with a 5-hour window.", t)
                } else {
                    accounts
                })
                .when(!hub_only.is_empty(), |d| {
                    d.child(small_text(
                        format!(
                            "{} {} a CLI profile on this PC.",
                            names(&hub_only),
                            if hub_only.len() == 1 { "needs" } else { "need" }
                        ),
                        t,
                    ))
                }),
        )
        .child(control_line(
            "Wake the PC",
            Some(wake_hint),
            wake.into_any_element(),
            t,
        ))
        .child(
            div()
                .pt(px(8.0))
                .border_t_1()
                .border_color(t.border)
                .child(small_text(
                    "Costs well under 1% of a window. Runs while SubHub is running; with Wake \
                     the PC also from sleep (wake timers must be allowed in the power options). \
                     A missed time still runs within an hour. You can turn it off any time.",
                    t,
                )),
        )
        .child(
            div()
                .mt(px(16.0))
                .flex()
                .justify_end()
                .gap(px(8.0))
                .child(text_button("warmup-cancel", "Cancel", false, true, t, cancel))
                .child(text_button(
                    "warmup-apply",
                    if editing { "Save" } else { "Turn on" },
                    true,
                    valid,
                    t,
                    apply,
                )),
        );
    let backdrop_close = cx.upd(|ov| ov.warm_dialog = None);
    Some(
        div()
            .id("warmup-backdrop")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::black().opacity(if t.dark { 0.45 } else { 0.25 }))
            .on_click(move |_, _, cx| backdrop_close(cx))
            .child(panel)
            .into_any_element(),
    )
}

// ----- Renewals -----

fn renewals_line(o: &Overview, t: &Tokens) -> (AnyElement, Option<AnyElement>) {
    let upcoming: Vec<&Renewal> = o.renewals.iter().filter(|r| r.days >= 0).collect();
    let Some(first) = upcoming.first() else {
        return (plain("No renewal or end dates set"), None);
    };
    let mut parts = vec![];
    for (i, r) in upcoming.iter().take(3).enumerate() {
        if i > 0 {
            parts.push(m(" · "));
        }
        parts.push(m(format!(
            "{} {} ",
            label_of(o, r.account),
            if r.auto_renew { "renews" } else { "ends" }
        )));
        parts.push(b(date_text(r.date)));
    }
    let right = div()
        .text_size(px(SMALL))
        .text_color(t.faint)
        .child(days_text(first.days))
        .into_any_element();
    (rich(parts, t).into_any_element(), Some(right))
}

fn renewals_content(o: &Overview, cx: &Ctx) -> AnyElement {
    let t = cx.t;
    let mut list = div().flex().flex_col();
    for r in o.renewals.iter().filter(|r| !cx.hidden(r.account)) {
        let id = r.account;
        let what = match (r.auto_renew, r.days >= 0) {
            (true, _) => "Renews",
            (false, true) => "Ends",
            (false, false) => "Ended",
        };
        let open = {
            let f = cx.open_settings.clone();
            move |cx: &mut App| f(SettingsTarget::Account(id), cx)
        };
        list = list.child(
            div()
                .flex()
                .items_center()
                .gap(px(C_GAP))
                .py(px(9.0))
                .border_t_1()
                .border_color(t.border.opacity(0.7))
                .text_size(px(BODY))
                .child(
                    div()
                        .w(px(C_NAME))
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(7.0))
                        .child(dot(cx.color(id)))
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .truncate()
                                .child(label_of(o, id)),
                        ),
                )
                .child(
                    div()
                        .w(px(190.0))
                        .flex_none()
                        .flex()
                        .gap(px(6.0))
                        .child(format!("{what} {}", date_text(r.date)))
                        .when(r.estimate, |d| {
                            d.child(div().text_color(t.faint).child("· estimated"))
                        }),
                )
                .child(
                    div()
                        .w(px(100.0))
                        .flex_none()
                        .text_color(t.muted)
                        .child(days_text(r.days)),
                )
                .child(
                    div()
                        .w(px(130.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .text_size(px(SMALL))
                        .text_color(t.muted)
                        .child(icon(
                            if r.reminders {
                                Icon::Bell
                            } else {
                                Icon::BellOff
                            },
                            13.0,
                            t.faint,
                        ))
                        .child(if r.reminders {
                            "Reminder on"
                        } else {
                            "Reminder off"
                        }),
                )
                .child(div().flex_1())
                .child(link(
                    SharedString::from(format!("renewal-settings-{id}")),
                    "Account settings…",
                    false,
                    t,
                    open,
                )),
        );
    }
    let missing: Vec<String> = o
        .accounts
        .iter()
        .filter(|a| !cx.hidden(a.account) && renewal_of(o, a.account).is_none())
        .map(|a| label_of(o, a.account))
        .collect();
    if !missing.is_empty() {
        list = list.child(
            div()
                .pt(px(10.0))
                .border_t_1()
                .border_color(t.border.opacity(0.7))
                .text_size(px(SMALL))
                .text_color(t.muted)
                .child(format!(
                    "No date set for {}. Dates go in each account's settings.",
                    names(&missing)
                )),
        );
    }
    list.into_any_element()
}

// ----- Where usage goes -----

fn usage_line(o: &Overview, cx: &Ctx) -> AnyElement {
    let u = &o.usage;
    if u.work == 0 {
        return plain(if cx.s.stats.is_none() {
            "Reading the session logs…"
        } else {
            "No session logs in this range"
        });
    }
    let mut parts = vec![m("Mostly ")];
    let kinds: Vec<&insights::Share<TaskKind>> = u.kinds.iter().take(2).collect();
    for (i, k) in kinds.iter().enumerate() {
        if i > 0 {
            parts.push(m(" and "));
        }
        parts.push(b(kind_lower(k.key)));
    }
    let projects: Vec<String> = u
        .projects
        .iter()
        .filter(|p| p.key != ProjectKey::Unknown && !p.label.is_empty())
        .take(2)
        .map(|p| p.label.clone())
        .collect();
    if !projects.is_empty() {
        parts.push(m(", in "));
        for (i, p) in projects.iter().enumerate() {
            if i > 0 {
                parts.push(m(" and "));
            }
            parts.push(b(p.clone()));
        }
    }
    rich(parts, cx.t).into_any_element()
}

/// One bar row: label (+ subline), main part solid and subagent part light, share.
fn share_row(
    label_w: f32,
    label: String,
    sub: Option<String>,
    share: f64,
    sub_part: f64,
    max: f64,
    t: &Tokens,
) -> gpui::Div {
    let scale = if max > 0.0 { share / max } else { 0.0 };
    let main_w = (scale * (1.0 - sub_part)).clamp(0.0, 1.0) as f32;
    let sub_w = (scale * sub_part).clamp(0.0, 1.0) as f32;
    div()
        .flex()
        .items_center()
        .gap(px(12.0))
        .py(px(5.0))
        .child(
            div()
                .w(px(label_w))
                .flex_none()
                .flex()
                .flex_col()
                .child(div().text_size(px(BODY)).truncate().child(label))
                .children(sub.map(|s| {
                    div()
                        .text_size(px(TINY))
                        .text_color(t.faint)
                        .truncate()
                        .child(s)
                })),
        )
        .child(
            div()
                .flex_1()
                .h(px(10.0))
                .rounded(px(3.0))
                .bg(t.track)
                .flex()
                .overflow_hidden()
                .child(div().h_full().w(relative(main_w)).bg(t.accent))
                .child(div().h_full().w(relative(sub_w)).bg(t.accent.opacity(0.4))),
        )
        .child(
            div()
                .w(px(44.0))
                .flex_none()
                .text_right()
                .text_size(px(BODY))
                .text_color(t.muted)
                .child(share_text(share)),
        )
}

fn share_text(frac: f64) -> String {
    if frac <= 0.0 {
        "—".into()
    } else if frac < 0.01 {
        "<1%".into()
    } else {
        pct(frac * 100.0)
    }
}

fn sub_part(by_tokens: bool, usd: f64, sub_usd: f64, work: u64, sub_work: u64) -> f64 {
    let (a, b) = if by_tokens {
        (sub_work as f64, work as f64)
    } else {
        (sub_usd, usd)
    };
    if b > 0.0 {
        (a / b).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn thread_title(th: &insights::ThreadShare) -> String {
    th.title
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "Untitled thread".into())
}

/// "writing code · 3 days" (Biggest threads); with the project in the Thread view, which has
/// the room.
fn thread_sub(th: &insights::ThreadShare, o: &Overview, with_project: bool) -> String {
    let mut s = vec![];
    if with_project {
        s.push(
            o.usage
                .projects
                .iter()
                .find(|p| p.key == th.project)
                .map(|p| project_label(&p.key, &p.label))
                .unwrap_or_else(|| project_label(&th.project, "")),
        );
    }
    if let Some(k) = th.kind {
        s.push(kind_lower(k));
    }
    s.push(format!(
        "{} day{}",
        th.days,
        if th.days == 1 { "" } else { "s" }
    ));
    s.join(" · ")
}

fn usage_content(ov: &OverviewState, o: &Overview, cx: &Ctx) -> AnyElement {
    let t = cx.t;
    let u = &o.usage;
    if u.work == 0 {
        return div()
            .text_size(px(SMALL))
            .text_color(t.muted)
            .child(if cx.s.stats.is_none() {
                "The session logs are being read."
            } else {
                "No Claude Code or Codex session logs in this range."
            })
            .into_any_element();
    }
    let me = cx.me.clone();
    let switch = seg(
        "ov-dim",
        &[
            (Dim::Task, "Task"),
            (Dim::Project, "Project"),
            (Dim::Thread, "Thread"),
            (Dim::Model, "Model"),
        ],
        &ov.dim,
        t,
        move |d, cx| {
            me.update(cx, |w, cx| {
                w.overview_mut().dim = d;
                cx.notify();
            })
        },
    );
    let swatch = |opacity: f32| {
        div()
            .w(px(12.0))
            .h(px(10.0))
            .rounded(px(2.0))
            .bg(t.accent.opacity(opacity))
    };
    let legend = div()
        .flex()
        .gap(px(14.0))
        .ml_auto()
        .text_size(px(SMALL))
        .text_color(t.muted)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(swatch(1.0))
                .child("main threads"),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(swatch(0.4))
                .child("subagents"),
        );

    let bt = u.by_tokens;
    let bars: Vec<gpui::Div> = match ov.dim {
        Dim::Task => {
            let max = u.kinds.first().map_or(0.0, |k| k.share);
            u.kinds
                .iter()
                .map(|k| {
                    share_row(
                        LABEL_W,
                        k.label.clone(),
                        Some(kind_covers(k.key).to_string()),
                        k.share,
                        sub_part(bt, k.usd, k.sub_usd, k.work, k.sub_work),
                        max,
                        t,
                    )
                })
                .collect()
        }
        Dim::Project => {
            let max = u.projects.first().map_or(0.0, |k| k.share);
            u.projects
                .iter()
                .take(10)
                .map(|p| {
                    share_row(
                        LABEL_W,
                        project_label(&p.key, &p.label),
                        None,
                        p.share,
                        sub_part(bt, p.usd, p.sub_usd, p.work, p.sub_work),
                        max,
                        t,
                    )
                })
                .collect()
        }
        Dim::Model => {
            let max = u.models.first().map_or(0.0, |k| k.share);
            u.models
                .iter()
                .take(10)
                .map(|p| {
                    share_row(
                        LABEL_W,
                        p.label.clone(),
                        None,
                        p.share,
                        sub_part(bt, p.usd, p.sub_usd, p.work, p.sub_work),
                        max,
                        t,
                    )
                })
                .collect()
        }
        Dim::Thread => {
            let max = u.threads.first().map_or(0.0, |k| k.share);
            u.threads
                .iter()
                .take(10)
                .map(|th| {
                    share_row(
                        THREAD_LABEL_W,
                        thread_title(th),
                        Some(thread_sub(th, o, true)),
                        th.share,
                        sub_part(bt, th.usd, th.sub_usd, th.work, 0),
                        max,
                        t,
                    )
                })
                .collect()
        }
    };

    // Biggest threads on the right (not in the Thread view, which lists them already).
    let side = (ov.dim != Dim::Thread).then(|| {
        let top: Vec<&insights::ThreadShare> = u.threads.iter().take(4).collect();
        let shown: f64 = top.iter().map(|x| x.share).sum();
        let others = u.thread_count.saturating_sub(top.len());
        let entry = |title: String, sub: Option<String>, share: f64, first: bool, faint: bool| {
            div()
                .flex()
                .gap(px(8.0))
                .py(px(6.0))
                .when(!first, |d| {
                    d.border_t_1().border_color(t.border.opacity(0.7))
                })
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_size(px(12.5))
                                .truncate()
                                .when(faint, |d| d.text_color(t.faint))
                                .child(title),
                        )
                        .children(sub.map(|s| {
                            div()
                                .text_size(px(TINY))
                                .text_color(t.faint)
                                .truncate()
                                .child(s)
                        })),
                )
                .child(
                    div()
                        .w(px(40.0))
                        .flex_none()
                        .text_right()
                        .text_size(px(12.5))
                        .text_color(t.muted)
                        .child(share_text(share)),
                )
        };
        let mut col = div().w(px(290.0)).flex_none().flex().flex_col().child(
            div()
                .mb(px(4.0))
                .text_size(px(SMALL))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.muted)
                .child("Biggest threads"),
        );
        for (i, th) in top.iter().enumerate() {
            let sub = thread_sub(th, o, false);
            col = col.child(entry(thread_title(th), Some(sub), th.share, i == 0, false));
        }
        if others > 0 {
            col = col.child(entry(
                format!(
                    "{others} other thread{}",
                    if others == 1 { "" } else { "s" }
                ),
                None,
                (1.0 - shown).max(0.0),
                top.is_empty(),
                true,
            ));
        }
        col
    });

    // One insight line.
    let builds = u.kind_share(TaskKind::BuildsTests);
    let sub_share = u.sub_share();
    let (title, rest) = if builds > insights::TASK_INSIGHT_SHARE {
        (
            format!("Builds & tests take {}.", pct(builds * 100.0)),
            "Mostly re-reading long build output. Quieter runs (only the failing test, less output) would cut a good part of it.".to_string(),
        )
    } else if sub_share >= 0.25 {
        (
            format!("Subagents do {} of the work.", pct(sub_share * 100.0)),
            "Cheaper models for searches and reading would lower that share.".to_string(),
        )
    } else if let Some(k) = u.kinds.first() {
        (
            format!("{} leads with {}.", k.label, pct(k.share * 100.0)),
            format!("Subagents do {} of the work.", share_text(sub_share)),
        )
    } else {
        (String::new(), String::new())
    };
    let basis = if bt {
        "Shares of work tokens (nothing in this range has a price)."
    } else {
        "Shares of the API estimate, so models compare."
    };

    div()
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(12.0))
                .mb(px(14.0))
                .child(switch)
                .child(legend),
        )
        .child(
            div()
                .flex()
                .gap(px(36.0))
                .child(div().flex_1().min_w_0().flex().flex_col().children(bars))
                .children(side),
        )
        .when(!title.is_empty(), |d| {
            d.child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(10.0))
                    .mt(px(16.0))
                    .child(chip(Msg::Info, 24.0, t))
                    .child(
                        div()
                            .flex_1()
                            .pt(px(3.0))
                            .text_size(px(12.5))
                            .line_height(relative(1.45))
                            .text_color(t.muted)
                            .child(rich(vec![b(title), m(format!(" {rest}"))], t)),
                    ),
            )
        })
        .child(
            div()
                .mt(px(10.0))
                .text_size(px(TINY))
                .text_color(t.faint)
                .child(basis),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_and_numbers_read_short() {
        assert_eq!(dur_text(Duration::minutes(40)), "40 min");
        assert_eq!(dur_text(Duration::minutes(180)), "3 h");
        assert_eq!(dur_text(Duration::minutes(220)), "3 h 40");
        assert_eq!(dur_text(Duration::minutes(185)), "3 h 05");
        assert_eq!(dur_text(Duration::minutes(-5)), "0 min");
        assert_eq!(num(5.0), "5");
        assert_eq!(num(4.98), "5");
        assert_eq!(num(1.34), "1.3");
        assert_eq!(days_text(0), "today");
        assert_eq!(days_text(17), "in 17 days");
        assert_eq!(days_text(-3), "3 days ago");
    }

    #[test]
    fn names_and_shares_in_words() {
        assert_eq!(names(&[]), "");
        assert_eq!(names(&["A".into()]), "A");
        assert_eq!(names(&["A".into(), "B".into()]), "A and B");
        assert_eq!(names(&["A".into(), "B".into(), "C".into()]), "A, B and C");
        assert_eq!(share_words(0.52), Some("half"));
        assert_eq!(share_words(0.31), Some("a third"));
        assert_eq!(share_words(0.95), Some("nearly all"));
        assert_eq!(share_words(0.03), None);
    }

    /// The demo's story reads as planned: a verdict naming the idle and the busy account, and a
    /// worded tip for every tip kind it produces.
    #[test]
    fn demo_overview_is_worded() {
        use subhub_core::demo;
        let now = Utc::now();
        let s = demo::settings(std::path::Path::new(r"C:\demo\home"), now, "0.1.2");
        let plans = s
            .accounts
            .iter()
            .filter_map(|a| Some((a.id, demo::plan(a.id)?.to_string())))
            .collect();
        let history = demo::history_lines(now);
        let o = insights::analyze(&insights::Input {
            history: &history,
            scan: None,
            settings: &s,
            plans: &plans,
            prices: &Default::default(),
            range: InsightRange::FourWeeks,
            now,
            zone: &Zone::utc(),
        });
        let (first, rest) = verdict(&o);
        assert!(first.starts_with("You use"), "{first}");
        assert!(rest.contains("Personal") && rest.contains("Work"), "{rest}");
        for tip in &o.tips {
            let w = tip_text(tip, &o, true);
            assert!(!w.title.is_empty(), "{tip:?}");
            assert!(w.title.ends_with('.'), "{}", w.title);
        }
        // The 5-hour row: a warm-up with an example day, a week lane per account with windows.
        let w = o.windows.ready().expect("demo has 5 workdays of windows");
        let warm = w.warmup.as_ref().expect("demo suggests a warm-up");
        let e = warm.example.as_ref().expect("example day");
        assert!(e.with[0].warm && !e.real.is_empty());
        assert!(w.week.iter().any(|l| !l.spans.is_empty()));
        let off = WarmupSettings::default();
        assert!(own_start_lines(w, &o, &off).len() >= 2);
    }
}

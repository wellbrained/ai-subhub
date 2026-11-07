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

use chrono::{Duration, NaiveDate, NaiveTime, Utc};
use gpui::{
    AnyElement, App, Context, Entity, FontWeight, HighlightStyle, Hsla, IntoElement, ParentElement,
    SharedString, Styled, StyledText, div, prelude::*, px, relative,
};
use subhub_core::insights::{
    self, AT_LIMIT_HITS, AccountInsight, Collect, FitVerdict, InsightRange, Overview, PlanFit,
    ProjectKey, Renewal, Tip, TipAction, TipKind, TipRow, WhatIf, WhatIfChange, WindowsReport,
    Zone,
};
use subhub_core::model::ProviderKind;
use subhub_core::stats::TaskKind;
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
}

impl OverviewState {
    pub fn new() -> Self {
        let (open, tips_open) = match crate::demo::open_row() {
            Some("plans") => (Some(TipRow::Plans), false),
            Some("windows") => (Some(TipRow::Windows), false),
            Some("renewals") => (Some(TipRow::Renewals), false),
            Some("usage") => (Some(TipRow::Usage), false),
            Some("tips") => (None, true),
            _ => (None, false),
        };
        Self {
            range: InsightRange::FourWeeks,
            result: None,
            computing: None,
            open,
            tips_open,
            dim: Dim::Task,
        }
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
        let plans = s.plans.clone();
        let prices = s.prices.clone();
        let range = self.range;
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
                ov.result = Some((sig, Instant::now(), Arc::new(overview)));
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
        s.plans.get(&a.id).hash(&mut h);
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
                action: row_action("Show me how"),
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
        move |cx: &mut App| match a {
            TipAction::OpenRow(_) => upd(cx),
            TipAction::Steering => settings(SettingsTarget::Hub, cx),
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
    let visible: Vec<&Tip> = o
        .tips
        .iter()
        .filter(|tp| !dismissed.contains(&tp.id))
        .collect();
    let hidden = o.tips.len() - visible.len();
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
    let (win_answer, win_right) = windows_line(o, t);
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
            || windows_content(o, t),
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
        .child(small("Now".into()).w(px(C_NOW)).flex_none())
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

    let now_bar = |label: &str, w: Option<insights::NowWindow>| {
        let (frac, text, color) = match w {
            Some(w) => (w.used / 100.0, pct(w.used), t.level(100.0 - w.used)),
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

// ----- 5-hour windows (this part: the line and the numbers; part 3 adds the explainer) -----

fn windows_line(o: &Overview, t: &Tokens) -> (AnyElement, Option<AnyElement>) {
    let w: &WindowsReport = match &o.windows {
        Collect::Ready(w) => w,
        Collect::Collecting { ready_on } => return (plain(collecting_text(*ready_on)), None),
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
    let right = (w.waited > Duration::zero()).then(|| {
        div()
            .text_size(px(SMALL))
            .text_color(t.faint)
            .child(format!("{} waited", dur_text(w.waited)))
            .into_any_element()
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

fn windows_content(o: &Overview, t: &Tokens) -> AnyElement {
    let w = match &o.windows {
        Collect::Ready(w) => w,
        Collect::Collecting { ready_on } => {
            return div()
                .text_size(px(SMALL))
                .text_color(t.muted)
                .child(format!(
                    "The 5-hour analysis needs windows on 5 workdays. {}.",
                    collecting_text(*ready_on)
                ))
                .into_any_element();
        }
    };
    let mut facts: Vec<String> = vec![];
    if let (Some(start), Some(hit)) = (w.rhythm.usual_start, w.rhythm.usual_first_hit) {
        facts.push(format!(
            "Your workday usually starts around {} and the first window runs out after about {}.",
            time_text(start),
            dur_text(hit)
        ));
    } else if let Some(start) = w.rhythm.usual_start {
        facts.push(format!(
            "Your workday usually starts around {}.",
            time_text(start)
        ));
    }
    if let Some(s) = &w.warmup {
        facts.push(format!(
            "Starting the first window at {} would have saved about {} over {} workdays.",
            time_text(s.at),
            dur_text(s.simulation.saved()),
            s.simulation.days.len()
        ));
    }
    div()
        .flex()
        .flex_col()
        .gap(px(14.0))
        .child(
            div()
                .flex()
                .child(stat(w.hits.to_string(), "5-hour limit hits", true, t))
                .child(stat(num(w.hits_per_week), "hits a week", false, t))
                .child(stat(dur_text(w.waited), "waited for resets", false, t))
                .child(stat(
                    format!("{}×", w.hits_with_free_account),
                    "while another account was free",
                    false,
                    t,
                )),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .text_size(px(SMALL))
                .text_color(t.muted)
                .children(facts.into_iter().map(|f| div().child(f))),
        )
        .into_any_element()
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
    }
}

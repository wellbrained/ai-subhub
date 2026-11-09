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
use crate::format::Clip as _;
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
    self, AT_LIMIT_HITS, AccountInsight, Collect, CycleLen, ExampleDay, FitVerdict, InsightRange,
    Overview, PlanFit, ProjectKey, Renewal, Span, Tip, TipAction, TipKind, TipRow, WhatIf,
    WhatIfChange, WindowsReport, Zone,
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
/// History read beyond the plan-fit lookback, so the oldest cycle in it starts inside (a monthly
/// cycle that ended inside the lookback began up to a month before it).
const HISTORY_EXTRA_DAYS: i64 = 35;

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
    /// The topic page shown (None: the summary page).
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

    /// The last result if it was computed from the current inputs (exports wait for it, so a
    /// log scan that finished meanwhile is in).
    pub fn current(&self, s: &AppState) -> Option<Arc<Overview>> {
        let sig = signature(s, self.range);
        self.result
            .as_ref()
            .filter(|(k, _, _)| *k == sig)
            .map(|(_, _, o)| o.clone())
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
        s.settings.manual_plans.get(&a.id).hash(&mut h);
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

mod parts;
mod plans;
mod renewals;
pub mod report;
#[cfg(test)]
mod tests;
mod usage;
mod warmup;
mod windows;
mod wording;
pub use parts::*;
use plans::*;
use renewals::*;
use usage::*;
pub use warmup::*;
use windows::*;
pub use wording::*;

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
    if let Some(page) = ov.open {
        return topic_page(page, ov, o, cx);
    }
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
        .child(rows(o, cx).mt(px(8.0)))
        .into_any_element()
}

/// The small count next to a topic in the sidebar: plans that need a look (red when one is at
/// its limit; every account's own row counts, also the app logins kept out of the header),
/// waits while another account was free, days to a renewal within two weeks.
pub fn nav_badge(ov: &OverviewState, r: TipRow, t: &Tokens) -> Option<AnyElement> {
    let o = ov.latest()?;
    let tint = |c: Hsla| c.opacity(if t.dark { 0.16 } else { 0.12 });
    let (text, fg, bg) = match r {
        TipRow::Plans => {
            let verdicts: Vec<FitVerdict> = o
                .accounts
                .iter()
                .filter_map(|a| a.fit.ready().map(|f| f.verdict))
                .filter(|v| *v != FitVerdict::Fits)
                .collect();
            let n = verdicts.len();
            if n == 0 {
                return None;
            }
            let c = if verdicts.contains(&FitVerdict::AtLimit) {
                t.danger
            } else {
                t.info
            };
            (n.to_string(), c, tint(c))
        }
        TipRow::Windows => {
            let n = o.windows.ready()?.hits_with_free_account;
            if n == 0 {
                return None;
            }
            (n.to_string(), t.danger, tint(t.danger))
        }
        TipRow::Renewals => {
            let next = o.header.next_renewal.as_ref()?;
            if next.days > 14 {
                return None;
            }
            (format!("{} d", next.days), t.muted, t.bg)
        }
        TipRow::Usage => return None,
    };
    Some(pill(text, fg, bg).into_any_element())
}

/// The name of a topic page (sidebar and page title).
pub fn topic_name(r: TipRow) -> &'static str {
    match r {
        TipRow::Plans => "Plans",
        TipRow::Windows => "5-hour windows",
        TipRow::Renewals => "Renewals",
        TipRow::Usage => "Where usage goes",
    }
}

/// The answer in words and the short right-hand note of a topic (summary line and page head).
fn topic_line(r: TipRow, o: &Overview, cx: &Ctx) -> (AnyElement, Option<AnyElement>) {
    match r {
        TipRow::Plans => plans_line(o, cx),
        TipRow::Windows => windows_line(o, &cx.s.settings.warmup, cx.t),
        TipRow::Renewals => renewals_line(o, cx.t),
        TipRow::Usage => (usage_line(o, cx), None),
    }
}

/// The sidebar page a topic lives on (owner, 2026-11-08: two topics per page). Plans and
/// Renewals are about what you pay for; 5-hour windows and Where usage goes about how you use it.
pub fn page_name(r: TipRow) -> &'static str {
    match r {
        TipRow::Plans | TipRow::Renewals => "Plans & renewals",
        TipRow::Windows | TipRow::Usage => "Usage patterns",
    }
}

/// The topics of the page `r` is on, in page order.
fn page_topics(r: TipRow) -> [TipRow; 2] {
    match r {
        TipRow::Plans | TipRow::Renewals => [TipRow::Plans, TipRow::Renewals],
        TipRow::Windows | TipRow::Usage => [TipRow::Windows, TipRow::Usage],
    }
}

/// The sidebar badge of the page `r` is on: the first of its topics that has one.
pub fn page_badge(ov: &OverviewState, r: TipRow, t: &Tokens) -> Option<AnyElement> {
    page_topics(r)
        .into_iter()
        .find_map(|topic| nav_badge(ov, topic, t))
}

/// One sidebar page (owner, 2026-11-08: no accordion; two topics per page): the page title,
/// then per topic its name, the answer and the details.
fn topic_page(r: TipRow, ov: &OverviewState, o: &Overview, cx: &Ctx) -> AnyElement {
    let t = cx.t;
    let mut page = div()
        .flex()
        .flex_col()
        .px(px(SIDE))
        .pt(px(12.0))
        .pb(px(24.0))
        .child(
            div()
                .text_size(px(20.0))
                .font_weight(FontWeight::SEMIBOLD)
                .child(page_name(r)),
        );
    for (i, topic) in page_topics(r).into_iter().enumerate() {
        let (answer, right) = topic_line(topic, o, cx);
        let content = match topic {
            TipRow::Plans => plans_content(o, cx),
            TipRow::Windows => windows_content(o, cx),
            TipRow::Renewals => renewals_content(o, cx),
            TipRow::Usage => usage_content(ov, o, cx),
        };
        // The topic that was opened: once laid out, scroll its section to the top of the view.
        let anchor = (i > 0 && cx.focus.get() == Some(topic)).then(|| {
            let (handle, focus) = (cx.scroll.clone(), cx.focus.clone());
            gpui::canvas(
                move |b, window, _| {
                    let view = handle.bounds();
                    let off = handle.offset();
                    let y = off.y - (b.origin.y - view.origin.y) + px(12.0);
                    handle.set_offset(gpui::point(off.x, y.min(px(0.0))));
                    focus.set(None);
                    window.refresh();
                },
                |_, _, _, _| {},
            )
            .w_full()
            .h(px(0.0))
        });
        page = page.children(anchor).child(
            div()
                .flex()
                .flex_col()
                .mt(px(if i == 0 { 18.0 } else { 40.0 }))
                .child(
                    div()
                        .text_size(px(15.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(topic_name(topic)),
                )
                .child(
                    div()
                        .mt(px(4.0))
                        .pb(px(14.0))
                        .flex()
                        .items_center()
                        .gap(px(16.0))
                        .border_b_1()
                        .border_color(t.border)
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(px(BODY))
                                .text_color(t.muted)
                                .child(answer),
                        )
                        .children(right),
                )
                .child(div().pt(px(18.0)).child(content)),
        );
    }
    page.into_any_element()
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

/// One summary line: name, the answer in words, a short note and a chevron; a click opens the
/// topic's page.
fn row(
    id: &'static str,
    name: &str,
    answer: AnyElement,
    right: Option<AnyElement>,
    on_open: impl Fn(&mut App) + 'static,
    t: &Tokens,
) -> gpui::Stateful<gpui::Div> {
    let (chev, chev_hover) = (t.faint, t.text);
    div()
        .id(id)
        .group(id)
        .flex()
        .items_center()
        .gap(px(16.0))
        .py(px(14.0))
        .border_b_1()
        .border_color(t.border)
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
        .child(div().w(px(18.0)).flex_none().flex().justify_end().child(
            icon(Icon::ChevronRight, 14.0, chev).group_hover(id, move |s| s.text_color(chev_hover)),
        ))
        .on_click(move |_, _, cx| on_open(cx))
}

fn rows(o: &Overview, cx: &Ctx) -> gpui::Div {
    let t = cx.t;
    let mut list = div().flex().flex_col();
    for (id, r) in [
        ("row-plans", TipRow::Plans),
        ("row-windows", TipRow::Windows),
        ("row-renewals", TipRow::Renewals),
        ("row-usage", TipRow::Usage),
    ] {
        let (answer, right) = topic_line(r, o, cx);
        list = list.child(row(
            id,
            topic_name(r),
            answer,
            right,
            cx.upd(move |ov| ov.open = Some(r)),
            t,
        ));
    }
    list
}

fn plain(text: impl Into<SharedString>) -> AnyElement {
    div().child(text.into()).into_any_element()
}

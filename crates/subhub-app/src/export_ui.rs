//! Export from the main window (owner, 2026-11-08): the usage report as PDF, tokens and cost as
//! CSV, the limit history as CSV. The title bar's Export button opens a small menu; a pick asks
//! where to save (Windows' save dialog, Documents first), builds the file in the background and
//! writes it. Demo: `--demo-export report|tokens|limits=PATH` writes without a dialog once the
//! data is ready (hidden screenshot runs).
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::{Duration as Days, Local, Utc};
use gpui::{
    Context, Entity, InteractiveElement, ParentElement, StatefulInteractiveElement, Styled, div, px,
};
use subhub_core::export::{AccountName, limits_csv, tokens_csv};
use subhub_core::insights::Overview;

use crate::icons::{Icon, icon};
use crate::state::AppState;
use crate::stats_window::StatsWindow;
use crate::theme::Tokens;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Report,
    Tokens,
    Limits,
}

impl Kind {
    fn file_name(self) -> String {
        let day = Local::now().format("%Y-%m-%d");
        match self {
            Kind::Report => format!("AI SubHub report {day}.pdf"),
            Kind::Tokens => format!("AI SubHub tokens and cost {day}.csv"),
            Kind::Limits => format!("AI SubHub limit history {day}.csv"),
        }
    }

    pub fn from_flag(s: &str) -> Option<Self> {
        match s {
            "report" => Some(Kind::Report),
            "tokens" => Some(Kind::Tokens),
            "limits" => Some(Kind::Limits),
            _ => None,
        }
    }
}

/// Where the save dialog starts: Documents, else the home folder.
fn start_dir() -> PathBuf {
    let home = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let docs = home.join("Documents");
    if docs.is_dir() { docs } else { home }
}

/// What a kind needs, taken on the UI thread (cheap clones), built into bytes on a background
/// thread.
enum Job {
    Report(Vec<crate::overview::report::Block>),
    Tokens(
        Arc<subhub_core::stats::Scan>,
        subhub_core::stats::PriceBook,
        chrono::NaiveDate,
    ),
    Limits {
        path: PathBuf,
        since: chrono::DateTime<Utc>,
        names: HashMap<uuid::Uuid, AccountName>,
        labels: HashMap<(uuid::Uuid, String), String>,
    },
}

impl Job {
    fn build(self) -> Result<Vec<u8>, String> {
        match self {
            Job::Report(blocks) => crate::report_pdf::render("AI SubHub · Usage report", &blocks),
            Job::Tokens(scan, prices, from) => Ok(tokens_csv(&scan, from, &prices).into_bytes()),
            Job::Limits {
                path,
                since,
                names,
                labels,
            } => {
                let lines = subhub_core::history::load_lines(&path, since);
                Ok(limits_csv(&lines, since, &names, &labels).into_bytes())
            }
        }
    }
}

/// The job for `kind` over the last `days`, or why there is nothing to export yet.
fn job(
    kind: Kind,
    s: &AppState,
    overview: Option<Arc<Overview>>,
    days: i64,
) -> Result<Job, &'static str> {
    match kind {
        Kind::Report => {
            let o = overview.ok_or("The Overview is still being prepared")?;
            let hidden =
                |id: uuid::Uuid| s.settings.accounts.iter().any(|a| a.id == id && a.hidden);
            Ok(Job::Report(crate::overview::report::blocks(
                &o,
                &hidden,
                &s.settings.dismissed_tips,
                s.settings.steering.enabled,
                Local::now(),
            )))
        }
        Kind::Tokens => {
            let scan = s.stats.clone().ok_or("No logs scanned yet")?;
            let from = Local::now().date_naive() - Days::days(days - 1);
            Ok(Job::Tokens(scan, s.prices.clone(), from))
        }
        Kind::Limits => {
            let names = s
                .settings
                .accounts
                .iter()
                .map(|a| {
                    (
                        a.id,
                        AccountName {
                            name: a.label.clone(),
                            provider: a.provider,
                        },
                    )
                })
                .collect();
            let labels = s
                .settings
                .readings
                .iter()
                .flat_map(|(id, r)| {
                    r.windows
                        .iter()
                        .map(move |w| ((*id, w.key.clone()), w.label.clone()))
                })
                .collect();
            Ok(Job::Limits {
                path: s.history.path().to_path_buf(),
                since: Utc::now() - Days::days(days),
                names,
                labels,
            })
        }
    }
}

/// Exports `kind` over the last `days`. Without `to`: asks where, then queues it on the window
/// (`StatsWindow::queue_export`), which calls back here with the path once the data is ready.
/// With `to`: builds, writes, and leaves a note in the title bar.
pub fn run(
    window: &mut StatsWindow,
    kind: Kind,
    days: i64,
    to: Option<PathBuf>,
    state: &Entity<AppState>,
    cx: &mut Context<StatsWindow>,
) {
    let Some(path) = to else {
        // Ask where first; the window exports once the data is ready.
        cx.spawn(async move |this, cx| {
            let Ok(rx) = this.update(cx, |_, cx| {
                cx.prompt_for_new_path(&start_dir(), Some(&kind.file_name()))
            }) else {
                return;
            };
            if let Ok(Ok(Some(path))) = rx.await {
                let _ = this.update(cx, |w, cx| w.queue_export(kind, path, cx));
            }
        })
        .detach();
        return;
    };
    let overview = window.overview_current(state.read(cx));
    let job = match job(kind, state.read(cx), overview, days) {
        Ok(j) => j,
        Err(why) => {
            window.set_export_note(why.to_string(), cx);
            return;
        }
    };
    cx.spawn(async move |this, cx| {
        let built = cx
            .background_executor()
            .spawn(async move { job.build() })
            .await;
        let msg = match built.and_then(|b| std::fs::write(&path, b).map_err(|e| e.to_string())) {
            Ok(()) => format!(
                "Saved {}",
                path.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default()
            ),
            Err(e) => format!("Export failed: {e}"),
        };
        let _ = this.update(cx, |w, cx| w.set_export_note(msg, cx));
    })
    .detach();
}

/// The menu under the Export button.
pub fn menu(t: &Tokens, on_pick: impl Fn(Kind, &mut gpui::App) + Clone + 'static) -> gpui::Div {
    let item =
        |id: &'static str, glyph: Icon, title: &'static str, hint: &'static str, kind: Kind| {
            let on_pick = on_pick.clone();
            div()
                .id(id)
                .flex()
                .items_start()
                .gap_2()
                .px(px(8.0))
                .py(px(7.0))
                .rounded(px(7.0))
                .cursor_pointer()
                .hover(|d| d.bg(t.surface_hover))
                .on_click(move |_, _, cx| on_pick(kind, cx))
                .child(div().pt(px(2.0)).child(icon(glyph, 14.0, t.muted)))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(div().text_sm().text_color(t.text).child(title))
                        .child(div().text_xs().text_color(t.faint).child(hint)),
                )
        };
    div()
        .w(px(260.0))
        .flex()
        .flex_col()
        .p(px(6.0))
        .rounded(px(10.0))
        .bg(t.surface)
        .border_1()
        .border_color(t.border)
        .shadow(vec![gpui::BoxShadow {
            color: gpui::hsla(0.0, 0.0, 0.0, if t.dark { 0.45 } else { 0.16 }),
            offset: gpui::point(px(0.0), px(8.0)),
            blur_radius: px(24.0),
            spread_radius: px(-4.0),
            inset: false,
        }])
        .child(item(
            "export-report",
            Icon::FileText,
            "Usage report (PDF)",
            "Verdict, accounts, renewals, tips, costs",
            Kind::Report,
        ))
        .child(item(
            "export-tokens",
            Icon::Sheet,
            "Tokens & cost (CSV)",
            "Per day, provider and model",
            Kind::Tokens,
        ))
        .child(item(
            "export-limits",
            Icon::Sheet,
            "Limit history (CSV)",
            "Every saved reading of each account",
            Kind::Limits,
        ))
}

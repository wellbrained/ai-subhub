//! The PDF usage report's content (owner, 2026-11-08: "Usage report" export): the Overview's own
//! sentences and numbers for its range, as plain blocks that `crate::report_pdf` lays out. Pure;
//! hidden accounts and dismissed tips stay out, as on screen.
use super::*;

/// One block of the report, top to bottom.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Title(String),
    Subtitle(String),
    /// A bold lead and the rest of the sentence.
    Lead(String, String),
    /// Key numbers: value and what it is.
    Kpis(Vec<(String, String)>),
    Heading(String),
    Text(String),
    /// Header, relative column widths, rows. Columns after the first are right-aligned when
    /// `numeric` says so.
    Table {
        header: Vec<String>,
        widths: Vec<usize>,
        numeric: Vec<bool>,
        rows: Vec<Vec<String>>,
    },
    Note(String),
}

fn table(cols: &[(&str, usize, bool)], rows: Vec<Vec<String>>) -> Block {
    Block::Table {
        header: cols.iter().map(|c| c.0.to_string()).collect(),
        widths: cols.iter().map(|c| c.1).collect(),
        numeric: cols.iter().map(|c| c.2).collect(),
        rows,
    }
}

/// Rows of a "where usage goes" share list (largest first, at most `max`).
fn share_rows<K>(
    shares: &[insights::Share<K>],
    by_tokens: bool,
    max: usize,
    name: impl Fn(&insights::Share<K>) -> String,
) -> Vec<Vec<String>> {
    shares
        .iter()
        .take(max)
        .map(|s| {
            vec![
                name(s),
                share_text(s.share),
                if by_tokens {
                    "–".into()
                } else {
                    format!("${:.2}", s.usd)
                },
                crate::stats_window::tokens_text(s.work as f64),
            ]
        })
        .collect()
}

/// The report for `o`. `hidden`: accounts hidden from the views; `dismissed`: tip ids the owner
/// set aside; `steering_on`: hub steering is on (tip wording).
pub fn blocks(
    o: &Overview,
    hidden: &dyn Fn(Uuid) -> bool,
    dismissed: &[String],
    steering_on: bool,
    made_at: chrono::DateTime<chrono::Local>,
) -> Vec<Block> {
    let from = o.today - chrono::Duration::days(o.range.days() - 1);
    let mut out = vec![
        Block::Title("AI SubHub · Usage report".into()),
        Block::Subtitle(format!(
            "{} · {} – {} · made {}",
            o.range.label(),
            date_text(from),
            date_text(o.today),
            made_at.format("%b %-d, %Y %H:%M")
        )),
    ];
    let (lead, rest) = verdict(o);
    out.push(Block::Lead(lead, rest));

    let h = &o.header;
    let renewal = match &h.next_renewal {
        Some(r) if r.days == 0 => ("Today".to_string(), "next renewal".to_string()),
        Some(r) => (days_text(r.days), "next renewal".to_string()),
        None => ("–".to_string(), "no renewal dates set".to_string()),
    };
    out.push(Block::Kpis(vec![
        (
            h.weekly_avg.map(pct).unwrap_or_else(|| "–".into()),
            "of weekly limits used (avg)".into(),
        ),
        (
            format!("{} / week", num(h.hits_per_week)),
            "5-hour limit hits".into(),
        ),
        (dur_text(h.waited), "waited for resets".into()),
        renewal,
    ]));

    // Accounts and their plan fit.
    let accounts: Vec<&AccountInsight> = o.accounts.iter().filter(|a| !hidden(a.account)).collect();
    if !accounts.is_empty() {
        out.push(Block::Heading("Accounts".into()));
        let rows = accounts
            .iter()
            .map(|a| {
                vec![
                    format!("{} ({})", a.label, provider_name(a.provider)),
                    a.plan.clone().unwrap_or_else(|| "–".into()),
                    a.weekly_avg.map(pct).unwrap_or_else(|| "–".into()),
                    a.weekly_peak.map(pct).unwrap_or_else(|| "–".into()),
                    a.hits.to_string(),
                    fit_line(a, o),
                ]
            })
            .collect();
        out.push(table(
            &[
                ("Account", 20, false),
                ("Plan", 11, false),
                ("Avg week", 11, true),
                ("Peak", 7, true),
                ("5h hits", 9, true),
                ("Plan fit", 42, false),
            ],
            rows,
        ));
    }

    // Renewals and ends.
    out.push(Block::Heading("Renewals".into()));
    let renewals: Vec<&Renewal> = o.renewals.iter().filter(|r| !hidden(r.account)).collect();
    if renewals.is_empty() {
        out.push(Block::Text("No renewal or end dates set.".into()));
    } else {
        let rows = renewals
            .iter()
            .map(|r| {
                let what = match (r.auto_renew, r.days >= 0) {
                    (true, _) => "Renews",
                    (false, true) => "Ends",
                    (false, false) => "Ended",
                };
                vec![
                    label_of(o, r.account),
                    what.to_string(),
                    format!(
                        "{}{}",
                        date_text(r.date),
                        if r.estimate { " (estimate)" } else { "" }
                    ),
                    days_text(r.days),
                ]
            })
            .collect();
        out.push(table(
            &[
                ("Account", 35, false),
                ("What", 20, false),
                ("Date", 25, false),
                ("When", 20, true),
            ],
            rows,
        ));
    }

    // Tips, as on the Overview.
    let tips: Vec<TipText> = o
        .tips
        .iter()
        .filter(|tp| !dismissed.contains(&tp.id))
        .map(|tp| tip_text(tp, o, steering_on))
        .collect();
    if !tips.is_empty() {
        out.push(Block::Heading("Tips".into()));
        for tp in tips {
            out.push(Block::Lead(tp.title, tp.rest));
        }
    }

    // Where usage goes.
    let u = &o.usage;
    if u.work > 0 {
        out.push(Block::Heading("Where usage goes".into()));
        out.push(Block::Text(if u.by_tokens {
            format!(
                "{} work tokens in the range (no model has a public price, so shares go by tokens).",
                crate::stats_window::tokens_text(u.work as f64)
            )
        } else {
            format!(
                "${:.2} at API prices, {} work tokens in the range.",
                u.usd,
                crate::stats_window::tokens_text(u.work as f64)
            )
        }));
        let cols = |first: &'static str| {
            [
                (first, 46, false),
                ("Share", 14, true),
                ("API estimate", 20, true),
                ("Work tokens", 20, true),
            ]
        };
        out.push(table(
            &cols("Model"),
            share_rows(&u.models, u.by_tokens, 12, |s| s.label.clone()),
        ));
        if !u.projects.is_empty() {
            out.push(table(
                &cols("Project"),
                share_rows(&u.projects, u.by_tokens, 12, |s| {
                    project_label(&s.key, &s.label)
                }),
            ));
        }
        if !u.kinds.is_empty() {
            out.push(table(
                &cols("Kind of work"),
                share_rows(&u.kinds, u.by_tokens, 12, |s| s.label.clone()),
            ));
        }
    }
    out.push(Block::Note(
        "API estimate: tokens priced at public API rates (LiteLLM price list); a subscription's \
         real cost differs. Work tokens: input + output + cache writes. Logs carry no account, \
         so token numbers are per provider."
            .into(),
    ));
    out
}

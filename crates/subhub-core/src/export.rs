//! CSV exports (owner, 2026-11-08): tokens and cost per day, provider and model from the log
//! scan, and the limit history (every saved reading). Plain RFC 4180 CSV with a header row,
//! comma separated, `.` decimals, ISO dates, so Excel and Google Sheets read it as is.
//! Logs carry no account (the owner's Claude profiles share one log folder), so token rows are
//! per provider, not per account.
use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Local, NaiveDate, Utc};
use uuid::Uuid;

use crate::history::HistoryLine;
use crate::model::ProviderKind;
use crate::stats::PriceBook;
use crate::stats::Scan;
use crate::stats::{Tokens, cost, hour_date};

/// One CSV field: quoted when it holds a comma, quote or line break (quotes doubled).
pub fn field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn line(fields: &[String]) -> String {
    let mut s = fields
        .iter()
        .map(|f| field(f))
        .collect::<Vec<_>>()
        .join(",");
    s.push_str("\r\n");
    s
}

fn provider_name(p: ProviderKind) -> &'static str {
    p.name()
}

/// Tokens and API-estimate cost per local day, provider and model, from `from` on (oldest
/// first). Cost is empty for models without a price.
pub fn tokens_csv(scan: &Scan, from: NaiveDate, prices: &PriceBook) -> String {
    let mut rows: BTreeMap<(NaiveDate, &'static str, &str), Tokens> = BTreeMap::new();
    for b in &scan.buckets {
        let day = hour_date(b.hour);
        if day < from {
            continue;
        }
        rows.entry((day, provider_name(b.provider), b.model.as_str()))
            .or_default()
            .add(&b.tokens);
    }
    let mut out = line(
        &[
            "date",
            "provider",
            "model",
            "input_tokens",
            "output_tokens",
            "cache_write_tokens",
            "cache_read_tokens",
            "cost_usd",
        ]
        .map(String::from),
    );
    for ((day, provider, model), t) in rows {
        if t.is_zero() {
            continue;
        }
        let usd = prices
            .price(model)
            .map(|p| format!("{:.4}", cost(&t, &p)))
            .unwrap_or_default();
        out.push_str(&line(&[
            day.to_string(),
            provider.to_string(),
            model.to_string(),
            t.input.to_string(),
            t.output.to_string(),
            t.cache_write.to_string(),
            t.cache_read.to_string(),
            usd,
        ]));
    }
    out
}

/// Who an account is in the limit history: its name and provider.
pub struct AccountName {
    pub name: String,
    pub provider: ProviderKind,
}

/// Every saved reading from `since` on (oldest first): local time, account, provider, window key
/// and its label when the account's latest reading still has it, % used, local reset time.
/// Readings of removed accounts keep their id as the name.
pub fn limits_csv(
    lines: &[HistoryLine],
    since: DateTime<Utc>,
    names: &HashMap<Uuid, AccountName>,
    window_labels: &HashMap<(Uuid, String), String>,
) -> String {
    let local = |t: DateTime<Utc>| {
        t.with_timezone(&Local)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string()
    };
    let mut kept: Vec<&HistoryLine> = lines
        .iter()
        .filter(|l| l.fetched_at >= since && l.used_percent.is_finite())
        .collect();
    kept.sort_by_key(|l| l.fetched_at);
    let mut out = line(
        &[
            "time",
            "account",
            "provider",
            "window",
            "window_label",
            "used_percent",
            "resets_at",
        ]
        .map(String::from),
    );
    for l in kept {
        let (name, provider) = match names.get(&l.account) {
            Some(a) => (a.name.clone(), provider_name(a.provider).to_string()),
            None => (l.account.to_string(), String::new()),
        };
        let label = window_labels
            .get(&(l.account, l.window.clone()))
            .cloned()
            .unwrap_or_default();
        out.push_str(&line(&[
            local(l.fetched_at),
            name,
            provider,
            l.window.clone(),
            label,
            // + 0.0 turns -0.0 into 0.0.
            format!("{:.1}", l.used_percent.clamp(0.0, 100.0) + 0.0),
            l.resets_at.map(local).unwrap_or_default(),
        ]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::Bucket;
    use crate::stats::TaskKind;
    use crate::stats::hour_index;
    use std::sync::Arc;

    fn bucket(day: &str, h: u32, provider: ProviderKind, model: &str, input: u64) -> Bucket {
        let d: NaiveDate = day.parse().unwrap();
        Bucket {
            hour: hour_index(d.and_hms_opt(h, 0, 0).unwrap()),
            provider,
            model: model.into(),
            tokens: Tokens {
                input,
                output: 10,
                cache_write: 0,
                cache_read: 5,
            },
            kind: TaskKind::WritingCode,
            sidechain: false,
            cwd: None,
            thread: Arc::from("t"),
        }
    }

    #[test]
    fn fields_are_quoted_only_when_needed() {
        assert_eq!(field("plain"), "plain");
        assert_eq!(field("a,b"), "\"a,b\"");
        assert_eq!(field("say \"hi\""), "\"say \"\"hi\"\"\"");
    }

    #[test]
    fn tokens_add_up_per_day_provider_and_model() {
        let scan = Scan {
            buckets: vec![
                bucket("2026-11-07", 9, ProviderKind::Claude, "claude-x", 100),
                bucket("2026-11-07", 15, ProviderKind::Claude, "claude-x", 50),
                bucket("2026-11-08", 9, ProviderKind::Codex, "gpt-y", 7),
                bucket("2026-11-01", 9, ProviderKind::Codex, "gpt-y", 999),
            ],
            ..Default::default()
        };
        let csv = tokens_csv(&scan, "2026-11-05".parse().unwrap(), &PriceBook::default());
        let rows: Vec<&str> = csv.lines().collect();
        assert_eq!(
            rows[0],
            "date,provider,model,input_tokens,output_tokens,cache_write_tokens,cache_read_tokens,cost_usd"
        );
        assert_eq!(rows.len(), 3);
        assert!(rows[1].starts_with("2026-11-07,Claude,claude-x,150,20,0,10,"));
        assert!(rows[2].starts_with("2026-11-08,Codex,gpt-y,7,10,0,5,"));
        assert!(csv.ends_with("\r\n"));
    }

    #[test]
    fn limit_history_names_accounts_and_windows() {
        let a = Uuid::from_u128(1);
        let gone = Uuid::from_u128(2);
        let at = |s: &str| s.parse::<DateTime<Utc>>().unwrap();
        let lines = vec![
            HistoryLine {
                account: a,
                fetched_at: at("2026-11-08T10:00:00Z"),
                window: "five_hour".into(),
                used_percent: 42.0,
                resets_at: Some(at("2026-11-08T12:00:00Z")),
            },
            HistoryLine {
                account: gone,
                fetched_at: at("2026-11-08T09:00:00Z"),
                window: "seven_day".into(),
                used_percent: 130.0,
                resets_at: None,
            },
            HistoryLine {
                account: a,
                fetched_at: at("2026-10-01T09:00:00Z"),
                window: "five_hour".into(),
                used_percent: 1.0,
                resets_at: None,
            },
        ];
        let names = HashMap::from([(
            a,
            AccountName {
                name: "Work, main".into(),
                provider: ProviderKind::Claude,
            },
        )]);
        let labels = HashMap::from([((a, "five_hour".to_string()), "Session".to_string())]);
        let csv = limits_csv(&lines, at("2026-11-01T00:00:00Z"), &names, &labels);
        let rows: Vec<&str> = csv.lines().collect();
        assert_eq!(rows.len(), 3);
        // Oldest first; a removed account keeps its id; % clamped.
        assert!(rows[1].contains(&gone.to_string()) && rows[1].contains(",seven_day,,100.0,"));
        assert!(rows[2].contains(",\"Work, main\",Claude,five_hour,Session,42.0,"));
    }
}

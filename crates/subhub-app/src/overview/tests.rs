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
    // The app logins' own rows: Copilot in months, Antigravity by capacity and never in $.
    let copilot = fit_words(o.account(demo::COPILOT).unwrap(), &o);
    assert!(copilot.contains("of its month"), "{copilot}");
    let agy = fit_words(o.account(demo::ANTIGRAVITY).unwrap(), &o);
    assert!(agy.contains("Google AI Pro") && !agy.contains('$'), "{agy}");
    let shown: Vec<&AccountInsight> = o.accounts.iter().collect();
    assert_eq!(cycle_heading(&shown), "Cycle use");
    let report = report::blocks(&o, &|_| false, &[], true, chrono::Local::now());
    let text = format!("{report:?}");
    assert!(text.contains("Pro · monthly") && text.contains("Avg cycle"));
}

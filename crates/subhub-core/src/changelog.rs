//! Patch notes from `CHANGELOG.md`: `## X.Y.Z – date` headers, each followed by short bullets.
//! `## Unreleased` (and any header that is not a version) is ignored. Pure text parsing.
use crate::update::Version;

/// One released version with its bullets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub version: Version,
    /// As written after the dash ("2026-11-07"); None when the header has no date.
    pub date: Option<String>,
    pub bullets: Vec<String>,
}

/// Every versioned section in file order.
pub fn sections(text: &str) -> Vec<Section> {
    let mut out: Vec<Section> = vec![];
    // None while inside a section we skip (Unreleased, other headers).
    let mut current: Option<Section> = None;
    for raw in text.lines() {
        let line = raw.trim_end();
        if let Some(head) = line.strip_prefix("## ") {
            if let Some(done) = current.take() {
                out.push(done);
            }
            current = parse_header(head);
            continue;
        }
        // Higher-level headers end the section too.
        if line.starts_with("# ") {
            if let Some(done) = current.take() {
                out.push(done);
            }
            continue;
        }
        let Some(sec) = current.as_mut() else {
            continue;
        };
        let trimmed = line.trim_start();
        if let Some(b) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            if !b.trim().is_empty() {
                sec.bullets.push(b.trim().to_string());
            }
        } else if !trimmed.is_empty()
            && line.starts_with(char::is_whitespace)
            && let Some(last) = sec.bullets.last_mut()
        {
            // An indented continuation line belongs to the bullet above.
            last.push(' ');
            last.push_str(trimmed);
        }
    }
    if let Some(done) = current {
        out.push(done);
    }
    out
}

/// "0.1.0 – 2026-11-07" (en dash, em dash or hyphen; the date is optional).
fn parse_header(head: &str) -> Option<Section> {
    let head = head.trim();
    let (ver, date) = match head.find(char::is_whitespace) {
        Some(i) => (&head[..i], head[i..].trim()),
        None => (head, ""),
    };
    let version = Version::parse(ver)?;
    let date = date.trim_start_matches(['–', '—', '-']).trim();
    Some(Section {
        version,
        date: (!date.is_empty()).then(|| date.to_string()),
        bullets: vec![],
    })
}

/// Sections newer than `last_seen` up to and including `current`, newest first. `last_seen`
/// None includes everything up to `current`.
pub fn since(text: &str, last_seen: Option<&Version>, current: &Version) -> Vec<Section> {
    let mut v: Vec<Section> = sections(text)
        .into_iter()
        .filter(|s| last_seen.is_none_or(|l| s.version > *l) && s.version <= *current)
        .collect();
    v.sort_by_key(|a| std::cmp::Reverse(a.version));
    v
}

/// The text of one version's section (header excluded), for the release notes.
pub fn notes_for(text: &str, version: &Version) -> Option<String> {
    let s = sections(text).into_iter().find(|s| s.version == *version)?;
    if s.bullets.is_empty() {
        return None;
    }
    Some(
        s.bullets
            .iter()
            .map(|b| format!("- {b}"))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = "# Changelog\n\nIntro text.\n\n## Unreleased\n\n- not yet\n\n## 0.1.2 – 2026-11-20\n\n- Third change\n- Fourth change that\n  wraps onto a second line\n\n## 0.1.1 - 2026-11-10\n\n* Second\n\n## 0.1.0 – 2026-11-07\n\n- First\n- Also first\n";

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn parses_sections_and_ignores_unreleased() {
        let s = sections(LOG);
        let versions: Vec<_> = s.iter().map(|s| s.version.to_string()).collect();
        assert_eq!(versions, ["0.1.2", "0.1.1", "0.1.0"]);
        assert_eq!(s[0].date.as_deref(), Some("2026-11-20"));
        assert_eq!(s[1].date.as_deref(), Some("2026-11-10"), "plain hyphen");
        assert_eq!(s[0].bullets.len(), 2);
        assert_eq!(
            s[0].bullets[1],
            "Fourth change that wraps onto a second line"
        );
        assert_eq!(s[1].bullets, ["Second"], "asterisk bullets");
    }

    #[test]
    fn since_is_exclusive_of_last_seen_and_inclusive_of_current() {
        let got = since(LOG, Some(&v("0.1.0")), &v("0.1.2"));
        let versions: Vec<_> = got.iter().map(|s| s.version.to_string()).collect();
        assert_eq!(
            versions,
            ["0.1.2", "0.1.1"],
            "newest first, skipped versions included"
        );

        let got = since(LOG, Some(&v("0.1.1")), &v("0.1.1"));
        assert!(got.is_empty());

        // A section newer than the running version is not shown.
        let got = since(LOG, Some(&v("0.1.0")), &v("0.1.1"));
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].version, v("0.1.1"));

        // No last-seen version: everything up to current.
        assert_eq!(since(LOG, None, &v("0.1.1")).len(), 2);
    }

    #[test]
    fn handles_messy_input() {
        assert!(sections("").is_empty());
        assert!(sections("## Unreleased\n- x\n").is_empty());
        let s = sections("## 1.0.0\n- only\n## v1.1.0 — 2027-01-01\n- dash\n");
        assert_eq!(s[0].date, None);
        assert_eq!(s[1].version, v("1.1.0"));
        assert_eq!(s[1].date.as_deref(), Some("2027-01-01"));
        // Order in the file does not matter.
        let got = since(
            "## 0.1.0\n- a\n## 0.3.0\n- c\n## 0.2.0\n- b\n",
            None,
            &v("0.3.0"),
        );
        assert_eq!(got[0].version, v("0.3.0"));
        assert_eq!(got[2].version, v("0.1.0"));
    }

    #[test]
    fn release_notes_text() {
        assert_eq!(
            notes_for(LOG, &v("0.1.0")).unwrap(),
            "- First\n- Also first"
        );
        assert_eq!(notes_for(LOG, &v("9.9.9")), None);
    }

    #[test]
    fn the_shipped_changelog_parses() {
        let text = include_str!("../../../CHANGELOG.md");
        let s = sections(text);
        assert!(!s.is_empty());
        for sec in &s {
            assert!(
                (3..=6).contains(&sec.bullets.len()),
                "{}: 3-6 bullets",
                sec.version
            );
        }
    }
}

//! The time zone the analysis sees local time in. The app passes [`Zone::Local`]; tests pass a
//! fixed offset or a table of offset changes, so DST days are deterministic.
use chrono::{DateTime, Duration, FixedOffset, LocalResult, NaiveDateTime, Offset, TimeZone, Utc};

#[derive(Debug, Clone, PartialEq)]
pub enum Zone {
    /// The PC's zone (chrono `Local`).
    Local,
    Fixed(FixedOffset),
    /// Offset changes: `(from, offset)` sorted by `from`; the first offset also applies before
    /// its instant. For tests of DST days.
    Changes(Vec<(DateTime<Utc>, FixedOffset)>),
}

fn zero() -> FixedOffset {
    FixedOffset::east_opt(0).expect("zero offset")
}

/// The instant a wall time `t` has at offset `o`.
fn at_offset(t: NaiveDateTime, o: &FixedOffset) -> DateTime<Utc> {
    Utc.from_utc_datetime(&(t - Duration::seconds(o.local_minus_utc() as i64)))
}

impl Zone {
    /// UTC (offset 0).
    pub fn utc() -> Self {
        Zone::Fixed(zero())
    }

    fn offset_at(changes: &[(DateTime<Utc>, FixedOffset)], t: DateTime<Utc>) -> FixedOffset {
        changes
            .iter()
            .rev()
            .find(|(from, _)| *from <= t)
            .or(changes.first())
            .map_or_else(zero, |(_, o)| *o)
    }

    /// Local wall time of an instant.
    pub fn local(&self, t: DateTime<Utc>) -> NaiveDateTime {
        match self {
            Zone::Local => t.with_timezone(&chrono::Local).naive_local(),
            Zone::Fixed(o) => t.with_timezone(o).naive_local(),
            Zone::Changes(c) => t.with_timezone(&Self::offset_at(c, t)).naive_local(),
        }
    }

    /// The instant of a local wall time. An ambiguous time (clocks going back) takes the earlier
    /// instant; a time that does not exist (clocks going forward) is read with the offset before
    /// the change, which lands the same distance after the jump.
    pub fn utc_of(&self, t: NaiveDateTime) -> DateTime<Utc> {
        match self {
            Zone::Local => match chrono::Local.from_local_datetime(&t) {
                LocalResult::Single(x) => x.with_timezone(&Utc),
                LocalResult::Ambiguous(a, _) => a.with_timezone(&Utc),
                LocalResult::None => {
                    // In the gap: the offset in force a few hours before it.
                    let before = chrono::Local
                        .from_local_datetime(&(t - Duration::hours(3)))
                        .earliest()
                        .map_or_else(zero, |x| x.offset().fix());
                    at_offset(t, &before)
                }
            },
            Zone::Fixed(o) => at_offset(t, o),
            Zone::Changes(c) => {
                let mut valid: Vec<DateTime<Utc>> = c
                    .iter()
                    .map(|(_, o)| at_offset(t, o))
                    .filter(|u| self.local(*u) == t)
                    .collect();
                valid.sort();
                if let Some(u) = valid.first() {
                    return *u;
                }
                // In a gap: read with the offset in force before the change it belongs to.
                for w in c.windows(2) {
                    let (prev, (from, next)) = (w[0].1, w[1]);
                    let jump = (next.local_minus_utc() - prev.local_minus_utc()) as i64;
                    let u = at_offset(t, &prev);
                    if jump > 0 && u >= from && u - from < Duration::seconds(jump) {
                        return u;
                    }
                }
                at_offset(t, &c.first().map_or_else(zero, |x| x.1))
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn h(hours: i32) -> FixedOffset {
        FixedOffset::east_opt(hours * 3600).unwrap()
    }

    /// Central Europe around the spring change: +1 until Mar 28 2027 01:00 UTC, then +2.
    pub(crate) fn spring() -> Zone {
        Zone::Changes(vec![
            ("2027-01-01T00:00:00Z".parse().unwrap(), h(1)),
            ("2027-03-28T01:00:00Z".parse().unwrap(), h(2)),
        ])
    }

    fn naive(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    #[test]
    fn fixed_and_changing_offsets() {
        let z = Zone::Fixed(h(1));
        let t: DateTime<Utc> = "2026-11-07T07:00:00Z".parse().unwrap();
        assert_eq!(z.local(t), naive("2026-11-07 08:00"));
        assert_eq!(z.utc_of(naive("2026-11-07 08:00")), t);

        let z = spring();
        // Before and after the change, 06:00 local is 05:00 and 04:00 UTC.
        assert_eq!(
            z.utc_of(naive("2027-03-27 06:00")),
            "2027-03-27T05:00:00Z".parse::<DateTime<Utc>>().unwrap()
        );
        assert_eq!(
            z.utc_of(naive("2027-03-28 06:00")),
            "2027-03-28T04:00:00Z".parse::<DateTime<Utc>>().unwrap()
        );
        // 02:30 does not exist that night: read with +1, it is 01:30 UTC = 03:30 local.
        let gap = z.utc_of(naive("2027-03-28 02:30"));
        assert_eq!(
            gap,
            "2027-03-28T01:30:00Z".parse::<DateTime<Utc>>().unwrap()
        );
        assert_eq!(z.local(gap), naive("2027-03-28 03:30"));
        assert_eq!(Zone::utc().local(t), t.naive_utc());
    }
}

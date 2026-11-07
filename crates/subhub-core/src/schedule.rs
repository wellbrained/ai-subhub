//! Refresh schedule: failure backoff and provider cooldown (port of C# RefreshSchedule).
use chrono::{DateTime, Duration, Utc};

use crate::model::{UsageResult, UsageState};
use crate::settings::CachedSchedule;

const MAX_FAILURES: u32 = 6;
const MAX_BACKOFF_HOURS: i64 = 24;

#[derive(Debug, Clone, PartialEq)]
pub struct Schedule(pub CachedSchedule);

impl Schedule {
    pub fn new(now: DateTime<Utc>) -> Self {
        Self(CachedSchedule {
            state: UsageState::Unavailable,
            failures: 0,
            next_attempt: now,
        })
    }

    /// From a persisted schedule (clamps failures like the C# app).
    pub fn from_cached(mut cached: CachedSchedule) -> Self {
        cached.failures = cached.failures.min(MAX_FAILURES);
        Self(cached)
    }

    /// Manual retry skips failure backoff but never a provider cooldown (RateLimited).
    pub fn can_attempt(&self, force: bool, now: DateTime<Utc>) -> bool {
        now >= self.0.next_attempt || (force && self.0.state != UsageState::RateLimited)
    }

    /// A provider cooldown is running: manual retry must wait for it.
    pub fn cooling_down(&self, now: DateTime<Utc>) -> bool {
        self.0.state == UsageState::RateLimited && now < self.0.next_attempt
    }

    /// interval * 2^failures (failures capped at 6, delay capped at 24 h); retry_after wins if longer.
    pub fn complete(&mut self, result: &UsageResult, now: DateTime<Utc>, interval_minutes: u32) {
        self.0.state = result.state;
        self.0.failures = if result.state == UsageState::Ready {
            0
        } else {
            (self.0.failures + 1).min(MAX_FAILURES)
        };
        let minutes = i64::from(interval_minutes.clamp(1, 60)) * (1i64 << self.0.failures);
        let mut delay = Duration::minutes(minutes).min(Duration::hours(MAX_BACKOFF_HOURS));
        if let Some(retry) = result.retry_after_secs.and_then(|s| i64::try_from(s).ok()) {
            // Duration::seconds panics beyond its range; a ten-year cap on absurd values is enough.
            let retry = Duration::seconds(retry.min(10 * 365 * 24 * 3600));
            if retry > delay {
                delay = retry;
            }
        }
        self.0.next_attempt = now + delay;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ReadSource;

    fn res(state: UsageState, retry: Option<u64>) -> UsageResult {
        UsageResult {
            state,
            message: String::new(),
            reading: None,
            retry_after_secs: retry,
            email: None,
            plan: None,
            source: ReadSource::Profile,
        }
    }

    fn t0() -> DateTime<Utc> {
        "2026-11-06T10:00:00Z".parse().unwrap()
    }

    #[test]
    fn new_schedule_is_due_now() {
        let s = Schedule::new(t0());
        assert!(s.can_attempt(false, t0()));
        assert_eq!(s.0.state, UsageState::Unavailable);
    }

    #[test]
    fn success_resets_failures_and_waits_one_interval() {
        let mut s = Schedule::new(t0());
        s.complete(&res(UsageState::Unavailable, None), t0(), 5);
        assert_eq!(
            (s.0.failures, s.0.next_attempt),
            (1, t0() + Duration::minutes(10))
        );
        s.complete(&res(UsageState::Ready, None), t0(), 5);
        assert_eq!(
            (s.0.failures, s.0.next_attempt),
            (0, t0() + Duration::minutes(5))
        );
    }

    #[test]
    fn backoff_doubles_and_caps() {
        let mut s = Schedule::new(t0());
        for _ in 0..10 {
            s.complete(&res(UsageState::Unavailable, None), t0(), 60);
        }
        assert_eq!(s.0.failures, 6);
        assert_eq!(s.0.next_attempt, t0() + Duration::hours(24));
    }

    #[test]
    fn retry_after_wins_when_longer_even_beyond_a_day() {
        let mut s = Schedule::new(t0());
        s.complete(&res(UsageState::RateLimited, Some(25 * 60)), t0(), 5);
        assert_eq!(s.0.next_attempt, t0() + Duration::minutes(25));
        s.complete(&res(UsageState::RateLimited, Some(48 * 3600)), t0(), 5);
        assert_eq!(s.0.next_attempt, t0() + Duration::hours(48));
        // Shorter than the backoff: the backoff stays.
        let mut s = Schedule::new(t0());
        s.complete(&res(UsageState::RateLimited, Some(10)), t0(), 5);
        assert_eq!(s.0.next_attempt, t0() + Duration::minutes(10));
    }

    #[test]
    fn force_skips_backoff_but_never_a_provider_cooldown() {
        let mut s = Schedule::new(t0());
        s.complete(&res(UsageState::Unavailable, None), t0(), 5);
        assert!(!s.can_attempt(false, t0()));
        assert!(s.can_attempt(true, t0()));
        s.complete(&res(UsageState::RateLimited, Some(1800)), t0(), 5);
        assert!(!s.can_attempt(true, t0() + Duration::minutes(5)));
        assert!(!s.can_attempt(false, t0() + Duration::minutes(5)));
        assert!(s.can_attempt(true, t0() + Duration::minutes(30)));
    }

    #[test]
    fn cooling_down_only_for_a_running_provider_cooldown() {
        let mut s = Schedule::new(t0());
        assert!(!s.cooling_down(t0()));
        s.complete(&res(UsageState::Unavailable, None), t0(), 5);
        assert!(!s.cooling_down(t0()));
        s.complete(&res(UsageState::RateLimited, Some(1800)), t0(), 5);
        assert!(s.cooling_down(t0() + Duration::minutes(29)));
        assert!(!s.cooling_down(t0() + Duration::minutes(30)));
    }

    #[test]
    fn restored_schedule_clamps_failures_and_keeps_cooldown() {
        let cached = CachedSchedule {
            state: UsageState::RateLimited,
            failures: 99,
            next_attempt: t0() + Duration::hours(1),
        };
        let s = Schedule::from_cached(cached);
        assert_eq!(s.0.failures, 6);
        assert!(!s.can_attempt(true, t0()));
    }
}

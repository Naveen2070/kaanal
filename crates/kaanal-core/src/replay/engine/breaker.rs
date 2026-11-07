//! The circuit breaker state machine (RULES 82-84, step 7c), pure: no engine, no clock of its own.
//!
//! resilience4j's count-based sliding-window breaker (Nygard, *Release It!*, 2007). **Closed** keeps the
//! outcomes of the last `window` calls and opens when, with at least `min_calls` of them, the failure
//! rate is at or above the threshold. **Open** rejects every call until `open_for` has passed; the first
//! call after that moves it to **half-open**, which lets `half_open_calls` calls through and decides over
//! their outcomes: it closes when their failure rate is below the threshold and reopens when it is at or
//! above. Integers only: a rate test is `fails * 1000 >= rate_milli * calls`.
//!
//! One safety valve beyond the library's default: a half-open breaker whose permits were never reported
//! (a probe cancelled with its request) hands them out again after `open_for`, as resilience4j's
//! `maxWaitDurationInHalfOpenState` does, so a lost probe cannot wedge it.

use super::BreakerState;
use crate::config::BreakerConfig;

/// What a breaker says to a call about to start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Decision {
    /// Closed: go ahead.
    Pass,
    /// Half-open: go ahead, this call is one of the probes.
    Probe,
    /// Open (or no probe left): fail the call at once.
    Reject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Closed,
    /// Rejecting until this instant (ns).
    Open {
        until: u64,
    },
    HalfOpen {
        /// When it went half-open (ns), for the safety valve.
        since: u64,
        permitted: u16,
        done: u16,
        fails: u16,
    },
}

/// One breaker: the window and the state. Created closed and empty.
#[derive(Debug, Clone)]
pub(super) struct Breaker {
    /// Outcomes of the last calls, `true` = failed; a ring once it holds `window` of them.
    ring: Vec<bool>,
    head: usize,
    fails: u32,
    state: State,
}

impl Breaker {
    pub(super) fn new() -> Breaker {
        Breaker {
            ring: Vec::new(),
            head: 0,
            fails: 0,
            state: State::Closed,
        }
    }

    /// Whether a call starting at `now` (ns) may go.
    pub(super) fn allow(&mut self, cfg: &BreakerConfig, now: u64) -> Decision {
        match self.state {
            State::Closed => Decision::Pass,
            State::Open { until } if now >= until => {
                self.state = State::HalfOpen {
                    since: now,
                    permitted: 1,
                    done: 0,
                    fails: 0,
                };
                Decision::Probe
            }
            State::Open { .. } => Decision::Reject,
            State::HalfOpen {
                since,
                permitted,
                done,
                fails,
            } => {
                let wait = u64::try_from(cfg.open_for.as_nanos()).unwrap_or(u64::MAX);
                if now.saturating_sub(since) >= wait {
                    // permits were lost: start the probes again
                    self.state = State::HalfOpen {
                        since: now,
                        permitted: 1,
                        done: 0,
                        fails: 0,
                    };
                    Decision::Probe
                } else if permitted < cfg.half_open_calls {
                    self.state = State::HalfOpen {
                        since,
                        permitted: permitted + 1,
                        done,
                        fails,
                    };
                    Decision::Probe
                } else {
                    Decision::Reject
                }
            }
        }
    }

    /// A call that was let through ended at `now` (ns): `ok` false for a counted failure.
    pub(super) fn record(&mut self, cfg: &BreakerConfig, now: u64, ok: bool) {
        match self.state {
            State::Open { .. } => {} // a call that started before it opened: not counted
            State::Closed => {
                let window = usize::from(cfg.window);
                if self.ring.len() < window {
                    self.ring.push(!ok);
                } else {
                    self.fails -= u32::from(self.ring[self.head]);
                    self.ring[self.head] = !ok;
                    self.head = (self.head + 1) % window;
                }
                self.fails += u32::from(!ok);
                let calls = self.ring.len() as u64;
                if calls >= u64::from(cfg.min_effective())
                    && u64::from(self.fails) * 1000 >= u64::from(cfg.rate_milli()) * calls
                {
                    self.open(cfg, now);
                }
            }
            State::HalfOpen {
                since,
                permitted,
                done,
                fails,
            } => {
                let (done, fails) = (done + 1, fails + u16::from(!ok));
                if done < cfg.half_open_calls {
                    self.state = State::HalfOpen {
                        since,
                        permitted,
                        done,
                        fails,
                    };
                } else if u64::from(fails) * 1000 >= u64::from(cfg.rate_milli()) * u64::from(done) {
                    self.open(cfg, now);
                } else {
                    self.ring.clear();
                    (self.head, self.fails) = (0, 0);
                    self.state = State::Closed;
                }
            }
        }
    }

    fn open(&mut self, cfg: &BreakerConfig, now: u64) {
        self.ring.clear();
        (self.head, self.fails) = (0, 0);
        let wait = u64::try_from(cfg.open_for.as_nanos()).unwrap_or(u64::MAX);
        self.state = State::Open {
            until: now.saturating_add(wait),
        };
    }

    /// What the breaker is doing now.
    pub(super) fn phase(&self) -> BreakerState {
        match self.state {
            State::Closed => BreakerState::Closed,
            State::Open { .. } => BreakerState::Open,
            State::HalfOpen { .. } => BreakerState::HalfOpen,
        }
    }

    /// Whether the breaker is open (rejecting), for tests.
    #[cfg(test)]
    fn is_open(&self) -> bool {
        matches!(self.state, State::Open { .. })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    const S: u64 = 1_000_000_000;

    fn cfg(window: u16, min: u16, rate: f64, open_s: u64, probes: u16) -> BreakerConfig {
        BreakerConfig {
            window,
            min_calls: min,
            failure_rate: rate,
            open_for: Duration::from_secs(open_s),
            half_open_calls: probes,
        }
    }

    /// A call that passes and ends at once with `ok`.
    fn call(b: &mut Breaker, c: &BreakerConfig, now: u64, ok: bool) -> Decision {
        let d = b.allow(c, now);
        if d != Decision::Reject {
            b.record(c, now, ok);
        }
        d
    }

    #[test]
    fn it_does_not_open_below_min_calls() {
        let c = cfg(10, 10, 0.5, 60, 4);
        let mut b = Breaker::new();
        for _ in 0..9 {
            assert_eq!(call(&mut b, &c, 0, false), Decision::Pass);
        }
        assert!(!b.is_open(), "9 failures in 9 calls is below min_calls 10");
        assert_eq!(call(&mut b, &c, 0, false), Decision::Pass);
        assert!(b.is_open(), "the 10th call reaches min_calls at 100%");
    }

    #[test]
    fn it_opens_exactly_at_the_threshold() {
        let c = cfg(10, 10, 0.5, 60, 4);
        // 4 failures of 10: 40% < 50%, stays closed
        let mut b = Breaker::new();
        for i in 0..10 {
            call(&mut b, &c, 0, i >= 4);
        }
        assert!(!b.is_open());
        // 5 of 10: 50% is at the threshold, opens
        let mut b = Breaker::new();
        for i in 0..10 {
            call(&mut b, &c, 0, i >= 5);
        }
        assert!(b.is_open());
    }

    #[test]
    fn the_window_slides_so_old_failures_leave() {
        let c = cfg(10, 10, 0.5, 60, 4);
        let mut b = Breaker::new();
        // 4 failures first, then successes: the window holds 4 fails + 6 oks (40%)
        for i in 0..10 {
            call(&mut b, &c, 0, i >= 4);
        }
        // each further success pushes an old failure out; it can never reach 50%
        for _ in 0..30 {
            call(&mut b, &c, 0, true);
        }
        assert!(!b.is_open());
        // 5 failures in a row now: the window is 5 fails + 5 oks at the end, so it opens on the 5th
        for _ in 0..4 {
            call(&mut b, &c, 0, false);
        }
        assert!(!b.is_open());
        call(&mut b, &c, 0, false);
        assert!(b.is_open());
    }

    #[test]
    fn open_rejects_until_open_for_then_one_probe_at_a_time_up_to_the_limit() {
        let c = cfg(4, 4, 0.5, 10, 3);
        let mut b = Breaker::new();
        for _ in 0..4 {
            call(&mut b, &c, 0, false);
        }
        assert!(b.is_open());
        assert_eq!(b.allow(&c, 5 * S), Decision::Reject);
        assert_eq!(b.allow(&c, 10 * S - 1), Decision::Reject);
        // the first call at the instant the wait ends is a probe, and only 3 go through
        assert_eq!(b.allow(&c, 10 * S), Decision::Probe);
        assert_eq!(b.allow(&c, 10 * S), Decision::Probe);
        assert_eq!(b.allow(&c, 10 * S), Decision::Probe);
        assert_eq!(b.allow(&c, 10 * S), Decision::Reject);
    }

    #[test]
    fn half_open_closes_below_the_threshold_and_reopens_at_it() {
        let c = cfg(4, 4, 0.5, 10, 10);
        // 4 failed probes of 10 (40%) close it
        let mut b = Breaker::new();
        for _ in 0..4 {
            call(&mut b, &c, 0, false);
        }
        for i in 0..10 {
            assert_eq!(call(&mut b, &c, 10 * S, i >= 4), Decision::Probe);
        }
        assert!(!b.is_open());
        assert_eq!(b.allow(&c, 10 * S), Decision::Pass, "closed again");
        // 5 failed probes of 10 (50%) reopen it, for a full open_for from the last probe
        let mut b = Breaker::new();
        for _ in 0..4 {
            call(&mut b, &c, 0, false);
        }
        for i in 0..10 {
            call(&mut b, &c, 10 * S, i >= 5);
        }
        assert!(b.is_open());
        assert_eq!(b.allow(&c, 19 * S), Decision::Reject);
        assert_eq!(b.allow(&c, 20 * S), Decision::Probe);
    }

    #[test]
    fn a_closed_breaker_starts_with_an_empty_window() {
        // after closing, old failures do not count against it
        let c = cfg(4, 4, 0.5, 10, 2);
        let mut b = Breaker::new();
        for _ in 0..4 {
            call(&mut b, &c, 0, false);
        }
        call(&mut b, &c, 10 * S, true);
        call(&mut b, &c, 10 * S, true);
        assert!(!b.is_open());
        for _ in 0..3 {
            call(&mut b, &c, 10 * S, false);
        }
        assert!(!b.is_open(), "3 calls are below min_calls 4");
    }

    #[test]
    fn lost_probes_are_handed_out_again_after_open_for() {
        let c = cfg(4, 4, 0.5, 10, 2);
        let mut b = Breaker::new();
        for _ in 0..4 {
            call(&mut b, &c, 0, false);
        }
        // two probes start and are never reported
        assert_eq!(b.allow(&c, 10 * S), Decision::Probe);
        assert_eq!(b.allow(&c, 10 * S), Decision::Probe);
        assert_eq!(b.allow(&c, 12 * S), Decision::Reject);
        assert_eq!(
            b.allow(&c, 20 * S),
            Decision::Probe,
            "a new round of probes"
        );
    }

    #[test]
    fn min_calls_above_the_window_counts_as_the_window() {
        let c = cfg(5, 100, 0.5, 60, 2);
        let mut b = Breaker::new();
        for _ in 0..5 {
            call(&mut b, &c, 0, false);
        }
        assert!(b.is_open());
    }
}

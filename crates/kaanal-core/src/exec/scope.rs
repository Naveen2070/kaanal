//! The zero-load view of retry scopes (RULES 82-84, step 7a): which attempts of a [`PlanItem::Retry`]
//! count, how long a failed attempt really ran, and which deadlines still decide the request.
//!
//! The sampler pre-samples **every** attempt of a retried call, because whether attempt `k` fails can
//! depend on load the sampler cannot see (a full queue, a rate limit, a deadline in the replay engine).
//! At zero load the story is simpler: attempts run in order until one succeeds or the failure is not
//! transient. The sampler writes down each failed attempt's outcome ([`ScopedFail`]); this module reads
//! it back. An attempt with no entry succeeded, so **reached attempts are those up to and including
//! the first one without an entry**; the rest never ran. A failed attempt ends at its failure instant:
//! nothing in it runs past that (E14 inside the scope).

use std::ops::Range;
use std::time::Duration;

use super::PlanItem;

/// How a failed attempt ended at zero load: the sampler's decision, kept so `execute` and the live
/// mocks need not rediscover it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScopedFail {
    /// Plan index of the [`PlanItem::Attempt`] marker that failed.
    pub attempt: usize,
    /// Plan index of what decided it: a [`PlanItem::Fail`], or a [`PlanItem::Span`] whose deadline passed.
    pub item: usize,
    /// Zero-load instant of the failure (offset from the request's start).
    pub at: Duration,
}

/// **Private.** The attempts of the retry scope at `retry` (`size` items after it), in order: marker
/// index and the items of the attempt.
fn attempts(
    items: &[PlanItem],
    retry: usize,
    size: usize,
) -> impl Iterator<Item = (usize, Range<usize>)> + '_ {
    let end = retry + 1 + size;
    let mut m = retry + 1;
    std::iter::from_fn(move || {
        if m >= end {
            return None;
        }
        let PlanItem::Attempt { size, .. } = items[m] else {
            return None;
        };
        let out = (m, m + 1..m + 1 + size);
        m += 1 + size;
        Some(out)
    })
}

/// **Private.** `scoped`'s entry for the attempt marked at `marker`.
fn failed(scoped: &[ScopedFail], marker: usize) -> Option<&ScopedFail> {
    scoped.iter().find(|s| s.attempt == marker)
}

/// What the retry scopes of a plan do at zero load: plan ranges that never ran, and ranges cut at the
/// instant their attempt failed.
#[derive(Debug, Default)]
pub(super) struct Ranges {
    /// Attempts after the one that settled the call.
    pub(super) unreached: Vec<Range<usize>>,
    /// Attempts that failed, with the instant everything in them stops.
    pub(super) cut: Vec<(Range<usize>, Duration)>,
}

impl Ranges {
    /// Whether the plan has no retry scope at all (the common case: nothing to do).
    pub(super) fn is_empty(&self) -> bool {
        self.unreached.is_empty() && self.cut.is_empty()
    }
}

/// Every unreached and failed attempt in `items`, scopes inside unreached attempts included.
pub(super) fn ranges(items: &[PlanItem], scoped: &[ScopedFail]) -> Ranges {
    let mut out = Ranges::default();
    for (i, item) in items.iter().enumerate() {
        let PlanItem::Retry { size, .. } = *item else {
            continue;
        };
        let mut settled = false;
        for (m, r) in attempts(items, i, size) {
            if settled {
                out.unreached.push(r);
            } else if let Some(f) = failed(scoped, m) {
                out.cut.push((r, f.at));
            } else {
                settled = true;
            }
        }
    }
    out
}

/// The latest end of any hop in `r` that ran, as an offset: unreached attempts do not count, a failed
/// attempt counts up to its failure instant. `None` when nothing ran.
pub(super) fn end_in(
    items: &[PlanItem],
    scoped: &[ScopedFail],
    r: Range<usize>,
) -> Option<Duration> {
    let mut best: Option<Duration> = None;
    let mut i = r.start;
    while i < r.end {
        match &items[i] {
            PlanItem::Hop(h) => best = best.max(Some(h.start + h.dur)),
            PlanItem::Retry { size, .. } => {
                let mut settled = false;
                for (m, a) in attempts(items, i, *size) {
                    if settled {
                        break;
                    }
                    let mut e = end_in(items, scoped, a);
                    if let Some(f) = failed(scoped, m) {
                        e = e.map(|e| e.min(f.at));
                    } else {
                        settled = true;
                    }
                    best = best.max(e);
                }
                i += *size;
            }
            _ => {}
        }
        i += 1;
    }
    best
}

/// The earliest span in `r` whose body ends after its deadline and whose deadline falls before
/// `planned` (a tie goes to the planned failure, Z13): plan index and instant. Spans inside a retry
/// scope are skipped: each attempt decided its own.
pub(super) fn deadline_in(
    items: &[PlanItem],
    scoped: &[ScopedFail],
    r: Range<usize>,
    planned: Option<Duration>,
) -> Option<(usize, Duration)> {
    let (mut best, mut out) = (planned, None);
    let mut i = r.start;
    while i < r.end {
        match &items[i] {
            PlanItem::Retry { size, .. } => i += *size,
            PlanItem::Span {
                timeout, at, size, ..
            } => {
                let end = end_in(items, scoped, i + 1..i + 1 + *size).unwrap_or(*at);
                let due = *at + *timeout;
                if end > due && best.is_none_or(|b| due < b) {
                    best = Some(due);
                    out = Some((i, due));
                }
            }
            _ => {}
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::{Hop, HopKind};

    fn hop(start: u64, dur: u64) -> PlanItem {
        PlanItem::Hop(Hop {
            node: 0,
            replica: 0,
            kind: HopKind::Proc,
            start: Duration::from_millis(start),
            dur: Duration::from_millis(dur),
        })
    }

    /// `Retry`, then three attempts of one hop each: [0] Retry, [1] Attempt, [2] hop, [3] Attempt,
    /// [4] hop, [5] Attempt, [6] hop.
    fn three() -> Vec<PlanItem> {
        let a = |size| PlanItem::Attempt {
            backoff: Duration::ZERO,
            size,
        };
        vec![
            PlanItem::Retry {
                attempts: 3,
                size: 6,
                budget: None,
                breaker: None,
                edge: 0,
            },
            a(1),
            hop(0, 10),
            a(1),
            hop(100, 10),
            a(1),
            hop(200, 10),
        ]
    }

    fn fail(attempt: usize, at: u64) -> ScopedFail {
        ScopedFail {
            attempt,
            item: attempt + 1,
            at: Duration::from_millis(at),
        }
    }

    #[test]
    fn attempts_after_the_first_success_never_ran_and_failed_ones_stop_at_their_failure() {
        let items = three();
        // attempt 0 failed at 4 ms, attempt 1 succeeded: attempt 2 never ran
        let scoped = [fail(1, 4)];
        let r = ranges(&items, &scoped);
        assert_eq!(r.unreached.len(), 1);
        assert_eq!(r.unreached[0], 6..7);
        assert_eq!(r.cut, [(2..3, Duration::from_millis(4))]);
        // the end: attempt 0 counts up to 4 ms, attempt 1's hop ends at 110 ms
        assert_eq!(
            end_in(&items, &scoped, 0..7),
            Some(Duration::from_millis(110))
        );
        // every attempt failed: all three count, each up to its own instant
        let all = [fail(1, 4), fail(3, 104), fail(5, 204)];
        let r = ranges(&items, &all);
        assert_eq!(r.unreached.len(), 0);
        assert_eq!(r.cut.len(), 3);
        assert_eq!(end_in(&items, &all, 0..7), Some(Duration::from_millis(204)));
        // no failure at all: attempt 0 settled the call, the other two never ran
        assert_eq!(ranges(&items, &[]).unreached, [4..5, 6..7]);
        assert_eq!(end_in(&items, &[], 0..7), Some(Duration::from_millis(10)));
    }

    #[test]
    fn a_span_around_a_retry_sees_only_the_attempts_that_ran() {
        // Span (due 50 ms) around the scope: with attempt 0 settling the call it ends at 10 ms
        let mut items = vec![PlanItem::Span {
            node: 0,
            timeout: Duration::from_millis(50),
            at: Duration::ZERO,
            size: 7,
            orphan: false,
        }];
        items.extend(three());
        assert_eq!(deadline_in(&items, &[], 0..8, None), None);
        // all three failed, the last at 204 ms: the span's deadline (50 ms) passed first
        let all = [fail(2, 4), fail(4, 104), fail(6, 204)];
        assert_eq!(
            deadline_in(&items, &all, 0..8, None),
            Some((0, Duration::from_millis(50)))
        );
        // a planned failure before the deadline wins the tie-break
        assert_eq!(
            deadline_in(&items, &all, 0..8, Some(Duration::from_millis(50))),
            None
        );
    }
}

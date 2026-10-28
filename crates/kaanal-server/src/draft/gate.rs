//! The admission gate: how Apply stops mock traffic for the length of a drain.
//!
//! Every mock request passes [`Gate::enter`] and holds the result until its answer is built.
//! Apply [`pause`](Gate::pause)s the gate, waits until nothing is [`drained`](Gate::drained),
//! swaps the system and lets the guard drop, which resumes it. A request that arrives while the
//! gate is paused waits and then runs on the new system; one already inside finishes on the old.
//!
//! The order matters: `enter` counts itself in first and looks at the flag second, and Apply
//! sets the flag first and reads the count second, so a request is either seen by the drain or
//! sees the pause. Control-API requests never pass through here.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
use std::time::Duration;

use tokio::sync::Notify;
use tokio::time::{Instant, timeout_at};

/// Counts the mock requests in flight and can hold new ones back.
#[derive(Default)]
pub struct Gate {
    paused: AtomicBool,
    inflight: AtomicUsize,
    /// Woken when a request leaves or steps back: what a drain waits on.
    changed: Notify,
    /// Woken when the gate resumes: what a held-back request waits on. Separate from `changed`
    /// so two waiting requests stepping back cannot wake each other in a loop.
    resumed: Notify,
}

/// A request inside the gate. Leaving (drop) lets a drain finish.
pub struct Entered<'a>(&'a Gate);

impl Drop for Entered<'_> {
    fn drop(&mut self) {
        self.0.inflight.fetch_sub(1, SeqCst);
        self.0.changed.notify_waiters();
    }
}

/// The gate held shut. Dropping it resumes admission, on every path out of an Apply.
pub struct Paused<'a>(&'a Gate);

impl Drop for Paused<'_> {
    fn drop(&mut self) {
        self.0.paused.store(false, SeqCst);
        self.0.resumed.notify_waiters();
    }
}

impl Gate {
    /// Waits while the gate is paused, then counts the request in.
    pub async fn enter(&self) -> Entered<'_> {
        loop {
            self.inflight.fetch_add(1, SeqCst);
            if !self.paused.load(SeqCst) {
                return Entered(self);
            }
            // Paused: step back out, so a request that is only waiting never holds the drain.
            self.inflight.fetch_sub(1, SeqCst);
            self.changed.notify_waiters();
            // Created after our own wake-up and before the re-check, so neither a resume nor a
            // pause in between can be missed.
            let resumed = self.resumed.notified();
            if self.paused.load(SeqCst) {
                resumed.await;
            }
        }
    }

    /// Holds new requests back until the returned guard drops.
    pub fn pause(&self) -> Paused<'_> {
        self.paused.store(true, SeqCst);
        Paused(self)
    }

    /// Waits until no request is inside, for at most `timeout`. True when it drained.
    pub async fn drained(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            let changed = self.changed.notified();
            if self.inflight.load(SeqCst) == 0 {
                return true;
            }
            if timeout_at(deadline, changed).await.is_err() {
                return self.inflight.load(SeqCst) == 0;
            }
        }
    }

    /// How many requests are inside.
    #[cfg(test)]
    pub fn inflight(&self) -> usize {
        self.inflight.load(SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    const SHORT: Duration = Duration::from_millis(60);

    #[tokio::test]
    async fn enter_waits_while_paused_and_goes_on_after_the_guard_drops() {
        let g = Arc::new(Gate::default());
        let hold = g.pause();
        let g2 = Arc::clone(&g);
        let task = tokio::spawn(async move {
            let _in = g2.enter().await;
        });
        tokio::time::sleep(SHORT).await;
        assert!(!task.is_finished(), "a paused gate holds the request back");
        assert_eq!(g.inflight(), 0, "a request that waits is not in flight");
        drop(hold);
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("resumed")
            .unwrap();
        assert_eq!(g.inflight(), 0);
    }

    #[tokio::test]
    async fn drained_waits_for_the_last_request_and_times_out_while_one_stays() {
        let g = Arc::new(Gate::default());
        let inside = g.enter().await;
        let _hold = g.pause();
        assert!(!g.drained(SHORT).await, "one request is still inside");
        let g2 = Arc::clone(&g);
        let waiter = tokio::spawn(async move { g2.drained(Duration::from_secs(5)).await });
        tokio::time::sleep(SHORT).await;
        drop(inside);
        assert!(waiter.await.unwrap(), "drained once the request left");
    }

    #[tokio::test]
    async fn waiting_requests_do_not_wake_each_other_while_paused() {
        let g = Arc::new(Gate::default());
        let hold = g.pause();
        let tasks: Vec<_> = (0..4)
            .map(|_| {
                let g = Arc::clone(&g);
                tokio::spawn(async move {
                    let _in = g.enter().await;
                })
            })
            .collect();
        tokio::time::sleep(SHORT).await;
        // a livelock would keep stepping in and out: count how often a drain is disturbed
        let changed = g.changed.notified();
        assert!(
            tokio::time::timeout(SHORT, changed).await.is_err(),
            "idle waiters must not keep notifying the drain"
        );
        drop(hold);
        for t in tasks {
            tokio::time::timeout(Duration::from_secs(2), t)
                .await
                .expect("resumed")
                .unwrap();
        }
    }

    #[tokio::test]
    async fn a_waiting_request_does_not_hold_the_drain() {
        let g = Arc::new(Gate::default());
        let _hold = g.pause();
        let g2 = Arc::clone(&g);
        let _waiting = tokio::spawn(async move {
            let _in = g2.enter().await;
        });
        tokio::time::sleep(SHORT).await;
        assert!(
            g.drained(SHORT).await,
            "nobody is inside, someone is only waiting"
        );
    }
}

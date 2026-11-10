//! Changing a running engine in place (step 8L, RULES 105 s): a parameter edit while traffic flows
//! must not reset what is in the engine, so the picture does not jump. [`Engine::reconfigure`]
//! compares the new topology with the one the engine runs and either applies the parameter changes
//! to the live state (slots and queue limits, rate limits, cores, node cards and wires, breaker
//! settings) or refuses a change of structure (the components or the wires), which the caller
//! answers with a new engine.
//!
//! What each change does to work already in the engine: a lower slot limit never preempts (the
//! slots close as running work finishes), a higher one starts waiters at once, a lower queue limit
//! refuses new arrivals and evicts nobody; cores and pipe rates re-time running work from the
//! instant of the change, exactly (the solvers settle first); a removed rate limit or bandwidth
//! becomes a limit nothing can reach, so a transfer or waiter already counted is not stranded.
//! Everything that lives in a request's plan (service times, chaos, timeouts, retries, balance
//! picks, copy counts) applies to requests that arrive after the change: the caller samples their
//! plans from the new topology.

use std::fmt;

use super::replicas::admission_of;
use super::{Bucket, Engine, EngineError};
use crate::topology::{Node, Topology};

/// The rate of a pipe that has lost its limit: far above anything a run can fill, so a transfer
/// already on it keeps moving.
const UNLIMITED_PIPE: u64 = 1 << 50;

/// Why a change could not be applied in place: it changes the shape of the system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Structural(pub String);

impl fmt::Display for Structural {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Structural {}

/// What differs in shape between two topologies, in words, or `None` when only parameters differ.
fn structure_differs(old: &Topology, new: &Topology) -> Option<String> {
    if old.nodes.len() != new.nodes.len() {
        return Some("the set of components changed".into());
    }
    for (a, b) in old.nodes.iter().zip(&new.nodes) {
        if a.name != b.name {
            return Some(format!("component {} was renamed or replaced", a.name));
        }
        if a.kind != b.kind {
            return Some(format!("component {} changed kind", a.name));
        }
    }
    if old.edges.len() != new.edges.len() {
        return Some("the set of wires changed".into());
    }
    for (a, b) in old.edges.iter().zip(&new.edges) {
        if a.from != b.from || a.to != b.to {
            return Some("a wire was rewired".into());
        }
    }
    None
}

impl Engine {
    /// Requests running (slots in use) and waiting for a slot, over all copies of `node`.
    #[must_use]
    pub fn node_load(&self, node: usize) -> (u32, usize) {
        let first = self.units.first[node];
        (0..usize::from(self.units.count[node])).fold((0, 0), |(busy, queued), r| {
            self.units.states[first + r]
                .gate
                .as_ref()
                .map_or((busy, queued), |g| (busy + g.busy(), queued + g.queued()))
        })
    }

    /// `(running, waiting)` of every node, in node order.
    #[must_use]
    pub fn node_loads(&self) -> Vec<(u32, usize)> {
        (0..self.units.nodes).map(|n| self.node_load(n)).collect()
    }

    /// Applies the parameter changes of `topo` to this live engine at `now` (ns), keeping queues,
    /// running work and breaker state. Events due before `now` run first. A change of structure is
    /// refused and nothing is touched.
    ///
    /// # Errors
    /// [`Structural`] for a change of the components or wires, an engine that was not built with
    /// [`Engine::live`], or a clock that overflowed.
    pub fn reconfigure(&mut self, now: u64, topo: &Topology) -> Result<(), Structural> {
        let Some(old) = self.live_topo.take() else {
            return Err(Structural(
                "this engine was not built for a live session".into(),
            ));
        };
        if let Some(why) = structure_differs(&old, topo) {
            self.live_topo = Some(old);
            return Err(Structural(why));
        }
        if let Err(EngineError::ClockOverflow) = self.advance_to(now) {
            self.live_topo = Some(old);
            return Err(Structural("the engine clock overflowed".into()));
        }
        self.now = self.now.max(now);
        let now = self.now;
        for (n, (a, b)) in old.nodes.iter().zip(&topo.nodes).enumerate() {
            self.reconfigure_node(n, a, b, now);
        }
        for (e, (a, b)) in old.edges.iter().zip(&topo.edges).enumerate() {
            if a.bandwidth != b.bandwidth {
                let unit = self.units.at(self.units.nodes + e, 0);
                self.pipe_set(unit, b.bandwidth.unwrap_or(UNLIMITED_PIPE));
            }
            if a.breaker != b.breaker {
                self.breaker_cfg[e] = b.breaker.map(|cfg| (cfg, b.to));
                // a changed breaker starts with a clean window
                self.breakers
                    .retain(|k, _| usize::try_from(k >> 16).ok() != Some(e));
            }
        }
        self.live_topo = Some(Box::new(topo.clone()));
        Ok(())
    }

    /// **Private.** One component's parameter changes, on every copy it has room for.
    fn reconfigure_node(&mut self, n: usize, old: &Node, new: &Node, now: u64) {
        let (first, count) = (self.units.first[n], self.units.count[n]);
        if admission_of(old) != admission_of(new) {
            // the busy area up to now is booked at the old limit, then the waiters the new one
            // lets in start
            self.mark(n, now);
            let adm = admission_of(new);
            let mut started = Vec::new();
            for r in 0..usize::from(count) {
                if let Some(g) = self.units.states[first + r].gate.as_mut() {
                    started.extend(g.set_limits(adm.slots, adm.queue, adm.overflow_status));
                }
            }
            for (qr, qk) in started {
                self.start_queued(qr, qk);
            }
        }
        if old.memory != new.memory {
            for r in 0..usize::from(count) {
                self.units.states[first + r].gc = super::state::Gc::of(new.memory.as_ref());
            }
            self.has.gc = self.has.gc || super::state::Gc::of(new.memory.as_ref()).is_some();
        }
        if old.oom_restart != new.oom_restart {
            for r in 0..usize::from(count) {
                self.units.states[first + r].oom = new.oom_restart.map(super::state::nanos);
            }
        }
        if old.rate_limit != new.rate_limit {
            for r in 0..usize::from(count) {
                let ns = &mut self.units.states[first + r];
                ns.bucket = match (new.rate_limit.as_ref(), ns.bucket.take()) {
                    (None, _) => None,
                    (Some(rl), Some(mut b)) => {
                        b.set_limit(rl);
                        Some(b)
                    }
                    (Some(rl), None) => {
                        Some(Bucket::new(rl).with_status(new.kind.rate_limit_status()))
                    }
                };
            }
        }
        if old.cpu != new.cpu {
            for r in 0..count {
                self.cpu_set(n, r, new.cpu);
            }
        }
        if old.bandwidth != new.bandwidth {
            for (side, rate) in [new.bandwidth.0, new.bandwidth.1].into_iter().enumerate() {
                for r in 0..count {
                    let unit = self
                        .units
                        .at(self.units.nodes + self.units.edges + 2 * n + side, r);
                    self.pipe_set(unit, rate.unwrap_or(UNLIMITED_PIPE));
                }
            }
        }
    }
}

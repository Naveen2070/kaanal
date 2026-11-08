//! Processor sharing of a node copy's own work (RULES 101, step 8): a pure solver the engine drives.
//! A copy has `cap` millicores; `n` hops running own work each get speed `min(1, cap / (1000 n))`
//! of their drawn time (egalitarian PS with `c` servers; Kleinrock, *Queueing Systems* vol. 1,
//! section 4.4, and the `min(n, c) x mu` birth-death chain for `c > 1`). A hop's drawn time is its
//! CPU demand; its wall time is that demand stretched by the share it gets.
//!
//! Every running hop has the same speed, so the solver keeps one virtual clock `V` (full-speed
//! nanoseconds of work each running hop has received since the copy was last idle-free, advanced
//! by `dt x speed`) and gives each hop a finish tag `V(join) + demand`; a hop is done when `V`
//! reaches its tag (generalised processor sharing, Parekh and Gallager 1993, with equal weights).
//! The tags sit in a min-heap: a join, a leave and the next end are O(log n) however many hops run,
//! so a node pushed past saturation (n grows without bound) stays cheap.
//!
//! Time is integer nanoseconds, no float drift, the same inputs give the same answer. Rounding:
//! the work done in `dt` is floored and the next end is rounded up, at most 1 ns per change; when
//! the shares divide evenly the answer is exact. Ties end in join order (a sequence number).

use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Millicores in one core.
const MILLI: u64 = 1000;

/// A running hop: its finish tag, its join order (ties) and who is waiting on it.
type Run = Reverse<(u64, u64, (usize, usize))>;

/// The hops of one node copy that are running their own work now.
#[derive(Debug, Clone)]
pub(super) struct CpuUnit {
    /// Millicores (at least 1).
    cap: u64,
    runs: BinaryHeap<Run>,
    /// Virtual clock: full-speed nanoseconds each running hop has received.
    v: u64,
    /// When `v` was last brought up to date (ns).
    at: u64,
    /// Join counter (the tie-break).
    seq: u64,
}

impl CpuUnit {
    /// A copy with `millicores` of CPU (at least 1).
    pub(super) fn new(millicores: u32) -> CpuUnit {
        CpuUnit {
            cap: u64::from(millicores).max(1),
            runs: BinaryHeap::new(),
            v: 0,
            at: 0,
            seq: 0,
        }
    }

    /// Whether `n` running hops all run at full speed: each has a whole core or more.
    fn full(&self, n: usize) -> bool {
        self.cap >= MILLI * n as u64
    }

    /// Advances the virtual clock to `now` at the speed the current hops ran at since the last change.
    fn settle(&mut self, now: u64) {
        let dt = now.saturating_sub(self.at);
        let n = self.runs.len();
        if dt > 0 && n > 0 {
            // work each hop did in dt: dt at full speed, else dt x cap / (1000 n); fits 128 bits
            self.v += if self.full(n) {
                dt
            } else {
                u64::try_from(u128::from(dt) * u128::from(self.cap) / u128::from(MILLI * n as u64))
                    .unwrap_or(u64::MAX)
            };
        }
        self.at = self.at.max(now);
    }

    /// Own work of `demand` ns starts at `now` for `who`.
    pub(super) fn join(&mut self, now: u64, who: (usize, usize), demand: u64) {
        self.settle(now);
        self.seq += 1;
        self.runs
            .push(Reverse((self.v.saturating_add(demand), self.seq, who)));
    }

    /// Every run that has received all its work by `now` is over: it leaves (pushed to `out` in
    /// finish order, ties in join order) and the rest speed up. A run is never cut short: a failed
    /// request's running hops keep going until they end (`finish.rs`).
    pub(super) fn take_due(&mut self, now: u64, out: &mut Vec<(usize, usize)>) {
        self.settle(now);
        while let Some(&Reverse((tag, _, who))) = self.runs.peek()
            && tag <= self.v
        {
            self.runs.pop();
            out.push(who);
        }
    }

    /// The run that ends first and when (ns), ties by join order; `None` when nothing runs.
    pub(super) fn next_end(&self) -> Option<(u64, (usize, usize))> {
        let &Reverse((tag, _, who)) = self.runs.peek()?;
        let left = tag.saturating_sub(self.v);
        let ns = if self.full(self.runs.len()) {
            u128::from(left)
        } else {
            (u128::from(left) * u128::from(MILLI * self.runs.len() as u64))
                .div_ceil(u128::from(self.cap))
        };
        Some((
            self.at
                .saturating_add(u64::try_from(ns).unwrap_or(u64::MAX)),
            who,
        ))
    }
}

use super::Engine;
use crate::exec::{Hop, HopKind};

impl Engine {
    /// **Private.** Token `k` of request `r` starts own work of `demand` ns at its node's copy. When
    /// that copy has `cpu` the hop joins the sharing solver and its end comes from
    /// [`Engine::cpu_event`]; returns whether it did (otherwise the caller schedules the end as
    /// always). One `bool` check on the path of a topology with no `cpu`.
    pub(super) fn cpu_join(&mut self, r: usize, k: usize, h: &Hop, demand: u64) -> bool {
        if !self.has_cpu || demand == 0 || !matches!(h.kind, HopKind::Proc | HopKind::Op(_)) {
            return false;
        }
        let i = self.units.at(h.node, h.replica);
        let now = self.now;
        let Some(cpu) = self.units.states[i].cpu.as_deref_mut() else {
            return false;
        };
        cpu.join(now, (r, k), demand);
        self.cpu_rearm(i);
        true
    }

    /// **Private.** The pending event of unit `i` fired. It is the current one only if it carries
    /// the time the unit last armed; an older one (a join moved the end earlier) is skipped. Every
    /// hop due now ends, as at the end of any hop, and the unit re-arms for the next.
    pub(super) fn cpu_event(&mut self, i: usize) {
        let now = self.now;
        if self.cpu_due.get(&i) != Some(&now) {
            return;
        }
        self.cpu_due.remove(&i);
        let mut done = std::mem::take(&mut self.cpu_done);
        if let Some(cpu) = self.units.states[i].cpu.as_deref_mut() {
            cpu.take_due(now, &mut done);
        }
        self.cpu_rearm(i);
        for (r, k) in done.drain(..) {
            self.transfer_done(r, k);
        }
        self.cpu_done = done;
    }

    /// **Private.** Makes sure an event is pending at or before the time unit `i`'s next hop ends.
    fn cpu_rearm(&mut self, i: usize) {
        let Some((t, _)) = self.units.states[i]
            .cpu
            .as_deref()
            .and_then(CpuUnit::next_end)
        else {
            return;
        };
        let t = t.max(self.now);
        if self.cpu_due.get(&i).is_some_and(|&d| d <= t) {
            return;
        }
        self.cpu_due.insert(i, t);
        let seq = self.stamp();
        self.heap.push(Reverse((t, seq, usize::MAX - 1, i)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: u64 = 1_000_000;

    /// Runs the unit to idle and returns `(who, end)` in the order they ended.
    fn drain(u: &mut CpuUnit) -> Vec<(usize, u64)> {
        let mut out = Vec::new();
        let mut due = Vec::new();
        while let Some((t, _)) = u.next_end() {
            u.take_due(t, &mut due);
            out.extend(due.drain(..).map(|w| (w.0, t)));
        }
        out
    }

    #[test]
    fn alone_a_hop_runs_at_full_speed() {
        let mut u = CpuUnit::new(4000);
        u.join(0, (0, 0), 10 * MS);
        assert_eq!(drain(&mut u), vec![(0, 10 * MS)]);
    }

    #[test]
    fn two_equal_hops_on_one_core_take_twice_as_long() {
        let mut u = CpuUnit::new(1000);
        u.join(0, (0, 0), 10 * MS);
        u.join(0, (1, 0), 10 * MS);
        assert_eq!(
            drain(&mut u),
            vec![(0, 20 * MS), (1, 20 * MS)],
            "ties end in join order"
        );
    }

    #[test]
    fn the_textbook_two_job_sequence() {
        let mut u = CpuUnit::new(1000);
        u.join(0, (0, 0), 10 * MS);
        u.join(4 * MS, (1, 0), 10 * MS);
        assert_eq!(drain(&mut u), vec![(0, 16 * MS), (1, 20 * MS)]);
    }

    #[test]
    fn half_a_core_halves_the_speed_of_a_lone_hop() {
        let mut u = CpuUnit::new(500);
        u.join(0, (0, 0), 10 * MS);
        assert_eq!(drain(&mut u), vec![(0, 20 * MS)]);
    }

    #[test]
    fn three_hops_on_two_cores_each_run_at_two_thirds() {
        // 2000 millicores, n = 3: speed 2/3, so 12 ms of work ends at 18 ms (exact)
        let mut u = CpuUnit::new(2000);
        for i in 0..3 {
            u.join(0, (i, 0), 12 * MS);
        }
        assert_eq!(
            drain(&mut u),
            vec![(0, 18 * MS), (1, 18 * MS), (2, 18 * MS)]
        );
    }

    #[test]
    fn work_is_conserved_whatever_the_order_of_joins() {
        // one core, three jobs of 6, 9, 15 ms from t=0: PS ends them at 18, 24, 30 ms (the last
        // end equals the total work: a busy single server never idles)
        let mut u = CpuUnit::new(1000);
        for (i, d) in [6, 9, 15].into_iter().enumerate() {
            u.join(0, (i, 0), d * MS);
        }
        assert_eq!(
            drain(&mut u),
            vec![(0, 18 * MS), (1, 24 * MS), (2, 30 * MS)]
        );
    }

    #[test]
    fn the_fast_path_gives_the_same_ends_as_a_full_pass() {
        // 4 cores, 3 hops joining and leaving (always full speed): ends are start + demand
        let mut u = CpuUnit::new(4000);
        u.join(0, (0, 0), 7 * MS);
        u.join(MS, (1, 0), 5 * MS);
        u.join(2 * MS, (2, 0), 9 * MS);
        assert_eq!(drain(&mut u), vec![(1, 6 * MS), (0, 7 * MS), (2, 11 * MS)]);
    }

    #[test]
    fn crossing_the_core_count_slows_the_ones_already_running() {
        // 1 core: A (10 ms) alone for 5 ms, then B (10 ms) joins: A has 5 left at half speed, ends at 15 ms;
        // B has done 5, 5 left alone, ends at 20 ms
        let mut u = CpuUnit::new(1000);
        u.join(0, (0, 0), 10 * MS);
        u.join(5 * MS, (1, 0), 10 * MS);
        assert_eq!(drain(&mut u), vec![(0, 15 * MS), (1, 20 * MS)]);
    }

    #[test]
    fn a_thread_per_request_server_stretches_by_n_over_cores() {
        // 200 hops of 10 ms on 4 cores: all run at 4/200, so each ends at 500 ms
        let mut u = CpuUnit::new(4000);
        for i in 0..200 {
            u.join(0, (i, 0), 10 * MS);
        }
        let ends = drain(&mut u);
        assert!(ends.iter().all(|&(_, t)| t == 500 * MS), "{:?}", &ends[..3]);
    }

    #[test]
    fn a_hundred_thousand_running_hops_cost_log_n_each_and_conserve_work() {
        // 1 core, 100k hops of 1 ms joined at t = 0: all end together at 100 s, the total work (a
        // busy single server never idles). With a pass over every hop per change this would be
        // quadratic; the virtual clock keeps it to a heap operation each.
        let mut u = CpuUnit::new(1000);
        for i in 0..100_000 {
            u.join(0, (i, 0), MS);
        }
        let mut due = Vec::new();
        let (t, _) = u.next_end().unwrap();
        assert_eq!(t, 100_000 * MS);
        u.take_due(t, &mut due);
        assert_eq!(due.len(), 100_000);
        assert!(
            due.windows(2).all(|w| w[0].0 < w[1].0),
            "ties end in join order"
        );
        assert!(u.next_end().is_none());
    }
}

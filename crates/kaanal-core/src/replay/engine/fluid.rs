//! Fair sharing of pipes between concurrent transfers (RULES 62, step 6a): a pure solver the engine
//! drives. Each pipe has a rate (bytes a second); each flow crosses up to three pipes (the wire, the
//! sender's send pipe, the receiver's receive pipe) and gets its max-min fair rate, found by
//! progressive filling and redone whenever a flow joins or leaves. Time is integer nanoseconds and
//! the bytes a flow has left are kept in nano-bytes (a rate in bytes a second times a time in
//! nanoseconds), so there is no float drift and the same inputs give the same answer.
//!
//! Rounding: a share is floored to whole bytes a second (never below 1, so no flow stalls), an error
//! under 1 B/s per share. A flow's end is rounded up to the next nanosecond.
//!
//! Cost: every join or leave costs a pass over the flows and the pipes they use, nothing over the
//! pipes that are idle, and allocates nothing (scratch buffers are kept); the end of each flow is
//! computed once per change, not once per question.

/// Nano-bytes in a byte.
const NANO: u128 = 1_000_000_000;
/// Most pipes one flow crosses: the wire, the sender's send pipe, the receiver's receive pipe.
pub(super) const MAX_PIPES: usize = 3;

/// When a flow with `left` nano-bytes at `rate` bytes a second ends, from `at` (ns). Bodies under
/// 18 GB fit 64 bits, whose division is several times quicker.
fn end_of(left: u128, rate: u64, at: u64) -> u64 {
    let ns = match u64::try_from(left) {
        Ok(left) => u128::from(left.div_ceil(rate)),
        Err(_) => left.div_ceil(u128::from(rate)),
    };
    at.saturating_add(u64::try_from(ns).unwrap_or(u64::MAX))
}

/// A flow in the solver, in the order it joined (ties end in this order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct FlowId(usize);

#[derive(Debug, Clone)]
struct Flow {
    /// Nano-bytes still to move.
    left: u128,
    pipes: [usize; MAX_PIPES],
    n: usize,
    /// A ceiling on its rate, bytes a second (`u64::MAX`: none).
    limit: u64,
    /// Current fair rate, bytes a second (at least 1).
    rate: u64,
    /// When it ends at the current rate (ns), worked out when the rates last changed.
    end: u64,
}

/// The pipes and the flows crossing them.
#[derive(Debug, Clone)]
pub(super) struct Fluid {
    /// Bytes a second of each pipe.
    caps: Vec<u64>,
    /// A slab: a finished flow leaves a hole that the next one reuses.
    flows: Vec<Option<Flow>>,
    free: Vec<usize>,
    /// When the bytes left were last brought up to date (ns).
    at: u64,
    /// Nano-bytes each pipe carried inside the window `[lo, hi]`.
    moved: Vec<u128>,
    lo: u64,
    hi: u64,
    /// Flows on each pipe now.
    load: Vec<usize>,
    // scratch for `share`, all zero or empty between calls
    used: Vec<u64>,
    count: Vec<usize>,
    active: Vec<usize>,
    frozen: Vec<bool>,
}

impl Fluid {
    /// Pipes with these rates (each at least 1 byte a second).
    pub(super) fn new(caps: Vec<u64>) -> Fluid {
        let n = caps.len();
        Fluid {
            caps: caps.into_iter().map(|c| c.max(1)).collect(),
            flows: Vec::new(),
            free: Vec::new(),
            at: 0,
            moved: vec![0; n],
            lo: 0,
            hi: u64::MAX,
            load: vec![0; n],
            used: vec![0; n],
            count: vec![0; n],
            active: Vec::new(),
            frozen: Vec::new(),
        }
    }

    /// Counts what pipes carry only inside `[lo, hi]` (the reporting window).
    pub(super) fn set_window(&mut self, lo: u64, hi: u64) {
        self.lo = lo;
        self.hi = hi;
    }

    /// Nano-bytes pipe `p` carried inside the window up to the last change.
    pub(super) fn moved(&self, p: usize) -> u128 {
        self.moved.get(p).copied().unwrap_or(0)
    }

    /// Changes pipe `pipe`'s rate to `bytes_per_s` at `now` (a live parameter edit): the bytes moved
    /// so far are settled at the old rates, then the flows crossing the pipe are shared out again.
    pub(super) fn set_cap(&mut self, now: u64, pipe: usize, bytes_per_s: u64) {
        self.settle(now);
        self.caps[pipe] = bytes_per_s.max(1);
        if self.load[pipe] > 0 {
            self.share();
        }
    }

    /// The slab index of a flow (to key a table of owners).
    pub(super) fn index(id: FlowId) -> usize {
        id.0
    }

    /// Flows moving now.
    #[cfg(test)]
    pub(super) fn active(&self) -> usize {
        self.flows.iter().flatten().count()
    }

    /// Brings every flow's bytes left up to `now` at the rates they ran at since the last change.
    pub(super) fn settle(&mut self, now: u64) {
        let dt = u128::from(now.saturating_sub(self.at));
        if dt > 0 {
            let inside = u128::from(now.min(self.hi).saturating_sub(self.at.max(self.lo)));
            for f in self.flows.iter_mut().flatten() {
                // a flow never moves more than it has left
                let step = (u128::from(f.rate) * dt).min(f.left);
                f.left -= step;
                if inside > 0 {
                    let part = step.min(u128::from(f.rate) * inside);
                    for &p in &f.pipes[..f.n] {
                        self.moved[p] += part;
                    }
                }
            }
        }
        self.at = self.at.max(now);
    }

    /// A transfer of `bytes` over `pipes` (1 to 3 distinct pipe indexes) starts at `now`, at most
    /// `limit` bytes a second when given.
    pub(super) fn add(
        &mut self,
        now: u64,
        bytes: u64,
        pipes: &[usize],
        limit: Option<u64>,
    ) -> FlowId {
        assert!(
            (1..=MAX_PIPES).contains(&pipes.len()),
            "a flow crosses 1 to {MAX_PIPES} pipes"
        );
        self.settle(now);
        let mut p = [0; MAX_PIPES];
        p[..pipes.len()].copy_from_slice(pipes);
        let flow = Flow {
            left: u128::from(bytes) * NANO,
            pipes: p,
            n: pipes.len(),
            limit: limit.map_or(u64::MAX, |l| l.max(1)),
            rate: 1,
            end: now,
        };
        // alone on every pipe it crosses: nobody else's rate changes and its own is the narrowest
        // pipe (or its ceiling), so there is nothing to share out
        let alone = pipes.iter().all(|&p| self.load[p] == 0);
        for &p in pipes {
            self.load[p] += 1;
        }
        let mut flow = flow;
        if alone {
            let narrowest = pipes.iter().map(|&p| self.caps[p]).min().unwrap_or(1);
            flow.rate = narrowest.min(flow.limit).max(1);
            flow.end = end_of(flow.left, flow.rate, self.at);
        }
        let id = if let Some(i) = self.free.pop() {
            self.flows[i] = Some(flow);
            i
        } else {
            self.flows.push(Some(flow));
            self.flows.len() - 1
        };
        if !alone {
            self.share();
        }
        FlowId(id)
    }

    /// The flow `id` is done at `now` (the time [`Fluid::next_finish`] gave): the rest speed up.
    /// Returns the pipes it used, so the caller can bring their counters up to date.
    pub(super) fn finish(&mut self, now: u64, id: FlowId) -> ([usize; MAX_PIPES], usize) {
        self.settle(now);
        let used = self.flows[id.0]
            .take()
            .map_or(([0; MAX_PIPES], 0), |f| (f.pipes, f.n));
        if used.1 > 0 {
            self.free.push(id.0);
        }
        // nobody shared a pipe with it: nobody speeds up
        let mut alone = true;
        for &p in &used.0[..used.1] {
            self.load[p] -= 1;
            alone &= self.load[p] == 0;
        }
        if !alone {
            self.share();
        }
        used
    }

    /// The flow that ends first and when (ns), ties by the order the flows joined; `None` when
    /// nothing moves.
    pub(super) fn next_finish(&self) -> Option<(u64, FlowId)> {
        self.flows
            .iter()
            .enumerate()
            .filter_map(|(i, f)| Some((f.as_ref()?.end, FlowId(i))))
            .min()
    }

    /// The current rate of a flow (bytes a second).
    #[cfg(test)]
    fn rate(&self, id: FlowId) -> u64 {
        self.flows[id.0].as_ref().expect("a live flow").rate
    }

    /// Max-min fair rates by progressive filling: the pipe with the smallest share per unfrozen
    /// flow freezes its flows at that share, its capacity is used up, and the rest go on. Then each
    /// flow's end at its new rate.
    fn share(&mut self) {
        let Fluid {
            caps,
            flows,
            at,
            used,
            count,
            active,
            frozen,
            ..
        } = self;
        frozen.clear();
        frozen.extend(flows.iter().map(Option::is_none));
        active.clear();
        let mut unfrozen = 0usize;
        for f in flows.iter().flatten() {
            unfrozen += 1;
            for &p in &f.pipes[..f.n] {
                if count[p] == 0 {
                    active.push(p);
                }
                count[p] += 1;
            }
        }
        while unfrozen > 0 {
            // the pipe with the least left per unfrozen flow (the lowest index on a tie)
            let Some((p, share)) = active
                .iter()
                .filter(|&&p| count[p] > 0)
                .map(|&p| {
                    let n = u64::try_from(count[p]).unwrap_or(u64::MAX);
                    (p, caps[p].saturating_sub(used[p]) / n)
                })
                .min_by_key(|&(p, share)| (share, p))
            else {
                break;
            };
            let share = share.max(1);
            // a flow with a ceiling below that share is held by it, not by the pipe
            let capped = flows
                .iter()
                .enumerate()
                .filter_map(|(i, f)| match (frozen[i], f) {
                    (false, Some(f)) if f.limit < share => Some((f.limit, i)),
                    _ => None,
                })
                .min();
            if let Some((limit, i)) = capped {
                let f = flows[i].as_mut().expect("a live flow");
                f.rate = limit;
                frozen[i] = true;
                unfrozen -= 1;
                for &q in &f.pipes[..f.n] {
                    used[q] = used[q].saturating_add(limit);
                    count[q] -= 1;
                }
                continue;
            }
            for (i, f) in flows.iter_mut().enumerate() {
                if let (false, Some(f)) = (frozen[i], f.as_mut())
                    && f.pipes[..f.n].contains(&p)
                {
                    f.rate = share;
                    frozen[i] = true;
                    unfrozen -= 1;
                    for &q in &f.pipes[..f.n] {
                        used[q] = used[q].saturating_add(share);
                        count[q] -= 1;
                    }
                }
            }
        }
        for &p in active.iter() {
            count[p] = 0;
            used[p] = 0;
        }
        for f in flows.iter_mut().flatten() {
            f.end = end_of(f.left, f.rate, *at);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: u64 = 1_000_000_000;

    #[test]
    fn one_flow_takes_bytes_over_rate() {
        let mut f = Fluid::new(vec![100]);
        f.add(0, 1000, &[0], None);
        assert_eq!(f.next_finish().map(|x| x.0), Some(10 * S));
    }

    #[test]
    fn two_equal_flows_share_and_end_together_at_double_the_time() {
        let mut f = Fluid::new(vec![100]);
        let a = f.add(0, 1000, &[0], None);
        let b = f.add(0, 1000, &[0], None);
        assert_eq!((f.rate(a), f.rate(b)), (50, 50));
        assert_eq!(f.next_finish(), Some((20 * S, a)));
        f.finish(20 * S, a);
        assert_eq!(
            f.next_finish(),
            Some((20 * S, b)),
            "b has nothing left either"
        );
    }

    #[test]
    fn a_small_flow_started_during_a_big_one_ends_first() {
        // FIFO would make the small one wait the big one's whole 100 s
        let mut f = Fluid::new(vec![100]);
        let big = f.add(0, 10_000, &[0], None);
        let small = f.add(10 * S, 100, &[0], None);
        let (t, who) = f.next_finish().expect("flows are moving");
        assert_eq!(who, small);
        assert_eq!(t, 12 * S, "100 B at 50 B/s after 10 s");
        f.finish(t, small);
        assert!(f.rate(big) == 100, "the big one gets the pipe back");
        // 9000 B were left at 10 s; 2 s at 50 B/s moved 100; 8900 B left at 100 B/s
        assert_eq!(f.next_finish(), Some((12 * S + 89 * S, big)));
    }

    #[test]
    fn a_flow_held_by_a_slower_pipe_leaves_its_spare_share_to_the_others() {
        // pipe 0 is 100 B/s, pipe 1 is 10 B/s. Flow a crosses both (held to 10 by pipe 1); flow b
        // crosses only pipe 0 and takes the other 90, not an equal 50.
        let mut f = Fluid::new(vec![100, 10]);
        let a = f.add(0, 1000, &[0, 1], None);
        let b = f.add(0, 1000, &[0], None);
        assert_eq!((f.rate(a), f.rate(b)), (10, 90));
    }

    #[test]
    fn finishing_a_flow_frees_its_share_at_once() {
        let mut f = Fluid::new(vec![100, 10]);
        let a = f.add(0, 100, &[0, 1], None);
        let b = f.add(0, 1000, &[0], None);
        f.finish(10 * S, a);
        assert_eq!(f.rate(b), 100);
        // b moved 900 B in 10 s at 90 B/s, 100 B left at 100 B/s
        assert_eq!(f.next_finish(), Some((11 * S, b)));
    }

    #[test]
    fn same_inputs_give_the_same_answer() {
        let run = || {
            let mut f = Fluid::new(vec![100, 30, 70]);
            let ids: Vec<_> = [
                (0, 500, vec![0, 1]),
                (1, 700, vec![0, 2]),
                (2, 300, vec![1]),
            ]
            .into_iter()
            .map(|(t, b, p)| f.add(t * S, b, &p, None))
            .collect();
            (
                ids.iter().map(|&i| f.rate(i)).collect::<Vec<_>>(),
                f.next_finish(),
            )
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn extremes_do_not_overflow_or_stall() {
        let mut f = Fluid::new(vec![1]);
        let a = f.add(0, 1 << 30, &[0], None);
        let b = f.add(0, 1, &[0], None);
        assert!(
            f.rate(a) >= 1 && f.rate(b) >= 1,
            "a floor of 1 B/s keeps both moving"
        );
        assert!(f.next_finish().is_some());
        let mut g = Fluid::new(vec![u64::MAX]);
        g.add(0, u64::MAX, &[0], None);
        assert!(g.next_finish().is_some());
    }

    #[test]
    fn a_zero_byte_flow_ends_now_and_a_hole_is_reused() {
        let mut f = Fluid::new(vec![100]);
        let a = f.add(5, 0, &[0], None);
        assert_eq!(f.next_finish(), Some((5, a)));
        f.finish(5, a);
        assert_eq!(f.active(), 0);
        let b = f.add(6, 100, &[0], None);
        assert_eq!(b, a, "the slab hole is reused");
    }

    #[test]
    fn a_flow_with_a_ceiling_leaves_the_rest_to_the_others() {
        let mut f = Fluid::new(vec![100]);
        let slow = f.add(0, 1000, &[0], Some(10));
        let other = f.add(0, 1000, &[0], None);
        assert_eq!((f.rate(slow), f.rate(other)), (10, 90));
        // a ceiling above the fair share does nothing
        let mut g = Fluid::new(vec![100]);
        let a = g.add(0, 1000, &[0], Some(80));
        let b = g.add(0, 1000, &[0], None);
        assert_eq!((g.rate(a), g.rate(b)), (50, 50));
    }

    #[test]
    fn pipes_count_the_bytes_they_carried_inside_the_window_only() {
        let mut f = Fluid::new(vec![100]);
        f.set_window(2 * S, 6 * S);
        let a = f.add(0, 1000, &[0], None);
        f.finish(10 * S, a);
        // 100 B/s for the 4 s inside the window
        assert_eq!(f.moved(0), 400 * u128::from(S));
    }

    #[test]
    fn a_transfer_in_flight_is_re_timed_by_the_closed_form() {
        // 1000 B over 100 B/s: at 5 s half is moved; the pipe goes to 200 B/s: 500 B left end at 7.5 s
        let mut f = Fluid::new(vec![100]);
        f.add(0, 1000, &[0], None);
        f.set_cap(5 * S, 0, 200);
        assert_eq!(f.next_finish().map(|x| x.0), Some(5 * S + 5 * S / 2));
    }

    #[test]
    fn two_flows_re_share_after_a_cap_change() {
        let mut f = Fluid::new(vec![100]);
        let a = f.add(0, 1000, &[0], None);
        let b = f.add(0, 1000, &[0], None);
        // each moved 250 B in 5 s at 50 B/s; the pipe halves to 50 B/s: 25 B/s each, 750 B left = 30 s
        f.set_cap(5 * S, 0, 50);
        assert_eq!((f.rate(a), f.rate(b)), (25, 25));
        assert_eq!(f.next_finish().map(|x| x.0), Some(35 * S));
    }

    #[test]
    fn a_cap_change_on_an_idle_pipe_just_stores_it() {
        let mut f = Fluid::new(vec![100, 100]);
        f.set_cap(S, 1, 10);
        let a = f.add(2 * S, 100, &[1], None);
        assert_eq!(f.rate(a), 10);
    }
}

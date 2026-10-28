//! Link chaos on the hops of a trace: which injected latency, jitter or bandwidth cap a `Net` or
//! `Link` hop carries. Split out of `mod.rs`.

use std::time::Duration;

use kaanal_core::{HopKind, NodeId};

use super::Run;

/// **Private.** What injected link chaos to blame on a hop to `to`: the latency it added (to join
/// the hop's `chaos +Nms`) and a note for its `why` (jitter, a cap), or nothing.
///
/// A `Net` hop does not say which edge it crossed, so it is attributed only when exactly one edge
/// leads into `to`; with several, none is blamed rather than guessing. A `Link` hop names its
/// edge, so a cap below the edge's bandwidth is always attributed.
pub(super) fn note(
    run: &Run,
    kind: HopKind,
    to: NodeId,
    dur: Duration,
) -> (Duration, Option<String>) {
    match kind {
        HopKind::Link { edge, .. } => {
            let edge = usize::from(edge);
            let cap = run.chaos.link(edge).and_then(|l| l.bandwidth_cap);
            match (cap, run.topo.edges[edge].bandwidth) {
                (Some(c), Some(b)) if c < b => {
                    (Duration::ZERO, Some(format!("link capped at {c} B/s")))
                }
                _ => (Duration::ZERO, None),
            }
        }
        HopKind::Net => {
            let mut into = (0..run.topo.edges.len()).filter(|e| run.topo.edges[*e].to == to);
            let (Some(edge), None) = (into.next(), into.next()) else {
                return (Duration::ZERO, None);
            };
            let Some(l) = run.chaos.link(edge) else {
                return (Duration::ZERO, None);
            };
            let jitter =
                (!l.jitter.is_zero()).then(|| format!("link jitter ±{}ms", l.jitter.as_millis()));
            (l.latency_add.min(dur), jitter)
        }
        _ => (Duration::ZERO, None),
    }
}

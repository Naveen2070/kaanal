//! The engine's own readings in a tick (step 8L, RULES 115): what the finished requests cannot say.
//! Between two probes of the engine the retry counts and the busy time of each wire grow, so their
//! difference is the tick's figure; the load of each copy and the state of each breaker are read
//! at the instant of the tick.

use kaanal_core::Topology;
use kaanal_core::replay::BreakerState;
use serde::Serialize;

use crate::actor::Probe;

/// What a wire's retry scopes did in the tick, counted by the engine when a call ends (so a call
/// that began in one tick and ended in the next counts in the second). `retried - recovered` over a
/// window is the calls that gave up, plus the few still retrying.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub(crate) struct Funnel {
    /// Calls that entered a retry scope on the wire.
    pub calls: u32,
    /// Calls that made at least one retry.
    pub retried: u32,
    /// Retried calls that ended well.
    pub recovered: u32,
    /// Calls whose last attempt failed, retried or not.
    pub failed: u32,
}

impl Funnel {
    pub(crate) fn is_empty(&self) -> bool {
        *self == Funnel::default()
    }
}

/// One circuit breaker (a wire and the caller copy holding it) at the instant of the tick.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct BreakerTick {
    pub edge: u16,
    pub replica: u16,
    /// `closed`, `half_open` or `open`.
    pub state: &'static str,
}

/// The engine's figures for the tick about to be taken.
#[derive(Debug, Default)]
pub(crate) struct Readings {
    /// `(running, waiting)` of each copy of each node.
    pub copies: Vec<Vec<(u32, usize)>>,
    /// Retry counts per wire, over the span.
    pub funnel: Vec<Funnel>,
    /// Load of each wire with a `bandwidth` over the span: 1 is full the whole time.
    pub link_load: Vec<Option<f64>>,
    pub breakers: Vec<BreakerTick>,
}

fn state_name(s: BreakerState) -> &'static str {
    match s {
        BreakerState::Closed => "closed",
        BreakerState::HalfOpen => "half_open",
        BreakerState::Open => "open",
    }
}

fn delta(after: u64, before: u64) -> u32 {
    u32::try_from(after.saturating_sub(before)).unwrap_or(u32::MAX)
}

/// The readings of the span between `before` and `after` (`None` before: no span yet, so only the
/// instantaneous figures), `span_ns` apart.
#[expect(
    clippy::cast_precision_loss,
    reason = "a ratio of nanosecond totals; f64 precision is ample"
)]
pub(crate) fn readings(
    topo: &Topology,
    before: Option<&Probe>,
    after: &Probe,
    span_ns: u64,
) -> Readings {
    let mut out = Readings {
        copies: after.copies.clone(),
        breakers: after
            .breakers
            .iter()
            .map(|&(edge, replica, s)| BreakerTick {
                edge,
                replica,
                state: state_name(s),
            })
            .collect(),
        ..Readings::default()
    };
    let Some(was) = before else {
        return out;
    };
    out.funnel = after
        .wires
        .iter()
        .zip(&was.wires)
        .map(|(a, b)| Funnel {
            calls: delta(a.calls, b.calls),
            retried: delta(a.retried, b.retried),
            recovered: delta(a.recovered, b.recovered),
            failed: delta(a.failed, b.failed),
        })
        .collect();
    out.link_load = topo
        .edges
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let (a, b) = (after.link_busy.get(i)?, was.link_busy.get(i)?);
            (e.bandwidth.is_some() && span_ns > 0)
                .then(|| a.saturating_sub(*b) as f64 / span_ns as f64)
        })
        .collect();
    out
}

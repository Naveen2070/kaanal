//! The per-part builders of the summary: streams, wires, node pipes, the traffic mix, one entry per
//! copy of a node, and a node's CPU figures. Split from `mod.rs` (R3.8, 800 lines) by responsibility;
//! `mod.rs` keeps the types, the tally and `summarize`.

use std::sync::Arc;

use kaanal_core::{Endpoint, Endpoints, Topology};

use super::{Acc, LinkSummary, PipeSummary, ReplicaSummary, SATURATED, StreamSummary, pct, series};
use crate::{SimConfig, SimReport};

/// **Private.** Per-stream requests, failures and percentiles.
pub(super) fn stream_summaries(
    cfg: &SimConfig,
    lat_by: &[Vec<f64>],
    failed_by: &[u64],
) -> Vec<StreamSummary> {
    cfg.streams
        .iter()
        .enumerate()
        .map(|(i, s)| StreamSummary {
            endpoint: s.endpoint_key.clone(),
            requests: lat_by[i].len() as u64,
            failed: failed_by[i],
            p50_ms: pct(&lat_by[i], 0.50),
            p95_ms: pct(&lat_by[i], 0.95),
            p99_ms: pct(&lat_by[i], 0.99),
        })
        .collect()
}

/// **Private.** Per edge that has a `bandwidth`.
pub(super) fn link_summaries(
    topo: &Topology,
    cfg: &SimConfig,
    report: &SimReport,
    link_acc: &[Acc],
    window_s: f64,
    mix_rate: f64,
) -> Vec<LinkSummary> {
    let mut series = series::link_series(topo, cfg, report);
    topo.edges
        .iter()
        .zip(link_acc)
        .enumerate()
        .filter(|(_, (e, _))| e.bandwidth.is_some())
        .map(|(k, (e, a))| {
            let load = if window_s > 0.0 {
                a.busy / window_s
            } else {
                0.0
            };
            let util = report.link_utilization[k];
            LinkSummary {
                from: topo.nodes[e.from].name.clone(),
                to: topo.nodes[e.to].name.clone(),
                transfers: a.n,
                mean_wait_ms: a.mean_wait_ms(),
                offered_load: load,
                utilization: util,
                saturated: load >= 1.0 || util >= SATURATED,
                saturates_at_req_s: (load > 0.0 && mix_rate > 0.0).then(|| mix_rate / load),
                series: series[k]
                    .take()
                    .expect("an edge with a bandwidth has a series"),
            }
        })
        .collect()
}

/// **Private.** Per node pipe with a rate.
pub(super) fn pipe_summaries(
    topo: &Topology,
    report: &SimReport,
    nic_acc: &[[Acc; 2]],
    window_s: f64,
    mix_rate: f64,
) -> Vec<PipeSummary> {
    let mut pipes = Vec::new();
    for (i, n) in topo.nodes.iter().enumerate() {
        for (side, (rate, direction)) in [(n.bandwidth.0, "send"), (n.bandwidth.1, "receive")]
            .into_iter()
            .enumerate()
        {
            if rate.is_none() {
                continue;
            }
            let a = nic_acc[i][side];
            let load = if window_s > 0.0 {
                a.busy / (window_s * f64::from(n.replicas.max(1)))
            } else {
                0.0
            };
            let util = report.nic_utilization.get(i).map_or(0.0, |u| u[side]);
            pipes.push(PipeSummary {
                node: n.name.clone(),
                direction,
                transfers: a.n,
                mean_wait_ms: a.mean_wait_ms(),
                offered_load: load,
                utilization: util,
                saturated: load >= 1.0 || util >= SATURATED,
                saturates_at_req_s: (load > 0.0 && mix_rate > 0.0).then(|| mix_rate / load),
            });
        }
    }
    pipes
}

/// **Private.** Each stream's endpoint with its request rate (a stream with no such endpoint is
/// left out).
pub(super) fn mix_of(endpoints: &Endpoints, cfg: &SimConfig) -> Vec<(Arc<Endpoint>, f64)> {
    cfg.streams
        .iter()
        .filter_map(|s| {
            endpoints
                .get(&s.endpoint_key)
                .map(|e| (Arc::clone(e), s.rate_per_s))
        })
        .collect()
}

/// **Private.** One entry per copy of a node: its visits, wait and load over the window.
#[expect(
    clippy::cast_precision_loss,
    reason = "visit counts are far below 2^52"
)]
pub(super) fn replica_summaries(
    accs: &[Acc],
    slots: Option<f64>,
    utilization: &[f64],
    window_s: f64,
) -> Vec<ReplicaSummary> {
    let per_s = |x: f64| if window_s > 0.0 { x / window_s } else { 0.0 };
    accs.iter()
        .enumerate()
        .map(|(j, a)| ReplicaSummary {
            replica: u16::try_from(j).unwrap_or(u16::MAX),
            visits: a.v,
            visits_per_s: per_s(a.v as f64),
            mean_wait_ms: a.mean_wait_ms(),
            offered_load: slots.map(|s| per_s(a.busy) / s),
            utilization: slots.map(|_| utilization[j]),
        })
        .collect()
}

/// **Private.** A node's CPU figures and its saturation point under the lower of its slot and CPU
/// capacities (step 8).
pub(super) struct Limits {
    pub(super) cpu_load: Option<f64>,
    pub(super) cpu_capacity: Option<f64>,
    pub(super) saturates_at: Option<f64>,
    pub(super) saturates_by: Option<&'static str>,
}

impl Limits {
    /// `millicores` of CPU per copy, `copies` copies, the node's summed counts with its own work per
    /// second, its slot `capacity` (visits a second), and the mix's `(expected visits, request rate)`.
    #[expect(clippy::cast_precision_loss, reason = "replica counts are at most 64")]
    pub(super) fn of(
        millicores: Option<u32>,
        copies: usize,
        (total, own_per_s): (Acc, f64),
        capacity: Option<f64>,
        (expected, mix_rate): (f64, f64),
    ) -> Limits {
        let cores = millicores.map(|m| f64::from(m) / 1000.0 * copies as f64);
        let cpu_capacity = cores.and_then(|c| total.cpu_capacity(c));
        // requests per second the mix can send before each limit is reached; the lower binds
        let reach = |c: Option<f64>| {
            c.filter(|_| expected > 0.0)
                .map(|c| c * mix_rate / expected)
        };
        let (saturates_at, saturates_by) = match (reach(capacity), reach(cpu_capacity)) {
            (Some(a), Some(b)) if b < a => (Some(b), Some("cpu")),
            (Some(a), _) => (Some(a), Some("slots")),
            (None, Some(b)) => (Some(b), Some("cpu")),
            (None, None) => (None, None),
        };
        Limits {
            cpu_load: cores.map(|c| own_per_s / c),
            cpu_capacity,
            saturates_at,
            saturates_by,
        }
    }
}

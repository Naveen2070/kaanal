//! The report Studio reads: [`summarize`] folds a finished [`SimReport`] into percentiles,
//! per node, replica and link figures, saturation and warnings, so the hop records can be
//! dropped. [`NodeStats`](crate::NodeStats) stays as it is (R1.48(5)); this is the structure
//! beside it.

use std::collections::BTreeMap;
use std::sync::Arc;

use kaanal_core::{ChainResult, Endpoint, Endpoints, HopKind, NodeKind, Topology, visits_per_s};
use serde::Serialize;

use crate::{SimConfig, SimReport};

/// The label every report carries (R6.5).
pub const LABEL: &str = "relative what-if, not a prediction";

/// Fewer reported requests than this make p99 noise.
const TAIL_SAMPLES: u64 = 1000;
/// Fewer than this make p95 noise too.
const FEW: u64 = 100;
/// Measured utilization at or above this counts as saturated (the engine's figure tops out at 1).
const SATURATED: f64 = 0.95;
/// Log2 latency buckets: 0.25 ms doubling up to about 65 s, then an overflow bucket.
const BUCKETS: usize = 19;

/// One run, summarized.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Summary {
    /// Always [`LABEL`].
    pub label: &'static str,
    /// Reporting window in seconds (duration minus warmup).
    pub window_s: f64,
    /// Reported requests.
    pub requests: u64,
    /// Reported requests that failed.
    pub failed: u64,
    /// Failures by kind (`status`, `reset`, `malformed`, `timeout`).
    pub errors: BTreeMap<String, u64>,
    /// Things a reader should know: too few samples for the tail, saturated nodes and links.
    pub warnings: Vec<String>,
    /// All requests.
    pub latency: Latency,
    /// Per stream (endpoint), in [`SimConfig::streams`] order.
    pub streams: Vec<StreamSummary>,
    /// Per node, by node id.
    pub nodes: Vec<NodeSummary>,
    /// Per edge that has a `bandwidth`.
    pub links: Vec<LinkSummary>,
    /// Node ids with visits, the most loaded first: offered load, then mean wait; nodes without
    /// slots last.
    pub bottlenecks: Vec<usize>,
}

/// Latency percentiles and a histogram, in milliseconds.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Latency {
    /// Median.
    pub p50_ms: f64,
    /// 95th percentile.
    pub p95_ms: f64,
    /// 99th percentile.
    pub p99_ms: f64,
    /// Slowest.
    pub max_ms: f64,
    /// Counts per bucket (not cumulative).
    pub buckets: Vec<Bucket>,
}

/// Requests with latency in `(previous le, le_ms]`; `le_ms` is `None` for the overflow bucket.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Bucket {
    /// Upper bound in ms.
    pub le_ms: Option<f64>,
    /// Requests in the bucket.
    pub count: u64,
}

/// One stream's requests and percentiles.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StreamSummary {
    /// The endpoint key.
    pub endpoint: String,
    /// Reported requests.
    pub requests: u64,
    /// Failed ones.
    pub failed: u64,
    /// Median in ms.
    pub p50_ms: f64,
    /// 95th percentile in ms.
    pub p95_ms: f64,
    /// 99th percentile in ms.
    pub p99_ms: f64,
}

/// One node over the reporting window.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NodeSummary {
    /// Node name.
    pub name: String,
    /// One entry per replica (one for a single-copy node).
    pub replicas: Vec<ReplicaSummary>,
    /// Measured visits per second, all replicas.
    pub visits_per_s: f64,
    /// Visits per second the chains predict at these rates (`visits_per_s` in core); failures
    /// cut chains short, so a failing run measures less.
    pub expected_visits_per_s: f64,
    /// Mean queue wait before a visit, ms.
    pub mean_wait_ms: f64,
    /// Recorded hop time over slots × replicas × window; `None` for a node without `concurrency`
    /// or `pool`. Can exceed 1. A hop cut short by a failure or a timeout is recorded cut, so
    /// this undercounts then; `utilization` does not.
    pub offered_load: Option<f64>,
    /// Measured utilization (the engine's busy slot-time in the window, at most 1); `None`
    /// without slots. A cancelled hop counts for as long as it held its slot.
    pub utilization: Option<f64>,
    /// Offered load of 1.0 or more, utilization of 0.95 or more, or requests shed here (a full
    /// queue or a fired deadline): the queue grows without bound, or the node already refuses work.
    pub saturated: bool,
    /// Longest queue seen.
    pub max_queue: usize,
    /// Refused by the rate limit.
    pub limited: u64,
    /// Refused by a full queue.
    pub overflowed: u64,
    /// Deadlines that fired here.
    pub timed_out: u64,
}

/// One replica of a node.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReplicaSummary {
    /// Replica index.
    pub replica: u16,
    /// Visits.
    pub visits: u64,
    /// Visits per second.
    pub visits_per_s: f64,
    /// Mean queue wait, ms.
    pub mean_wait_ms: f64,
    /// Recorded hop time over slots × window; `None` without slots.
    pub offered_load: Option<f64>,
    /// Measured utilization of this replica; `None` without slots.
    pub utilization: Option<f64>,
}

/// One edge with a `bandwidth`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LinkSummary {
    /// Calling node.
    pub from: String,
    /// Called node.
    pub to: String,
    /// Bodies carried (request and response each count).
    pub transfers: u64,
    /// Mean queue wait before a transfer, ms.
    pub mean_wait_ms: f64,
    /// Transfer time over the window: the share of the link's capacity in use.
    pub offered_load: f64,
    /// Measured utilization of the link (busy time in the window, at most 1).
    pub utilization: f64,
    /// Offered load of 1.0 or more, or utilization of 0.95 or more.
    pub saturated: bool,
}

/// A running total: count, summed wait and summed busy time (seconds).
#[derive(Default, Clone, Copy)]
struct Acc {
    /// Hops (for the mean wait).
    n: u64,
    /// Visits: a monolith's `Proc` and `Op` are one visit.
    v: u64,
    wait: f64,
    busy: f64,
}

impl Acc {
    fn add(&mut self, wait: f64, busy: f64, visit: bool) {
        self.n += 1;
        self.v += u64::from(visit);
        self.wait += wait;
        self.busy += busy;
    }

    fn plus(self, o: Acc) -> Acc {
        Acc {
            n: self.n + o.n,
            v: self.v + o.v,
            wait: self.wait + o.wait,
            busy: self.busy + o.busy,
        }
    }

    #[expect(
        clippy::cast_precision_loss,
        reason = "visit counts are far below 2^52"
    )]
    fn mean_wait_ms(self) -> f64 {
        if self.n == 0 {
            0.0
        } else {
            self.wait * 1000.0 / self.n as f64
        }
    }
}

fn ms(d: std::time::Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Nearest-rank percentile of a sorted slice; 0.0 when empty.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "the rank is clamped to the slice"
)]
fn pct(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = ((p * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len());
    sorted[rank - 1]
}

fn latency(sorted: &[f64]) -> Latency {
    let mut counts = vec![0u64; BUCKETS + 1];
    for &v in sorted {
        let (mut i, mut le) = (0, 0.25);
        while i < BUCKETS && v > le {
            le *= 2.0;
            i += 1;
        }
        counts[i] += 1;
    }
    let mut le = 0.25;
    let buckets = counts
        .into_iter()
        .enumerate()
        .map(|(i, count)| {
            let b = Bucket {
                le_ms: (i < BUCKETS).then_some(le),
                count,
            };
            le *= 2.0;
            b
        })
        .collect();
    Latency {
        p50_ms: pct(sorted, 0.50),
        p95_ms: pct(sorted, 0.95),
        p99_ms: pct(sorted, 0.99),
        max_ms: sorted.last().copied().unwrap_or(0.0),
        buckets,
    }
}

/// What one pass over the records adds up.
struct Tally {
    node_acc: Vec<Vec<Acc>>,
    link_acc: Vec<Acc>,
    /// Sorted latencies in ms, all requests.
    lat_all: Vec<f64>,
    /// Sorted latencies per stream.
    lat_by: Vec<Vec<f64>>,
    failed_by: Vec<u64>,
    errors: BTreeMap<String, u64>,
}

/// **Private.** One pass over the records: per replica and per link totals, latencies, failures.
fn tally(topo: &Topology, cfg: &SimConfig, report: &SimReport) -> Tally {
    let mut node_acc: Vec<Vec<Acc>> = topo
        .nodes
        .iter()
        .map(|n| vec![Acc::default(); usize::from(n.replicas.max(1))])
        .collect();
    let mut link_acc = vec![Acc::default(); topo.edges.len()];
    let mut lat_all = Vec::with_capacity(report.requests.len());
    let mut lat_by: Vec<Vec<f64>> = vec![Vec::new(); cfg.streams.len()];
    let mut failed_by = vec![0u64; cfg.streams.len()];
    let mut errors: BTreeMap<String, u64> = BTreeMap::new();
    for r in &report.requests {
        let l = ms(r.latency);
        lat_all.push(l);
        lat_by[r.stream].push(l);
        if let ChainResult::Err { kind, .. } = r.result {
            failed_by[r.stream] += 1;
            *errors.entry(kind.as_str().to_string()).or_default() += 1;
        }
        for (hop, wait) in &r.hops {
            let (w, d) = (wait.as_secs_f64(), hop.dur.as_secs_f64());
            match hop.kind {
                HopKind::Proc | HopKind::Op(_) | HopKind::CacheHit | HopKind::CacheMiss => {
                    // a monolith runs a Proc hop and an Op hop per visit: count the Proc one
                    let visit = !(matches!(hop.kind, HopKind::Op(_))
                        && topo.nodes[hop.node].kind == NodeKind::Monolith);
                    node_acc[hop.node][usize::from(hop.replica)].add(w, d, visit);
                }
                HopKind::Link { edge, .. } => link_acc[usize::from(edge)].add(w, d, true),
                HopKind::Net | HopKind::Delay => {}
            }
        }
    }
    lat_all.sort_by(f64::total_cmp);
    for v in &mut lat_by {
        v.sort_by(f64::total_cmp);
    }
    Tally {
        node_acc,
        link_acc,
        lat_all,
        lat_by,
        failed_by,
        errors,
    }
}

/// **Private.** What a reader should know before trusting the numbers.
fn warnings(requests: u64, nodes: &[NodeSummary], links: &[LinkSummary]) -> Vec<String> {
    let mut warnings = Vec::new();
    if requests < FEW {
        warnings.push(format!(
            "only {requests} requests: p95 and p99 are noise; run longer"
        ));
    } else if requests < TAIL_SAMPLES {
        warnings.push(format!("p99 rests on {requests} requests; run longer"));
    }
    for n in nodes.iter().filter(|n| n.saturated) {
        warnings.push(format!(
            "node {} is saturated (offered load {:.2}, utilization {:.2}): its queue grows for as long as the run lasts, so its latency is not a steady-state figure",
            n.name,
            n.offered_load.unwrap_or(0.0),
            n.utilization.unwrap_or(0.0)
        ));
    }
    for n in nodes
        .iter()
        .filter(|n| n.offered_load.is_none() && n.timed_out > 0)
    {
        warnings.push(format!(
            "{} deadlines fired at node {} (it has no slot limit, so a callee was too slow)",
            n.timed_out, n.name
        ));
    }
    for l in links.iter().filter(|l| l.saturated) {
        warnings.push(format!("link {} -> {} is saturated", l.from, l.to));
    }
    warnings
}

/// **Private.** Node ids with visits, the most loaded first.
fn bottlenecks(nodes: &[NodeSummary]) -> Vec<usize> {
    let mut bottlenecks: Vec<usize> = (0..nodes.len())
        .filter(|&i| nodes[i].visits_per_s > 0.0)
        .collect();
    bottlenecks.sort_by(|&a, &b| {
        let (x, y) = (&nodes[a], &nodes[b]);
        // nodes with slots first, then the heavier load, then the longer wait
        let load = |n: &NodeSummary| n.offered_load.map(|o| o.max(n.utilization.unwrap_or(0.0)));
        load(y)
            .is_some()
            .cmp(&load(x).is_some())
            .then(
                load(y)
                    .partial_cmp(&load(x))
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
            .then(y.mean_wait_ms.total_cmp(&x.mean_wait_ms))
    });
    bottlenecks
}

/// **Private.** Per-stream requests, failures and percentiles.
fn stream_summaries(cfg: &SimConfig, lat_by: &[Vec<f64>], failed_by: &[u64]) -> Vec<StreamSummary> {
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

/// Folds `report` (a run of `cfg` on `topo`) into a [`Summary`].
#[must_use]
#[expect(
    clippy::cast_precision_loss,
    reason = "counts and sizes are far below 2^52"
)]
pub fn summarize(
    topo: &Topology,
    endpoints: &Endpoints,
    cfg: &SimConfig,
    report: &SimReport,
) -> Summary {
    let window_s = cfg.duration.saturating_sub(cfg.warmup).as_secs_f64();
    let per_s = |x: f64| if window_s > 0.0 { x / window_s } else { 0.0 };
    let Tally {
        node_acc,
        link_acc,
        lat_all,
        lat_by,
        failed_by,
        errors,
    } = tally(topo, cfg, report);
    let mix: Vec<(Arc<Endpoint>, f64)> = cfg
        .streams
        .iter()
        .filter_map(|s| {
            endpoints
                .get(&s.endpoint_key)
                .map(|e| (Arc::clone(e), s.rate_per_s))
        })
        .collect();
    let expected = visits_per_s(topo, &mix);
    let load_of = |busy: f64, capacity: f64| per_s(busy) / capacity;
    let nodes: Vec<NodeSummary> = topo
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let slots = n.admission.as_ref().map(|a| f64::from(a.slots));
            let replicas = node_acc[i]
                .iter()
                .enumerate()
                .map(|(j, a)| ReplicaSummary {
                    replica: u16::try_from(j).unwrap_or(u16::MAX),
                    visits: a.v,
                    visits_per_s: per_s(a.v as f64),
                    mean_wait_ms: a.mean_wait_ms(),
                    offered_load: slots.map(|s| load_of(a.busy, s)),
                    utilization: slots.map(|_| report.replica_utilization[i][j]),
                })
                .collect();
            let total = node_acc[i].iter().fold(Acc::default(), |t, a| t.plus(*a));
            let load = slots.map(|s| load_of(total.busy, s * node_acc[i].len() as f64));
            let st = &report.nodes[i];
            let util = slots.map(|_| st.utilization);
            NodeSummary {
                name: n.name.clone(),
                replicas,
                visits_per_s: per_s(total.v as f64),
                expected_visits_per_s: expected[i],
                mean_wait_ms: total.mean_wait_ms(),
                offered_load: load,
                utilization: util,
                // only a node with slots can be saturated; a deadline that fires at a node
                // without slots says its callee was slow, and gets its own warning
                saturated: slots.is_some()
                    && (load.is_some_and(|l| l >= 1.0)
                        || util.is_some_and(|u| u >= SATURATED)
                        || st.overflowed > 0
                        || st.timed_out > 0),
                max_queue: st.max_queue,
                limited: st.limited,
                overflowed: st.overflowed,
                timed_out: st.timed_out,
            }
        })
        .collect();
    let links: Vec<LinkSummary> = topo
        .edges
        .iter()
        .zip(&link_acc)
        .enumerate()
        .filter(|(_, (e, _))| e.bandwidth.is_some())
        .map(|(k, (e, a))| {
            let load = load_of(a.busy, 1.0);
            let util = report.link_utilization[k];
            LinkSummary {
                from: topo.nodes[e.from].name.clone(),
                to: topo.nodes[e.to].name.clone(),
                transfers: a.n,
                mean_wait_ms: a.mean_wait_ms(),
                offered_load: load,
                utilization: util,
                saturated: load >= 1.0 || util >= SATURATED,
            }
        })
        .collect();
    let requests = report.requests.len() as u64;
    let warnings = warnings(requests, &nodes, &links);
    let bottlenecks = bottlenecks(&nodes);
    let streams = stream_summaries(cfg, &lat_by, &failed_by);
    Summary {
        label: LABEL,
        window_s,
        requests,
        failed: failed_by.iter().sum(),
        errors,
        warnings,
        latency: latency(&lat_all),
        streams,
        nodes,
        links,
        bottlenecks,
    }
}

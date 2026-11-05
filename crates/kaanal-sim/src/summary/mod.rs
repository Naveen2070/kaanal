//! The report Studio reads: [`summarize`] folds a finished [`SimReport`] into percentiles,
//! per node, replica and link figures, saturation and warnings, so the hop records can be
//! dropped. [`NodeStats`](crate::NodeStats) stays as it is (R1.48(5)); this is the structure
//! beside it.

use std::collections::BTreeMap;
use std::sync::Arc;

use kaanal_core::{ChainResult, Endpoint, Endpoints, HopKind, NodeKind, Topology, visits_per_s};
use serde::Serialize;

mod series;

pub use series::{LinkSeries, Series, SystemSeries};

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
    /// Requests finished per window, split by outcome, with each window's p99; 60 windows.
    pub series: SystemSeries,
    /// Per stream (endpoint), in [`SimConfig::streams`] order.
    pub streams: Vec<StreamSummary>,
    /// Per node, by node id.
    pub nodes: Vec<NodeSummary>,
    /// Per edge that has a `bandwidth`.
    pub links: Vec<LinkSummary>,
    /// Per node pipe with a `bandwidth` (a node's send and receive side each).
    pub pipes: Vec<PipeSummary>,
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
    /// Visits per second the node can take: slots × replicas ÷ mean service time per visit;
    /// `None` without slots or visits.
    pub capacity_per_s: Option<f64>,
    /// Requests per second the whole traffic mix can reach before this node saturates:
    /// `capacity_per_s` divided by the visits one request makes here (the mix's expected visits
    /// over its request rate). `None` without a capacity or when the mix never visits the node.
    pub saturates_at_req_s: Option<f64>,
    /// `1 - offered_load`, at least 0; `None` without slots.
    pub headroom: Option<f64>,
    /// Median hop work time in ms (wait excluded), exact over every hop; `None` without visits.
    pub p50_ms: Option<f64>,
    /// 99th percentile of the hop work time in ms (wait excluded), exact over every hop; `None`
    /// without visits. (Before 2026-11-02 this was the upper edge of a power-of-two bucket.)
    pub p99_ms: Option<f64>,
    /// Busy fraction, longest queue and hop latencies over time, 60 windows.
    pub series: Series,
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
    /// Mean time a transfer lost to sharing the rate with others, ms.
    pub mean_wait_ms: f64,
    /// Transfer time over the window: the share of the link's capacity in use.
    pub offered_load: f64,
    /// Measured utilization of the link (busy time in the window, at most 1).
    pub utilization: f64,
    /// Offered load of 1.0 or more, or utilization of 0.95 or more.
    pub saturated: bool,
    /// Requests per second the whole traffic mix can reach before this link saturates: the mix's
    /// request rate over `offered_load` (transfer time grows in step with the rate). `None` when
    /// nothing crossed the link. A node's counterpart is `NodeSummary::saturates_at_req_s`.
    pub saturates_at_req_s: Option<f64>,
    /// Busy share and time on the wire over the run, 60 windows.
    pub series: LinkSeries,
}

/// One side of a node's network card (`bandwidth:` on a node).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PipeSummary {
    /// The node the card belongs to.
    pub node: String,
    /// `send` or `receive`.
    pub direction: &'static str,
    /// Bodies carried over this pipe (request and response each count) that it was the narrowest
    /// pipe for.
    pub transfers: u64,
    /// Mean time a transfer lost to sharing the rate with others, ms.
    pub mean_wait_ms: f64,
    /// Transfer time over the window and the node's replicas: the share of capacity in use.
    pub offered_load: f64,
    /// Measured utilization of the pipe (busy time in the window, at most 1).
    pub utilization: f64,
    /// Offered load of 1.0 or more, or utilization of 0.95 or more.
    pub saturated: bool,
    /// Requests per second the whole traffic mix can reach before this pipe saturates; `None`
    /// when nothing crossed it.
    pub saturates_at_req_s: Option<f64>,
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

    /// Visits per second `slots` slots can take at this mean busy time per visit; `None`
    /// without visits or busy time.
    #[expect(
        clippy::cast_precision_loss,
        reason = "visit counts are far below 2^52"
    )]
    fn capacity(self, slots: f64) -> Option<f64> {
        (self.v > 0 && self.busy > 0.0).then(|| slots * self.v as f64 / self.busy)
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

/// Index of the bucket a latency in ms falls in: `(previous le, le]`, the overflow bucket last.
fn bucket_of(v: f64) -> usize {
    let (mut i, mut le) = (0, 0.25);
    while i < BUCKETS && v > le {
        le *= 2.0;
        i += 1;
    }
    i
}

/// The exact percentile of sorted values, or `None` when there are none.
fn exact(sorted: &[f64], p: f64) -> Option<f64> {
    (!sorted.is_empty()).then(|| pct(sorted, p))
}

fn latency(sorted: &[f64]) -> Latency {
    let mut counts = vec![0u64; BUCKETS + 1];
    for &v in sorted {
        counts[bucket_of(v)] += 1;
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
    /// Per node, `[send, receive]`, summed over its replicas.
    nic_acc: Vec<[Acc; 2]>,
    /// Sorted latencies in ms, all requests.
    lat_all: Vec<f64>,
    /// Sorted latencies per stream.
    lat_by: Vec<Vec<f64>>,
    /// Sorted work time in ms of every hop at each node (wait excluded), for exact percentiles.
    work_by: Vec<Vec<f64>>,
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
    let mut nic_acc = vec![[Acc::default(); 2]; topo.nodes.len()];
    let mut lat_all = Vec::with_capacity(report.requests.len());
    let mut lat_by: Vec<Vec<f64>> = vec![Vec::new(); cfg.streams.len()];
    let mut work_by: Vec<Vec<f64>> = vec![Vec::new(); topo.nodes.len()];
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
        // a node that held its slot across a call is busy for the whole hold (own work and the wait
        // on what it called), so that, not its own hops, is its busy time
        for h in &r.holds {
            node_acc[h.node][usize::from(h.replica)].busy += h.dur.as_secs_f64();
        }
        for (hop, wait) in &r.hops {
            let (w, d) = (wait.as_secs_f64(), hop.dur.as_secs_f64());
            match hop.kind {
                HopKind::Proc | HopKind::Op(_) | HopKind::CacheHit | HopKind::CacheMiss => {
                    // a monolith runs a Proc hop and an Op hop per visit: count the Proc one
                    let visit = !(matches!(hop.kind, HopKind::Op(_))
                        && topo.nodes[hop.node].kind == NodeKind::Monolith);
                    let held = r
                        .holds
                        .iter()
                        .any(|h| h.node == hop.node && h.replica == hop.replica);
                    node_acc[hop.node][usize::from(hop.replica)].add(
                        w,
                        if held { 0.0 } else { d },
                        visit,
                    );
                    work_by[hop.node].push(d * 1000.0);
                }
                HopKind::Link { edge, .. } => link_acc[usize::from(edge)].add(w, d, true),
                HopKind::Nic { send, .. } => {
                    nic_acc[hop.node][usize::from(!send)].add(w, d, true);
                }
                HopKind::Net | HopKind::Delay | HopKind::Connect { .. } | HopKind::FirstByte => {}
            }
        }
    }
    lat_all.sort_by(f64::total_cmp);
    for v in lat_by.iter_mut().chain(&mut work_by) {
        v.sort_by(f64::total_cmp);
    }
    Tally {
        node_acc,
        link_acc,
        nic_acc,
        lat_all,
        lat_by,
        work_by,
        failed_by,
        errors,
    }
}

/// **Private.** What a reader should know before trusting the numbers.
fn warnings(
    requests: u64,
    nodes: &[NodeSummary],
    links: &[LinkSummary],
    pipes: &[PipeSummary],
) -> Vec<String> {
    let mut warnings = Vec::new();
    if requests < FEW {
        warnings.push(format!(
            "only {requests} requests: p95 and p99 are noise; run longer"
        ));
    } else if requests < TAIL_SAMPLES {
        warnings.push(format!("p99 rests on {requests} requests; run longer"));
    }
    for n in nodes.iter().filter(|n| n.saturated) {
        // a queue with a limit does not grow for ever: once full it turns requests away instead
        let then = if n.overflowed > 0 {
            "its queue is full and it turns requests away, so its latency is not a steady-state figure"
        } else {
            "its queue grows for as long as the run lasts, so its latency is not a steady-state figure"
        };
        warnings.push(format!(
            "node {} is saturated (offered load {:.2}, utilization {:.2}): {then}",
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
    for p in pipes.iter().filter(|p| p.saturated) {
        warnings.push(format!("{} of node {} is saturated", p.direction, p.node));
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

/// **Private.** Per edge that has a `bandwidth`.
fn link_summaries(
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
fn pipe_summaries(
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
        nic_acc,
        lat_all,
        lat_by,
        work_by,
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
    let mix_rate: f64 = mix.iter().map(|(_, r)| r).sum();
    let mut series = series::series(topo, cfg, report).into_iter();
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
            let capacity = slots.and_then(|s| total.capacity(s * node_acc[i].len() as f64));
            let util = slots.map(|_| st.utilization);
            let ns = series.next().expect("one series per node");
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
                capacity_per_s: capacity,
                saturates_at_req_s: capacity
                    .filter(|_| expected[i] > 0.0)
                    .map(|c| c * mix_rate / expected[i]),
                headroom: load.map(|l| (1.0 - l).max(0.0)),
                // exact, from every hop's work time; the buckets only bound it within a factor of two
                p50_ms: exact(&work_by[i], 0.50),
                p99_ms: exact(&work_by[i], 0.99),
                series: ns,
                limited: st.limited,
                overflowed: st.overflowed,
                timed_out: st.timed_out,
            }
        })
        .collect();
    let links = link_summaries(topo, cfg, report, &link_acc, window_s, mix_rate);
    let requests = report.requests.len() as u64;
    let pipes = pipe_summaries(topo, report, &nic_acc, window_s, mix_rate);
    let warnings = warnings(requests, &nodes, &links, &pipes);
    let bottlenecks = bottlenecks(&nodes);
    let sys_series = series::system_series(cfg, report);
    let streams = stream_summaries(cfg, &lat_by, &failed_by);
    Summary {
        label: LABEL,
        window_s,
        requests,
        failed: failed_by.iter().sum(),
        errors,
        warnings,
        latency: latency(&lat_all),
        series: sys_series,
        streams,
        nodes,
        links,
        pipes,
        bottlenecks,
    }
}

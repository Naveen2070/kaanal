//! **Private.** The event loop behind [`crate::run`]: stream arrivals on a virtual clock in
//! integer nanoseconds, merged by `(time, seq)` with the events of the replay engine in
//! `kaanal_core::replay`, which walks every request's `Plan` (fork/join, FIFO queueing, failure
//! cancellation). This module has no timing model of its own.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::Arc;
use std::time::Duration;

use kaanal_core::replay::{Engine, EngineError, FailCause};
use kaanal_core::{
    ChainRequest, Dist, EffectiveChaos, Endpoint, Topology, sample_plan, stable_hash,
};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

use crate::{Cause, NodeStats, RequestRecord, SimConfig, SimReport};

/// **Private.** Low bits of a request tag hold the stream's request counter, high bits the stream.
const TAG_BITS: u32 = 48;

/// **Private.** The error text of a clock overflow.
const OVERFLOW: &str = "simulated clock overflow (about 584 years): the workload queues for too long, lower the rate or the service times";

/// **Private.** One stream's arrival RNG, gap distribution and request counter.
struct Arrivals {
    /// Seeded from `(seed, "arrivals", endpoint_key)`.
    rng: ChaCha8Rng,
    /// Exponential with mean `1 / rate_per_s`.
    gap: Dist,
    /// Next request counter.
    n: u64,
}

/// **Private.** Duration → virtual nanoseconds. Validation (duration ≤ 365 days, every
/// service/net/chaos time ≤ 10 h) keeps each value far below `u64::MAX`.
fn nanos(d: Duration) -> u64 {
    u64::try_from(d.as_nanos()).expect("validated durations fit in u64 nanoseconds")
}

/// **Private.** Schedules stream `s`'s next arrival after `base`, unless it falls at or after
/// `end`. The arrival takes its sequence number from the engine so it orders against hop ends.
fn schedule(
    engine: &mut Engine,
    arrivals: &mut [Arrivals],
    pending: &mut BinaryHeap<Reverse<(u64, u64, usize)>>,
    s: usize,
    base: u64,
    end: u64,
) -> Result<(), String> {
    let a = &mut arrivals[s];
    let gap = nanos(a.gap.sample(&mut a.rng));
    let at = base.checked_add(gap).ok_or_else(|| OVERFLOW.to_string())?;
    if at < end {
        pending.push(Reverse((at, engine.stamp(), s)));
    }
    Ok(())
}

/// **Private.** Runs a validated config until nothing is left to do. The second value is
/// the most requests held at once (the request table's length), for the release test.
pub(crate) fn simulate(
    topo: &Topology,
    endpoints: &[Arc<Endpoint>],
    chaos: &EffectiveChaos,
    cfg: &SimConfig,
) -> Result<(SimReport, usize), String> {
    let overflow = |_: EngineError| OVERFLOW.to_string();
    let (warmup, end) = (nanos(cfg.warmup), nanos(cfg.duration));
    let mut engine = Engine::new(topo);
    engine.set_window(warmup, end);
    let mut arrivals: Vec<Arrivals> = cfg
        .streams
        .iter()
        .map(|s| Arrivals {
            rng: ChaCha8Rng::seed_from_u64(stable_hash(&[
                &cfg.seed.to_le_bytes(),
                b"arrivals",
                s.endpoint_key.as_bytes(),
            ])),
            gap: Dist::Exp {
                mean: Duration::from_secs_f64(1.0 / s.rate_per_s),
            },
            n: 0,
        })
        .collect();
    // (time, seq, stream): the next arrival of each stream.
    let mut pending = BinaryHeap::new();
    for s in 0..cfg.streams.len() {
        schedule(&mut engine, &mut arrivals, &mut pending, s, 0, end)?;
    }
    let mut records: Vec<(u64, RequestRecord)> = Vec::new();
    loop {
        let next_arrival = pending.peek().map(|Reverse((t, q, _))| (*t, *q));
        let arrival_first = match (next_arrival, engine.peek()) {
            (None, None) => break,
            (Some(a), Some(e)) => a < e,
            (Some(_), None) => true,
            (None, Some(_)) => false,
        };
        if arrival_first {
            let Reverse((at, _, s)) = pending.pop().expect("peeked");
            let n = arrivals[s].n;
            arrivals[s].n += 1;
            let (plan, _) = sample_plan(
                topo,
                chaos,
                &ChainRequest {
                    endpoint: &endpoints[s],
                    n,
                    seed: cfg.seed,
                    pin_delay: std::time::Duration::ZERO,
                    req_bytes: cfg.streams[s].req_bytes,
                    resp_bytes: cfg.streams[s].resp_bytes,
                },
            );
            schedule(&mut engine, &mut arrivals, &mut pending, s, at, end)?;
            let tag = ((s as u64) << TAG_BITS) | n;
            engine.submit(at, Arc::new(plan), tag).map_err(overflow)?;
        } else {
            engine.step().map_err(overflow)?;
        }
        for f in engine.drain_finished_iter() {
            records.push((
                f.order,
                RequestRecord {
                    stream: usize::try_from(f.tag >> TAG_BITS).expect("stream index fits"),
                    n: f.tag & ((1 << TAG_BITS) - 1),
                    arrival: Duration::from_nanos(f.arrival),
                    latency: Duration::from_nanos(f.done - f.arrival),
                    result: f.result,
                    cause: f.cause.map(|c| match c {
                        FailCause::Chaos => Cause::Chaos,
                        FailCause::RateLimit => Cause::RateLimit,
                        FailCause::Overflow => Cause::Overflow,
                        FailCause::Deadline => Cause::Deadline,
                    }),
                    hops: f.hops.into_iter().map(|h| (h.hop, h.wait)).collect(),
                },
            ));
        }
    }
    let peak = engine.table_len();
    Ok((report(topo, &engine, records, warmup, end), peak))
}

/// **Private.** Builds the report from requests arriving at or after warmup.
#[expect(
    clippy::cast_precision_loss,
    reason = "utilization is a ratio of nanosecond totals; f64 precision is ample"
)]
fn report(
    topo: &Topology,
    engine: &Engine,
    mut records: Vec<(u64, RequestRecord)>,
    warmup: u64,
    end: u64,
) -> SimReport {
    records.sort_by_key(|(order, _)| *order);
    let requests = records.into_iter().map(|(_, r)| r).collect();
    let window = (end - warmup) as f64;
    let nodes = engine
        .node_counters()
        .iter()
        .zip(&topo.nodes)
        .map(|(c, n)| NodeStats {
            completed: c.completed,
            mean_wait: match u128::from(c.completed) {
                0 => Duration::ZERO,
                k => Duration::from_nanos(u64::try_from(c.wait / k).unwrap_or(u64::MAX)),
            },
            utilization: n.admission.as_ref().map_or(0.0, |a| {
                c.area as f64 / (f64::from(a.slots) * f64::from(n.replicas.max(1)) * window)
            }),
            max_queue: c.max_queue,
            limited: c.limited,
            overflowed: c.overflowed,
            timed_out: c.timed_out,
        })
        .collect();
    // measured utilization of each replica and link: busy slot-time inside the window over its
    // slots (a link is one slot), clipped by the engine, so at most 1
    let replica_utilization = engine
        .replica_counters()
        .iter()
        .zip(&topo.nodes)
        .map(|(rs, n)| {
            rs.iter()
                .map(|c| {
                    n.admission
                        .as_ref()
                        .map_or(0.0, |a| c.area as f64 / (f64::from(a.slots) * window))
                })
                .collect()
        })
        .collect();
    let link_utilization = engine
        .link_counters()
        .iter()
        .map(|c| c.area as f64 / window)
        .collect();
    SimReport {
        requests,
        nodes,
        replica_utilization,
        link_utilization,
    }
}

#[cfg(test)]
mod tests {
    use super::simulate;
    use crate::{SimConfig, Stream};
    use kaanal_core::chain::{EndpointsInput, RouteSpec, compile};
    use kaanal_core::{Config, EffectiveChaos, Topology};
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn pre_warmup_requests_release_their_plans() {
        // R6.13 b: ~10 000 requests run, but at ρ = 0.5 only a handful are ever in flight,
        // so the request table stays small instead of holding every request.
        let t = Topology::build(
            &Config::from_yaml("version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service, concurrency: 1, proc: { mean: 10ms } }\nedges:\n  - { from: gw, to: app }\n").unwrap(),
        )
        .unwrap();
        let eps = compile(
            &t,
            &[RouteSpec::inferred("GET /x/{id}", t.id("app").unwrap())],
            EndpointsInput::default(),
        )
        .unwrap();
        let cfg = SimConfig {
            streams: vec![Stream {
                endpoint_key: "GET /x/{id}".into(),
                rate_per_s: 50.0,
                req_bytes: 0,
                resp_bytes: 0,
            }],
            duration: Duration::from_secs(200),
            warmup: Duration::from_secs(190),
            seed: 7,
        };
        let one = [Arc::clone(eps.get("GET /x/{id}").unwrap())];
        let (rep, peak) = simulate(&t, &one, &EffectiveChaos::default(), &cfg).unwrap();
        assert!(rep.requests.len() > 300, "{}", rep.requests.len());
        assert!(peak < 64, "requests held at once: {peak}");
    }
}

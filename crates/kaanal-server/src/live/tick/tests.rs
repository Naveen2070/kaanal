//! The tests of this module, kept in their own file so the module stays under the size limit.

use super::*;
use kaanal_core::chain::{EndpointsInput, RouteSpec, compile};
use kaanal_core::replay::Engine;
use kaanal_core::{ChainRequest, Config, EffectiveChaos, sample_plan};
use std::sync::Arc;

const STATUS: TickStatus = TickStatus {
    rate: 100.0,
    paused: false,
    overloaded: false,
    in_flight: 0,
};

/// Runs `n` requests through `yaml`'s gw -> app -> db chain at one instant and returns the
/// topology and what finished.
fn finished(yaml: &str, n: u64, spacing_ms: u64) -> (Topology, Vec<Finished>) {
    let t = Topology::build(&Config::from_yaml(yaml).unwrap()).unwrap();
    let eps = compile(
        &t,
        &[RouteSpec::inferred("GET /x/{id}", t.id("app").unwrap())],
        EndpointsInput::default(),
    )
    .unwrap();
    let ep = eps.get("GET /x/{id}").unwrap();
    let mut e = Engine::live(&t);
    let mut out = Vec::new();
    for i in 0..n {
        let (plan, _) = sample_plan(
            &t,
            &EffectiveChaos::default(),
            &ChainRequest {
                endpoint: ep,
                at: Duration::ZERO,
                n: i,
                seed: 1,
                pin_delay: Duration::ZERO,
                req_bytes: 0,
                resp_bytes: 0,
            },
        );
        e.submit(i * spacing_ms * 1_000_000, Arc::new(plan), i)
            .unwrap();
        out.extend(e.drain_finished());
    }
    e.advance_to(u64::MAX / 4).unwrap();
    out.extend(e.drain_finished());
    (t, out)
}

const CHAIN: &str = "version: 1\nnodes:\n  gw: { type: gateway, proc: 1ms }\n  app: { type: service, proc: 2ms }\n  db: { type: database, ops: { read: 1ms, write: 1ms, scan: 1ms } }\nedges:\n  - { from: gw, to: app, net: 1ms }\n  - { from: app, to: db, net: 1ms }\n";

/// Review finding 3: requests finished by an old, larger system must not panic an aggregator
/// built for the new, smaller one (an Apply swaps the generation before the old feed stops).
#[test]
fn requests_from_a_different_topology_are_ignored_not_indexed_out_of_range() {
    let (_, fin) = finished(CHAIN, 10, 5);
    let small = Topology::build(
        &Config::from_yaml(
            "version: 1
nodes:
  gw: { type: gateway, proc: 1ms }
  app: { type: service, proc: 2ms }
edges:
  - { from: gw, to: app, net: 1ms }
",
        )
        .unwrap(),
    )
    .unwrap();
    let mut a = Aggregator::new(&small);
    for f in &fin {
        a.add(f, false);
    }
    let tick = a.take((250, 250), &[(0, 0), (0, 0)], &small, STATUS);
    assert_eq!(tick.nodes.len(), 2);
}

/// Deferred from 8A: after an Apply the old engine still finishes requests; with a new system
/// of the same size they would be summed into nodes they never touched.
#[test]
fn what_the_old_engine_finishes_after_an_apply_is_dropped_not_counted() {
    let (t, fin) = finished(CHAIN, 6, 5);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    for f in &fin {
        tx.send(f.clone()).unwrap();
    }
    let mut a = Aggregator::new(&t);
    crate::live::drain_finished(&mut a, &mut rx, false);
    assert!(rx.try_recv().is_err(), "read and dropped");
    let none = a.take((250, 250), &[(0, 0); 3], &t, STATUS);
    assert_eq!(none.requests, 0);
    for f in &fin {
        tx.send(f.clone()).unwrap();
    }
    crate::live::drain_finished(&mut a, &mut rx, true);
    let some = a.take((500, 500), &[(0, 0); 3], &t, STATUS);
    assert_eq!(some.requests, 6);
}

#[test]
fn measured_cpu_use_is_the_gained_busy_area_over_cores_copies_and_span() {
    let t = Topology::build(&Config::from_yaml(CHAIN).unwrap()).unwrap();
    let t2 = Topology::build(
        &Config::from_yaml(&CHAIN.replace(
            "app: { type: service, proc: 2ms }",
            "app: { type: service, proc: 2ms, cpu: { cores: 2 }, replicas: 2 }",
        ))
        .unwrap(),
    )
    .unwrap();
    let app = t2.id("app").unwrap();
    // 4 cores in all, a quarter second: 1 core-second of busy area is 25% use
    let mut after = vec![0u128; 3];
    after[app] = 1_000_000_000_000; // 1 core-second in millicore-ns
    let u = cpu_use(&t2, &[0, 0, 0], &after, 1_000_000_000);
    assert!((u[app].unwrap() - 0.25).abs() < 1e-12);
    assert_eq!(u[0], None, "no cpu, no figure");
    // more than the cores could do in the span reads as 1, a span of zero as nothing
    after[app] = u128::MAX / 4;
    assert_eq!(cpu_use(&t2, &[0; 3], &after, 1_000_000_000)[app], Some(1.0));
    assert_eq!(cpu_use(&t2, &[0; 3], &after, 0)[app], None);
    assert!(cpu_use(&t, &[0; 3], &[0; 3], 1)[1].is_none());
}

#[test]
fn a_tick_counts_sum_to_the_requests_the_engine_finished() {
    let (t, fin) = finished(CHAIN, 40, 5);
    let mut a = Aggregator::new(&t);
    for f in &fin {
        a.add(f, false);
    }
    let tick = a.take((250, 250), &[(0, 0), (0, 0), (0, 0)], &t, STATUS);
    assert_eq!(tick.requests, 40);
    assert_eq!(tick.failed, 0);
    let hist: u32 = tick.hist.iter().map(|(_, c)| c).sum();
    assert_eq!(hist, 40, "every served request is in the histogram");
    // every request ran its own hop once at each node, and crossed each wire once
    assert!(
        tick.nodes.iter().all(|n| n.arrived == 40),
        "{:?}",
        tick.nodes
    );
    assert_eq!(
        tick.wires.iter().map(|w| w.carried).collect::<Vec<_>>(),
        vec![40, 40]
    );
    // the oracle: each crossing is the wire's constant 1 ms, and nothing shares a plain wire
    for w in &tick.wires {
        assert_eq!(w.wire_ns, u64::from(w.carried) * 1_000_000, "{w:?}");
        assert_eq!(w.wait_ns, 0, "{w:?}");
    }
    // the next tick starts empty
    let next = a.take((500, 500), &[], &t, STATUS);
    assert_eq!((next.requests, next.hist.len()), (0, 0));
}

#[test]
fn drops_are_counted_by_cause_at_the_node_that_dropped() {
    // one slot and no queue at the app: simultaneous requests overflow there
    let yaml = CHAIN.replace(
        "app: { type: service, proc: 2ms }",
        "app: { type: service, proc: 20ms, concurrency: { limit: 1, queue: 0 } }",
    );
    let (t, fin) = finished(&yaml, 10, 0);
    let mut a = Aggregator::new(&t);
    for f in &fin {
        a.add(f, false);
    }
    let tick = a.take((250, 250), &[(0, 0); 3], &t, STATUS);
    let app = t.id("app").unwrap();
    assert!(tick.failed > 0 && tick.requests == 10);
    assert_eq!(tick.nodes[app].dropped.overflow, tick.failed);
    assert_eq!(tick.nodes[app].dropped.rate_limit, 0);
    let served: u32 = tick.hist.iter().map(|(_, c)| c).sum();
    assert_eq!(
        served + tick.failed,
        10,
        "a dropped request is not in the latency histogram"
    );
    assert!(
        tick.journeys.iter().any(|j| j.dropped_at.is_some()),
        "a dropped request is drawn"
    );
    // a dropped journey says why, and a served one does not
    for j in &tick.journeys {
        assert_eq!(j.cause, j.dropped_at.map(|_| "overflow"), "{j:?}");
    }
}

#[test]
fn journeys_are_capped_per_tick_and_carry_their_hops() {
    let (t, fin) = finished(CHAIN, 400, 1);
    let mut a = Aggregator::new(&t);
    for f in &fin {
        a.add(f, false);
    }
    let tick = a.take((250, 250), &[(0, 0); 3], &t, STATUS);
    assert!(
        tick.journeys.len() <= MAX_JOURNEYS,
        "{}",
        tick.journeys.len()
    );
    let j = &tick.journeys[0];
    assert!(j.hops.iter().any(|h| h.kind == "proc"));
    assert!(
        j.hops.iter().any(|h| h.kind == "net" && h.edge.is_some()),
        "{:?}",
        j.hops
    );
    assert!(
        j.hops.iter().any(|h| h.kind == "proc" && h.edge.is_some()),
        "an own hop entered over a wire names it: {:?}",
        j.hops
    );
    assert!(j.hops.windows(2).all(|w| w[0].start_ns <= w[1].start_ns));
    // the stride adapts: the next tick at the same rate samples about the cap, not every request
    for f in &fin {
        a.add(f, false);
    }
    let again = a.take((500, 500), &[(0, 0); 3], &t, STATUS);
    assert!(again.journeys.len() <= MAX_JOURNEYS);
}

#[test]
fn queues_and_cpu_load_come_from_the_engine_snapshot() {
    let yaml = CHAIN.replace(
        "app: { type: service, proc: 2ms }",
        "app: { type: service, proc: 2ms, cpu: { cores: 2 }, replicas: 2 }",
    );
    let (t, _) = finished(&yaml, 1, 0);
    let mut a = Aggregator::new(&t);
    let app = t.id("app").unwrap();
    let mut loads = vec![(0, 0); 3];
    loads[app] = (8, 3);
    let tick = a.take((250, 250), &loads, &t, STATUS);
    assert_eq!((tick.nodes[app].busy_slots, tick.nodes[app].queue), (8, 3));
    // 8 running over 2 cores x 2 copies
    assert_eq!(tick.nodes[app].cpu_load, Some(2.0));
    assert_eq!(tick.nodes[t.id("gw").unwrap()].cpu_load, None);
}

#[test]
fn real_requests_are_in_the_total_and_in_real() {
    let (t, fin) = finished(CHAIN, 8, 5);
    let mut a = Aggregator::new(&t);
    for (i, f) in fin.iter().enumerate() {
        a.add(f, i < 3);
    }
    let tick = a.take((250, 250), &[(0, 0); 3], &t, STATUS);
    assert_eq!(tick.requests, 8);
    assert_eq!(tick.real.as_ref().unwrap().requests, 3);
}

#[test]
fn real_and_generated_add_up_to_the_same_total_node_by_node_and_wire_by_wire() {
    let (t, fin) = finished(CHAIN, 40, 5);
    let mut all = Aggregator::new(&t);
    for f in &fin {
        all.add(f, false);
    }
    let all = all.take((250, 250), &[(0, 0); 3], &t, STATUS);
    let mut mixed = Aggregator::new(&t);
    for (i, f) in fin.iter().enumerate() {
        mixed.add(f, i % 4 == 0);
    }
    let m = mixed.take((250, 250), &[(0, 0); 3], &t, STATUS);
    let arrived = |t: &Tick| t.nodes.iter().map(|n| n.arrived).collect::<Vec<_>>();
    assert_eq!(arrived(&m), arrived(&all), "the total ignores the source");
    assert_eq!(m.wires, all.wires);
    let real = m.real.expect("10 real requests");
    assert_eq!(real.requests, 10);
    // each request ran one own hop at each of the 3 nodes and crossed both wires once
    assert!(
        real.nodes.iter().all(|n| n.arrived == 10),
        "{:?}",
        real.nodes
    );
    assert_eq!(
        real.wires.iter().map(|w| w.carried).collect::<Vec<_>>(),
        vec![10, 10]
    );
    let served: u32 = real.hist.iter().map(|(_, c)| c).sum();
    assert_eq!(served, 10, "a served real request is in the real histogram");
}

#[test]
fn a_dropped_real_request_is_counted_by_cause_in_real_too() {
    let yaml = CHAIN.replace(
        "app: { type: service, proc: 2ms }",
        "app: { type: service, proc: 20ms, concurrency: { limit: 1, queue: 0 } }",
    );
    let (t, fin) = finished(&yaml, 10, 0);
    let mut a = Aggregator::new(&t);
    for f in &fin {
        a.add(f, true);
    }
    let tick = a.take((250, 250), &[(0, 0); 3], &t, STATUS);
    let real = tick.real.unwrap();
    let app = t.id("app").unwrap();
    assert!(real.failed > 0);
    assert_eq!(real.nodes[app].dropped.overflow, real.failed);
    assert_eq!((real.requests, real.failed), (tick.requests, tick.failed));
}

#[test]
fn with_no_real_request_real_is_none_and_the_json_has_no_real_key() {
    let (t, fin) = finished(CHAIN, 5, 5);
    let mut a = Aggregator::new(&t);
    for f in &fin {
        a.add(f, false);
    }
    let tick = a.take((250, 250), &[(0, 0); 3], &t, STATUS);
    assert!(tick.real.is_none());
    let v = serde_json::to_value(&tick).unwrap();
    assert!(v.get("real").is_none());
    assert!(
        v["journeys"]
            .as_array()
            .unwrap()
            .iter()
            .all(|j| j.get("real").is_none()),
        "a generated journey has no real flag"
    );
}

#[test]
fn real_journeys_are_flagged_and_capped_apart_from_generated() {
    let (t, fin) = finished(CHAIN, 1000, 1);
    let mut a = Aggregator::new(&t);
    for (i, f) in fin.iter().enumerate() {
        a.add(f, i < 30);
    }
    let tick = a.take((250, 250), &[(0, 0); 3], &t, STATUS);
    let real = tick.journeys.iter().filter(|j| j.real).count();
    let generated = tick.journeys.len() - real;
    assert_eq!(real, MAX_REAL_JOURNEYS);
    assert!(generated <= MAX_JOURNEYS && generated > 0, "{generated}");
}

#[test]
fn a_real_request_of_another_topology_is_ignored_in_both_sets() {
    let (_, fin) = finished(CHAIN, 10, 5);
    let small = Topology::build(
        &Config::from_yaml(
            "version: 1
nodes:
  gw: { type: gateway, proc: 1ms }
  app: { type: service, proc: 2ms }
edges:
  - { from: gw, to: app, net: 1ms }
",
        )
        .unwrap(),
    )
    .unwrap();
    let mut a = Aggregator::new(&small);
    for f in &fin {
        a.add(f, true);
    }
    let tick = a.take((250, 250), &[(0, 0); 2], &small, STATUS);
    assert_eq!(tick.requests, 0);
    assert!(tick.real.is_none());
}

#[test]
fn each_nodes_work_times_are_in_its_histogram_at_the_work_it_was_given() {
    let (t, fin) = finished(CHAIN, 200, 5);
    let mut a = Aggregator::new(&t);
    for f in &fin {
        a.add(f, false);
    }
    let tick = a.take((250, 250), &[(0, 0); 3], &t, STATUS);
    for n in &tick.nodes {
        let recorded: u32 = n.hist.iter().map(|(_, c)| c).sum();
        assert_eq!(recorded, n.arrived, "one work time per own hop");
    }
    // the oracle: the app works a constant 2 ms, so its median is 2 ms, to the bucket's 1%
    let app = t.id("app").unwrap();
    let h = LatencyHist::from_sparse(&tick.nodes[app].hist);
    let p50 = f64::from(u32::try_from(h.quantile(0.5)).unwrap());
    assert!((p50 / 2e6 - 1.0).abs() < 0.02, "p50 {p50} ns");
}

#[test]
fn hops_are_split_by_copy_only_for_a_node_that_has_several() {
    let yaml = CHAIN.replace(
        "app: { type: service, proc: 2ms }",
        "app: { type: service, proc: 2ms, replicas: 3 }",
    );
    let (t, fin) = finished(&yaml, 300, 2);
    let mut a = Aggregator::new(&t);
    for f in &fin {
        a.add(f, false);
    }
    let tick = a.take((250, 250), &[(0, 0); 3], &t, STATUS);
    let app = &tick.nodes[t.id("app").unwrap()];
    assert_eq!(
        app.replicas.iter().sum::<u32>(),
        app.arrived,
        "every hop ran on a copy"
    );
    assert!(app.replicas.len() > 1, "{:?}", app.replicas);
    assert!(
        tick.nodes[t.id("db").unwrap()].replicas.is_empty(),
        "one copy: nothing to split"
    );
}

#[test]
fn retries_on_a_wire_match_the_retry_hops_the_engine_ran() {
    // one slot and no queue at the app: simultaneous requests overflow there and the wire retries
    let yaml = CHAIN
        .replace(
            "app: { type: service, proc: 2ms }",
            "app: { type: service, proc: 20ms, concurrency: { limit: 1, queue: 0 } }",
        )
        .replace(
            "{ from: gw, to: app, net: 1ms }",
            "{ from: gw, to: app, net: 1ms, retry: { max_attempts: 3 } }",
        );
    let (t, fin) = finished(&yaml, 10, 0);
    let app = t.id("app").unwrap();
    // the oracle, counted from the engine's own records and not by the aggregator
    let ran = fin
        .iter()
        .flat_map(|f| &f.hops)
        .filter(|h| h.attempt > 1 && h.hop.node == app && matches!(h.hop.kind, HopKind::Proc))
        .count();
    let second = fin
        .iter()
        .flat_map(|f| &f.hops)
        .filter(|h| h.attempt == 2 && h.hop.node == app && matches!(h.hop.kind, HopKind::Proc))
        .count();
    assert!(
        ran > 0 && second > 0,
        "the fixture must retry, or this proves nothing"
    );
    let mut a = Aggregator::new(&t);
    for f in &fin {
        a.add(f, false);
    }
    let tick = a.take((250, 250), &[(0, 0); 3], &t, STATUS);
    let w = &tick.wires[0];
    assert_eq!(
        (w.retries as usize, w.retried as usize),
        (ran, second),
        "{w:?}"
    );
}

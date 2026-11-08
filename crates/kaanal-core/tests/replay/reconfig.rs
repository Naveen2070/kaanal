//! Step 8L: a running engine is changed in place (`Engine::live`, `Engine::reconfigure`).

#![expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the theory runs turn counts and times into f64 and back"
)]

use crate::common::*;
use kaanal_core::replay::Engine;
use kaanal_core::{Hop, HopKind, PlanItem, Topology};
use std::time::Duration;

const TWO_COPIES: &str =
    "version: 1\nnodes:\n  app: { type: service, replicas: 2, concurrency: 1 }\n";

fn hop_on(node: usize, replica: u16, ms: u64) -> PlanItem {
    PlanItem::Hop(Hop {
        node,
        replica,
        kind: HopKind::Proc,
        start: Duration::ZERO,
        dur: Duration::from_millis(ms),
    })
}

fn t_of(app: &str) -> Topology {
    topo(&format!(
        "version: 1\nnodes:\n  app: {{ type: service, {app} }}\n"
    ))
}

/// `n` plans of one 100 ms hop at `app`, submitted at 0.
fn burst(e: &mut Engine, app: usize, n: u64) {
    for i in 0..n {
        e.submit(0, plan(vec![hop(app, HopKind::Proc, 100)]), i)
            .unwrap();
    }
}

fn sorted_done(f: &[kaanal_core::replay::Finished]) -> Vec<u64> {
    let mut done: Vec<u64> = f.iter().map(|x| x.done).collect();
    done.sort_unstable();
    done
}

#[test]
fn a_live_engine_runs_a_hop_on_a_copy_beyond_the_configured_count() {
    let t = topo(TWO_COPIES);
    let app = t.id("app").unwrap();
    let mut e = Engine::live(&t);
    e.submit(0, plan(vec![hop_on(app, 7, 10)]), 0).unwrap();
    let f = run_all(&mut e);
    assert_eq!(fin(&f, 0).done, ms(10));
}

#[test]
fn a_structure_change_is_refused() {
    let base = |nodes: &str, edge: &str| {
        topo(&format!(
            "version: 1
nodes:
  gw: {{ type: gateway }}
{nodes}edges:
  - {{ from: gw, to: {edge} }}
"
        ))
    };
    let t = base(
        "  app: { type: service }
  web: { type: service }
",
        "app",
    );
    let mut e = Engine::live(&t);
    let more = base(
        "  app: { type: service }
  web: { type: service }
  db: { type: service }
",
        "app",
    );
    assert!(e.reconfigure(0, &more).is_err(), "an extra component");
    let renamed = base(
        "  api: { type: service }
  web: { type: service }
",
        "api",
    );
    assert!(e.reconfigure(0, &renamed).is_err(), "a renamed component");
    let rewired = base(
        "  app: { type: service }
  web: { type: service }
",
        "web",
    );
    assert!(e.reconfigure(0, &rewired).is_err(), "a different wire");
    let kind = base(
        "  app: { type: monolith }
  web: { type: service }
",
        "app",
    );
    assert!(e.reconfigure(0, &kind).is_err(), "a different kind");
    // a refusal leaves the engine as it was, and it still takes a parameter-only change
    assert!(e.reconfigure(0, &t).is_ok());
}

#[test]
fn queue_length_is_the_same_just_after_a_change() {
    let t = t_of("concurrency: 2");
    let app = t.id("app").unwrap();
    let mut e = Engine::live(&t);
    burst(&mut e, app, 6);
    assert_eq!(e.node_load(app), (2, 4));
    e.reconfigure(ms(10), &t_of("concurrency: 2, cpu: { cores: 8 }"))
        .unwrap();
    assert_eq!(e.node_load(app), (2, 4), "no jump when a parameter changes");
}

#[test]
fn slots_8_to_2_with_8_running_finishes_all_and_idles() {
    let t = t_of("concurrency: 8");
    let app = t.id("app").unwrap();
    let mut e = Engine::live(&t);
    burst(&mut e, app, 12);
    assert_eq!(e.node_load(app), (8, 4));
    e.reconfigure(ms(10), &t_of("concurrency: 2")).unwrap();
    assert_eq!(e.node_load(app).0, 8, "running work is never preempted");
    let f = run_all(&mut e);
    // 8 finish at 100 ms; slots close until 2 are left, so the 4 waiters run two at a time
    let want: Vec<u64> = [vec![100; 8], vec![200; 2], vec![300; 2]]
        .concat()
        .into_iter()
        .map(ms)
        .collect();
    assert_eq!(sorted_done(&f), want);
    assert_eq!(e.node_load(app), (0, 0), "nothing leaked");
}

#[test]
fn raising_slots_starts_the_queue_at_once() {
    let t = t_of("concurrency: 1");
    let app = t.id("app").unwrap();
    let mut e = Engine::live(&t);
    burst(&mut e, app, 4);
    e.reconfigure(ms(10), &t_of("concurrency: 4")).unwrap();
    assert_eq!(e.node_load(app), (4, 0));
    let f = run_all(&mut e);
    assert_eq!(sorted_done(&f), vec![ms(100), ms(110), ms(110), ms(110)]);
    assert_eq!(e.node_load(app), (0, 0));
}

#[test]
fn copies_3_to_1_with_in_flight_work_drains() {
    let t = t_of("replicas: 3, concurrency: 1");
    let app = t.id("app").unwrap();
    let mut e = Engine::live(&t);
    for r in 0..3u16 {
        e.submit(0, plan(vec![hop_on(app, r, 50)]), u64::from(r))
            .unwrap();
    }
    e.reconfigure(ms(10), &t_of("replicas: 1, concurrency: 1"))
        .unwrap();
    let f = run_all(&mut e);
    assert!(
        f.iter().all(|x| x.done == ms(50)),
        "in-flight work on removed copies finishes"
    );
    assert_eq!(e.node_load(app), (0, 0));
}

#[test]
fn cores_changed_under_running_work_re_times_it_exactly() {
    // 1 core, two 10 ms hops from 0: at 4 ms each has done 2 ms; the node drops to half a core:
    // 8 ms left each at speed 500 / (1000 x 2 hops) = a quarter, so both end at 4 + 32 = 36 ms
    let t = t_of("cpu: { cores: 1 }");
    let app = t.id("app").unwrap();
    let mut e = Engine::live(&t);
    for i in 0..2 {
        e.submit(0, plan(vec![hop(app, HopKind::Proc, 10)]), i)
            .unwrap();
    }
    e.reconfigure(ms(4), &t_of("cpu: { cores: 0.5 }")).unwrap();
    let f = run_all(&mut e);
    assert_eq!(sorted_done(&f), vec![ms(36), ms(36)]);
    assert_eq!(e.node_load(app), (0, 0));
}

#[test]
fn removing_the_cores_lets_running_work_finish_at_full_speed() {
    let t = t_of("cpu: { cores: 1 }");
    let app = t.id("app").unwrap();
    let mut e = Engine::live(&t);
    for i in 0..2 {
        e.submit(0, plan(vec![hop(app, HopKind::Proc, 10)]), i)
            .unwrap();
    }
    e.reconfigure(ms(4), &t_of("concurrency: 100")).unwrap();
    let f = run_all(&mut e);
    // 2 ms each done at 4 ms, 8 ms left at full speed
    assert_eq!(sorted_done(&f), vec![ms(12), ms(12)]);
}

#[test]
fn values_at_the_limits_do_not_break_the_engine() {
    let t = t_of("concurrency: 4");
    let app = t.id("app").unwrap();
    let mut e = Engine::live(&t);
    burst(&mut e, app, 6);
    let steps = [
        "concurrency: 1, cpu: { cores: 0.001 }",
        "concurrency: 100000, cpu: { cores: 4096 }",
        "concurrency: 4",
    ];
    for (i, fields) in steps.into_iter().enumerate() {
        e.reconfigure(ms(10 + i as u64), &t_of(fields)).unwrap();
    }
    let f = run_all(&mut e);
    assert_eq!(f.len(), 6);
    assert_eq!(e.node_load(app), (0, 0));
}

#[test]
fn a_wire_bandwidth_change_re_times_a_transfer_in_flight() {
    use kaanal_core::chain::{EndpointsInput, RouteSpec, compile};
    use kaanal_core::{ChainRequest, EffectiveChaos, sample_plan};
    let yaml = |bw: u64| {
        format!(
            "version: 1\nnodes:\n  gw: {{ type: gateway }}\n  app: {{ type: service }}\nedges:\n  - {{ from: gw, to: app, bandwidth: {bw} }}\n"
        )
    };
    let t = topo(&yaml(1_000_000));
    let eps = compile(
        &t,
        &[RouteSpec::inferred("GET /x/{id}", t.id("app").unwrap())],
        EndpointsInput::default(),
    )
    .unwrap();
    let ep = eps.get("GET /x/{id}").unwrap();
    let (p, _) = sample_plan(
        &t,
        &EffectiveChaos::default(),
        &ChainRequest {
            at: Duration::ZERO,
            endpoint: ep,
            n: 0,
            seed: 1,
            pin_delay: Duration::ZERO,
            req_bytes: 1_000_000,
            resp_bytes: 0,
        },
    );
    let mut e = Engine::live(&t);
    e.submit(0, std::sync::Arc::new(p), 0).unwrap();
    // 1 MB over 1 MB/s is 1 s; at 0.5 s the wire doubles: 0.5 MB left at 2 MB/s takes 0.25 s more
    e.reconfigure(ms(500), &topo(&yaml(2_000_000))).unwrap();
    let f = run_all(&mut e);
    // the request carries a few hundred header bytes on top of the 1 MB body (0.5 ms at 2 MB/s)
    let done = fin(&f, 0).done;
    assert!(done.abs_diff(ms(750)) < ms(1), "{done}");
}

/// A small deterministic generator for the theory runs (xorshift64*).
struct Rng(u64);

impl Rng {
    fn unit(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        let v = self.0.wrapping_mul(0x2545_F491_4F6C_DD1D);
        ((v >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    }

    fn exp(&mut self, mean: f64) -> f64 {
        -mean * self.unit().ln()
    }
}

/// Poisson arrivals at `lam` a second with exponential service of mean `s` seconds through one
/// node on `cores` cores; at `switch_s` the cores become `after`. Returns `(arrival, done)` of
/// every finished request, the requests running at the switch, and those running at `end_s`.
fn mg1_run(
    cores: f64,
    after: f64,
    lam: f64,
    s: f64,
    switch_s: f64,
    end_s: f64,
) -> (Vec<(u64, u64)>, u32, u32) {
    let t = t_of(&format!("cpu: {{ cores: {cores} }}"));
    let t2 = t_of(&format!("cpu: {{ cores: {after} }}"));
    let app = t.id("app").unwrap();
    let mut e = Engine::live(&t);
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let (mut at, mut tag, mut done) = (0.0_f64, 0u64, Vec::new());
    let (mut switched, mut at_switch) = (false, 0u32);
    let ns = |x: f64| (x * 1e9) as u64;
    while at < end_s {
        at += rng.exp(1.0 / lam);
        if !switched && at >= switch_s {
            e.advance_to(ns(switch_s)).unwrap();
            at_switch = e.node_load(app).0;
            e.reconfigure(ns(switch_s), &t2).unwrap();
            switched = true;
        }
        let dur = Duration::from_secs_f64(rng.exp(s));
        let p = plan(vec![PlanItem::Hop(Hop {
            node: app,
            replica: 0,
            kind: HopKind::Proc,
            start: Duration::ZERO,
            dur,
        })]);
        e.submit(ns(at), p, tag).unwrap();
        tag += 1;
        done.extend(e.drain_finished().into_iter().map(|f| (f.arrival, f.done)));
    }
    e.advance_to(ns(end_s)).unwrap();
    let running = e.node_load(app).0;
    done.extend(e.drain_finished().into_iter().map(|f| (f.arrival, f.done)));
    (done, at_switch, running)
}

#[test]
fn cores_changed_mid_run_match_mg1_ps_afterwards() {
    // 0.7 core (a server of speed 0.7, load 0.71); at 100 s the node gets 1 core: the load is 0.5 and
    // the mean sojourn E[S] / (1 - 0.5) = 20 ms (M/G/1-PS, Kleinrock vol. 1 s4.4)
    let (done, _, _) = mg1_run(0.7, 1.0, 50.0, 0.010, 100.0, 1000.0);
    let late: Vec<f64> = done
        .iter()
        .filter(|(a, _)| *a >= 120_000_000_000)
        .map(|(a, d)| (d - a) as f64 / 1e9)
        .collect();
    let mean = late.iter().sum::<f64>() / late.len() as f64;
    let want = 0.010 / (1.0 - 0.5);
    assert!(
        (mean - want).abs() / want < 0.05,
        "mean {mean} want {want} over {} requests",
        late.len()
    );
}

#[test]
fn cores_cut_below_the_load_make_the_queue_grow_at_lambda_minus_mu() {
    // 1 core at load 0.5, then 0.4 core: service capacity mu = 0.4 / 10 ms = 40 a second under
    // lambda = 50, so the number in the system grows by lambda - mu = 10 a second
    let (_, at_switch, at_end) = mg1_run(1.0, 0.4, 50.0, 0.010, 100.0, 200.0);
    let grown = f64::from(at_end) - f64::from(at_switch);
    assert!(
        (grown - 1000.0).abs() / 1000.0 < 0.15,
        "grew by {grown}, want about 1000"
    );
}

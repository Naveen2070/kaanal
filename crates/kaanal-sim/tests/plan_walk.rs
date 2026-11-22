//! The walk (Step 9c-1): the planner's steps from the design in force to one that holds a target,
//! against closed forms. M/M/c fixtures (exponential service, Poisson arrivals), seed 7; capacity of
//! an M/M/c node is c x mu (the stability bound of Erlang C: rho = lambda / (c mu) < 1).

use kaanal_core::chain::{EndpointsInput, RouteSpec, compile};
use kaanal_core::resize::{MoveKind, moves_for};
use kaanal_core::{Config, EffectiveChaos, Endpoints, Topology};
use kaanal_sim::plan::{Plan, PlanVerdict, Request, Step, Target, walk};
use kaanal_sim::{Limits, SimConfig, Stream};
use std::time::Duration;

const MU: f64 = 100.0; // 10 ms mean service per slot

/// A gateway and a pass-through service (no work of its own) in front of `x` (its YAML line); the
/// one route ends at `x`.
fn design(x: &str) -> Config {
    Config::from_yaml(&format!(
        "version: 1\nnodes:\n  gw: {{ type: gateway }}\n  app: {{ type: service }}\n{x}edges:\n  - {{ from: gw, to: app }}\n  - {{ from: app, to: x }}\n"
    ))
    .unwrap()
}

/// An M/M/1 database: one slot, every op 10 ms exponential.
fn saturated_db() -> Config {
    design(
        "  x: { type: database, concurrency: 1, ops: { read: { mean: 10ms }, write: { mean: 10ms }, scan: { mean: 10ms } } }\n",
    )
}

fn endpoints(cfg: &Config) -> Endpoints {
    let t = Topology::build(cfg).unwrap();
    let spec = RouteSpec::inferred("GET /x/{id}", t.id("app").unwrap());
    compile(&t, &[spec], EndpointsInput::default()).unwrap()
}

fn base(rate: f64) -> SimConfig {
    SimConfig {
        streams: vec![Stream {
            endpoint_key: "GET /x/{id}".into(),
            rate_per_s: rate,
            req_bytes: 0,
            resp_bytes: 0,
        }],
        duration: Duration::from_secs(1),
        warmup: Duration::ZERO,
        seed: 7,
    }
}

fn req(k: f64) -> Request {
    Request {
        target: Target::Multiple(k),
        ..Request::default()
    }
}

fn plan_with(cfg: &Config, rate: f64, r: &Request, seen: &mut Vec<Step>) -> Plan {
    walk(
        cfg,
        &endpoints(cfg),
        &EffectiveChaos::default(),
        &base(rate),
        r,
        &Limits::default(),
        &mut |s| seen.push(s.clone()),
    )
    .unwrap()
}

fn plan(cfg: &Config, rate: f64, r: &Request) -> Plan {
    plan_with(cfg, rate, r, &mut Vec::new())
}

fn copies(p: &Plan, node: &str) -> u16 {
    p.config.nodes[node].replicas()
}

/// Erlang C: probability an arrival waits in M/M/c (offered load `a = lambda / mu`).
fn erlang_c(c: u32, a: f64) -> f64 {
    let (mut sum, mut term) = (0.0, 1.0); // term = a^k / k!
    for k in 0..c {
        if k > 0 {
            term *= a / f64::from(k);
        }
        sum += term;
    }
    let tail = term * a / f64::from(c) / (1.0 - a / f64::from(c));
    tail / (sum + tail)
}

#[test]
fn already_holding_the_target_gives_no_steps() {
    let p = plan(&saturated_db(), 50.0, &req(1.0));
    assert!(
        matches!(p.verdict, PlanVerdict::AlreadyHolds),
        "{:?}",
        p.verdict
    );
    assert_eq!(p.steps, []);
    assert!(p.families.is_empty());
    assert_eq!(copies(&p, "x"), 1);
}

/// 250 req/s on mu = 100 per copy: Erlang C needs c > 2.5 for a steady state, and at c = 3 the
/// chance a request waits more than the 900 ms margin is C(3, 2.5) e^{-(3 mu - lambda) 0.9},
/// far under 1 percent, so the walk must stop at exactly 3 copies. The capacity estimate after the
/// walk is c mu = 300 req/s, within 5 percent (the theory-test bound).
#[test]
fn a_saturated_database_gets_copies_until_the_target_holds() {
    let lam = 250.0;
    let mut seen = Vec::new();
    let p = plan_with(&saturated_db(), 100.0, &req(lam / 100.0), &mut seen);
    assert!(matches!(p.verdict, PlanVerdict::Holds), "{:?}", p.verdict);
    // the oracle: the fewest copies that are stable and keep P(wait > 0.9 s) under 1 percent
    let holds = |c: u32| {
        let c_mu = f64::from(c) * MU;
        c_mu > lam && erlang_c(c, lam / MU) * (-(c_mu - lam) * 0.9).exp() < 0.01
    };
    let c = (1..=64u32).find(|&c| holds(c)).unwrap();
    assert_eq!(u32::from(copies(&p, "x")), c);
    let cap = p.end.capacity_req_s.unwrap();
    let want = f64::from(c) * MU;
    assert!((cap - want).abs() / want < 0.05, "{cap} vs {want}");
    // every step a copy on the database, emitted as it was taken
    assert_eq!(p.steps.len(), 2);
    assert!(
        p.steps
            .iter()
            .all(|s| s.mv.node == "x" && matches!(s.mv.kind, MoveKind::Copies { .. }))
    );
    assert_eq!(format!("{seen:?}"), format!("{:?}", p.steps));
    let start = p.start.capacity_req_s.unwrap();
    assert!((start - MU).abs() / MU < 0.05, "{start}");
}

#[test]
fn the_menu_lists_every_move_tried_for_the_chosen_node() {
    let cfg =
        design("  x: { type: service, concurrency: 1, proc: { mean: 10ms }, cpu: { cores: 1 } }\n");
    let p = plan(&cfg, 100.0, &req(2.5));
    let first = &p.steps[0];
    let kinds: Vec<_> = first.menu.iter().map(|c| c.mv.clone()).collect();
    assert_eq!(kinds, moves_for(&cfg, "x", &[]));
    assert_eq!(kinds.len(), 2, "cores and a copy");
    assert!(first.menu.iter().any(|c| c.mv == first.mv));
}

#[test]
fn an_unsizable_bottleneck_says_so_and_names_it() {
    let cfg = design("  x: { type: external-api, concurrency: 1, proc: { mean: 10ms } }\n");
    let p = plan(&cfg, 100.0, &req(2.0));
    match &p.verdict {
        PlanVerdict::CannotHold { node, reason } => {
            assert_eq!(node.as_deref(), Some("x"));
            assert!(reason.contains("external"), "{reason}");
        }
        v => panic!("{v:?}"),
    }
    assert_eq!(p.steps, []);
}

/// Copies only (`cpu` written next to the instance), priced: n copies cost n x the unit.
fn priced() -> Config {
    design(
        "  x: { type: service, concurrency: 1, proc: { mean: 10ms }, instance: aws/m5.large, cpu: { cores: 2 } }\n",
    )
}

fn unit_price() -> u64 {
    let c = kaanal_core::cost::estimate(&priced(), &std::collections::BTreeMap::new());
    c.nodes
        .iter()
        .find(|n| n.name == "x")
        .unwrap()
        .unit_micro_usd_hr
        .unwrap()
}

#[test]
fn a_budget_below_the_plan_is_reported_over_budget_with_the_difference() {
    let u = unit_price();
    let r = Request {
        budget_micro_usd_hr: Some(2 * u),
        ..req(2.5)
    };
    let mut seen = Vec::new();
    let p = plan_with(&priced(), 100.0, &r, &mut seen);
    assert_eq!(seen, p.steps, "the steps sent are the plan's");
    // the plan that holds needs 3 copies (Erlang C, above): 3u, one unit over
    match p.verdict {
        PlanVerdict::OverBudget {
            over_by_micro_usd_hr,
            holds,
        } => {
            assert_eq!(over_by_micro_usd_hr, u);
            assert!(holds);
        }
        ref v => panic!("{v:?}"),
    }
    assert_eq!(copies(&p, "x"), 3);
    assert_eq!(p.price_after, Some(3 * u));
    assert_eq!(p.price_before, Some(u));
    assert_eq!(
        p.steps.last().unwrap().price_delta_micro_usd_hr,
        Some(i64::try_from(u).unwrap())
    );
    // a budget that covers it holds
    let r = Request {
        budget_micro_usd_hr: Some(3 * u),
        ..req(2.5)
    };
    assert!(matches!(
        plan(&priced(), 100.0, &r).verdict,
        PlanVerdict::Holds
    ));
}

#[test]
fn unpriced_nodes_are_listed_and_left_out_of_the_budget() {
    let r = Request {
        budget_micro_usd_hr: Some(0),
        ..req(2.5)
    };
    let p = plan(&saturated_db(), 100.0, &r);
    assert!(matches!(p.verdict, PlanVerdict::Holds), "{:?}", p.verdict);
    assert_eq!(p.unpriced_nodes, ["gw", "app", "x"]);
    assert_eq!((p.price_before, p.price_after), (None, None));
    assert!(
        p.steps
            .iter()
            .all(|s| s.price_micro_usd_hr.is_none() && s.price_delta_micro_usd_hr.is_none())
    );
    // the priced design lists all but `x`
    let p = plan(&priced(), 100.0, &req(2.5));
    assert_eq!(p.unpriced_nodes, ["gw", "app"]);
}

/// A rate limit of 50 req/s at the gateway turns away 1 - 50/80 of the arrivals whatever the copies
/// behind it (a token bucket passes exactly its rps under overload). The cause is a limit you set, so
/// the walk sizes nothing and names the gateway (spec: it stops at that node instead of raising it).
#[test]
fn a_rate_limit_you_set_cannot_hold_and_names_its_node() {
    let cfg = Config::from_yaml(
        "version: 1\nnodes:\n  gw: { type: gateway, rate_limit: { rps: 50, burst: 5 } }\n  app: { type: service }\n  x: { type: service, concurrency: 1, proc: { mean: 10ms } }\nedges:\n  - { from: gw, to: app }\n  - { from: app, to: x }\n",
    )
    .unwrap();
    let p = plan(&cfg, 80.0, &req(1.0));
    match &p.verdict {
        PlanVerdict::CannotHold { node, reason } => {
            assert_eq!(node.as_deref(), Some("gw"));
            assert!(reason.contains("rate limit you set"), "{reason}");
        }
        v => panic!("{v:?}"),
    }
    assert_eq!(p.steps, []);
    assert_eq!(copies(&p, "x"), 1);
    let want = 1.0 - 50.0 / 80.0;
    assert!(
        (p.end.error_rate - want).abs() / want < 0.05,
        "{}",
        p.end.error_rate
    );
}

/// A fixed 2 s service with 64 slots at 1 req/s never queues (capacity 64 / 2 s = 32 req/s), so its
/// p99 is 2 s plus the fixed wires whatever the copies: over the 1 s limit, and each copy gains
/// exactly nothing. Two steps without gain end the walk; neither is proposed.
#[test]
fn two_steps_without_progress_stop_the_walk() {
    let cfg = design("  x: { type: service, concurrency: 64, proc: 2s }\n");
    let p = plan(&cfg, 1.0, &req(1.0));
    match &p.verdict {
        PlanVerdict::NoProgress { node } => assert_eq!(node, "x"),
        v => panic!("{v:?}"),
    }
    assert_eq!(p.steps, []);
    assert_eq!(copies(&p, "x"), 1);
    assert!(p.end.p99_ms >= 2000.0, "{}", p.end.p99_ms);
}

#[test]
fn the_step_cap_stops_the_walk() {
    let r = Request {
        max_steps: 1,
        ..req(2.5)
    };
    let p = plan(&saturated_db(), 100.0, &r);
    assert!(matches!(p.verdict, PlanVerdict::StepCap), "{:?}", p.verdict);
    assert_eq!(p.steps.len(), 1);
    assert_eq!(Request::default().max_steps, 30);
}

#[test]
fn the_walk_is_reproducible_with_one_seed() {
    let a = serde_json::to_string(&plan(&saturated_db(), 100.0, &req(2.5))).unwrap();
    let b = serde_json::to_string(&plan(&saturated_db(), 100.0, &req(2.5))).unwrap();
    assert_eq!(a, b);
}

#[test]
fn the_walk_never_changes_the_input_config() {
    let cfg = saturated_db();
    let before = format!("{cfg:?}");
    let p = plan(&cfg, 100.0, &req(2.5));
    assert_eq!(format!("{cfg:?}"), before);
    assert_ne!(format!("{:?}", p.config), before);
}

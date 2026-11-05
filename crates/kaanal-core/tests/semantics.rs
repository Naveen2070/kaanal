//! The A2 decisions (E9-E14, R1.34) as executable timelines. Costs are fixed whole
//! milliseconds, so every expected start and duration below is hand-computed.

use kaanal_core::chain::{EndpointsInput, RouteSpec, compile};
use kaanal_core::exec::PlanItem;
use kaanal_core::{
    ChainOutcome, ChainRequest, ChainResult, ChaosState, Config, Dist, Endpoints, FailKind,
    HopKind, NodeChaos, Plan, Topology, execute, resolve_error, stable_hash,
};
use rand::{Rng, SeedableRng};
use std::time::Duration;

/// 20 lines; tests append entries under `endpoints:`.
const TOPO: &str = "version: 1
nodes:
  gw: { type: gateway, proc: 1ms }
  login: { type: auth, proc: 2ms }
  app: { type: service, proc: 3ms }
  orders: { type: service, proc: 4ms }
  db: { type: database, ops: { read: 5ms, write: 6ms, scan: 7ms } }
  c: { type: cache, hit_ratio: 0.0, hit: 1ms, miss: 2ms }
  pay: { type: external-api, proc: 10ms }
edges:
  - { from: gw, to: app, net: 1ms }
  - { from: gw, to: orders, net: 1ms }
  - { from: gw, to: login, net: 1ms }
  - { from: app, to: orders, net: 1ms }
  - { from: app, to: pay, net: 1ms }
  - { from: app, to: c, net: 1ms }
  - { from: app, to: db, net: 1ms }
  - { from: orders, to: c, net: 1ms }
  - { from: orders, to: db, net: 1ms }
endpoints:
";

const ROUTES: [(&str, &str); 4] = [
    ("GET /users/{id}", "app"),
    ("POST /users", "app"),
    ("GET /orders/{id}", "orders"),
    ("POST /orders", "orders"),
];

fn build(src: &str) -> (Topology, Endpoints) {
    let cfg = Config::from_yaml(src).unwrap();
    let t = Topology::build(&cfg).unwrap();
    let routes: Vec<RouteSpec> = ROUTES
        .iter()
        .map(|(k, o)| RouteSpec::inferred(k, t.id(o).unwrap()))
        .collect();
    let eps = compile(&t, &routes, EndpointsInput::inline(&cfg)).unwrap();
    (t, eps)
}

/// `TOPO` plus endpoint entries (cache always misses).
fn eps(extra: &str) -> (Topology, Endpoints) {
    build(&format!("{TOPO}{extra}"))
}

/// `TOPO` with a cache that always hits, plus endpoint entries.
fn hot(extra: &str) -> (Topology, Endpoints) {
    build(&format!(
        "{}{extra}",
        TOPO.replace("hit_ratio: 0.0", "hit_ratio: 1.0")
    ))
}

fn chaos(t: &Topology, on: &[(&str, NodeChaos)]) -> ChaosState {
    on.iter()
        .map(|(n, c)| (t.id(n).unwrap(), c.clone()))
        .collect()
}

fn fails(rate: f64, status: u16) -> NodeChaos {
    NodeChaos {
        error_rate: rate,
        error_status: status,
        ..NodeChaos::default()
    }
}

fn run(t: &Topology, e: &Endpoints, key: &str, chaos: &ChaosState) -> ChainOutcome {
    execute(
        t,
        &chaos.compile(),
        &ChainRequest {
            endpoint: e.get(key).unwrap(),
            n: 0,
            seed: 1,
            pin_delay: std::time::Duration::ZERO,
            req_bytes: 0,
            resp_bytes: 0,
        },
    )
}

/// `node.kind@start+dur` in whole milliseconds, e.g. `gw.proc@0+1`.
fn timeline(t: &Topology, o: &ChainOutcome) -> Vec<String> {
    o.hops
        .iter()
        .map(|h| {
            let kind = match h.kind {
                HopKind::Net => "net",
                HopKind::Proc => "proc",
                HopKind::Op(op) => op.name(),
                HopKind::CacheHit => "hit",
                HopKind::CacheMiss => "miss",
                HopKind::Delay => "delay",
                HopKind::Connect { .. } => "connect",
                HopKind::FirstByte => "first-byte",
                HopKind::Retransmit { .. } => "retransmit",
                HopKind::Link { .. } => "link",
                HopKind::Nic { .. } => "nic",
            };
            format!(
                "{}.{kind}@{}+{}",
                t.nodes[h.node].name,
                h.start.as_millis(),
                h.dur.as_millis()
            )
        })
        .collect()
}

/// `H` hop, `X` fail, `F<branches>/<size>` fork, `B<size>` branch, `S<size>` span, `K<size>` hold, `P` a `least-conn` or `p2c` pick, `R<attempts>/<size>` a retry scope, `A<size>` one of its attempts.
fn shape(p: &Plan) -> String {
    p.items
        .iter()
        .map(|i| match i {
            PlanItem::Hop(_) => "H".to_string(),
            PlanItem::Fail { .. } => "X".to_string(),
            PlanItem::Fork { branches, size } => format!("F{branches}/{size}"),
            PlanItem::Branch { size } => format!("B{size}"),
            PlanItem::Span { size, .. } => format!("S{size}"),
            PlanItem::Hold { size, .. } => format!("K{size}"),
            PlanItem::Pick { .. } => "P".to_string(),
            PlanItem::Retry { attempts, size, .. } => format!("R{attempts}/{size}"),
            PlanItem::Attempt { size, .. } => format!("A{size}"),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The gateway part and the owner's proc of every `GET /users/{id}` below.
const USERS: [&str; 5] = [
    "gw.proc@0+1",
    "login.net@1+1",
    "login.proc@2+2",
    "app.net@4+1",
    "app.proc@5+3",
];

fn users(rest: &[&str]) -> Vec<String> {
    USERS.iter().chain(rest).map(|s| (*s).to_string()).collect()
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

#[test]
fn gateway_chain_runs_proc_then_auth_then_the_entry() {
    let (t, e) = eps("");
    let o = run(&t, &e, "GET /orders/{id}", &ChaosState::default());
    assert_eq!(
        timeline(&t, &o),
        [
            "gw.proc@0+1",
            "login.net@1+1",
            "login.proc@2+2",
            "orders.net@4+1",
            "orders.proc@5+4",
            "c.net@9+1",
            "c.miss@10+2",
            "db.net@12+1",
            "db.read@13+5",
        ]
    );
    assert_eq!((o.result.clone(), o.total()), (ChainResult::Ok, ms(18)));
}

#[test]
fn default_chain_inherits_the_write_op_and_skips_the_cache() {
    let (t, e) = eps("");
    let o = run(&t, &e, "POST /orders", &ChaosState::default());
    assert_eq!(
        timeline(&t, &o),
        [
            "gw.proc@0+1",
            "login.net@1+1",
            "login.proc@2+2",
            "orders.net@4+1",
            "orders.proc@5+4",
            "db.net@9+1",
            "db.write@10+6",
        ]
    );
    assert_eq!(o.total(), ms(16));
}

#[test]
fn step_op_overrides_for_that_step_only() {
    let (t, e) = eps("  \"GET /users/{id}\": { chain: [ { call: db, op: write }, db ] }\n");
    let o = run(&t, &e, "GET /users/{id}", &ChaosState::default());
    assert_eq!(
        timeline(&t, &o),
        users(&["db.net@8+1", "db.write@9+6", "db.net@15+1", "db.read@16+5"])
    );
}

#[test]
fn via_uses_the_target_endpoints_own_op() {
    let (t, e) =
        eps("  \"GET /users/{id}\": { chain: [ { call: orders, via: \"POST /orders\" } ] }\n");
    let o = run(&t, &e, "GET /users/{id}", &ChaosState::default());
    assert_eq!(
        timeline(&t, &o),
        users(&[
            "orders.net@8+1",
            "orders.proc@9+4",
            "db.net@13+1",
            "db.write@14+6"
        ]),
        "POST /orders is a write: no cache, db.write"
    );
    // The same call without `via` runs the default body with the inherited read.
    let (t, e) = eps("  \"GET /users/{id}\": { chain: [ orders ] }\n");
    let o = run(&t, &e, "GET /users/{id}", &ChaosState::default());
    assert_eq!(
        timeline(&t, &o),
        users(&[
            "orders.net@8+1",
            "orders.proc@9+4",
            "c.net@13+1",
            "c.miss@14+2",
            "db.net@16+1",
            "db.read@17+5",
        ])
    );
}

#[test]
fn error_map_belongs_to_the_served_endpoint() {
    let (t, e) = eps(r#"  "GET /users/{id}":
    chain: [ { call: orders, via: "POST /orders" } ]
    error_map: { "5xx": 502 }
  "POST /orders": { error_map: { db: 503 } }
"#);
    let c = chaos(&t, &[("db", fails(1.0, 504))]);
    let db = t.id("db").unwrap();
    let get = e.get("GET /users/{id}").unwrap();
    assert_eq!(
        run(&t, &e, "GET /users/{id}", &c).result,
        ChainResult::Err {
            node: db,
            status: 504,
            kind: FailKind::Status,
            retry_after: None
        }
    );
    assert_eq!(
        resolve_error(&get.error_map, "db", FailKind::Status, 504),
        (502, None),
        "not POST's 503"
    );
    let post = e.get("POST /orders").unwrap();
    assert_eq!(
        run(&t, &e, "POST /orders", &c).result,
        ChainResult::Err {
            node: db,
            status: 504,
            kind: FailKind::Status,
            retry_after: None
        }
    );
    assert_eq!(
        resolve_error(&post.error_map, "db", FailKind::Status, 504),
        (503, None)
    );
    assert_eq!(e.warnings.len(), 1, "{:?}", e.warnings);
}

#[test]
fn timeout_loads_on_every_node_kind_and_reaches_the_topology() {
    let cfg = Config::from_yaml(
        "version: 1\nnodes:\n  gw: { type: gateway, timeout: 2s }\n  db: { type: database, ops: { read: 1ms, write: 1ms, scan: 1ms }, timeout: 500ms }\n  app: { type: service }\nedges:\n  - { from: gw, to: app, net: 1ms }\n  - { from: app, to: db, net: 1ms }\n",
    )
    .unwrap();
    let t = Topology::build(&cfg).unwrap();
    assert_eq!(
        t.nodes[t.id("gw").unwrap()].timeout,
        Some(Duration::from_secs(2))
    );
    assert_eq!(
        t.nodes[t.id("db").unwrap()].timeout,
        Some(Duration::from_millis(500))
    );
}

#[test]
fn timeout_range_is_checked_with_a_positioned_error() {
    let at = |bad: &str| {
        let src = format!("version: 1\nnodes:\n  gw: {{ type: gateway, timeout: {bad} }}\n");
        Config::from_yaml(&src)
    };
    for bad in ["0s", "301s", "-1s", "soon"] {
        assert!(at(bad).is_err(), "{bad} must be rejected");
    }
    for (bad, want) in [("0s", "above 0"), ("301s", "at most 5 min")] {
        let e = at(bad).unwrap_err().to_string();
        assert!(e.contains(want), "{bad}: {e}");
    }
}

#[test]
fn timeout_boundaries_load_on_gateway_service_and_cache() {
    for ok in ["300s", "1ns"] {
        for node in [
            format!("gw: {{ type: gateway, timeout: {ok} }}"),
            format!("app: {{ type: service, timeout: {ok} }}"),
            format!("c: {{ type: cache, timeout: {ok} }}"),
        ] {
            let src = format!("version: 1\nnodes:\n  {node}\n");
            assert!(Config::from_yaml(&src).is_ok(), "{node} must load");
        }
    }
}

#[test]
fn cache_hit_ends_only_the_enclosing_sequence() {
    // A hit inside `orders` ends orders' body; app still calls db afterwards.
    let (t, e) = hot("  \"GET /users/{id}\": { chain: [ orders, db ] }\n");
    let o = run(&t, &e, "GET /users/{id}", &ChaosState::default());
    assert_eq!(
        timeline(&t, &o),
        users(&[
            "orders.net@8+1",
            "orders.proc@9+4",
            "c.net@13+1",
            "c.hit@14+1",
            "db.net@15+1",
            "db.read@16+5",
        ])
    );
    // A hit at the top of the endpoint's chain ends the endpoint's chain.
    let (t, e) = hot("  \"GET /users/{id}\": { chain: [ { cache: c }, db ] }\n");
    let o = run(&t, &e, "GET /users/{id}", &ChaosState::default());
    assert_eq!(timeline(&t, &o), users(&["c.net@8+1", "c.hit@9+1"]));
    assert_eq!(o.total(), ms(10));
}

#[test]
fn cache_hit_in_a_parallel_branch_ends_only_that_branch() {
    // Branch 0 hits the cache and stops; branch 1 still runs db; the block ends with the slower one.
    let (t, e) =
        hot("  \"GET /users/{id}\": { chain: [ { parallel: [ [ { cache: c }, db ], db ] } ] }\n");
    let o = run(&t, &e, "GET /users/{id}", &ChaosState::default());
    let tl = timeline(&t, &o);
    let hops: Vec<&str> = tl.iter().map(String::as_str).collect();
    // Exactly one db read (branch 1's); branch 0 ends at its hit.
    assert_eq!(
        hops.iter().filter(|h| h.starts_with("db.read")).count(),
        1,
        "{hops:?}"
    );
    assert!(hops.iter().any(|h| h.starts_with("c.hit")), "{hops:?}");
    let hit_end = o
        .hops
        .iter()
        .filter(|h| h.kind == HopKind::CacheHit)
        .map(|h| h.start + h.dur)
        .max()
        .unwrap();
    assert!(o.total() > hit_end, "the block waits for the db branch");
}

#[test]
fn cache_hit_in_a_repeated_call_ends_only_that_repetition() {
    let (t, e) = hot("  \"GET /users/{id}\": { chain: [ { call: orders, repeat: 2 } ] }\n");
    let o = run(&t, &e, "GET /users/{id}", &ChaosState::default());
    assert_eq!(
        timeline(&t, &o),
        users(&[
            "orders.net@8+1",
            "orders.proc@9+4",
            "c.net@13+1",
            "c.hit@14+1",
            "orders.net@15+1",
            "orders.proc@16+4",
            "c.net@20+1",
            "c.hit@21+1",
        ])
    );
}

#[test]
fn failing_cache_is_a_miss_not_a_failure() {
    let (t, e) = hot("");
    let want = |tail: &[&str]| -> Vec<String> {
        [
            "gw.proc@0+1",
            "login.net@1+1",
            "login.proc@2+2",
            "orders.net@4+1",
            "orders.proc@5+4",
            "c.net@9+1",
        ]
        .iter()
        .chain(tail)
        .map(|s| (*s).to_string())
        .collect()
    };
    let down = chaos(
        &t,
        &[(
            "c",
            NodeChaos {
                down: true,
                ..NodeChaos::default()
            },
        )],
    );
    let o = run(&t, &e, "GET /orders/{id}", &down);
    assert_eq!(
        timeline(&t, &o),
        want(&["c.miss@10+2", "db.net@12+1", "db.read@13+5"])
    );
    assert_eq!(o.result, ChainResult::Ok);
    let erroring = chaos(
        &t,
        &[(
            "c",
            NodeChaos {
                latency_add: ms(3),
                ..fails(1.0, 503)
            },
        )],
    );
    let o = run(&t, &e, "GET /orders/{id}", &erroring);
    assert_eq!(
        timeline(&t, &o),
        want(&["c.miss@10+5", "db.net@15+1", "db.read@16+5"])
    );
    assert_eq!(o.result, ChainResult::Ok);
}

#[test]
fn parallel_ends_with_its_slowest_branch() {
    let (t, e) = eps("  \"GET /users/{id}\": { chain: [ { parallel: [ pay, orders ] }, db ] }\n");
    let o = run(&t, &e, "GET /users/{id}", &ChaosState::default());
    assert_eq!(
        timeline(&t, &o),
        users(&[
            "pay.net@8+1",
            "pay.proc@9+10",
            "orders.net@8+1",
            "orders.proc@9+4",
            "c.net@13+1",
            "c.miss@14+2",
            "db.net@16+1",
            "db.read@17+5",
            "db.net@22+1",
            "db.read@23+5",
        ])
    );
    assert_eq!(o.total(), ms(28));
    assert_eq!(shape(&o.plan), "H H H H H F2/10 B2 H H B6 H H H H H H H H");
}

#[test]
fn parallel_fails_at_the_earliest_failure_and_cuts_siblings() {
    let (t, e) = eps("  \"GET /users/{id}\": { chain: [ { parallel: [ pay, orders ] } ] }\n");
    let o = run(
        &t,
        &e,
        "GET /users/{id}",
        &chaos(&t, &[("db", fails(1.0, 504))]),
    );
    // orders' db fails at its hop start, 17 ms; pay.proc (9..19) is cut to 8 ms.
    assert_eq!(
        timeline(&t, &o),
        users(&[
            "pay.net@8+1",
            "pay.proc@9+8",
            "orders.net@8+1",
            "orders.proc@9+4",
            "c.net@13+1",
            "c.miss@14+2",
            "db.net@16+1",
        ])
    );
    assert_eq!(
        o.result,
        ChainResult::Err {
            node: t.id("db").unwrap(),
            status: 504,
            kind: FailKind::Status,
            retry_after: None
        }
    );
    assert_eq!(o.total(), ms(17));
    assert_eq!(shape(&o.plan), "H H H H H F2/10 B2 H H B6 H H H H H X");
    assert_eq!(
        (o.kept.clone(), o.fail),
        (vec![0, 1, 2, 3, 4, 7, 8, 10, 11, 12, 13, 14], Some(15))
    );
    assert_eq!(o.plan.cancelled_by(15), vec![7..9]);
    // An earlier failure in a higher branch wins; the lower branch's later failure never does.
    let (t, e) = eps("  \"GET /users/{id}\": { chain: [ { parallel: [ orders, pay ] } ] }\n");
    let c = chaos(&t, &[("db", fails(1.0, 504)), ("pay", fails(1.0, 500))]);
    let o = run(&t, &e, "GET /users/{id}", &c);
    assert_eq!(
        timeline(&t, &o),
        users(&["orders.net@8+1", "pay.net@8+1"]),
        "orders.proc starts at 9: dropped"
    );
    assert_eq!(
        o.result,
        ChainResult::Err {
            node: t.id("pay").unwrap(),
            status: 500,
            kind: FailKind::Status,
            retry_after: None
        }
    );
    assert_eq!(o.total(), ms(9));
}

#[test]
fn parallel_tie_goes_to_the_lowest_branch() {
    for (branches, winner, status, rest) in [
        ("[ db, pay ]", "db", 504, ["db.net@8+1", "pay.net@8+1"]),
        ("[ pay, db ]", "pay", 500, ["pay.net@8+1", "db.net@8+1"]),
    ] {
        let (t, e) = eps(&format!(
            "  \"GET /users/{{id}}\": {{ chain: [ {{ parallel: {branches} }} ] }}\n"
        ));
        let c = chaos(&t, &[("db", fails(1.0, 504)), ("pay", fails(1.0, 500))]);
        let o = run(&t, &e, "GET /users/{id}", &c);
        assert_eq!(
            o.result,
            ChainResult::Err {
                node: t.id(winner).unwrap(),
                status,
                kind: FailKind::Status,
                retry_after: None,
            },
            "{branches}"
        );
        assert_eq!(timeline(&t, &o), users(&rest), "{branches}");
    }
}

/// `TOPO` with random costs on the hops the determinism tests compare.
fn random(extra: &str) -> (Topology, Endpoints) {
    build(&format!(
        "{}{extra}",
        TOPO.replace("proc: 10ms", "proc: { mean: 10ms }")
            .replace("read: 5ms", "read: { mean: 5ms }")
            .replace(
                "{ from: app, to: db, net: 1ms }",
                "{ from: app, to: db, net: { mean: 1ms } }"
            )
    ))
}

/// `(node, kind, dur)` of the hops at `node`, in order.
fn draws(t: &Topology, o: &ChainOutcome, node: &str) -> Vec<(HopKind, Duration)> {
    let id = t.id(node).unwrap();
    o.hops
        .iter()
        .filter(|h| h.node == id)
        .map(|h| (h.kind, h.dur))
        .collect()
}

#[test]
fn repeat_runs_in_sequence_with_independent_draws() {
    let (t, e) = random("  \"GET /users/{id}\": { chain: [ { call: pay, repeat: 3 } ] }\n");
    let o = run(&t, &e, "GET /users/{id}", &ChaosState::default());
    let pay: Vec<_> = o
        .hops
        .iter()
        .filter(|h| h.node == t.id("pay").unwrap())
        .collect();
    assert_eq!(pay.len(), 6, "net + proc per repetition");
    for w in pay.windows(2) {
        assert_eq!(w[1].start, w[0].start + w[0].dur, "in sequence");
    }
    let procs: Vec<Duration> = pay
        .iter()
        .filter(|h| h.kind == HopKind::Proc)
        .map(|h| h.dur)
        .collect();
    assert!(
        procs[0] != procs[1] && procs[1] != procs[2] && procs[0] != procs[2],
        "{procs:?}"
    );
}

#[test]
fn inserting_a_step_keeps_other_steps_draws() {
    let (t, a) = random("  \"GET /users/{id}\": { chain: [ db ] }\n");
    let (_, b) = random("  \"GET /users/{id}\": { chain: [ pay, db ] }\n");
    let (oa, ob) = (
        run(&t, &a, "GET /users/{id}", &ChaosState::default()),
        run(&t, &b, "GET /users/{id}", &ChaosState::default()),
    );
    assert_eq!(draws(&t, &oa, "db"), draws(&t, &ob, "db"));
    assert_eq!(draws(&t, &oa, "app"), draws(&t, &ob, "app"));
    assert_eq!(draws(&t, &oa, "db").len(), 2);
}

#[test]
fn parallel_branch_order_does_not_change_draws() {
    let (t, a) = random("  \"GET /users/{id}\": { chain: [ { parallel: [ pay, db ] } ] }\n");
    let (_, b) = random("  \"GET /users/{id}\": { chain: [ { parallel: [ db, pay ] } ] }\n");
    let (oa, ob) = (
        run(&t, &a, "GET /users/{id}", &ChaosState::default()),
        run(&t, &b, "GET /users/{id}", &ChaosState::default()),
    );
    for node in ["pay", "db"] {
        assert_eq!(draws(&t, &oa, node), draws(&t, &ob, node), "{node}");
    }
    assert_eq!(oa.total(), ob.total());
}

#[test]
fn draws_follow_the_r1_34_formula() {
    let src = "version: 1\nnodes:\n  app: { type: service, proc: { mean: 10ms } }\n  db: { type: database, ops: { read: { mean: 5ms }, write: 1ms, scan: 1ms } }\nedges:\n  - { from: app, to: db, net: { mean: 1ms } }\n";
    let cfg = Config::from_yaml(src).unwrap();
    let t = Topology::build(&cfg).unwrap();
    let e = compile(
        &t,
        &[RouteSpec::inferred("GET /x/{id}", t.id("app").unwrap())],
        EndpointsInput::default(),
    )
    .unwrap();
    let o = execute(
        &t,
        &ChaosState::default().compile(),
        &ChainRequest {
            endpoint: e.get("GET /x/{id}").unwrap(),
            n: 3,
            seed: 7,
            pin_delay: std::time::Duration::ZERO,
            req_bytes: 0,
            resp_bytes: 0,
        },
    );
    // D8: root = H(seed, key, n); a call's path = H(parent, target, occurrence, repetition);
    // each hop draws from H(path, node, tag), chaos roll first for own hops.
    let root = stable_hash(&[&7u64.to_le_bytes(), b"GET /x/{id}", &3u64.to_le_bytes()]);
    let db_path = stable_hash(&[
        &root.to_le_bytes(),
        b"db",
        &0u32.to_le_bytes(),
        &0u32.to_le_bytes(),
    ]);
    let draw = |path: u64, node: &str, tag: &[u8], own: bool, d: Dist| {
        let mut rng = kaanal_core::HopRng::seed_from_u64(stable_hash(&[
            &path.to_le_bytes(),
            node.as_bytes(),
            tag,
        ]));
        if own {
            let _: f64 = rng.random();
        }
        d.sample(&mut rng)
    };
    let want = [
        draw(root, "app", b"proc", true, Dist::Exp { mean: ms(10) }),
        draw(db_path, "db", b"net", false, Dist::Exp { mean: ms(1) }),
        draw(db_path, "db", b"op", true, Dist::Exp { mean: ms(5) }),
    ];
    assert_eq!(o.hops.iter().map(|h| h.dur).collect::<Vec<_>>(), want);
}

#[test]
fn error_rate_holds_across_seeds() {
    // `determinism: realistic` only changes the seed; a chaos rate must hold over seeds.
    let (t, e) = eps("");
    let c = chaos(&t, &[("db", fails(0.3, 503))]);
    let ep = e.get("GET /orders/{id}").unwrap();
    let failed = (0..2000u64)
        .filter(|&seed| {
            execute(
                &t,
                &c.compile(),
                &ChainRequest {
                    endpoint: ep,
                    n: 0,
                    seed,
                    pin_delay: std::time::Duration::ZERO,
                    req_bytes: 0,
                    resp_bytes: 0,
                },
            )
            .result
                != ChainResult::Ok
        })
        .count();
    // Binomial(2000, 0.3): mean 600, sd 20.5; ±62 is about 3 sd.
    assert!((538..=662).contains(&failed), "{failed}");
}

#[test]
fn diamond_calls_to_a_shared_body_draw_independently() {
    // Review (A2 Task 2, C4): `StepKey` counts per sequence, so the inner `db` call is
    // `db#0` in both bodies below. Their draws still differ because the parent path (the
    // outer `orders` occurrence) is chained into the seed.
    let (t, e) = random(
        "  \"GET /users/{id}\": { chain: [ { call: orders, chain: [ db ] }, { call: orders, chain: [ db ] } ] }\n",
    );
    let o = run(&t, &e, "GET /users/{id}", &ChaosState::default());
    let reads: Vec<Duration> = draws(&t, &o, "db")
        .into_iter()
        .filter(|(k, _)| matches!(k, HopKind::Op(_)))
        .map(|(_, d)| d)
        .collect();
    assert_eq!(reads.len(), 2);
    assert_ne!(reads[0], reads[1], "{reads:?}");
}

/// `app` with an edge `calls: 3` to a cache, then a database; no gateway.
const CACHED: &str = "version: 1
nodes:
  app: { type: service, proc: 3ms }
  db: { type: database, ops: { read: 5ms, write: 6ms, scan: 7ms } }
  c: { type: cache, hit_ratio: 0.0, hit: 1ms, miss: 2ms }
edges:
  - { from: app, to: c, net: 1ms, calls: 3 }
  - { from: app, to: db, net: 1ms }
";

fn cached(src: &str) -> (Topology, Endpoints) {
    let cfg = Config::from_yaml(src).unwrap();
    let t = Topology::build(&cfg).unwrap();
    let app = t.id("app").unwrap();
    let routes = [
        RouteSpec::inferred("GET /x/{id}", app),
        RouteSpec::inferred("POST /x", app),
    ];
    let e = compile(&t, &routes, EndpointsInput::default()).unwrap();
    (t, e)
}

#[test]
fn an_edge_with_calls_looks_the_cache_up_until_a_hit() {
    // Miss: three lookups in sequence, then the database.
    let (t, e) = cached(CACHED);
    let o = run(&t, &e, "GET /x/{id}", &ChaosState::default());
    assert_eq!(
        timeline(&t, &o),
        [
            "app.proc@0+3",
            "c.net@3+1",
            "c.miss@4+2",
            "c.net@6+1",
            "c.miss@7+2",
            "c.net@9+1",
            "c.miss@10+2",
            "db.net@12+1",
            "db.read@13+5",
        ]
    );
    // Hit: the first lookup ends app's body, so no more lookups and no database.
    let (t, e) = cached(&CACHED.replace("hit_ratio: 0.0", "hit_ratio: 1.0"));
    let o = run(&t, &e, "GET /x/{id}", &ChaosState::default());
    assert_eq!(timeline(&t, &o), ["app.proc@0+3", "c.net@3+1", "c.hit@4+1"]);
    // A write skips the default chain's cache lookups.
    let o = run(&t, &e, "POST /x", &ChaosState::default());
    assert_eq!(
        timeline(&t, &o),
        ["app.proc@0+3", "db.net@3+1", "db.write@4+6"]
    );
}

#[test]
fn cache_repetitions_draw_from_their_own_paths() {
    let src = CACHED.replace("net: 1ms, calls: 3", "net: { mean: 1ms }, calls: 3");
    let (t, e) = cached(&src);
    let o = run(&t, &e, "GET /x/{id}", &ChaosState::default());
    let nets: Vec<Duration> = o
        .hops
        .iter()
        .filter(|h| t.nodes[h.node].name == "c" && h.kind == HopKind::Net)
        .map(|h| h.dur)
        .collect();
    assert_eq!(nets.len(), 3);
    // D8: each repetition extends the step path with its index.
    let root = stable_hash(&[&1u64.to_le_bytes(), b"GET /x/{id}", &0u64.to_le_bytes()]);
    for (rep, got) in nets.iter().enumerate() {
        let path = stable_hash(&[
            &root.to_le_bytes(),
            b"c",
            &0u32.to_le_bytes(),
            &u32::try_from(rep).unwrap().to_le_bytes(),
        ]);
        let mut rng =
            kaanal_core::HopRng::seed_from_u64(stable_hash(&[&path.to_le_bytes(), b"c", b"net"]));
        assert_eq!(
            *got,
            Dist::Exp { mean: ms(1) }.sample(&mut rng),
            "rep {rep}"
        );
    }
    assert!(nets[0] != nets[1] && nets[1] != nets[2], "{nets:?}");
}

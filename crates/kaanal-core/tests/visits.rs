//! `expected_visits`: how many times a request is expected to enter each node.

use kaanal_core::chain::{EndpointsInput, RouteSpec, compile};
use kaanal_core::{Config, Endpoints, Topology, expected_visits};

/// A gateway in front of `app`, which can call `db` and the cache `c` (hit ratio 0.8); `extra`
/// is appended to the edges and `endpoints` to the endpoints.
fn world(db_edge: &str, endpoints: &str) -> (Topology, Endpoints) {
    let src = format!(
        "version: 1\nnodes:\n  gw: {{ type: gateway }}\n  app: {{ type: service }}\n  db: {{ type: database, flavor: postgres }}\n  c: {{ type: cache, hit_ratio: 0.8, hit: 1ms, miss: 1ms }}\nedges:\n  - {{ from: gw, to: app }}\n  - {{ from: app, to: db{db_edge} }}\n  - {{ from: app, to: c }}\nendpoints:\n{endpoints}"
    );
    let cfg = Config::from_yaml(&src).unwrap();
    let t = Topology::build(&cfg).unwrap();
    let routes = [
        RouteSpec::inferred("GET /x/{id}", t.id("app").unwrap()),
        RouteSpec::inferred("POST /x", t.id("app").unwrap()),
    ];
    let eps = compile(&t, &routes, EndpointsInput::inline(&cfg)).unwrap();
    (t, eps)
}

fn visits(t: &Topology, e: &Endpoints, key: &str) -> Vec<(String, f64)> {
    let v = expected_visits(t, e.get(key).unwrap());
    t.nodes.iter().map(|n| n.name.clone()).zip(v).collect()
}

fn of(v: &[(String, f64)], name: &str) -> f64 {
    v.iter().find(|(n, _)| n == name).unwrap().1
}

fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "{a} != {b}");
}

#[test]
fn a_plain_chain_counts_one_per_call() {
    let (t, e) = world("", "  \"GET /x/{id}\": { chain: [ db ] }\n");
    let v = visits(&t, &e, "GET /x/{id}");
    for n in ["gw", "app", "db"] {
        near(of(&v, n), 1.0);
    }
    near(of(&v, "c"), 0.0);
}

#[test]
fn calls_3_and_prob_multiply() {
    let (t, e) = world(", calls: 3, prob: 0.5", "");
    let v = visits(&t, &e, "POST /x");
    near(of(&v, "db"), 1.5);
}

#[test]
fn a_cache_hit_scales_later_steps() {
    let (t, e) = world("", "  \"GET /x/{id}\": { chain: [ { cache: c }, db ] }\n");
    let v = visits(&t, &e, "GET /x/{id}");
    near(of(&v, "c"), 1.0);
    near(of(&v, "db"), 0.2);
}

#[test]
fn repeated_lookups_stop_at_the_first_hit() {
    // two lookups: the second only when the first missed (0.2), and 0.2^2 reach the rest
    let (t, e) = world(
        "",
        "  \"GET /x/{id}\": { chain: [ { cache: c }, { cache: c }, db ] }\n",
    );
    let v = visits(&t, &e, "GET /x/{id}");
    near(of(&v, "c"), 1.2);
    near(of(&v, "db"), 0.04);
}

#[test]
fn default_cache_lookups_skip_writes() {
    let (t, e) = world("", "");
    near(of(&visits(&t, &e, "GET /x/{id}"), "c"), 1.0);
    near(of(&visits(&t, &e, "POST /x"), "c"), 0.0);
    // a read: the lookup hits 80% of the time and ends the body, so the database is rarely reached
    near(of(&visits(&t, &e, "GET /x/{id}"), "db"), 0.2);
    near(of(&visits(&t, &e, "POST /x"), "db"), 1.0);
}

#[test]
fn parallel_branches_add() {
    let (t, e) = world(
        "",
        "  \"GET /x/{id}\": { chain: [ { parallel: [ db, db ] } ] }\n",
    );
    near(of(&visits(&t, &e, "GET /x/{id}"), "db"), 2.0);
}

#[test]
fn a_hit_in_one_branch_does_not_scale_the_other_or_what_follows() {
    let (t, e) = world(
        "",
        "  \"GET /x/{id}\": { chain: [ { parallel: [ [ { cache: c }, db ], db ] }, db ] }\n",
    );
    // branch one: 0.2 of a call; branch two: 1; the step after the block: 1
    near(of(&visits(&t, &e, "GET /x/{id}"), "db"), 2.2);
}

#[test]
fn the_entry_node_counts_once_without_a_gateway() {
    let cfg = Config::from_yaml(
        "version: 1\nnodes:\n  app: { type: service }\n  db: { type: database, flavor: postgres }\nedges:\n  - { from: app, to: db }\n",
    )
    .unwrap();
    let t = Topology::build(&cfg).unwrap();
    let routes = [RouteSpec::inferred("GET /x", t.id("app").unwrap())];
    let e = compile(&t, &routes, EndpointsInput::inline(&cfg)).unwrap();
    let v = visits(&t, &e, "GET /x");
    near(of(&v, "app"), 1.0);
    near(of(&v, "db"), 1.0);
}

/// A gateway, an auth node and a service; `auth_edge` adds the gateway's `auth_request` edge.
fn auth_world(auth_edge: &str) -> (Topology, Endpoints) {
    let src = format!(
        "version: 1\nnodes:\n  gw: {{ type: gateway }}\n  login: {{ type: auth, proc: 1ms }}\n  app: {{ type: service }}\nedges:\n  - {{ from: gw, to: app }}\n{auth_edge}"
    );
    let cfg = Config::from_yaml(&src).unwrap();
    let t = Topology::build(&cfg).unwrap();
    let routes = [RouteSpec::inferred("GET /x", t.id("app").unwrap())];
    let e = compile(&t, &routes, EndpointsInput::inline(&cfg)).unwrap();
    (t, e)
}

#[test]
fn an_auth_node_validating_locally_is_visited_inline() {
    // no gateway-to-auth edge: the login route's body runs inline in the gateway's sequence
    let (t, e) = auth_world("");
    let v = visits(&t, &e, "POST /login");
    near(of(&v, "gw"), 1.0);
    near(of(&v, "login"), 1.0);
    near(of(&v, "app"), 0.0);
}

#[test]
fn an_auth_request_edge_visits_the_auth_node_once() {
    let (t, e) = auth_world("  - { from: gw, to: login }\n");
    let v = visits(&t, &e, "POST /login");
    near(of(&v, "login"), 1.0);
    // an ordinary route also calls auth first
    near(of(&visits(&t, &e, "GET /x"), "login"), 1.0);
}

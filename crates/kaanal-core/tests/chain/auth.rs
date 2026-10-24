//! The built-in login and JWKS routes and how endpoints customise them.

use crate::common::*;
use kaanal_core::chain::{JWKS_KEY, LOGIN_KEY, Responder};
use kaanal_core::config::ErrorMapping;
use kaanal_core::{
    ChainRequest, ChaosState, ConfigError, Endpoints, HopKind, Op, Topology, execute,
};
use serde_json::json;

/// An auth node `login` with a user store, a session cache and a mail API behind it, and a
/// service `app`. `gw` is a gateway node line or ""; `edges` adds edges; `entries` goes under
/// `endpoints:` (line 14 with a gateway, so entries start at line 15).
pub(crate) fn auth_topo(
    gw: &str,
    edges: &str,
    entries: &str,
    routes: &[(&str, &str)],
) -> (Topology, Result<Endpoints, ConfigError>) {
    build(
        &format!(
            "version: 1\nnodes:\n{gw}  login: {{ type: auth, proc: 1ms }}\n  app: {{ type: service, proc: 1ms }}\n  users: {{ type: database, flavor: postgres }}\n  sessions: {{ type: cache, flavor: redis }}\n  mail: {{ type: external-api, proc: 1ms }}\nedges:\n  - {{ from: login, to: users }}\n  - {{ from: login, to: sessions }}\n  - {{ from: login, to: mail }}\n{edges}endpoints:\n{entries}"
        ),
        routes,
    )
}

pub(crate) const GW: &str = "  gw: { type: gateway, proc: 1ms }\n";

/// X12: the login body walks auth's out-edges like a service body (callees, caches, databases).
pub(crate) const LOGIN_BODY: &str = "login.proc mail[mail.proc] cache(sessions?) users[users.op]";

#[test]
fn login_and_jwks_are_built_in_with_the_auth_edge() {
    let (t, eps) = auth_topo(
        GW,
        "  - { from: gw, to: app }\n  - { from: gw, to: login }\n",
        "",
        &[("GET /users/{id}", "app")],
    );
    let eps = eps.unwrap();
    let login = eps.get(LOGIN_KEY).unwrap();
    assert_eq!(
        (login.owner, login.op, login.responder),
        (t.id("login"), Op::Read, Some(Responder::Login))
    );
    // X7: the gateway's auth call runs the login body, so auth is charged exactly once.
    assert_eq!(
        outline(&t, &login.chain),
        format!("gw.proc login[{LOGIN_BODY}]")
    );
    let jwks = eps.get(JWKS_KEY).unwrap();
    assert_eq!(jwks.responder, Some(Responder::Jwks));
    assert_eq!(outline(&t, &jwks.chain), "gw.proc login[login.proc]");
    // Ordinary routes keep the auth_request call with auth's `proc` only (A2 D5).
    let get = eps.get("GET /users/{id}").unwrap();
    assert_eq!(
        outline(&t, &get.chain),
        "gw.proc login[login.proc] app[app.proc]"
    );
    assert!(get.responder.is_none());
    assert!(eps.warnings.is_empty(), "{:?}", eps.warnings);
}

#[test]
fn login_without_the_auth_edge_runs_auth_inline() {
    let (t, eps) = auth_topo(GW, "  - { from: gw, to: app }\n", "", &[]);
    let eps = eps.unwrap();
    // X7 local validation: no network hop between gateway and auth, auth's proc once (P7).
    assert_eq!(
        outline(&t, &eps.get(LOGIN_KEY).unwrap().chain),
        format!("gw.proc {LOGIN_BODY}")
    );
    let jwks = eps.get(JWKS_KEY).unwrap();
    let o = execute(
        &t,
        &ChaosState::default().compile(),
        &ChainRequest {
            endpoint: jwks,
            n: 0,
            seed: 1,
            pin_delay: std::time::Duration::ZERO,
        },
    );
    let hops: Vec<(String, HopKind)> = o
        .hops
        .iter()
        .map(|h| (t.nodes[h.node].name.clone(), h.kind))
        .collect();
    assert_eq!(
        hops,
        [
            ("gw".to_string(), HopKind::Proc),
            ("login".to_string(), HopKind::Proc)
        ]
    );
}

#[test]
fn gatewayless_and_auth_only_logins_start_at_the_auth_node() {
    let (t, eps) = auth_topo("", "", "", &[("GET /users/{id}", "app")]);
    let eps = eps.unwrap();
    assert_eq!(outline(&t, &eps.get(LOGIN_KEY).unwrap().chain), LOGIN_BODY);
    assert_eq!(outline(&t, &eps.get(JWKS_KEY).unwrap().chain), "login.proc");
    // X11: an auth node alone compiles both routes and the JWKS HEAD copy.
    let (t, eps) = build(
        "version: 1\nnodes:\n  login: { type: auth, proc: 1ms }\nendpoints:\n",
        &[],
    );
    let eps = eps.unwrap();
    let keys: Vec<&str> = eps.iter().map(|e| e.key.as_str()).collect();
    assert_eq!(
        keys,
        [
            "GET /.well-known/jwks.json",
            "HEAD /.well-known/jwks.json",
            "POST /login"
        ]
    );
    assert_eq!(
        outline(&t, &eps.get(LOGIN_KEY).unwrap().chain),
        "login.proc"
    );
}

#[test]
fn login_charges_auth_once_even_with_edge_calls() {
    let (t, eps) = auth_topo(
        GW,
        "  - { from: gw, to: app }\n  - { from: gw, to: login, calls: 2 }\n",
        "",
        &[("GET /users/{id}", "app")],
    );
    let eps = eps.unwrap();
    // Two auth_request checks per ordinary request, but one login (P6).
    assert_eq!(
        outline(&t, &eps.get("GET /users/{id}").unwrap().chain),
        "gw.proc login*2[login.proc] app[app.proc]"
    );
    assert_eq!(
        outline(&t, &eps.get(LOGIN_KEY).unwrap().chain),
        format!("gw.proc login[{LOGIN_BODY}]")
    );
    assert_eq!(
        outline(&t, &eps.get(JWKS_KEY).unwrap().chain),
        "gw.proc login[login.proc]"
    );
}

#[test]
fn a_login_chain_runs_with_the_auth_node_as_runner() {
    let (t, eps) = auth_topo(
        GW,
        "  - { from: gw, to: login }\n",
        "  \"POST /login\": { chain: [ users, { call: mail, repeat: 2 } ] }\n  \"GET /.well-known/jwks.json\": { chain: [ { cache: sessions } ] }\n",
        &[],
    );
    let eps = eps.unwrap();
    assert_eq!(
        outline(&t, &eps.get(LOGIN_KEY).unwrap().chain),
        "gw.proc login[login.proc users[users.op] mail*2[mail.proc]]"
    );
    assert_eq!(
        outline(&t, &eps.get(JWKS_KEY).unwrap().chain),
        "gw.proc login[login.proc cache(sessions)]"
    );
    // A step auth has no edge for is the usual compile error, placed at the step.
    let (_, r) = auth_topo(
        GW,
        "  - { from: gw, to: login }\n",
        "  \"POST /login\": { chain: [ app ] }\n",
        &[],
    );
    let e = r.unwrap_err();
    assert_eq!((e.line, e.col), (Some(15), Some(29)), "{e}");
    assert!(e.msg.contains("'login' has no edge to 'app'"), "{e}");
}

#[test]
fn a_spec_route_replaces_the_built_in_route_with_a_warning() {
    let (t, eps) = auth_topo(
        GW,
        "  - { from: gw, to: app }\n  - { from: gw, to: login }\n",
        "",
        &[("POST /login", "app")],
    );
    let eps = eps.unwrap();
    let login = eps.get(LOGIN_KEY).unwrap();
    assert_eq!((login.owner, login.responder), (t.id("app"), None));
    assert_eq!(
        eps.get(JWKS_KEY).unwrap().responder,
        Some(Responder::Jwks),
        "the other built-in route stays"
    );
    assert_eq!(
        eps.warnings,
        [
            "endpoint 'POST /login': the spec of 'app' defines it, so it replaces the built-in route of auth node 'login'"
        ]
    );
}

#[test]
fn endpoint_entries_customise_built_in_routes() {
    let (_, eps) = auth_topo(
        GW,
        "  - { from: gw, to: login }\n",
        "  \"post /login\": { error_map: { login: 502 } }\n  \"GET /.well-known/jwks.json\": { override: { body: { keys: [] } } }\n",
        &[],
    );
    let eps = eps.unwrap();
    let login = eps.get(LOGIN_KEY).unwrap();
    assert_eq!(
        login.error_map["login"],
        ErrorMapping {
            status: 502,
            body: None
        }
    );
    assert_eq!(login.responder, Some(Responder::Login));
    let jwks = eps.get(JWKS_KEY).unwrap();
    assert_eq!(
        jwks.response.as_ref().map(|o| o.body.clone()),
        Some(json!({ "keys": [] }))
    );
    assert_eq!(
        jwks.responder,
        Some(Responder::Jwks),
        "the responder stays; the server lets the override win"
    );
    assert!(eps.warnings.is_empty(), "not stale: {:?}", eps.warnings);
}

#[test]
fn head_copy_of_jwks_keeps_its_responder() {
    let (t, eps) = auth_topo(GW, "  - { from: gw, to: login }\n", "", &[]);
    let eps = eps.unwrap();
    let head = eps.get("HEAD /.well-known/jwks.json").unwrap();
    assert!(head.synthesized);
    assert_eq!(head.responder, Some(Responder::Jwks));
    assert_eq!(outline(&t, &head.chain), "gw.proc login[login.proc]");
    assert!(eps.get("HEAD /login").is_none(), "POST only, no HEAD copy");
}

#[test]
fn a_head_entry_for_the_jwks_is_a_copy_of_the_built_in_route() {
    let entry = "  \"HEAD /.well-known/jwks.json\": { override: { status: 204, body: null } }\n";
    // Without a gateway this used to be "static route needs a gateway"; with one it was
    // an ownerless static route. Both now serve the auth node's JWKS route, customised.
    for (gw, edges, shown) in [
        ("", "", "login.proc"),
        (
            GW,
            "  - { from: gw, to: login }\n",
            "gw.proc login[login.proc]",
        ),
    ] {
        let (t, eps) = auth_topo(gw, edges, entry, &[]);
        let eps = eps.unwrap();
        let head = eps.get("HEAD /.well-known/jwks.json").unwrap();
        assert_eq!(
            (head.owner, head.op, head.responder, head.synthesized),
            (t.id("login"), Op::Read, Some(Responder::Jwks), false)
        );
        assert!(head.response.is_some());
        assert_eq!(outline(&t, &head.chain), shown);
        assert!(eps.warnings.is_empty(), "{:?}", eps.warnings);
    }
}

#[test]
fn without_an_auth_node_there_are_no_built_in_routes() {
    let (_, r) = build(
        "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service }\nedges:\n  - { from: gw, to: app }\nendpoints:\n  \"POST /login\": {}\n",
        &[("GET /users/{id}", "app")],
    );
    let eps = r.unwrap();
    assert!(eps.get(LOGIN_KEY).is_none() && eps.get(JWKS_KEY).is_none());
    assert_eq!(
        eps.warnings,
        [
            "endpoint 'POST /login': no spec defines this route and it has no override (stale entry: remove it, or add an override to serve it)"
        ]
    );
}

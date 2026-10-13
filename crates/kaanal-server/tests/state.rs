use kaanal_core::Config;
use kaanal_core::diagnostics::keys;
use std::path::{Path, PathBuf};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn check_yaml(yaml: &str) -> Result<(), String> {
    kaanal_server::check(&Config::from_yaml(yaml).unwrap(), &fixtures())
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[test]
fn fixture_config_is_valid() {
    let src = std::fs::read_to_string(fixtures().join("kaanal.yaml")).unwrap();
    kaanal_server::check(&Config::from_yaml(&src).unwrap(), &fixtures()).unwrap();
}

#[test]
fn missing_spec_file_is_reported_with_path() {
    let err = check_yaml(
        "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service, spec: missing.yaml }\nedges:\n  - { from: gw, to: app }\n",
    )
    .unwrap_err();
    assert!(err.contains("missing.yaml"), "{err}");
}

#[test]
fn same_route_in_two_services_conflicts() {
    let err = check_yaml(
        "version: 1\nnodes:\n  gw: { type: gateway }\n  a: { type: service, spec: openapi.yaml }\n  b: { type: service, spec: openapi.yaml }\nedges:\n  - { from: gw, to: a }\n  - { from: gw, to: b }\n",
    )
    .unwrap_err();
    assert!(err.contains("defined by both"), "{err}");
}

#[test]
fn malformed_route_key_is_rejected() {
    let err =
        check_yaml("version: 1\nnodes:\n  gw: { type: gateway }\nendpoints:\n  nope: { override: { body: 1 } }\n")
            .unwrap_err();
    assert!(err.contains("must look like"), "{err}");
}

#[test]
fn topology_errors_surface() {
    let err = check_yaml("version: 1\nnodes:\n  a: { type: gateway }\n  b: { type: gateway }\n")
        .unwrap_err();
    assert!(err.contains("at most one gateway"), "{err}");
}

#[test]
fn gatewayless_topology_validates() {
    check_yaml(
        "version: 1\nnodes:\n  app: { type: service, spec: openapi.yaml }\n  db: { type: database, flavor: postgres }\nedges:\n  - { from: app, to: db }\n",
    )
    .unwrap();
}

#[test]
fn static_route_without_gateway_is_rejected() {
    let base = "version: 1\nnodes:\n  app: { type: service, spec: openapi.yaml }\nendpoints:\n";
    let err = check_yaml(&format!(
        "{base}  \"GET /health\": {{ override: {{ body: 1 }} }}\n"
    ))
    .unwrap_err();
    assert!(
        err.contains(
            "endpoint 'GET /health': static route needs a gateway or a service that owns it"
        ),
        "{err}"
    );
    // Overriding a route the service owns is fine: it still has an owner.
    check_yaml(&format!(
        "{base}  \"GET /users/{{id}}\": {{ override: {{ body: 1 }} }}\n"
    ))
    .unwrap();
}

#[test]
fn spec_on_non_service_is_rejected() {
    // `spec` is not a gateway field, so the config itself fails to parse (R1.15).
    let err =
        Config::from_yaml("version: 1\nnodes:\n  gw: { type: gateway, spec: openapi.yaml }\n")
            .unwrap_err();
    assert!(err.msg.contains("unknown field 'spec'"), "{err}");
}

#[test]
fn unsupported_keywords_become_warnings() {
    let cfg = Config::from_yaml(
        "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service, spec: openapi-warn.yaml }\nedges:\n  - { from: gw, to: app }\n",
    )
    .unwrap();
    let warnings = kaanal_server::check(&cfg, &fixtures()).unwrap();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("openapi-warn.yaml") && warnings[0].contains("'pattern'"),
        "{warnings:?}"
    );
}

#[test]
fn clean_config_has_no_warnings() {
    let src = std::fs::read_to_string(fixtures().join("kaanal.yaml")).unwrap();
    let warnings = kaanal_server::check(&Config::from_yaml(&src).unwrap(), &fixtures()).unwrap();
    assert_eq!(warnings, Vec::<String>::new());
}

#[test]
fn token_is_64_hex_and_random() {
    let (a, b) = (kaanal_server::new_token(), kaanal_server::new_token());
    assert_eq!(a.len(), 64);
    assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    assert_ne!(a, b);
}

const GW_APP: &str = "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service, spec: openapi.yaml }\nedges:\n  - { from: gw, to: app }\n";

#[test]
fn override_differing_only_in_param_name_is_a_clear_error() {
    let err = check_yaml(&format!(
        "{GW_APP}endpoints:\n  \"GET /users/{{x}}\": {{ override: {{ body: 1 }} }}\n"
    ))
    .unwrap_err();
    assert!(err.contains("differ only in parameter names"), "{err}");
    assert!(
        err.contains("/users/{id}") && err.contains("/users/{x}"),
        "{err}"
    );
}

#[test]
fn override_with_same_param_name_replaces_the_spec_route() {
    check_yaml(&format!(
        "{GW_APP}endpoints:\n  \"GET /users/{{id}}\": {{ override: {{ body: 1 }} }}\n"
    ))
    .unwrap();
}

#[test]
fn override_needs_known_method_and_leading_slash() {
    let err = check_yaml(&format!(
        "{GW_APP}endpoints:\n  \"FOO /x\": {{ override: {{ body: 1 }} }}\n"
    ))
    .unwrap_err();
    assert!(err.contains("unknown method 'FOO'"), "{err}");
    let err = check_yaml(&format!(
        "{GW_APP}endpoints:\n  \"GET users\": {{ override: {{ body: 1 }} }}\n"
    ))
    .unwrap_err();
    assert!(err.contains("must start with '/'"), "{err}");
}

#[test]
fn override_status_must_be_a_real_http_status() {
    for bad in ["99", "100", "199", "1000"] {
        let err = check_yaml(&format!(
            "{GW_APP}endpoints:\n  \"GET /x\": {{ override: {{ status: {bad}, body: 1 }} }}\n"
        ))
        .unwrap_err();
        assert!(err.contains("between 200 and 599"), "{err}");
    }
}

#[test]
fn control_namespace_is_reserved() {
    for p in ["/__kaanal", "/__kaanal/x"] {
        let err = check_yaml(&format!(
            "{GW_APP}endpoints:\n  \"GET {p}\": {{ override: {{ body: 1 }} }}\n"
        ))
        .unwrap_err();
        assert!(err.contains("reserved"), "{err}");
    }
}

#[test]
fn concurrency_no_longer_warns_in_live_mode() {
    let cfg = Config::from_yaml(
        "version: 1\nnodes:\n  gw: { type: gateway, concurrency: 2 }\n  app: { type: service, spec: openapi.yaml, concurrency: 1 }\nedges:\n  - { from: gw, to: app }\n",
    )
    .unwrap();
    let warnings = kaanal_server::check(&cfg, &fixtures()).unwrap();
    assert!(
        !warnings.iter().any(|w| w.contains("concurrency")),
        "{warnings:?}"
    );
}

#[test]
fn head_override_keeps_the_owning_service() {
    // R6.8: a HEAD override of an owned path is not a gateway-only static route, so it
    // loads with or without a gateway.
    let nodes = "version: 1\nnodes:\n  app: { type: service, spec: openapi.yaml }\n";
    let routes = "endpoints:\n  \"HEAD /users/{id}\": { override: { status: 204 } }\n";
    check_yaml(&format!("{nodes}{routes}")).unwrap();
    check_yaml(&format!(
        "{nodes}  gw: {{ type: gateway }}\nedges:\n  - {{ from: gw, to: app }}\n{routes}"
    ))
    .unwrap();
    // A different method on an owned path is still a static route: it needs a gateway.
    let err = check_yaml(&format!(
        "{nodes}endpoints:\n  \"PATCH /users/{{id}}\": {{ override: {{ status: 204 }} }}\n"
    ))
    .unwrap_err();
    assert!(err.contains("static route needs a gateway"), "{err}");
}

#[test]
fn load_errors_carry_their_yaml_place() {
    let src = "version: 1
nodes:
  app: { type: service, spec: missing.yaml }
";
    let err = kaanal_server::check(&Config::from_yaml(src).unwrap(), &fixtures()).unwrap_err();
    assert_eq!(err.path, keys(&["nodes", "app", "spec"]));
    let e = err.in_source(src);
    assert_eq!((e.line, e.col), (Some(3), Some(25)), "{e}");
    // Review focus 1: a static route with no gateway is placed at its key.
    let src = "version: 1
nodes:
  app: { type: service, spec: openapi.yaml }
endpoints:
  \"GET /health\": { override: { body: 1 } }
";
    let err = kaanal_server::check(&Config::from_yaml(src).unwrap(), &fixtures()).unwrap_err();
    assert!(err.msg.contains("static route needs a gateway"), "{err}");
    assert_eq!(err.in_source(src).line, Some(5));
}

/// A temp dir holding a copy of the fixture spec.
fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("kaanal-state-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::copy(fixtures().join("openapi.yaml"), d.join("openapi.yaml")).unwrap();
    d
}

#[test]
fn endpoints_file_errors_name_the_file_and_place() {
    let dir = scratch("eps-file");
    let src = "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service, spec: openapi.yaml }\n  db: { type: database, flavor: postgres }\nedges:\n  - { from: gw, to: app }\n  - { from: app, to: db }\nendpoints: kaanal.endpoints.yaml\n";
    let cfg = Config::from_yaml(src).unwrap();
    // A missing file is placed at the `endpoints:` value in kaanal.yaml.
    let e = kaanal_server::check(&cfg, &dir).unwrap_err();
    assert_eq!(e.file, None);
    let at = e.in_source(src);
    assert_eq!((at.line, at.col), (Some(9), Some(12)), "{at}");
    assert!(at.msg.contains("kaanal.endpoints.yaml"), "{at}");
    // A compile error inside the file is placed in the file; BOM and CRLF change nothing.
    let file = dir.join("kaanal.endpoints.yaml");
    let eps = "\"GET /users/{id}\":\n  chain:\n    - call: pay\n";
    for text in [
        eps.to_string(),
        format!("\u{feff}{}", eps.replace('\n', "\r\n")),
    ] {
        std::fs::write(&file, &text).unwrap();
        let e = kaanal_server::check(&cfg, &dir).unwrap_err();
        assert_eq!(e.file.as_deref(), Some(file.as_path()));
        let at = e.in_source(&text);
        assert_eq!(
            (at.line, at.col, at.msg.as_str()),
            (
                Some(3),
                Some(13),
                "endpoint 'GET /users/{id}': unknown node 'pay'"
            )
        );
    }
    // A shape error keeps serde's position in the file (Located::parsed).
    std::fs::write(
        &file,
        "\"GET /users/{id}\":\n  chain:\n    - { cache: c, op: read }\n",
    )
    .unwrap();
    let e = kaanal_server::check(&cfg, &dir).unwrap_err();
    assert_eq!(
        (e.file.as_deref(), e.pos),
        (Some(file.as_path()), Some((3, 7)))
    );
}

#[test]
fn stale_entries_and_ignored_error_maps_are_warnings() {
    let src = format!(
        "{GW_APP}endpoints:\n  \"GET /old\": {{}}\n  \"GET /health\": {{ override: {{ body: 1 }}, chain: [ {{ call: app, via: \"POST /orders\" }} ] }}\n  \"POST /orders\": {{ error_map: {{ \"5xx\": 502 }} }}\n"
    );
    let warnings = kaanal_server::check(&Config::from_yaml(&src).unwrap(), &fixtures()).unwrap();
    assert_eq!(
        warnings,
        [
            "endpoint 'GET /old': no spec defines this route and it has no override (stale entry: remove it, or add an override to serve it)",
            "endpoint 'GET /health': via 'POST /orders': that endpoint's error_map is ignored here (each endpoint maps its own errors)",
        ]
    );
}

#[test]
fn spec_node_without_a_gateway_edge_is_rejected_before_compile() {
    // P4: compile would fail on such a route with no place; the topology check catches it first
    // and names the node.
    let err = check_yaml(
        "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service, spec: openapi.yaml }\n",
    )
    .unwrap_err();
    assert!(
        err.contains("node 'app' has an OpenAPI spec but no edge from the gateway"),
        "{err}"
    );
}

#[test]
fn auth_without_users_warns() {
    let w = kaanal_server::check(
        &Config::from_yaml("version: 1\nnodes:\n  login: { type: auth }\n").unwrap(),
        &fixtures(),
    )
    .unwrap();
    assert_eq!(
        w,
        [
            "node 'login': no users, so every POST /login answers 401 (add users: [{ username, password }])"
        ]
    );
    let w = kaanal_server::check(
        &Config::from_yaml(
            "version: 1\nnodes:\n  login: { type: auth, users: [ { username: a, password: b } ] }\n",
        )
        .unwrap(),
        &fixtures(),
    )
    .unwrap();
    assert!(w.is_empty(), "{w:?}");
}

use kaanal_core::Config;
use std::path::{Path, PathBuf};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn check_yaml(yaml: &str) -> Result<(), String> {
    kaanal_server::check(&Config::from_yaml(yaml).unwrap(), &fixtures()).map(|_| ())
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
        check_yaml("version: 1\nnodes:\n  gw: { type: gateway }\nroutes:\n  nope: { body: 1 }\n")
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
    let base = "version: 1\nnodes:\n  app: { type: service, spec: openapi.yaml }\nroutes:\n";
    let err = check_yaml(&format!("{base}  \"GET /health\": {{ body: 1 }}\n")).unwrap_err();
    assert!(
        err.contains(
            "routes: 'GET /health': static route needs a gateway or a service that owns it"
        ),
        "{err}"
    );
    // Overriding a route the service owns is fine: it still has an owner.
    check_yaml(&format!("{base}  \"GET /users/{{id}}\": {{ body: 1 }}\n")).unwrap();
}

#[test]
fn spec_on_non_service_is_rejected() {
    // `spec` is not a gateway field, so the config itself fails to parse (R1.15).
    let err =
        Config::from_yaml("version: 1\nnodes:\n  gw: { type: gateway, spec: openapi.yaml }\n")
            .unwrap_err();
    assert!(err.msg.contains("unknown field `spec`"), "{err}");
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
        "{GW_APP}routes:\n  \"GET /users/{{x}}\": {{ body: 1 }}\n"
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
        "{GW_APP}routes:\n  \"GET /users/{{id}}\": {{ body: 1 }}\n"
    ))
    .unwrap();
}

#[test]
fn override_needs_known_method_and_leading_slash() {
    let err = check_yaml(&format!("{GW_APP}routes:\n  \"FOO /x\": {{ body: 1 }}\n")).unwrap_err();
    assert!(err.contains("unknown method 'FOO'"), "{err}");
    let err = check_yaml(&format!(
        "{GW_APP}routes:\n  \"GET users\": {{ body: 1 }}\n"
    ))
    .unwrap_err();
    assert!(err.contains("must start with '/'"), "{err}");
}

#[test]
fn override_status_must_be_a_real_http_status() {
    for bad in ["99", "100", "199", "1000"] {
        let err = check_yaml(&format!(
            "{GW_APP}routes:\n  \"GET /x\": {{ status: {bad}, body: 1 }}\n"
        ))
        .unwrap_err();
        assert!(err.contains("between 200 and 599"), "{err}");
    }
}

#[test]
fn control_namespace_is_reserved() {
    for p in ["/__kaanal", "/__kaanal/x"] {
        let err =
            check_yaml(&format!("{GW_APP}routes:\n  \"GET {p}\": {{ body: 1 }}\n")).unwrap_err();
        assert!(err.contains("reserved"), "{err}");
    }
}

#[test]
fn concurrency_warns_that_live_mode_ignores_it() {
    let cfg = Config::from_yaml(
        "version: 1\nnodes:\n  gw: { type: gateway, concurrency: 2 }\n  app: { type: service, spec: openapi.yaml, concurrency: 1 }\nedges:\n  - { from: gw, to: app }\n",
    )
    .unwrap();
    let warnings = kaanal_server::check(&cfg, &fixtures()).unwrap();
    let c: Vec<&String> = warnings
        .iter()
        .filter(|w| w.contains("concurrency"))
        .collect();
    assert_eq!(c.len(), 2, "one per node: {warnings:?}");
    assert!(
        c[0].starts_with("node 'gw': ") && c[0].contains("simulator only"),
        "{warnings:?}"
    );
    assert!(c[1].starts_with("node 'app': "), "{warnings:?}");
}

#[test]
fn head_override_keeps_the_owning_service() {
    // R6.8: a HEAD override of an owned path is not a gateway-only static route, so it
    // loads with or without a gateway.
    let nodes = "version: 1\nnodes:\n  app: { type: service, spec: openapi.yaml }\n";
    let routes = "routes:\n  \"HEAD /users/{id}\": { status: 204 }\n";
    check_yaml(&format!("{nodes}{routes}")).unwrap();
    check_yaml(&format!(
        "{nodes}  gw: {{ type: gateway }}\nedges:\n  - {{ from: gw, to: app }}\n{routes}"
    ))
    .unwrap();
    // A different method on an owned path is still a static route: it needs a gateway.
    let err = check_yaml(&format!(
        "{nodes}routes:\n  \"PATCH /users/{{id}}\": {{ status: 204 }}\n"
    ))
    .unwrap_err();
    assert!(err.contains("static route needs a gateway"), "{err}");
}

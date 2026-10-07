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
    let err = check_yaml("version: 1\nnodes:\n  app: { type: service }\n").unwrap_err();
    assert!(err.contains("gateway"), "{err}");
}

#[test]
fn spec_on_non_service_is_rejected() {
    let err = check_yaml("version: 1\nnodes:\n  gw: { type: gateway, spec: openapi.yaml }\n")
        .unwrap_err();
    assert!(err.contains("service nodes"), "{err}");
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
    assert!(warnings.is_empty(), "{warnings:?}");
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

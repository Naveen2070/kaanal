//! Parsing basics: defaults, positioned errors, durations.

use crate::common::*;
use kaanal_core::config::{Determinism, DistConfig, EndpointsConfig, SeedSource};
use kaanal_core::{Config, NodeKind};
use std::time::Duration;

#[test]
fn parses_sample() {
    let cfg = Config::from_yaml(SAMPLE).unwrap();
    assert_eq!(cfg.seed, 42);
    assert_eq!(cfg.port, 8080);
    assert_eq!(cfg.nodes.len(), 3);
    assert_eq!(cfg.nodes["nginx"].kind(), NodeKind::Gateway);
    assert_eq!(cfg.nodes["app"].spec(), Some("openapi.yaml"));
    assert_eq!(cfg.edges.len(), 2);
    let Some(EndpointsConfig::Inline(eps)) = &cfg.endpoints else {
        panic!("inline endpoints: {:?}", cfg.endpoints)
    };
    assert_eq!(eps["GET /health"].response.as_ref().unwrap().status, 200);
    match cfg.nodes["app"].proc().unwrap() {
        DistConfig::Fixed(d) => assert_eq!(*d, Duration::from_millis(5)),
        other => panic!("expected fixed, got {other:?}"),
    }
}

#[test]
fn defaults_apply() {
    let cfg = Config::from_yaml("version: 1\nnodes: {}\n").unwrap();
    assert_eq!(cfg.seed, 0);
    assert_eq!(cfg.port, 8080);
    assert!(cfg.edges.is_empty());
    assert_eq!(cfg.determinism, Determinism::Controlled);
    assert_eq!(cfg.seed_source(), SeedSource::Fixed(0));
    assert!(cfg.endpoints.is_none());
}

#[test]
fn bad_value_reports_line() {
    let err = Config::from_yaml("version: 1\nseed: 1\nport: notanumber\nnodes: {}\n").unwrap_err();
    assert_eq!(err.line, Some(3), "{err}");
    assert!(err.to_string().starts_with("3:"), "{err}");
}

#[test]
fn unknown_field_is_rejected() {
    let err = Config::from_yaml("version: 1\nnodes: {}\nbogus: 1\n").unwrap_err();
    assert!(err.msg.contains("bogus"), "{err}");
}

#[test]
fn wrong_version_is_rejected() {
    let err = Config::from_yaml("version: 2\nnodes: {}\n").unwrap_err();
    assert!(err.msg.contains("version"), "{err}");
}

#[test]
fn bom_and_crlf_parse_like_lf() {
    let windows = format!("\u{feff}{}", SAMPLE.replace('\n', "\r\n"));
    let cfg = Config::from_yaml(&windows).unwrap();
    assert_eq!(cfg.nodes.len(), 3);
}

#[test]
fn bad_duration_explains_what_is_expected() {
    let yaml = "version: 1\nnodes:\n  gw: { type: gateway, proc: { p50: oops, p99: 3ms } }\n";
    let err = Config::from_yaml(yaml).unwrap_err();
    assert!(err.msg.contains("expected a duration like"), "{err}");
    assert_eq!(err.line, Some(3), "{err}");
    let err =
        Config::from_yaml("version: 1\nnodes:\n  gw: { type: gateway, proc: fast }\n").unwrap_err();
    assert!(err.msg.contains("expected a duration like"), "{err}");
}

#[test]
fn huge_durations_are_rejected_with_a_position() {
    let yaml = "version: 1\nnodes:\n  gw: { type: gateway, proc: 10000000000000000000s }\n";
    let err = Config::from_yaml(yaml).unwrap_err();
    assert!(err.to_string().contains("at most 1h"), "{err}");
    assert_eq!(err.line, Some(3), "{err}");
    // A p99 over the limit is caught too; 1h itself is fine.
    let yaml = "version: 1\nnodes:\n  gw: { type: gateway, proc: { p50: 1ms, p99: 2h } }\n";
    assert!(Config::from_yaml(yaml).is_err());
    let yaml = "version: 1\nnodes:\n  gw: { type: gateway, proc: 1h }\n";
    assert!(Config::from_yaml(yaml).is_ok());
    // p50 above p99 would become an uncapped Fixed(p50); equal is fine.
    let yaml = "version: 1\nnodes:\n  gw: { type: gateway, proc: { p50: 100h, p99: 1ms } }\n";
    assert!(Config::from_yaml(yaml).is_err());
    let yaml = "version: 1\nnodes:\n  gw: { type: gateway, proc: { p50: 1ms, p99: 1ms } }\n";
    assert!(Config::from_yaml(yaml).is_ok());
}

#[test]
fn exponential_duration_parses_and_is_strict() {
    let cfg =
        Config::from_yaml("version: 1\nnodes:\n  gw: { type: gateway, proc: { mean: 5ms } }\n")
            .unwrap();
    assert!(
        matches!(
            cfg.nodes["gw"].proc(),
            Some(DistConfig::Exp { mean }) if *mean == Duration::from_millis(5)
        ),
        "{:?}",
        cfg.nodes["gw"].proc()
    );
    // Over the 1 h cap: rejected with its line.
    let err =
        Config::from_yaml("version: 1\nnodes:\n  gw: { type: gateway, proc: { mean: 2h } }\n")
            .unwrap_err();
    assert!(err.to_string().contains("at most 1h"), "{err}");
    assert_eq!(err.line, Some(3), "{err}");
    // Mixed shapes are not silently read as one of them.
    let err = Config::from_yaml(
        "version: 1\nnodes:\n  gw: { type: gateway, proc: { mean: 5ms, p99: 9ms } }\n",
    )
    .unwrap_err();
    assert!(err.msg.contains("{ mean: 5ms }"), "{err}");
    // Extra keys on a percentile pair are rejected too, not dropped.
    let err = Config::from_yaml(
        "version: 1
nodes:
  gw: { type: gateway, proc: { p50: 1ms, p99: 2ms, mean: 3ms } }
",
    )
    .unwrap_err();
    assert!(err.msg.contains("{ mean: 5ms }"), "{err}");
}

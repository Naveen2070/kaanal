use kaanal_core::{Config, NodeKind, Topology};
use std::num::NonZeroU32;
use std::time::Duration;

const SAMPLE: &str = r#"
version: 1
seed: 42
port: 8080
nodes:
  nginx: { type: gateway,  flavor: nginx, proc: { p50: 1ms, p99: 3ms } }
  app:   { type: service,  spec: openapi.yaml, proc: 5ms }
  db:    { type: database, flavor: postgres,
           ops: { read: { p50: 4ms, p99: 20ms }, write: 8ms, scan: 30ms } }
edges:
  - { from: nginx, to: app, net: 1ms }
  - { from: app,   to: db }
routes:
  "GET /health": { body: { ok: true } }
"#;

#[test]
fn parses_sample() {
    let cfg = Config::from_yaml(SAMPLE).unwrap();
    assert_eq!(cfg.seed, 42);
    assert_eq!(cfg.port, 8080);
    assert_eq!(cfg.nodes.len(), 3);
    assert_eq!(cfg.nodes["nginx"].kind, NodeKind::Gateway);
    assert_eq!(cfg.nodes["app"].spec.as_deref(), Some("openapi.yaml"));
    assert_eq!(cfg.edges.len(), 2);
    assert_eq!(cfg.routes["GET /health"].status, 200);
    match cfg.nodes["app"].proc.as_ref().unwrap() {
        kaanal_core::config::DistConfig::Fixed(d) => assert_eq!(*d, Duration::from_millis(5)),
        other => panic!("expected fixed, got {other:?}"),
    }
}

#[test]
fn defaults_apply() {
    let cfg = Config::from_yaml("version: 1\nnodes: {}\n").unwrap();
    assert_eq!(cfg.seed, 0);
    assert_eq!(cfg.port, 8080);
    assert!(cfg.edges.is_empty());
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
            cfg.nodes["gw"].proc,
            Some(kaanal_core::config::DistConfig::Exp { mean }) if mean == Duration::from_millis(5)
        ),
        "{:?}",
        cfg.nodes["gw"].proc
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

#[test]
fn concurrency_parses_on_every_node_type_and_must_be_positive() {
    let yaml = "version: 1\nnodes:\n  gw: { type: gateway, concurrency: 4 }\n  app: { type: service, concurrency: 2 }\n  db: { type: database, concurrency: 1, ops: { read: 1ms, write: 1ms, scan: 1ms } }\nedges:\n  - { from: gw, to: app }\n  - { from: app, to: db }\n";
    let cfg = Config::from_yaml(yaml).unwrap();
    let got: Vec<u32> = ["gw", "app", "db"]
        .iter()
        .map(|n| cfg.nodes[*n].concurrency.unwrap().get())
        .collect();
    assert_eq!(got, [4, 2, 1]);
    let t = Topology::build(&cfg).unwrap();
    assert_eq!(
        t.nodes[t.id("db").unwrap()]
            .concurrency
            .map(NonZeroU32::get),
        Some(1)
    );
    for bad in ["0", "-1", "1.5"] {
        let err = Config::from_yaml(&format!(
            "version: 1\nnodes:\n  gw: {{ type: gateway, concurrency: {bad} }}\n"
        ))
        .unwrap_err();
        assert_eq!(err.line, Some(3), "{bad}: {err}");
    }
    let cfg = Config::from_yaml("version: 1\nnodes:\n  gw: { type: gateway }\n").unwrap();
    assert!(
        cfg.nodes["gw"].concurrency.is_none(),
        "absent means unlimited"
    );
}

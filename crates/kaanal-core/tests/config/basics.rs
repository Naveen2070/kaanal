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

/// A "billion laughs" text: `levels` anchors, each listing the previous one ten times.
pub fn bomb(levels: usize) -> String {
    use std::fmt::Write as _;
    let mut t = String::from(
        "version: 1\nnodes: {}\nx0: &a0 [lol, lol, lol, lol, lol, lol, lol, lol, lol, lol]\n",
    );
    for n in 1..=levels {
        let _ = writeln!(
            t,
            "x{n}: &a{n} [{}]",
            vec![format!("*a{}", n - 1); 10].join(", ")
        );
    }
    t
}

/// Runs `f` on its own thread and fails (rather than hangs) when it takes over 5 s.
fn within_5s<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(Duration::from_secs(5))
        .expect("the alias bomb was not refused quickly: the guard is gone or bypassed")
}

#[test]
fn an_alias_bomb_is_refused_before_it_expands() {
    let (two_docs_late, two_docs_early) = (
        format!("version: 1\nnodes: {{}}\n---\n{}", bomb(300)),
        format!("{}---\nversion: 1\nnodes: {{}}\n", bomb(300)),
    );
    // 20 KB (hundreds of levels), ~1 MiB (thousands), and a bomb in the second / first document
    for text in [bomb(300), bomb(20_000), two_docs_late, two_docs_early] {
        within_5s(move || {
            let e = Config::from_yaml(&text).unwrap_err();
            assert!(e.msg.contains("too much alias expansion"), "{e}");
            assert!(
                e.line.is_some(),
                "the alias that crossed the limit is placed"
            );
            assert!(kaanal_core::parse_scenario(&text).is_err());
            assert!(kaanal_core::config::parse_endpoints(&text).is_err());
            assert!(kaanal_core::preprocess(&text).is_err());
        });
    }
}

#[test]
fn an_anchor_reused_in_a_later_document_counts_each_document_on_its_own() {
    // a legit two-document text: `a` is defined again in document 2; serde refuses it (one
    // document only) but the guard must neither panic nor refuse it as a bomb
    let text = "version: 1\nx: &a [1, 2]\ny: *a\n---\nx: &a [3]\ny: *a\n";
    within_5s(move || {
        assert!(kaanal_core::preprocess(text).is_ok());
        let e = Config::from_yaml(text).unwrap_err();
        assert!(!e.msg.contains("alias expansion"), "{e}");
    });
}

#[test]
fn a_handful_of_legitimate_anchors_and_aliases_load() {
    let yaml = "version: 1
nodes:
  nginx: { type: gateway, proc: 1ms }
  app:   { type: service, proc: &p 1ms }
  api:   { type: service, proc: *p }
  db:    { type: database, ops: &ops { read: 1ms, write: 1ms, scan: 1ms } }
  db2:   { type: database, ops: *ops }
edges:
  - { from: nginx, to: app, net: &n 1ms }
  - { from: nginx, to: api, net: *n }
  - { from: app, to: db, net: *n }
  - { from: api, to: db2, net: *n }
";
    Config::from_yaml(yaml).unwrap();
}

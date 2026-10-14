use kaanal_core::{ChaosKey, ChaosLayer, ChaosState, ErrorKind, NodeChaos, merge};
use std::time::Duration;

#[allow(clippy::unnecessary_wraps)] // reads as a field value at every call site
fn ms(n: u64) -> Option<Duration> {
    Some(Duration::from_millis(n))
}

#[test]
fn merge_sums_latency_ors_down_unions_rates() {
    let a = ChaosLayer {
        latency_add: ms(200),
        error_rate: Some(0.5),
        down: Some(false),
        ..Default::default()
    };
    let b = ChaosLayer {
        latency_add: ms(1000),
        error_rate: Some(0.5),
        down: Some(true),
        ..Default::default()
    };
    let m = merge(&[&a, &b]);
    assert_eq!(m.latency_add, Duration::from_millis(1200));
    assert!((m.error_rate - 0.75).abs() < 1e-12, "1-(1-.5)(1-.5)");
    assert!(m.down);
}

#[test]
fn layer_without_kind_keeps_the_inherited_kind() {
    let node = ChaosLayer {
        error_kind: Some(ErrorKind::Reset),
        error_status: Some(502),
        error_rate: Some(1.0),
        ..Default::default()
    };
    let at = ChaosLayer {
        error_rate: Some(0.2),
        ..Default::default()
    }; // sets only the rate
    let m = merge(&[&node, &at]);
    assert_eq!(m.error_kind, ErrorKind::Reset);
    assert_eq!(m.error_status, 502);
}

#[test]
fn last_set_kind_and_status_win_and_defaults_apply_after() {
    let a = ChaosLayer {
        error_kind: Some(ErrorKind::Reset),
        ..Default::default()
    };
    let b = ChaosLayer {
        error_kind: Some(ErrorKind::Malformed),
        error_status: Some(500),
        ..Default::default()
    };
    let m = merge(&[&a, &b]);
    assert_eq!((m.error_kind, m.error_status), (ErrorKind::Malformed, 500));
    let none = merge(&[]);
    assert_eq!(none, NodeChaos::default());
    assert_eq!(
        (none.error_kind, none.error_status),
        (ErrorKind::Status, 503)
    );
}

#[test]
fn resolve_merges_node_then_endpoint_scoped_node_most_specific_last() {
    let mut s = ChaosState::default();
    s.set(
        ChaosKey::Node(0),
        ChaosLayer {
            latency_add: ms(200),
            ..Default::default()
        },
    );
    s.set(
        ChaosKey::NodeAt(0, "GET /orders".into()),
        ChaosLayer {
            latency_add: ms(1000),
            ..Default::default()
        },
    );
    assert_eq!(
        s.resolve(0, "GET /orders").latency_add,
        Duration::from_millis(1200)
    );
    assert_eq!(
        s.resolve(0, "GET /other").latency_add,
        Duration::from_millis(200)
    );
    assert_eq!(s.resolve(1, "GET /orders").latency_add, Duration::ZERO);
}

#[test]
fn remove_deletes_exactly_the_named_key() {
    let mut s = ChaosState::default();
    s.set(
        ChaosKey::Node(0),
        ChaosLayer {
            down: Some(true),
            ..Default::default()
        },
    );
    s.set(
        ChaosKey::NodeAt(0, "GET /a".into()),
        ChaosLayer {
            down: Some(true),
            ..Default::default()
        },
    );
    assert!(s.remove(&ChaosKey::Node(0)));
    assert!(!s.remove(&ChaosKey::Node(0)));
    assert!(
        s.resolve(0, "GET /a").down,
        "NodeAt survives removal of Node"
    );
}

#[test]
fn validate_checks_only_the_fields_that_are_set() {
    assert!(ChaosLayer::default().validate().is_ok());
    assert!(
        ChaosLayer {
            error_rate: Some(1.5),
            ..Default::default()
        }
        .validate()
        .is_err()
    );
    assert!(
        ChaosLayer {
            latency_add: ms(300_001),
            ..Default::default()
        }
        .validate()
        .is_err()
    );
    assert!(
        ChaosLayer {
            error_status: Some(404),
            ..Default::default()
        }
        .validate()
        .is_err()
    );
    // the status is range-checked whatever the kind
    let l = ChaosLayer {
        error_kind: Some(ErrorKind::Reset),
        error_status: Some(404),
        ..Default::default()
    };
    assert!(l.validate().is_err());
    let l = ChaosLayer {
        error_kind: Some(ErrorKind::Reset),
        error_status: Some(503),
        ..Default::default()
    };
    assert!(l.validate().is_ok());
}

#[test]
fn old_collect_and_insert_still_work_and_first_error_is_deterministic() {
    let bad = NodeChaos {
        error_rate: 2.0,
        ..NodeChaos::default()
    };
    let s: ChaosState = [(3usize, bad.clone()), (1usize, bad)].into_iter().collect();
    let msg = s.first_error().unwrap();
    assert!(msg.contains("node 1"), "smallest key first: {msg}");
}

#[test]
fn first_error_names_the_endpoint_of_keyed_entries() {
    let bad = ChaosLayer {
        error_rate: Some(2.0),
        ..Default::default()
    };
    let mut s = ChaosState::default();
    s.set(ChaosKey::NodeAt(1, "GET /x".into()), bad.clone());
    assert!(s.first_error().unwrap().contains("GET /x"));
    let mut s = ChaosState::default();
    s.set(ChaosKey::Endpoint("GET /y".into()), bad);
    assert!(s.first_error().unwrap().contains("GET /y"));
}

#[test]
fn error_kind_parses_its_names() {
    assert_eq!(ErrorKind::parse("hang"), Some(ErrorKind::Hang));
    assert_eq!(ErrorKind::parse("nope"), None);
}

#[test]
fn a_single_rate_survives_merge_bit_for_bit() {
    for r in [0.3, 0.1, 0.0, 1.0] {
        let l = ChaosLayer {
            error_rate: Some(r),
            ..Default::default()
        };
        assert_eq!(merge(&[&l]).error_rate, r);
    }
    assert_eq!(merge(&[]).error_rate, 0.0);
}

#[test]
fn resolve_orders_node_then_node_at_and_leaves_the_endpoint_layer_to_the_front_door() {
    let mut s = ChaosState::default();
    let k = |kind| ChaosLayer {
        error_kind: Some(kind),
        ..Default::default()
    };
    s.set(ChaosKey::Node(0), k(ErrorKind::Reset));
    s.set(ChaosKey::Endpoint("GET /a".into()), k(ErrorKind::Hang));
    assert_eq!(s.resolve(0, "GET /a").error_kind, ErrorKind::Reset);
    assert_eq!(s.resolve(0, "GET /b").error_kind, ErrorKind::Reset);
    s.set(
        ChaosKey::NodeAt(0, "GET /a".into()),
        k(ErrorKind::Malformed),
    );
    assert_eq!(s.resolve(0, "GET /a").error_kind, ErrorKind::Malformed);
    assert!(s.endpoint("GET /a").is_some());
    assert!(s.endpoint("GET /b").is_none());
}

#[test]
fn fail_kind_names() {
    use kaanal_core::FailKind;
    let names = [
        FailKind::Status,
        FailKind::Reset,
        FailKind::Malformed,
        FailKind::Timeout,
    ]
    .map(FailKind::as_str);
    assert_eq!(names, ["status", "reset", "malformed", "timeout"]);
}

#[test]
fn three_layer_rate_union_skips_unset_layers() {
    let r = |error_rate| ChaosLayer {
        error_rate,
        ..Default::default()
    };
    let (a, none, b, c) = (r(Some(0.5)), r(None), r(Some(0.5)), r(Some(0.5)));
    for m in [merge(&[&a, &b, &c]), merge(&[&a, &none, &b, &c])] {
        assert!((m.error_rate - 0.875).abs() < 1e-12, "1-(1-.5)^3");
    }
}

#[test]
fn entries_are_sorted_nodes_then_endpoints_then_node_at() {
    let mut s = ChaosState::default();
    let l = ChaosLayer::default();
    s.set(ChaosKey::NodeAt(1, "b".into()), l.clone());
    s.set(ChaosKey::NodeAt(0, "z".into()), l.clone());
    s.set(ChaosKey::Endpoint("y".into()), l.clone());
    s.set(ChaosKey::Endpoint("x".into()), l.clone());
    s.set(ChaosKey::Node(2), l.clone());
    s.set(ChaosKey::Node(1), l);
    let keys: Vec<ChaosKey> = s.entries().into_iter().map(|(k, _)| k).collect();
    assert_eq!(
        keys,
        [
            ChaosKey::Node(1),
            ChaosKey::Node(2),
            ChaosKey::Endpoint("x".into()),
            ChaosKey::Endpoint("y".into()),
            ChaosKey::NodeAt(0, "z".into()),
            ChaosKey::NodeAt(1, "b".into()),
        ]
    );
}

use kaanal_core::{
    ChaosKey, ChaosLayer, ChaosState, ErrorKind, MAX_LATENCY_ADD, NodeChaos, compile_layers, merge,
};
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
        s.compile().visit(0, "GET /orders").unwrap().latency_add,
        Duration::from_millis(1200)
    );
    assert_eq!(
        s.compile().visit(0, "GET /other").unwrap().latency_add,
        Duration::from_millis(200)
    );
    assert!(s.compile().visit(1, "GET /orders").is_none());
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
        s.compile().visit(0, "GET /a").unwrap().down,
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
fn node_at_goes_before_node_in_a_layer_and_the_endpoint_layer_is_left_to_the_front_door() {
    let mut s = ChaosState::default();
    // B2b1: a kind only counts with its own rate, so each layer carries rate 1.0.
    let k = |kind| l(1.0, kind, 502);
    let first = |s: &ChaosState, ep: &str| s.compile().visit(0, ep).unwrap().hit(0.5).unwrap().0;
    s.set(ChaosKey::Node(0), k(ErrorKind::Reset));
    s.set(ChaosKey::Endpoint("GET /a".into()), k(ErrorKind::Hang));
    assert_eq!(first(&s, "GET /a"), ErrorKind::Reset);
    assert_eq!(first(&s, "GET /b"), ErrorKind::Reset);
    s.set(
        ChaosKey::NodeAt(0, "GET /a".into()),
        k(ErrorKind::Malformed),
    );
    assert_eq!(first(&s, "GET /a"), ErrorKind::Malformed);
    let e = s.compile();
    assert!(e.endpoint("GET /a").is_some());
    assert!(e.endpoint("GET /b").is_none());
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

fn l(rate: f64, kind: ErrorKind, status: u16) -> ChaosLayer {
    ChaosLayer {
        error_rate: Some(rate),
        error_kind: Some(kind),
        error_status: Some(status),
        ..ChaosLayer::default()
    }
}

fn state(entries: Vec<(ChaosKey, ChaosLayer)>) -> ChaosState {
    let mut s = ChaosState::default();
    for (k, v) in entries {
        s.set(k, v);
    }
    s
}

#[test]
fn one_fault_is_bit_identical_to_the_old_rule() {
    let s = state(vec![(ChaosKey::Node(0), l(0.3, ErrorKind::Status, 503))]);
    let e = s.compile();
    let r = e.visit(0, "GET /x").unwrap();
    assert_eq!(r.faults().len(), 1);
    assert_eq!(r.faults()[0].upto, 0.3); // not 0.30000000000000004
    assert_eq!(r.hit(0.29), Some((ErrorKind::Status, 503)));
    assert_eq!(r.hit(0.3), None); // roll < rate, as before
}

#[test]
fn mixed_faults_are_banded_top_layer_first() {
    let low = state(vec![(ChaosKey::Node(0), l(0.3, ErrorKind::Status, 503))]);
    let top = state(vec![(ChaosKey::Node(0), l(0.3, ErrorKind::Reset, 502))]);
    let e = compile_layers(&[&low, &top]);
    let r = e.visit(0, "GET /x").unwrap();
    assert_eq!(r.hit(0.10).unwrap().0, ErrorKind::Reset);
    assert_eq!(r.hit(0.40).unwrap().0, ErrorKind::Status); // second band (0.30..0.51)
    assert!(r.hit(0.52).is_none());
    assert!((r.rate() - 0.51).abs() < 1e-12);
}

#[test]
fn layer_order_beats_specificity() {
    let low = state(vec![(
        ChaosKey::NodeAt(0, "GET /x".into()),
        l(1.0, ErrorKind::Reset, 502),
    )]);
    let top = state(vec![(ChaosKey::Node(0), l(1.0, ErrorKind::Hang, 504))]);
    let e = compile_layers(&[&low, &top]);
    assert_eq!(
        e.visit(0, "GET /x").unwrap().hit(0.5).unwrap().0,
        ErrorKind::Hang
    );
}

#[test]
fn same_layer_tie_goes_to_the_more_specific_key() {
    let s = state(vec![
        (ChaosKey::Node(0), l(1.0, ErrorKind::Hang, 504)),
        (
            ChaosKey::NodeAt(0, "GET /x".into()),
            l(1.0, ErrorKind::Reset, 502),
        ),
    ]);
    let e = s.compile();
    assert_eq!(
        e.visit(0, "GET /x").unwrap().hit(0.5).unwrap().0,
        ErrorKind::Reset
    );
    assert_eq!(
        e.visit(0, "GET /y").unwrap().hit(0.5).unwrap().0,
        ErrorKind::Hang
    );
}

#[test]
fn kind_and_status_are_one_unit() {
    let low = state(vec![(ChaosKey::Node(0), l(0.5, ErrorKind::Hang, 504))]);
    let top = state(vec![(
        ChaosKey::Node(0),
        ChaosLayer {
            error_rate: Some(0.5),
            error_status: Some(500),
            ..ChaosLayer::default()
        },
    )]);
    let r = compile_layers(&[&low, &top]).visit(0, "e").unwrap().clone();
    assert_eq!(r.hit(0.1), Some((ErrorKind::Status, 500)));
    assert_eq!(r.hit(0.6).unwrap().0, ErrorKind::Hang); // no hang+500 hybrid anywhere
}

#[test]
fn override_drops_lower_layers_for_that_key_only() {
    let low = state(vec![
        (
            ChaosKey::Node(0),
            ChaosLayer {
                down: Some(true),
                ..l(1.0, ErrorKind::Reset, 502)
            },
        ),
        (
            ChaosKey::Node(1),
            ChaosLayer {
                down: Some(true),
                ..ChaosLayer::default()
            },
        ),
    ]);
    let top = state(vec![(
        ChaosKey::Node(0),
        ChaosLayer {
            down: Some(false),
            error_rate: Some(0.0),
            replace: Some(true),
            ..ChaosLayer::default()
        },
    )]);
    let e = compile_layers(&[&low, &top]);
    let r0 = e.visit(0, "e").unwrap();
    assert!(!r0.down && r0.hit(0.0).is_none()); // calmed
    assert!(e.visit(1, "e").unwrap().down); // other key untouched
}

#[test]
fn latency_is_clamped_after_summing_everywhere() {
    let big = ChaosLayer {
        latency_add: Some(Duration::from_secs(240)),
        ..ChaosLayer::default()
    };
    let a = state(vec![
        (ChaosKey::Node(0), big.clone()),
        (ChaosKey::Endpoint("GET /x".into()), big.clone()),
        (ChaosKey::NodeAt(0, "GET /x".into()), big),
    ]);
    let e = compile_layers(&[&a, &a]); // 2 x 240 s per key
    assert_eq!(e.visit(0, "GET /x").unwrap().latency_add, MAX_LATENCY_ADD);
    assert_eq!(e.endpoint("GET /x").unwrap().latency_add, MAX_LATENCY_ADD);
}

#[test]
fn faults_past_the_cap_fold_into_the_last_band() {
    let layers: Vec<ChaosState> = (0..10)
        .map(|_| state(vec![(ChaosKey::Node(0), l(0.1, ErrorKind::Reset, 502))]))
        .collect();
    let refs: Vec<&ChaosState> = layers.iter().collect();
    let r = compile_layers(&refs).visit(0, "e").unwrap().clone();
    assert_eq!(r.faults().len(), kaanal_core::MAX_FAULTS);
    assert!((r.rate() - (1.0 - 0.9f64.powi(10))).abs() < 1e-9);
    // The two lowest layers (hang) fold into the last band, which keeps its own kind (reset).
    let hang = state(vec![(ChaosKey::Node(0), l(0.1, ErrorKind::Hang, 504))]);
    let mut stack: Vec<&ChaosState> = vec![&hang, &hang];
    stack.extend(layers.iter().take(8));
    let r = compile_layers(&stack).visit(0, "e").unwrap().clone();
    assert_eq!(r.faults().len(), kaanal_core::MAX_FAULTS);
    assert!(r.faults().iter().all(|f| f.kind == ErrorKind::Reset));
    assert!((r.rate() - (1.0 - 0.9f64.powi(10))).abs() < 1e-9);
    assert_eq!(r.hit(r.rate() - 1e-9).unwrap().0, ErrorKind::Reset);
}

#[test]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn compile_matches_a_slow_reference_over_random_stacks() {
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    for _ in 0..200 {
        let n_layers = 1 + (rnd() * 4.0) as usize;
        let layers: Vec<ChaosState> = (0..n_layers)
            .map(|_| {
                let mut s = ChaosState::default();
                let pick = |a: f64, b: f64| {
                    let k = [
                        ErrorKind::Status,
                        ErrorKind::Reset,
                        ErrorKind::Hang,
                        ErrorKind::Malformed,
                    ][(a * 4.0) as usize];
                    (k, 500 + (b * 5.0) as u16)
                };
                if rnd() < 0.7 {
                    let (k, st) = pick(rnd(), rnd());
                    s.set(ChaosKey::Node(0), l(rnd(), k, st));
                }
                if rnd() < 0.4 {
                    let (k, st) = pick(rnd(), rnd());
                    s.set(ChaosKey::NodeAt(0, "e".into()), l(rnd(), k, st));
                }
                s
            })
            .collect();
        let refs: Vec<&ChaosState> = layers.iter().collect();
        let got = compile_layers(&refs);
        // Layer-major reference: top layer first, node-at before node within a layer.
        let mut want: Vec<(f64, ErrorKind, u16)> = Vec::new();
        for s in layers.iter().rev() {
            for k in [ChaosKey::NodeAt(0, "e".into()), ChaosKey::Node(0)] {
                if let Some(x) = s.layer(&k)
                    && let Some(r) = x.error_rate
                    && r > 0.0
                {
                    let u = want.last().map_or(0.0, |w| w.0);
                    let upto = if want.is_empty() { r } else { u + r - u * r };
                    want.push((upto, x.error_kind.unwrap(), x.error_status.unwrap()));
                }
            }
        }
        let have = got.visit(0, "e").map_or(&[][..], |r| r.faults());
        assert_eq!(have.len(), want.len());
        for (h, w) in have.iter().zip(&want) {
            assert!((h.upto - w.0).abs() < 1e-12, "{} vs {}", h.upto, w.0);
            assert_eq!((h.kind, h.status), (w.1, w.2));
        }
    }
}

#[test]
fn empty_state_is_empty() {
    assert!(ChaosState::default().compile().is_empty());
}

fn down_layer() -> ChaosLayer {
    ChaosLayer {
        down: Some(true),
        ..ChaosLayer::default()
    }
}

fn calm(replace: bool) -> ChaosLayer {
    ChaosLayer {
        down: Some(false),
        error_rate: Some(0.0),
        replace: Some(replace),
        ..ChaosLayer::default()
    }
}

#[test]
fn node_override_calms_the_nodes_pairs_from_lower_layers() {
    let scenario = state(vec![(ChaosKey::NodeAt(0, "GET /x".into()), down_layer())]);
    let live = state(vec![(ChaosKey::Node(0), calm(true))]);
    let e = compile_layers(&[&scenario, &live]);
    assert!(!e.visit(0, "GET /x").unwrap().down);
    // a pair set in the same or a higher layer still applies
    let above = state(vec![(ChaosKey::NodeAt(0, "GET /x".into()), down_layer())]);
    let e = compile_layers(&[&scenario, &live, &above]);
    assert!(e.visit(0, "GET /x").unwrap().down);
}

#[test]
fn pair_override_leaves_node_level_entries() {
    let low = state(vec![
        (ChaosKey::Node(0), down_layer()),
        (ChaosKey::NodeAt(0, "GET /x".into()), down_layer()),
    ]);
    let top = state(vec![(ChaosKey::NodeAt(0, "GET /x".into()), calm(true))]);
    let e = compile_layers(&[&low, &top]);
    assert!(e.visit(0, "GET /x").unwrap().down, "node-level down stays");
    assert!(e.visit(0, "GET /y").unwrap().down);
}

#[test]
fn endpoint_override_only_touches_that_endpoint() {
    let low = state(vec![
        (ChaosKey::Endpoint("GET /x".into()), down_layer()),
        (ChaosKey::Endpoint("GET /y".into()), down_layer()),
        (ChaosKey::Node(0), down_layer()),
    ]);
    let top = state(vec![(ChaosKey::Endpoint("GET /x".into()), calm(true))]);
    let e = compile_layers(&[&low, &top]);
    assert!(!e.endpoint("GET /x").unwrap().down);
    assert!(e.endpoint("GET /y").unwrap().down);
    assert!(e.visit(0, "GET /x").unwrap().down);
}

#[test]
fn override_also_drops_lower_latency() {
    let slow = ChaosLayer {
        latency_add: Some(Duration::from_secs(2)),
        ..ChaosLayer::default()
    };
    let low = state(vec![(ChaosKey::Node(0), slow)]);
    let top = state(vec![(
        ChaosKey::Node(0),
        ChaosLayer {
            latency_add: Some(Duration::from_millis(5)),
            replace: Some(true),
            ..ChaosLayer::default()
        },
    )]);
    let e = compile_layers(&[&low, &top]);
    assert_eq!(
        e.visit(0, "e").unwrap().latency_add,
        Duration::from_millis(5)
    );
}

#[test]
fn a_kind_without_a_rate_adds_no_fault_and_warns() {
    let mut s = state(vec![
        (ChaosKey::Node(0), l(0.3, ErrorKind::Status, 503)),
        (
            ChaosKey::NodeAt(0, "GET /x".into()),
            ChaosLayer {
                error_kind: Some(ErrorKind::Hang),
                ..ChaosLayer::default()
            },
        ),
    ]);
    let r = s.compile().visit(0, "GET /x").unwrap().clone();
    assert_eq!(r.faults().len(), 1);
    assert_eq!(r.hit(0.2), Some((ErrorKind::Status, 503)));
    assert!(r.hit(0.35).is_none());
    assert_eq!(
        s.warnings(),
        [
            "chaos on node 0 at endpoint 'GET /x': error_kind/error_status have no effect without error_rate"
        ]
    );
    s.set(
        ChaosKey::NodeAt(0, "GET /x".into()),
        l(0.1, ErrorKind::Hang, 504),
    );
    assert_eq!(s.warnings().len(), 0);
}

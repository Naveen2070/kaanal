use kaanal_core::{
    ChaosKey, ChaosLayer, ChaosState, ErrorKind, MAX_FAULTS, MAX_LATENCY_ADD, NodeChaos,
    compile_layers,
};
use std::time::Duration;

#[allow(clippy::unnecessary_wraps)] // reads as a field value at every call site
fn ms(n: u64) -> Option<Duration> {
    Some(Duration::from_millis(n))
}

#[test]
fn dense_fills_defaults_for_one_layer() {
    assert_eq!(ChaosLayer::default().dense(), NodeChaos::default());
    let l = ChaosLayer {
        latency_add: ms(40),
        error_rate: Some(0.3),
        error_kind: Some(ErrorKind::Reset),
        down: Some(false),
        ..Default::default()
    };
    let d = l.dense();
    assert_eq!(d.latency_add, Duration::from_millis(40));
    assert_eq!(d.error_rate, 0.3, "bit for bit");
    assert_eq!((d.error_kind, d.error_status), (ErrorKind::Reset, 503));
    assert!(!d.down);
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

/// Slow, obvious model of what `compile_layers` must do for one key: the entries are
/// `(layer index, specificity, layer)`; a `replace` drops everything below it.
type Model = Vec<(usize, u8, ChaosLayer)>;

fn truncate_at_replace(v: &mut Model) {
    if let Some(p) = v.iter().rposition(|e| e.2.replace == Some(true)) {
        v.drain(..p);
    }
}

/// Latency, down and failure bands (cumulative `upto`, kind, status) of a model.
fn fold(mut v: Model) -> (Duration, bool, Vec<(f64, ErrorKind, u16)>) {
    v.sort_by_key(|e| std::cmp::Reverse((e.0, e.1)));
    let (mut lat, mut down) = (Duration::ZERO, false);
    let mut bands: Vec<(f64, ErrorKind, u16)> = Vec::new();
    for (_, _, x) in &v {
        lat += x.latency_add.unwrap_or_default();
        down |= x.down == Some(true);
        if let Some(r) = x.error_rate
            && r > 0.0
        {
            let upto = bands.last().map_or(r, |b| b.0 + r - b.0 * r);
            let band = (
                upto,
                x.error_kind.unwrap_or(ErrorKind::Status),
                x.error_status.unwrap_or(503),
            );
            if bands.len() < MAX_FAULTS {
                bands.push(band);
            } else if let Some(last) = bands.last_mut() {
                last.0 = upto; // fold the tail into the last band
            }
        }
    }
    (lat.min(MAX_LATENCY_ADD), down, bands)
}

fn assert_resolved(got: Option<&kaanal_core::Resolved>, want: &Model, what: &str) {
    let (lat, down, bands) = fold(want.clone());
    let Some(r) = got else {
        assert!(
            lat.is_zero() && !down && bands.is_empty(),
            "{what}: missing entry"
        );
        return;
    };
    assert_eq!(r.latency_add, lat, "{what}: latency");
    assert_eq!(r.down, down, "{what}: down");
    assert_eq!(r.faults().len(), bands.len(), "{what}: bands");
    for (h, w) in r.faults().iter().zip(&bands) {
        assert!(
            (h.upto - w.0).abs() < 1e-12,
            "{what}: {} vs {}",
            h.upto,
            w.0
        );
        assert_eq!((h.kind, h.status), (w.1, w.2), "{what}");
    }
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
    let ep = || "e".to_string();
    for _ in 0..400 {
        // up to 12 layers, so some stacks pass MAX_FAULTS on one key
        let n_layers = 1 + (rnd() * 12.0) as usize;
        let layers: Vec<ChaosState> = (0..n_layers)
            .map(|_| {
                let mut s = ChaosState::default();
                for key in [
                    ChaosKey::Node(0),
                    ChaosKey::NodeAt(0, ep()),
                    ChaosKey::Endpoint(ep()),
                ] {
                    if rnd() > 0.6 {
                        continue;
                    }
                    let kind = [
                        ErrorKind::Status,
                        ErrorKind::Reset,
                        ErrorKind::Hang,
                        ErrorKind::Malformed,
                    ][(rnd() * 4.0) as usize];
                    let mut layer = l(rnd() * 0.5, kind, 500 + (rnd() * 5.0) as u16);
                    if rnd() < 0.3 {
                        layer.error_rate = None; // a kind without a rate adds nothing
                    }
                    if rnd() < 0.5 {
                        // up to 150 s, so stacks sometimes pass the 5 minute clamp
                        layer.latency_add = Some(Duration::from_millis((rnd() * 150_000.0) as u64));
                    }
                    if rnd() < 0.2 {
                        layer.down = Some(rnd() < 0.7);
                    }
                    if rnd() < 0.2 {
                        layer.replace = Some(true);
                    }
                    s.set(key, layer);
                }
                s
            })
            .collect();
        let refs: Vec<&ChaosState> = layers.iter().collect();
        let got = compile_layers(&refs);

        let pick = |key: &ChaosKey, spec: u8| -> Model {
            layers
                .iter()
                .enumerate()
                .filter_map(|(i, s)| s.layer(key).map(|x| (i, spec, x.clone())))
                .collect()
        };
        let mut node = pick(&ChaosKey::Node(0), 0);
        truncate_at_replace(&mut node);
        // A node override also drops the lower layers' pair entries.
        let floor = layers
            .iter()
            .rposition(|s| {
                s.layer(&ChaosKey::Node(0))
                    .is_some_and(|x| x.replace == Some(true))
            })
            .unwrap_or(0);
        let mut at = pick(&ChaosKey::NodeAt(0, ep()), 1);
        truncate_at_replace(&mut at);
        at.retain(|x| x.0 >= floor);
        let has_pair = !at.is_empty()
            || layers
                .iter()
                .any(|s| s.layer(&ChaosKey::NodeAt(0, ep())).is_some());
        let visit: Model = if has_pair {
            node.iter().cloned().chain(at).collect()
        } else {
            node
        };
        assert_resolved(got.visit(0, "e"), &visit, "visit");
        let mut endpoint = pick(&ChaosKey::Endpoint(ep()), 0);
        truncate_at_replace(&mut endpoint);
        assert_resolved(got.endpoint("e"), &endpoint, "endpoint");
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

#![expect(
    clippy::many_single_char_names,
    reason = "short fixture names (t, e, c, s, w) keep the scenario tests readable"
)]

use kaanal_core::chain::{EndpointsInput, RouteSpec, compile};
use kaanal_core::{
    ChainRequest, ChaosKey, ChaosLayer, ChaosStack, Config, Endpoints, ErrorKind, HopKind,
    PinEntry, PinLayer, PinSource, PinState, PlanItem, Scenario, ScenarioConfig, ScenarioSet,
    Source, StateOverride, StateOverrides, Topology, execute, parse_scenario, resolve,
    resolve_scenario, sample_plan, validate_entry,
};
use std::time::Duration;

const BASE: &str = r#"
version: 1
nodes:
  nginx: { type: gateway, proc: { p50: 1ms, p99: 3ms } }
  app:   { type: service, spec: openapi.yaml, proc: { p50: 5ms, p99: 20ms } }
  db:    { type: database, ops: { read: { p50: 4ms, p99: 20ms }, write: 8ms, scan: 30ms } }
edges:
  - { from: nginx, to: app, net: 1ms }
  - { from: app, to: db, net: { p50: 1ms, p99: 2ms } }
endpoints:
  "GET /health": { override: { body: { ok: true } } }
"#;

fn build(src: &str) -> (Topology, Endpoints) {
    let cfg = Config::from_yaml(src).unwrap();
    let t = Topology::build(&cfg).unwrap();
    let routes = [RouteSpec::inferred("GET /users/{id}", t.id("app").unwrap())];
    let e = compile(&t, &routes, EndpointsInput::inline(&cfg)).unwrap();
    (t, e)
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn pin(state: PinState) -> PinEntry {
    PinEntry {
        state,
        status: None,
        delay: None,
    }
}

fn scenario(name: &str, pins: &[(&str, PinState)]) -> Scenario {
    let mut layer = PinLayer::default();
    for (k, s) in pins {
        layer.insert((*k).to_string(), pin(*s));
    }
    Scenario {
        name: name.into(),
        description: None,
        layers: kaanal_core::ChaosState::default(),
        pins: layer,
    }
}

fn cfg(y: &str) -> ScenarioConfig {
    serde_yaml_ng::from_str(y).unwrap()
}

#[test]
fn next_cycles_and_skips_disallowed() {
    use PinState::*;
    let all = |_| true;
    assert_eq!(PinState::next(None, all), Loading);
    let mut seen = vec![];
    let mut cur = None;
    for _ in 0..6 {
        let n = PinState::next(cur, all);
        seen.push(n);
        cur = Some(n);
    }
    assert_eq!(seen, [Loading, Empty, Error, Slow, Success, Loading]);
    let no_empty = |s| s != Empty;
    let mut cur = None;
    let mut four = vec![];
    for _ in 0..5 {
        let n = PinState::next(cur, no_empty);
        four.push(n);
        cur = Some(n);
    }
    assert_eq!(four, [Loading, Error, Slow, Success, Loading]);
    assert_eq!(PinState::parse("slow"), Some(Slow));
    assert_eq!(PinState::parse("Slow"), None);
    assert_eq!(Slow.name(), "slow");
}

#[test]
fn validate_entry_table() {
    use PinState::*;
    let ms = Duration::from_millis;
    let ok = |s, st, d| validate_entry(s, st, d).is_ok();
    assert!(ok(Success, None, None));
    assert!(!ok(Success, Some(200), None));
    assert!(!ok(Success, None, Some(secs(1))));
    assert!(ok(Error, Some(400), None) && ok(Error, Some(599), None));
    assert!(!ok(Error, Some(399), None) && !ok(Error, Some(600), None));
    assert!(ok(Empty, Some(200), None) && ok(Empty, Some(299), None));
    assert!(!ok(Empty, Some(300), None) && !ok(Empty, Some(199), None));
    assert!(!ok(Loading, Some(200), None) && !ok(Slow, Some(500), None));
    for s in [Loading, Slow, Error, Empty] {
        assert!(ok(s, None, Some(ms(1))), "{s:?}");
        assert!(ok(s, None, Some(secs(300))), "{s:?}");
        assert!(!ok(s, None, Some(Duration::ZERO)), "{s:?}");
        assert!(!ok(s, None, Some(secs(300) + ms(1))), "{s:?}");
    }
}

#[test]
fn resolve_precedence_and_defaults() {
    use PinState::*;
    let none = StateOverrides::default();
    let o = resolve(&pin(Loading), &none);
    assert_eq!((o.state, o.delay, o.status), (Loading, secs(10), None));
    assert_eq!(resolve(&pin(Slow), &none).delay, secs(2));
    assert_eq!(resolve(&pin(Empty), &none).delay, Duration::ZERO);
    let e = resolve(&pin(Error), &none);
    assert_eq!((e.status, e.body), (Some(500), None));
    assert_eq!(resolve(&pin(Success), &none).delay, Duration::ZERO);

    let mut ov = StateOverrides::default();
    ov.set(
        Error,
        StateOverride {
            status: Some(502),
            body: Some(serde_json::json!({"x": 1})),
            delay: Some(secs(3)),
        },
    );
    let e = resolve(&pin(Error), &ov);
    assert_eq!(
        (e.status, e.body.clone(), e.delay),
        (Some(502), Some(serde_json::json!({"x": 1})), secs(3))
    );
    // the entry beats `states:`
    let mine = PinEntry {
        state: Error,
        status: Some(429),
        delay: Some(secs(1)),
    };
    let e = resolve(&mine, &ov);
    assert_eq!((e.status, e.delay), (Some(429), secs(1)));
    assert_eq!(e.body, Some(serde_json::json!({"x": 1})));
}

#[test]
fn fold_higher_layer_wins_whole() {
    let mut set = ScenarioSet::default();
    set.insert(
        scenario("a", &[("GET /x", PinState::Loading)]),
        Source::Runtime,
    )
    .unwrap();
    set.insert(
        scenario(
            "b",
            &[("GET /x", PinState::Error), ("GET /y", PinState::Slow)],
        ),
        Source::Runtime,
    )
    .unwrap();
    let mut st = ChaosStack::default();
    st.activate(&set, "a").unwrap();
    st.activate(&set, "b").unwrap();
    let p = st.compile_pins(&set);
    let x = p.get("GET /x").unwrap();
    assert_eq!(x.entry.state, PinState::Error);
    assert_eq!(x.source, PinSource::Scenario("b".into()));
    assert_eq!(p.get("GET /y").unwrap().entry.state, PinState::Slow);
    // re-activating a moves it to the top
    st.activate(&set, "a").unwrap();
    let p = st.compile_pins(&set);
    assert_eq!(p.get("GET /x").unwrap().entry.state, PinState::Loading);
    assert!(!p.is_empty());
    assert_eq!(p.iter().count(), 2);
}

#[test]
fn success_masks_a_lower_pin() {
    let mut set = ScenarioSet::default();
    set.insert(
        scenario("a", &[("GET /x", PinState::Error)]),
        Source::Runtime,
    )
    .unwrap();
    let mut st = ChaosStack::default();
    st.activate(&set, "a").unwrap();
    st.live_pins_mut()
        .insert("GET /x".into(), pin(PinState::Success));
    let r = st.compile_pins(&set);
    let r = r.get("GET /x").unwrap();
    assert_eq!(r.entry.state, PinState::Success);
    assert_eq!(r.source, PinSource::Live);
}

#[test]
fn live_beats_scenarios() {
    let mut set = ScenarioSet::default();
    set.insert(
        scenario("a", &[("GET /x", PinState::Empty)]),
        Source::Runtime,
    )
    .unwrap();
    let mut st = ChaosStack::default();
    st.activate(&set, "a").unwrap();
    st.live_pins_mut()
        .insert("GET /x".into(), pin(PinState::Slow));
    let p = st.compile_pins(&set);
    assert_eq!(p.get("GET /x").unwrap().entry.state, PinState::Slow);
    st.live_pins_mut().remove("GET /x");
    assert_eq!(
        st.compile_pins(&set).get("GET /x").unwrap().source,
        PinSource::Scenario("a".into())
    );
}

#[test]
fn clear_all_clears_live_pins() {
    let mut st = ChaosStack::default();
    st.live_pins_mut()
        .insert("GET /x".into(), pin(PinState::Slow));
    assert_eq!(st.live_pins().len(), 1);
    st.clear_all();
    assert!(st.live_pins().is_empty());
}

#[test]
fn scenario_with_only_pins_loads() {
    let (t, e) = build(BASE);
    let c =
        cfg("pins: [ { endpoint: \"get /users/{id}\", state: error, status: 503, delay: 1s } ]");
    let (s, w) = resolve_scenario("p", &c, &t, &e, &|_| true).unwrap();
    assert_eq!(w.len(), 0);
    assert_eq!(s.layers.entry_count(), 0);
    let got = s.pins.get("GET /users/{id}").unwrap();
    assert_eq!(
        (got.state, got.status, got.delay),
        (PinState::Error, Some(503), Some(secs(1)))
    );
}

#[test]
fn scenario_with_neither_is_an_error() {
    let (t, e) = build(BASE);
    for y in ["description: x", "chaos: []\npins: []", "pins: null"] {
        let err = resolve_scenario("p", &cfg(y), &t, &e, &|_| true).unwrap_err();
        assert!(
            err.message.contains("at least one of chaos or pins"),
            "{y}: {}",
            err.message
        );
    }
}

#[test]
fn pin_range_errors_are_placed_at_the_field() {
    let (t, e) = build(BASE);
    let at = |y: &str| {
        let err = resolve_scenario("p", &cfg(y), &t, &e, &|_| true).unwrap_err();
        format!("{:?}", err.path)
    };
    let p = at("pins: [ { endpoint: \"GET /users/{id}\", state: error, status: 200 } ]");
    assert!(p.contains("Index(0)") && p.contains("\"status\""), "{p}");
    let p = at("pins: [ { endpoint: \"GET /users/{id}\", state: loading, delay: 0s } ]");
    assert!(p.contains("\"delay\""), "{p}");
    let p = at("pins: [ { endpoint: nope, state: loading } ]");
    assert!(p.contains("\"endpoint\""), "{p}");
}

#[test]
fn duplicate_pin_endpoint_is_rejected_with_both_indexes() {
    let (t, e) = build(BASE);
    let c = cfg(
        "pins:\n  - { endpoint: \"GET /users/{id}\", state: loading }\n  - { endpoint: \"GET /health\", state: slow }\n  - { endpoint: \"get /users/{id}\", state: slow }\n",
    );
    let err = resolve_scenario("p", &c, &t, &e, &|_| true).unwrap_err();
    assert!(
        err.message.contains("pins[2]") && err.message.contains("pins[0]"),
        "{}",
        err.message
    );
}

#[test]
fn entries_count_chaos_and_pins() {
    let (t, e) = build(BASE);
    let chaos: Vec<String> = (0..200)
        .map(|i| format!("{{ endpoint: \"GET /e{i}\", down: true }}"))
        .collect();
    let pins: Vec<String> = (0..100)
        .map(|i| format!("{{ endpoint: \"GET /p{i}\", state: slow }}"))
        .collect();
    let c = cfg(&format!(
        "chaos: [{}]\npins: [{}]",
        chaos.join(","),
        pins.join(",")
    ));
    let err = resolve_scenario("p", &c, &t, &e, &|_| true).unwrap_err();
    assert!(err.message.contains("at most 256"), "{}", err.message);
}

#[test]
fn pin_on_synthesized_head_and_on_no_entry_endpoint_rejected() {
    let (t, e) = build(BASE);
    let c = cfg("pins: [ { endpoint: \"HEAD /users/{id}\", state: loading } ]");
    let err = resolve_scenario("p", &c, &t, &e, &|_| true).unwrap_err();
    assert!(err.message.contains("pin the GET route"), "{}", err.message);
    // a static route with no gateway has no entry node
    let src = BASE
        .replace(
            "  nginx: { type: gateway, proc: { p50: 1ms, p99: 3ms } }\n",
            "",
        )
        .replace("  - { from: nginx, to: app, net: 1ms }\n", "")
        .replace(
            "  \"GET /health\": { override: { body: { ok: true } } }\n",
            "  \"GET /users/{id}\": {}\n",
        );
    let cfg0 = Config::from_yaml(&src).unwrap();
    let topo = Topology::build(&cfg0).unwrap();
    let routes = [RouteSpec::inferred(
        "GET /users/{id}",
        topo.id("app").unwrap(),
    )];
    let eps = compile(&topo, &routes, EndpointsInput::inline(&cfg0)).unwrap();
    // the app owns the route, so it is the entry node: pinning works
    let c = cfg("pins: [ { endpoint: \"GET /users/{id}\", state: loading } ]");
    resolve_scenario("p", &c, &topo, &eps, &|_| true).unwrap();
    let ep = eps.get("GET /users/{id}").unwrap();
    assert!(ep.entry_node.is_some());
}

#[test]
fn empty_pin_rejected_when_empty_ok_false() {
    let (t, e) = build(BASE);
    let c = cfg("pins: [ { endpoint: \"GET /users/{id}\", state: empty } ]");
    let err = resolve_scenario("p", &c, &t, &e, &|_| false).unwrap_err();
    assert!(
        err.message.contains("no empty form for 'GET /users/{id}'")
            && err.message.contains("states.empty.body"),
        "{}",
        err.message
    );
    resolve_scenario("p", &c, &t, &e, &|_| true).unwrap();
    // only empty pins ask
    let c = cfg("pins: [ { endpoint: \"GET /users/{id}\", state: error } ]");
    resolve_scenario("p", &c, &t, &e, &|_| false).unwrap();
}

#[test]
fn pins_in_a_file_scenario_parse_with_positions() {
    let ok = parse_scenario("pins:\n  - { endpoint: \"GET /a\", state: slow, delay: 1500ms }\n")
        .unwrap();
    assert_eq!(ok.pins[0].delay, Some(Duration::from_millis(1500)));
    let e = parse_scenario("pins:\n  - { endpoint: \"GET /a\", state: slwo }\n").unwrap_err();
    assert_eq!(e.line, Some(2), "{e}");
    let e = parse_scenario("pins:\n  - { endpoint: \"GET /a\", state: slow, colour: red }\n")
        .unwrap_err();
    assert_eq!(e.line, Some(2), "{e}");
    assert!(
        parse_scenario("limits: {}\npins: [ { endpoint: \"GET /a\", state: slow } ]\n")
            .unwrap()
            .check_supported()
            .is_err()
    );
}

#[test]
fn endpoint_states_parse_and_reject_success_and_unknown_keys() {
    let with = |states: &str| {
        let src = BASE.replace(
            "endpoints:\n",
            &format!("endpoints:\n  \"GET /users/{{id}}\": {{ states: {states} }}\n"),
        );
        let cfg = Config::from_yaml(&src).unwrap();
        let t = Topology::build(&cfg).unwrap();
        let routes = [RouteSpec::inferred("GET /users/{id}", t.id("app").unwrap())];
        compile(&t, &routes, EndpointsInput::inline(&cfg))
    };
    let e = with("{ empty: { body: { items: [] } }, error: { status: 502, delay: 2s } }").unwrap();
    let st = &e.get("GET /users/{id}").unwrap().states;
    assert_eq!(
        st.empty.as_ref().unwrap().body,
        Some(serde_json::json!({"items": []}))
    );
    let er = st.error.as_ref().unwrap();
    assert_eq!((er.status, er.delay), (Some(502), Some(secs(2))));
    assert!(st.loading.is_none() && st.slow.is_none());
    assert!(e.get("GET /health").unwrap().states.empty.is_none());
    // a HEAD copy carries the GET's states
    assert!(e.get("HEAD /users/{id}").unwrap().states.empty.is_some());

    let m = with("{ success: {} }").err().unwrap().msg;
    assert!(m.contains("success"), "{m}");
    let m = with("{ boom: {} }").err().unwrap().msg;
    assert!(m.contains("boom"), "{m}");
    let m = with("{ error: { status: 200 } }").err().unwrap().msg;
    assert!(m.contains("400-599"), "{m}");
    let m = with("{ loading: { delay: 6m } }").err().unwrap().msg;
    assert!(m.contains("5 min"), "{m}");
    for st in ["loading", "slow"] {
        let m = with(&format!("{{ {st}: {{ body: {{ x: 1 }} }} }}"))
            .err()
            .unwrap()
            .msg;
        assert!(m.contains(&format!("a {st} override takes no body")), "{m}");
    }
}

// ---- plan delay -------------------------------------------------------------------------

fn req(ep: &kaanal_core::Endpoint, n: u64, seed: u64, d: Duration) -> ChainRequest<'_> {
    ChainRequest {
        endpoint: ep,
        n,
        seed,
        pin_delay: d,
    }
}

fn chaos_ep(key: &str, kind: ErrorKind) -> kaanal_core::EffectiveChaos {
    let mut s = kaanal_core::ChaosState::default();
    s.set(
        ChaosKey::Endpoint(key.into()),
        ChaosLayer {
            error_rate: Some(1.0),
            error_kind: Some(kind),
            ..ChaosLayer::default()
        },
    );
    s.compile()
}

#[test]
fn pin_delay_zero_plan_is_identical() {
    let (t, e) = build(BASE);
    // chaos on the front door exercises the failure, hang and latency branches
    let mut state = kaanal_core::ChaosState::default();
    state.set(
        ChaosKey::Endpoint("GET /users/{id}".into()),
        ChaosLayer {
            error_rate: Some(0.4),
            error_kind: Some(ErrorKind::Hang),
            latency_add: Some(Duration::from_millis(7)),
            ..ChaosLayer::default()
        },
    );
    state.set(
        ChaosKey::Node(t.id("db").unwrap()),
        ChaosLayer {
            error_rate: Some(0.3),
            ..ChaosLayer::default()
        },
    );
    let chaos = [
        kaanal_core::ChaosState::default().compile(),
        state.compile(),
    ];
    let mut hung = 0;
    for key in ["GET /users/{id}", "GET /health", "HEAD /users/{id}"] {
        let ep = e.get(key).unwrap();
        for c in &chaos {
            for seed in 0..40 {
                for n in 0..5 {
                    let (zero, zf) = sample_plan(&t, c, &req(ep, n, seed, Duration::ZERO));
                    let (with, wf) = sample_plan(&t, c, &req(ep, n, seed, secs(3)));
                    // the delayed plan is the zero plan shifted, behind one leading hop; the
                    // zero plan is today's plan, so a pin adds no draw and moves nothing else
                    assert_eq!(
                        with.items[1..],
                        shifted(&zero.items, secs(3))[..],
                        "{key} {seed} {n}"
                    );
                    assert_eq!(wf, zf.map(|(i, at)| (i + 1, at + secs(3))));
                    hung += usize::from(zf.is_some());
                }
            }
        }
    }
    assert!(hung > 0, "the sweep must reach a planned failure");
    // with no front-door chaos the zero plan has no delay hop at all
    let ep = e.get("GET /users/{id}").unwrap();
    let r = req(ep, 3, 42, Duration::ZERO);
    let (plan, _) = sample_plan(&t, &chaos[0], &r);
    assert!(
        plan.items
            .iter()
            .all(|i| !matches!(i, PlanItem::Hop(h) if h.kind == HopKind::Delay))
    );
    assert_eq!(execute(&t, &chaos[0], &r).plan, plan);
}

fn shifted(items: &[PlanItem], d: Duration) -> Vec<PlanItem> {
    items
        .iter()
        .map(|i| match i {
            PlanItem::Hop(h) => PlanItem::Hop(kaanal_core::Hop {
                start: h.start + d,
                ..h.clone()
            }),
            PlanItem::Fail {
                node,
                status,
                kind,
                at,
            } => PlanItem::Fail {
                node: *node,
                status: *status,
                kind: *kind,
                at: *at + d,
            },
            PlanItem::Span {
                node,
                timeout,
                at,
                size,
            } => PlanItem::Span {
                node: *node,
                timeout: *timeout,
                at: *at + d,
                size: *size,
            },
            other => other.clone(),
        })
        .collect()
}

#[test]
fn pin_delay_shifts_everything_without_new_draws() {
    let (t, e) = build(BASE);
    let ep = e.get("GET /users/{id}").unwrap();
    let c = kaanal_core::ChaosState::default().compile();
    for seed in 0..20 {
        let base = sample_plan(&t, &c, &req(ep, 1, seed, Duration::ZERO)).0;
        let with = sample_plan(&t, &c, &req(ep, 1, seed, secs(10))).0;
        let PlanItem::Hop(first) = &with.items[0] else {
            panic!("{:?}", with.items[0])
        };
        assert_eq!(
            (first.kind, first.start, first.dur),
            (HopKind::Delay, Duration::ZERO, secs(10))
        );
        assert_eq!(first.node, ep.entry_node.unwrap());
        assert_eq!(with.items[1..], shifted(&base.items, secs(10))[..]);
    }
}

#[test]
fn pin_delay_shifts_the_chain_and_stays_outside_the_timeout_span() {
    let src = BASE.replace(
        "nginx: { type: gateway, proc: { p50: 1ms, p99: 3ms } }",
        "nginx: { type: gateway, timeout: 5s, proc: { p50: 1ms, p99: 3ms } }",
    );
    let (t, e) = build(&src);
    let ep = e.get("GET /users/{id}").unwrap();
    let c = kaanal_core::ChaosState::default().compile();
    let r = req(ep, 0, 7, secs(10));
    let (plan, planned) = sample_plan(&t, &c, &r);
    assert!(planned.is_none());
    // the delay hop comes before the root span, which opens at 10 s
    assert!(
        matches!(&plan.items[0], PlanItem::Hop(h) if h.kind == HopKind::Delay && h.dur == secs(10))
    );
    let PlanItem::Span { at, timeout, .. } = plan.items[1] else {
        panic!("{:?}", plan.items[1])
    };
    assert_eq!((at, timeout), (secs(10), secs(5)));
    let out = execute(&t, &c, &r);
    assert_eq!(
        out.result,
        kaanal_core::ChainResult::Ok,
        "no deadline failure"
    );
    assert!(out.fail.is_none());
    let chain_start = out
        .hops
        .iter()
        .filter(|h| h.kind != HopKind::Delay)
        .map(|h| h.start)
        .min()
        .unwrap();
    assert_eq!(chain_start, secs(10));
    assert!(out.total() > secs(10) && out.total() < secs(11));
}

#[test]
fn pin_delay_survives_endpoint_failure_and_hang() {
    let src = BASE.replace(
        "nginx: { type: gateway, proc: { p50: 1ms, p99: 3ms } }",
        "nginx: { type: gateway, timeout: 5s, proc: { p50: 1ms, p99: 3ms } }",
    );
    let (t, e) = build(&src);
    let ep = e.get("GET /users/{id}").unwrap();
    let key = "GET /users/{id}";

    let fail = chaos_ep(key, ErrorKind::Status);
    let out = execute(&t, &fail, &req(ep, 0, 1, secs(10)));
    assert!(
        matches!(
            out.result,
            kaanal_core::ChainResult::Err { status: 503, .. }
        ),
        "{:?}",
        out.result
    );
    let Some(PlanItem::Fail { at, .. }) = out.fail.map(|i| &out.plan.items[i]) else {
        panic!()
    };
    assert_eq!(*at, secs(10));
    assert_eq!(out.total(), secs(10));

    let hang = chaos_ep(key, ErrorKind::Hang);
    let out = execute(&t, &hang, &req(ep, 0, 1, secs(10)));
    // the hang lasts the gateway timeout (5 s) and starts after the pin delay
    let hops: Vec<_> = out.hops.iter().map(|h| (h.kind, h.start, h.dur)).collect();
    assert_eq!(hops[0], (HopKind::Delay, Duration::ZERO, secs(10)));
    assert_eq!(hops[1], (HopKind::Delay, secs(10), secs(5)));
    assert!(
        matches!(
            out.result,
            kaanal_core::ChainResult::Err { status: 504, .. }
        ),
        "{:?}",
        out.result
    );
    let Some(PlanItem::Fail { at, .. }) = out.fail.map(|i| &out.plan.items[i]) else {
        panic!()
    };
    assert_eq!(*at, secs(15));
}

#[test]
fn endpoint_latency_hop_starts_after_the_pin_delay() {
    let (t, e) = build(BASE);
    let ep = e.get("GET /users/{id}").unwrap();
    let mut s = kaanal_core::ChaosState::default();
    s.set(
        ChaosKey::Endpoint("GET /users/{id}".into()),
        ChaosLayer {
            latency_add: Some(Duration::from_millis(50)),
            ..ChaosLayer::default()
        },
    );
    let (plan, _) = sample_plan(&t, &s.compile(), &req(ep, 0, 3, secs(2)));
    let delays: Vec<_> = plan
        .items
        .iter()
        .filter_map(|i| match i {
            PlanItem::Hop(h) if h.kind == HopKind::Delay => Some((h.start, h.dur)),
            _ => None,
        })
        .collect();
    assert_eq!(
        delays,
        [
            (Duration::ZERO, secs(2)),
            (secs(2), Duration::from_millis(50))
        ]
    );
    let first_chain = plan
        .items
        .iter()
        .find_map(|i| match i {
            PlanItem::Hop(h) if h.kind != HopKind::Delay => Some(h.start),
            _ => None,
        })
        .unwrap();
    assert_eq!(first_chain, secs(2) + Duration::from_millis(50));
}

/// Gateway and db with timeouts, a cache, and a `parallel` block: the constructs the pin delay
/// must shift (spans, forks and branches, cache hits, hangs, deadline cuts).
const RICH: &str = r#"
version: 1
nodes:
  nginx: { type: gateway, timeout: 5s, proc: { p50: 1ms, p99: 3ms } }
  app:   { type: service, spec: openapi.yaml, proc: { p50: 5ms, p99: 20ms } }
  c:     { type: cache, hit_ratio: 0.5, hit: 1ms, miss: 1ms }
  db:    { type: database, timeout: 12ms, ops: { read: { p50: 8ms, p99: 30ms }, write: 8ms, scan: 30ms } }
  db2:   { type: database, ops: { read: 6ms, write: 8ms, scan: 30ms } }
edges:
  - { from: nginx, to: app, net: 1ms }
  - { from: app, to: c, net: 1ms }
  - { from: app, to: db, net: 1ms }
  - { from: app, to: db2, net: 1ms }
endpoints:
  "GET /users/{id}":
    chain: [ { cache: c }, { parallel: [ db, db2 ] } ]
"#;

#[test]
fn pin_delay_sweep_covers_spans_forks_and_cache() {
    let (t, e) = build(RICH);
    let ep = e.get("GET /users/{id}").unwrap();
    let mut front = kaanal_core::ChaosState::default();
    front.set(
        ChaosKey::Endpoint("GET /users/{id}".into()),
        ChaosLayer {
            error_rate: Some(0.2),
            error_kind: Some(ErrorKind::Hang),
            latency_add: Some(Duration::from_millis(3)),
            ..ChaosLayer::default()
        },
    );
    let mut node = kaanal_core::ChaosState::default();
    node.set(
        ChaosKey::Node(t.id("db").unwrap()),
        ChaosLayer {
            error_rate: Some(0.3),
            error_kind: Some(ErrorKind::Hang),
            ..ChaosLayer::default()
        },
    );
    node.set(
        ChaosKey::Node(t.id("db2").unwrap()),
        ChaosLayer {
            error_rate: Some(0.2),
            ..ChaosLayer::default()
        },
    );
    let chaos = [
        kaanal_core::ChaosState::default().compile(),
        front.compile(),
        node.compile(),
    ];
    let (mut spans, mut forks, mut branches, mut hits) = (0, 0, 0, 0);
    let (mut gw_cuts, mut db_cuts) = (0, 0);
    for c in &chaos {
        for seed in 0..60 {
            for n in 0..5 {
                let (zero, zf) = sample_plan(&t, c, &req(ep, n, seed, Duration::ZERO));
                let (with, wf) = sample_plan(&t, c, &req(ep, n, seed, secs(3)));
                assert_eq!(
                    with.items[1..],
                    shifted(&zero.items, secs(3))[..],
                    "{seed} {n}"
                );
                assert_eq!(wf, zf.map(|(i, at)| (i + 1, at + secs(3))));
                // whole-plan equality with what `execute` plans at zero delay
                let r = req(ep, n, seed, Duration::ZERO);
                assert_eq!(execute(&t, c, &r).plan, zero);
                for i in &zero.items {
                    match i {
                        PlanItem::Span { .. } => spans += 1,
                        PlanItem::Fork { .. } => forks += 1,
                        PlanItem::Branch { .. } => branches += 1,
                        PlanItem::Hop(h) if h.kind == HopKind::CacheHit => hits += 1,
                        _ => {}
                    }
                }
                // a span deadline can fire at run time; it must also fire (or not) alike
                let a = execute(&t, c, &req(ep, n, seed, Duration::ZERO));
                let b = execute(&t, c, &req(ep, n, seed, secs(3)));
                assert_eq!(a.result, b.result, "{seed} {n}");
                if let kaanal_core::ChainResult::Err {
                    node, status: 504, ..
                } = a.result
                {
                    if node == t.id("nginx").unwrap() {
                        gw_cuts += 1;
                    } else if node == t.id("db").unwrap() {
                        db_cuts += 1;
                    }
                }
            }
        }
    }
    assert!(
        spans > 0 && forks > 0 && branches > 0 && hits > 0,
        "{spans} {forks} {branches} {hits}"
    );
    assert!(
        gw_cuts > 0 && db_cuts > 0,
        "gateway timeouts {gw_cuts}, db timeouts {db_cuts}"
    );
}

#[test]
fn states_are_checked_on_a_stale_entry_too() {
    let try_states = |states: &str| {
        let src = BASE.replace(
            "endpoints:
",
            &format!(
                "endpoints:
  \"GET /nothing\": {{ states: {states} }}
"
            ),
        );
        let cfg = Config::from_yaml(&src).unwrap();
        let t = Topology::build(&cfg).unwrap();
        let routes = [RouteSpec::inferred("GET /users/{id}", t.id("app").unwrap())];
        compile(&t, &routes, EndpointsInput::inline(&cfg))
    };
    let e = try_states("{ boom: {} }").err().unwrap();
    assert!(e.msg.contains("boom"), "{}", e.msg);
    let e = try_states("{ error: { status: 200 } }").err().unwrap();
    assert!(e.msg.contains("400-599"), "{}", e.msg);
    let ok = try_states("{ loading: {} }").unwrap();
    assert!(
        ok.warnings.iter().any(|w| w.contains("stale")),
        "{:?}",
        ok.warnings
    );
}

#[test]
fn schema_allows_null_pins() {
    let v = jsonschema::validator_for(&kaanal_core::config_schema()).unwrap();
    let mut doc: serde_json::Value = serde_yaml_ng::from_str(BASE).unwrap();
    doc["scenarios"] = serde_json::json!({
        "a": { "pins": null, "chaos": [ { "node": "db", "down": true } ] },
        "b": { "pins": [ { "endpoint": "GET /x", "state": "slow" } ] },
    });
    assert!(
        v.is_valid(&doc),
        "{:?}",
        v.iter_errors(&doc).next().map(|e| e.to_string())
    );
}

//! Pin resolution, layering, validation and loading of pins.

use crate::common::*;
use kaanal_core::chain::{EndpointsInput, RouteSpec, compile};
use kaanal_core::{
    ChaosStack, Config, PinEntry, PinSource, PinState, ScenarioSet, Source, StateOverride,
    StateOverrides, Topology, parse_scenario, resolve, resolve_scenario, validate_entry,
};
use std::time::Duration;

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

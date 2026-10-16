use kaanal_core::*;

fn sc(name: &str, key: ChaosKey, l: ChaosLayer) -> Scenario {
    let mut layers = ChaosState::default();
    layers.set(key, l);
    Scenario {
        name: name.into(),
        description: None,
        layers,
    }
}
fn rate(r: f64) -> ChaosLayer {
    ChaosLayer {
        error_rate: Some(r),
        ..ChaosLayer::default()
    }
}

#[test]
fn names_follow_the_grammar() {
    for ok in ["a", "payments-down", "x_1", "0abc"] {
        assert!(valid_name(ok).is_ok(), "{ok}");
    }
    for bad in [
        "",
        "A",
        "Payments",
        "-x",
        "_x",
        "a b",
        "a/b",
        "..",
        "a\\b",
        "a:b",
        "x.yaml",
        "active",
        "save",
        "reset",
        "con",
        "NUL",
        "com1",
        "lpt9",
        &"a".repeat(65),
    ] {
        assert!(valid_name(bad).is_err(), "{bad}");
    }
}

#[test]
fn endpoint_key_upper_cases_the_method() {
    assert_eq!(endpoint_key("get /users/{id}").unwrap(), "GET /users/{id}");
    assert!(endpoint_key("GET").is_err());
    assert!(endpoint_key("/x").is_err());
}

#[test]
fn activate_stacks_in_order_and_reactivate_moves_to_top() {
    let mut set = ScenarioSet::default();
    set.insert(sc("a", ChaosKey::Node(0), rate(0.5)), Source::Inline)
        .unwrap();
    set.insert(sc("b", ChaosKey::Node(0), rate(0.5)), Source::Inline)
        .unwrap();
    let mut st = ChaosStack::default();
    st.activate(&set, "a").unwrap();
    st.activate(&set, "b").unwrap();
    assert_eq!(st.active(), ["a", "b"]);
    st.activate(&set, "a").unwrap();
    assert_eq!(st.active(), ["b", "a"]);
    assert!(st.deactivate("b"));
    assert!(!st.deactivate("b"));
    assert_eq!(st.active(), ["a"]);
    assert!(st.activate(&set, "nope").is_err());
}

#[test]
fn live_is_always_on_top_and_deactivate_restores() {
    let mut set = ScenarioSet::default();
    set.insert(
        sc(
            "down",
            ChaosKey::Node(0),
            ChaosLayer {
                error_rate: Some(1.0),
                error_kind: Some(ErrorKind::Reset),
                ..ChaosLayer::default()
            },
        ),
        Source::Inline,
    )
    .unwrap();
    let mut st = ChaosStack::default();
    st.activate(&set, "down").unwrap();
    st.live_mut().set(
        ChaosKey::Node(0),
        ChaosLayer {
            error_rate: Some(1.0),
            error_kind: Some(ErrorKind::Hang),
            ..ChaosLayer::default()
        },
    );
    assert_eq!(
        st.compile(&set).visit(0, "e").unwrap().hit(0.5).unwrap().0,
        ErrorKind::Hang
    ); // live first
    st.live_mut().clear();
    assert_eq!(
        st.compile(&set).visit(0, "e").unwrap().hit(0.5).unwrap().0,
        ErrorKind::Reset
    );
    st.deactivate("down");
    assert!(st.compile(&set).is_empty());
}

#[test]
fn set_active_is_atomic() {
    let mut set = ScenarioSet::default();
    set.insert(sc("a", ChaosKey::Node(0), rate(0.1)), Source::Inline)
        .unwrap();
    let mut st = ChaosStack::default();
    st.activate(&set, "a").unwrap();
    assert!(st.set_active(&set, &["a".into(), "ghost".into()]).is_err());
    assert_eq!(st.active(), ["a"], "a failed set must change nothing");
    st.set_active(&set, &[]).unwrap();
    assert_eq!(st.active(), [] as [String; 0]);
}

#[test]
fn set_rules() {
    let mut set = ScenarioSet::default();
    set.insert(sc("a", ChaosKey::Node(0), rate(0.1)), Source::Inline)
        .unwrap();
    assert!(
        set.insert(sc("a", ChaosKey::Node(0), rate(0.2)), Source::File)
            .is_err(),
        "clash across sources"
    );
    set.insert(sc("r", ChaosKey::Node(0), rate(0.1)), Source::Runtime)
        .unwrap();
    assert!(
        set.remove("a").is_err(),
        "inline cannot be undefined at runtime"
    );
    assert!(set.remove("r").is_ok());
    assert!(
        set.insert(sc("Bad", ChaosKey::Node(0), rate(0.1)), Source::Runtime)
            .is_err()
    );
}

fn many(name: &str, n: usize) -> Scenario {
    let mut layers = ChaosState::default();
    for i in 0..n {
        layers.set(ChaosKey::Endpoint(format!("GET /e{i}")), rate(0.1));
    }
    Scenario {
        name: name.into(),
        description: None,
        layers,
    }
}

#[test]
fn name_edges() {
    assert!(valid_name(&"a".repeat(64)).is_ok());
    assert!(valid_name(&"a".repeat(65)).is_err());
    for bad in [
        "prn", "aux", "nul", "com9", "lpt1", "con.txt", "é", "ａ", "ı", "Ａ",
    ] {
        assert!(valid_name(bad).is_err(), "{bad}");
    }
    for ok in ["com0", "com10", "lpt0", "comx"] {
        assert!(valid_name(ok).is_ok(), "{ok}");
    }
}

#[test]
fn endpoint_key_edges() {
    assert_eq!(endpoint_key("get  /x").unwrap(), "GET /x");
    assert_eq!(endpoint_key("get /x ").unwrap(), "GET /x");
    assert_eq!(
        endpoint_key("GET").unwrap_err(),
        "invalid endpoint 'GET': expected 'METHOD /path'"
    );
}

#[test]
fn replace_rules() {
    let mut set = ScenarioSet::default();
    set.insert(sc("i", ChaosKey::Node(0), rate(0.1)), Source::Inline)
        .unwrap();
    set.insert(sc("f", ChaosKey::Node(0), rate(0.1)), Source::File)
        .unwrap();
    set.insert(sc("r1", ChaosKey::Node(0), rate(0.1)), Source::Runtime)
        .unwrap();
    set.insert(sc("r2", ChaosKey::Node(0), rate(0.1)), Source::Runtime)
        .unwrap();
    set.replace(sc("r1", ChaosKey::Node(0), rate(0.9))).unwrap();
    assert_eq!(set.names(), ["i", "f", "r1", "r2"], "position kept");
    assert_eq!(set.get("r1").unwrap().1, Source::Runtime);
    assert_eq!(
        set.get("r1")
            .unwrap()
            .0
            .layers
            .layer(&ChaosKey::Node(0))
            .unwrap()
            .error_rate,
        Some(0.9)
    );
    assert_eq!(
        set.replace(sc("i", ChaosKey::Node(0), rate(0.2)))
            .unwrap_err(),
        "scenario 'i' is defined in the config; save a new name or edit the file"
    );
    assert_eq!(
        set.replace(sc("f", ChaosKey::Node(0), rate(0.2)))
            .unwrap_err(),
        "scenario 'f' is defined in a scenario file; save a new name or edit the file"
    );
    // Unknown name goes through insert: creates, and obeys the name rules.
    set.replace(sc("new", ChaosKey::Node(0), rate(0.1)))
        .unwrap();
    assert_eq!(set.get("new").unwrap().1, Source::Runtime);
    assert!(
        set.replace(sc("Bad", ChaosKey::Node(0), rate(0.1)))
            .is_err()
    );
}

#[test]
fn replace_of_a_new_name_obeys_the_bound() {
    let mut set = ScenarioSet::default();
    for i in 0..MAX_SCENARIOS {
        set.insert(
            sc(&format!("s{i}"), ChaosKey::Node(0), rate(0.1)),
            Source::Runtime,
        )
        .unwrap();
    }
    assert!(
        set.replace(sc("extra", ChaosKey::Node(0), rate(0.1)))
            .is_err()
    );
    set.replace(sc("s0", ChaosKey::Node(0), rate(0.3))).unwrap();
}

#[test]
fn messages_name_the_sources() {
    let mut set = ScenarioSet::default();
    set.insert(sc("a", ChaosKey::Node(0), rate(0.1)), Source::Inline)
        .unwrap();
    set.insert(sc("r", ChaosKey::Node(0), rate(0.1)), Source::Runtime)
        .unwrap();
    assert_eq!(
        set.insert(sc("a", ChaosKey::Node(0), rate(0.1)), Source::File)
            .unwrap_err(),
        "scenario 'a' is defined in both the config and a scenario file"
    );
    assert_eq!(
        set.insert(sc("r", ChaosKey::Node(0), rate(0.1)), Source::Runtime)
            .unwrap_err(),
        "scenario 'r' is already defined in the control API"
    );
    assert_eq!(set.remove("ghost").unwrap_err(), "unknown scenario 'ghost'");
    assert!(set.remove("a").unwrap_err().contains("the config"));
}

#[test]
fn entries_are_bounded() {
    let mut set = ScenarioSet::default();
    set.insert(many("ok", MAX_ENTRIES), Source::Runtime)
        .unwrap();
    assert!(
        set.insert(many("big", MAX_ENTRIES + 1), Source::Runtime)
            .is_err()
    );
    assert!(set.replace(many("ok", MAX_ENTRIES + 1)).is_err());
    assert!(set.replace(many("big2", MAX_ENTRIES + 1)).is_err());
    assert_eq!(set.names(), ["ok"]);
    let mut l = ChaosState::default();
    l.set(ChaosKey::Node(0), rate(0.1));
    l.set(ChaosKey::Endpoint("GET /x".into()), rate(0.1));
    l.set(ChaosKey::NodeAt(0, "GET /x".into()), rate(0.1));
    l.set(ChaosKey::NodeAt(1, "GET /x".into()), rate(0.1));
    assert_eq!(l.entry_count(), 4);
}

#[test]
fn stale_active_names_are_harmless_and_prune_clears_them() {
    let mut set = ScenarioSet::default();
    set.insert(sc("a", ChaosKey::Node(0), rate(0.5)), Source::Runtime)
        .unwrap();
    let mut st = ChaosStack::default();
    st.activate(&set, "a").unwrap();
    set.remove("a").unwrap();
    assert!(st.compile(&set).is_empty());
    assert_eq!(st.active(), ["a"]);
    st.prune(&set);
    assert_eq!(st.active(), [] as [String; 0]);
}

#[test]
fn set_active_dedups_and_failure_leaves_a_deep_stack_alone() {
    let mut set = ScenarioSet::default();
    for n in ["a", "b", "c"] {
        set.insert(sc(n, ChaosKey::Node(0), rate(0.1)), Source::Inline)
            .unwrap();
    }
    let mut st = ChaosStack::default();
    st.set_active(&set, &["a".into(), "b".into(), "a".into()])
        .unwrap();
    assert_eq!(st.active(), ["b", "a"]);
    st.activate(&set, "c").unwrap();
    assert!(st.set_active(&set, &["a".into(), "ghost".into()]).is_err());
    assert_eq!(st.active(), ["b", "a", "c"]);
}

#[test]
fn reactivation_changes_the_winner() {
    let kind = |k| ChaosLayer {
        error_rate: Some(1.0),
        error_kind: Some(k),
        ..ChaosLayer::default()
    };
    let mut set = ScenarioSet::default();
    set.insert(
        sc("x", ChaosKey::Node(0), kind(ErrorKind::Reset)),
        Source::Inline,
    )
    .unwrap();
    set.insert(
        sc("y", ChaosKey::Node(0), kind(ErrorKind::Hang)),
        Source::Inline,
    )
    .unwrap();
    let win = |st: &ChaosStack| st.compile(&set).visit(0, "e").unwrap().hit(0.5).unwrap().0;
    let mut st = ChaosStack::default();
    st.activate(&set, "x").unwrap();
    st.activate(&set, "y").unwrap();
    assert_eq!(win(&st), ErrorKind::Hang);
    st.activate(&set, "x").unwrap();
    assert_eq!(win(&st), ErrorKind::Reset);
}

#[test]
fn bounds() {
    let mut set = ScenarioSet::default();
    for i in 0..MAX_SCENARIOS {
        set.insert(
            sc(&format!("s{i}"), ChaosKey::Node(0), rate(0.1)),
            Source::Runtime,
        )
        .unwrap();
    }
    assert!(
        set.insert(
            sc("one-more", ChaosKey::Node(0), rate(0.1)),
            Source::Runtime
        )
        .is_err()
    );
}

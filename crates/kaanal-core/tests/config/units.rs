//! Unit suffixes on byte and rate fields (`250KB`, `1MB/s`): the same file written with
//! integer bytes loads to the same values, and the bounds run after the conversion.

use kaanal_core::{Config, ScenarioConfig, Topology, config_schema};

/// An edge, a node pair and a journey, written with `bw` (edge), `out` (node) and `bytes`.
fn file(bw: &str, out: &str, bytes: &str) -> String {
    format!(
        "version: 1\nnodes:\n  gw: {{ type: gateway, bandwidth: {{ out: {out} }} }}\n  app: {{ type: service, bandwidth: {bw} }}\nedges:\n  - {{ from: gw, to: app, bandwidth: {bw} }}\ntraffic:\n  p:\n    rps: 1\n    journeys:\n      - {{ endpoint: 'GET /a', req_bytes: {bytes}, resp_bytes: {bytes} }}\n"
    )
}

#[test]
fn suffixes_load_to_the_same_values_as_integer_bytes() {
    let text = Config::from_yaml(&file("250KB", "'1MB/s'", "2KB")).unwrap();
    let ints = Config::from_yaml(&file("256000", "1048576", "2048")).unwrap();
    assert_eq!(text.edges[0].bandwidth, ints.edges[0].bandwidth);
    assert_eq!(text.edges[0].bandwidth, Some(256_000));
    for n in ["gw", "app"] {
        assert_eq!(text.nodes[n].bandwidth(), ints.nodes[n].bandwidth(), "{n}");
    }
    assert_eq!(text.nodes["gw"].bandwidth(), (Some(1 << 20), None));
    let (t, i) = (
        &text.traffic["p"].journeys[0],
        &ints.traffic["p"].journeys[0],
    );
    assert_eq!((t.req_bytes, t.resp_bytes), (i.req_bytes, i.resp_bytes));
    assert_eq!(t.req_bytes, Some(2048));
    // the compiled topologies agree too
    let (a, b) = (
        Topology::build(&text).unwrap(),
        Topology::build(&ints).unwrap(),
    );
    assert_eq!(a.edges[0].bandwidth, b.edges[0].bandwidth);
    assert_eq!(a.nodes[0].bandwidth, b.nodes[0].bandwidth);
    assert_eq!(a.nodes[1].bandwidth, b.nodes[1].bandwidth);
}

#[test]
fn every_spelling_is_accepted_in_yaml() {
    for bw in [
        "250KB",
        "'250 KB'",
        "250kb",
        "250KiB/s",
        "'250 kb/S'",
        "256000",
        "'256000B'",
    ] {
        let c = Config::from_yaml(&file(bw, "1", "1")).unwrap_or_else(|e| panic!("{bw}: {e}"));
        assert_eq!(c.edges[0].bandwidth, Some(256_000), "{bw}");
    }
}

#[test]
fn a_bound_still_fails_after_conversion() {
    let err = Config::from_yaml(&file("0KB", "1", "1"))
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("bandwidth must be at least 1 byte per second (got 0)"),
        "{err}"
    );
    let err = Config::from_yaml(&file("1", "'0 MB/s'", "1"))
        .unwrap_err()
        .to_string();
    assert!(err.contains("bandwidth must be at least 1"), "{err}");
    let err = Config::from_yaml(&file("1", "1", "2GB"))
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("req_bytes must be at most 1073741824 (1 GiB)"),
        "{err}"
    );
    assert!(
        Config::from_yaml(&file("1", "1", "1GB")).is_ok(),
        "1 GB is exactly the limit"
    );
}

#[test]
fn refusals_are_positioned_and_name_the_field() {
    let cases = [
        (
            file("5XB", "1", "1"),
            "4:",
            "field 'bandwidth': unknown unit in `5XB`; use B, KB, MB, GB, optionally /s for a rate",
        ),
        (file("1", "'-5KB'", "1"), "3:", "out: `-5KB` is negative"),
        (
            file("1", "1", "'5KB/s'"),
            "11:",
            "req_bytes: `5KB/s` is a size, not a rate; drop the `/s` (units: B, KB, MB, GB)",
        ),
        (
            file("1", "1", "1.5"),
            "11:",
            "req_bytes: 1.5 is not a whole number of bytes",
        ),
        (
            file("1", "1", "'KB'"),
            "11:",
            "req_bytes: `KB` has no number",
        ),
    ];
    for (src, at, want) in cases {
        let err = Config::from_yaml(&src).unwrap_err().to_string();
        assert!(
            err.starts_with(at) && err.contains(want),
            "want {at} {want}: {err}"
        );
        // the field is named once, never as `bandwidth: bandwidth:` or `field 'x': x:`
        assert!(
            !err.contains("bandwidth: bandwidth") && !err.contains("'bandwidth': bandwidth:"),
            "{err}"
        );
    }
    // the split form names the side
    let err = Config::from_yaml(&file("1", "'1 XB'", "1"))
        .unwrap_err()
        .to_string();
    assert!(err.contains("out: unknown unit in `1 XB`"), "{err}");
}

#[test]
fn a_scenario_cap_takes_a_unit_like_the_yaml() {
    let parse = |cap: &str| -> Result<ScenarioConfig, String> {
        kaanal_core::parse_scenario(&format!(
            "chaos:\n  - {{ edge: {{ from: gw, to: app }}, bandwidth_cap: {cap} }}\n"
        ))
        .map_err(|e| e.to_string())
    };
    for (cap, want) in [
        ("500000", 500_000),
        ("'500000B'", 500_000),
        ("'500KB/s'", 512_000),
        ("'488.28125 KiB/s'", 500_000),
    ] {
        let s = parse(cap).unwrap_or_else(|e| panic!("{cap}: {e}"));
        assert_eq!(s.chaos[0].bandwidth_cap, Some(want), "{cap}");
    }
    let err = parse("'5XB/s'").unwrap_err();
    assert!(
        err.contains("bandwidth_cap: unknown unit in `5XB/s`"),
        "{err}"
    );
}

#[test]
fn the_schema_takes_a_number_or_a_unit_text() {
    let v = jsonschema::validator_for(&config_schema()).unwrap();
    let doc: serde_json::Value = serde_yaml_ng::from_str(&file("250KB", "'1MB/s'", "2KB")).unwrap();
    assert!(
        v.is_valid(&doc),
        "{:?}",
        v.iter_errors(&doc)
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
    );
    for bad in ["5XB", "KB", "-5KB"] {
        let doc: serde_json::Value =
            serde_yaml_ng::from_str(&file(&format!("'{bad}'"), "1", "1")).unwrap();
        assert!(!v.is_valid(&doc), "{bad}");
    }
    // a size does not take `/s`
    let doc: serde_json::Value = serde_yaml_ng::from_str(&file("1", "1", "'2KB/s'")).unwrap();
    assert!(!v.is_valid(&doc));
}

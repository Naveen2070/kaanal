//! The `storage` node (an object store): its fields, what it refuses, who may call it (RULES 61).

use kaanal_core::{Config, Dist, NodeKind, Topology, presets};

fn load(nodes: &str, edges: &str) -> Result<Config, String> {
    Config::from_yaml(&format!("version: 1\nnodes:\n{nodes}edges:\n{edges}"))
        .map_err(|e| e.to_string())
}

/// One storage node `s3` behind a service `app`, `body` being the storage node's fields.
fn with(body: &str) -> Result<Config, String> {
    load(
        &format!("  app: {{ type: service }}\n  s3: {{ type: storage, {body} }}\n"),
        "  - { from: app, to: s3 }\n",
    )
}

#[test]
fn a_storage_node_loads_and_reads_its_fields() {
    let c = with("flavor: gcs, first_byte: { p50: 20ms, p99: 80ms }, throughput: 80MB/s").unwrap();
    let n = &c.nodes["s3"];
    assert_eq!(n.kind(), NodeKind::Storage);
    assert_eq!(n.kind().as_str(), "storage");
    assert_eq!(n.flavor(), Some("gcs"));
    assert!(n.proc().is_some(), "first_byte is the service time");
    assert!(NodeKind::ALL.contains(&NodeKind::Storage));
    assert!(!NodeKind::Storage.has_proc());
    let t = Topology::build(&c).unwrap();
    assert_eq!(t.nodes[t.id("s3").unwrap()].flavor.as_deref(), Some("gcs"));
}

#[test]
fn flavor_takes_s3_gcs_blob_and_refuses_the_rest_naming_the_choices() {
    for f in ["s3", "gcs", "blob"] {
        let c = with(&format!("flavor: {f}, first_byte: 20ms")).unwrap();
        assert_eq!(c.nodes["s3"].flavor(), Some(f));
    }
    let err = with("flavor: s4, first_byte: 20ms").unwrap_err();
    assert!(err.contains("s4"), "{err}");
    assert!(
        err.contains("s3") && err.contains("gcs") && err.contains("blob"),
        "{err}"
    );
    // the flavor is optional
    assert_eq!(with("first_byte: 20ms").unwrap().nodes["s3"].flavor(), None);
}

#[test]
fn first_byte_defaults_to_a_25ms_estimate_when_not_written() {
    // RULES 61 point 3: no published figure, so the default is a labelled estimate (Studio's too)
    let c = with("flavor: s3, throughput: 80MB/s").unwrap();
    let t = Topology::build(&c).unwrap();
    assert_eq!(
        t.nodes[t.id("s3").unwrap()].proc,
        Dist::Fixed(std::time::Duration::from_millis(25))
    );
}

#[test]
fn first_byte_takes_the_three_duration_forms() {
    for fb in ["20ms", "{ p50: 20ms, p99: 80ms }", "{ mean: 25ms }"] {
        with(&format!("first_byte: {fb}")).unwrap();
    }
    assert!(with("first_byte: { p50: 80ms, p99: 20ms }").is_err());
}

#[test]
fn proc_per_kb_hold_ops_and_pool_are_refused_with_a_hint() {
    for (field, hint) in [
        ("proc: 5ms", "service time is `first_byte`"),
        ("per_kb: 1ms", "`throughput`"),
        ("hold: call", "calls nothing"),
        ("ops: { read: 1ms }", "no `ops`"),
        ("pool: { size: 4 }", "concurrency"),
        ("hit_ratio: 0.5", "concurrency"),
    ] {
        let err = with(&format!("first_byte: 20ms, {field}")).unwrap_err();
        let name = field.split(':').next().unwrap();
        assert!(err.contains(&format!("unknown field '{name}'")), "{err}");
        assert!(err.contains(hint), "{field}: {err}");
        assert!(
            err.contains("first_byte") && err.contains("throughput"),
            "{err}"
        );
    }
}

#[test]
fn the_error_is_placed_at_the_field() {
    let src = "version: 1\nnodes:\n  s3: { type: storage, first_byte: 20ms, proc: 5ms }\n";
    let e = Config::from_yaml(src).unwrap_err();
    assert_eq!((e.line, e.col), (Some(3), Some(42)), "{e}");
    let src = "version: 1\nnodes:\n  s3: { type: storage, first_byte: 20ms, throughput: 0KB/s }\n";
    let e = Config::from_yaml(src).unwrap_err();
    assert_eq!(e.line, Some(3), "{e}");
    assert!(
        e.msg
            .contains("throughput must be at least 1 byte per second"),
        "{e}"
    );
}

#[test]
fn throughput_reads_the_unit_text_like_bandwidth() {
    let bytes = |v: &str| {
        let c = with(&format!("first_byte: 20ms, throughput: {v}")).unwrap();
        let t = Topology::build(&c).unwrap();
        t.nodes[t.id("s3").unwrap()].per_kb.clone()
    };
    // 80MB/s = 83886080 B/s, so one KB takes 1024 / 83886080 s = 12207.03 ns
    let want = Dist::Fixed(std::time::Duration::from_nanos(12_207));
    assert_eq!(bytes("80MB/s"), want);
    assert_eq!(bytes("83886080"), want, "a bare integer is bytes a second");
    // 250KB/s = 256000 B/s: 1024 / 256000 s = 4 ms a KB
    assert_eq!(
        bytes("250KB/s"),
        Dist::Fixed(std::time::Duration::from_millis(4))
    );
    assert_eq!(
        bytes("1B/s"),
        Dist::Fixed(std::time::Duration::from_secs(1024))
    );
    // absent: no size term
    let c = with("first_byte: 20ms").unwrap();
    let t = Topology::build(&c).unwrap();
    assert_eq!(t.nodes[t.id("s3").unwrap()].per_kb, Dist::ZERO);
}

#[test]
fn throughput_below_one_byte_a_second_is_refused_and_bad_text_names_the_field() {
    for v in ["0", "0KB/s", "0.2B/s"] {
        let err = with(&format!("first_byte: 20ms, throughput: {v}")).unwrap_err();
        assert!(
            err.contains("throughput must be at least 1 byte per second"),
            "{v}: {err}"
        );
    }
    let err = with("first_byte: 20ms, throughput: fast").unwrap_err();
    assert!(err.contains("throughput"), "{err}");
    let err = with("first_byte: 20ms, throughput: 5XB/s").unwrap_err();
    assert!(err.contains("unknown unit"), "{err}");
}

#[test]
fn the_shared_capacity_fields_are_accepted() {
    with("first_byte: 20ms, concurrency: { limit: 64, queue: 10 }, timeout: 2s, rate_limit: { rps: 5500, burst: 100 }, replicas: 2, balance: random, bandwidth: 1GB/s, locale: en-US").unwrap();
}

#[test]
fn the_starter_node_is_valid_and_its_numbers_are_the_labelled_estimates() {
    let text = format!(
        "version: 1\nnodes:\n  app: {{ type: service }}\n  files: {}\nedges:\n  - {{ from: app, to: files }}\n",
        presets::starter(NodeKind::Storage)
    );
    let c = Config::from_yaml(&text).unwrap();
    Topology::build(&c).unwrap();
    let v: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(presets::starter(NodeKind::Storage)).unwrap();
    assert_eq!(v["type"], "storage");
    assert_eq!(v["flavor"], "s3");
    assert_eq!(v["first_byte"], "25ms", "a plain value: Studio can edit it");
    assert_eq!(v["throughput"], "80MB/s");
    assert!(
        v.get("concurrency").is_none(),
        "no slots: rate_limit holds request rates"
    );
}

#[test]
fn who_may_call_a_storage_node_and_that_it_calls_nothing() {
    let storage = "  s3: { type: storage, first_byte: 20ms }\n";
    // service, monolith and auth may call it
    let direct = |from_decl: &str, from: &str| {
        let nodes = format!("  front: {{ type: service }}\n{from_decl}{storage}");
        let edges = format!("  - {{ from: {from}, to: s3 }}\n");
        let c = load(&nodes, &edges).unwrap();
        Topology::build(&c).map(|_| ()).map_err(|e| e.to_string())
    };
    assert_eq!(direct("", "front"), Ok(()));
    assert_eq!(direct("  m: { type: monolith }\n", "m"), Ok(()));
    assert_eq!(direct("  login: { type: auth }\n", "login"), Ok(()));
    assert_eq!(
        direct("  lb: { type: load-balancer }\n", "lb"),
        Ok(()),
        "a balancer may front it, as it may a database"
    );
    // gateway, cache, database, external-api and another storage node may not
    for (decl, from, kind) in [
        ("  gw: { type: gateway }\n", "gw", "gateway"),
        ("  c: { type: cache, flavor: redis }\n", "c", "cache"),
        (
            "  d: { type: database, flavor: postgres }\n",
            "d",
            "database",
        ),
        ("  x: { type: external-api }\n", "x", "external-api"),
        (
            "  s2: { type: storage, first_byte: 1ms }\n",
            "s2",
            "storage",
        ),
    ] {
        let err = direct(decl, from).unwrap_err();
        assert!(
            err.contains(&format!("{kind} '{from}' cannot call storage '")),
            "{kind}: {err}"
        );
    }
    // and it calls nothing
    for (decl, to, kind) in [
        ("  a: { type: service }\n", "a", "service"),
        (
            "  d: { type: database, flavor: postgres }\n",
            "d",
            "database",
        ),
        (
            "  s2: { type: storage, first_byte: 1ms }\n",
            "s2",
            "storage",
        ),
    ] {
        let nodes = format!("  front: {{ type: service }}\n{decl}{storage}");
        let edges = format!("  - {{ from: front, to: s3 }}\n  - {{ from: s3, to: {to} }}\n");
        let err = Topology::build(&load(&nodes, &edges).unwrap())
            .unwrap_err()
            .to_string();
        assert!(
            err.contains(&format!("storage 's3' cannot call {kind} '{to}'")),
            "{kind}: {err}"
        );
    }
}

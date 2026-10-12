use kaanal_core::diagnostics::{Located, PathSeg, keys};
use kaanal_core::{Config, ConfigError, Topology};
use std::path::{Path, PathBuf};

/// The error of a config that must fail to parse.
fn fail(src: &str) -> ConfigError {
    Config::from_yaml(src).unwrap_err()
}

/// A graph error of a config that parses, placed in its source.
fn located(src: &str) -> ConfigError {
    let cfg = Config::from_yaml(src).unwrap();
    Topology::build(&cfg)
        .unwrap_err()
        .located(&cfg)
        .in_source(src)
}

/// Lines 1–6; each case adds line 7 (or edits a line).
const HEAD: &str = "version: 1\nnodes:\n  app: { type: service }\n  orders-db:\n    type: database\n    flavor: postgres\n";

#[test]
fn unknown_field_in_the_second_node_is_reported_at_its_key() {
    let e = fail(&format!("{HEAD}    bogus: 1\n"));
    assert_eq!(
        e.to_string(),
        "7:5: node 'orders-db': unknown field 'bogus' (expected: concurrency, flavor, ops, pool, rate_limit)"
    );
}

#[test]
fn spike_cases_point_at_the_problem() {
    let cases: [(String, (usize, usize), &str); 6] = [
        (
            format!("{HEAD}    pool: {{ queue: 1 }}\n"),
            (7, 11),
            "missing field 'pool.size'",
        ),
        (
            format!("{HEAD}    concurrency: lots\n"),
            (7, 18),
            "field 'concurrency'",
        ),
        (
            format!("{HEAD}    ops: {{ read: fast }}\n"),
            (7, 10),
            "expected a duration like",
        ),
        (
            format!("{HEAD}    ops: {{ reed: 1ms }}\n"),
            (7, 12),
            "unknown field 'ops.reed' (expected: read, scan, write)",
        ),
        (
            HEAD.replace("type: database", "type: router"),
            (5, 11),
            "unknown type 'router'",
        ),
        (
            HEAD.replace("    type: database\n", ""),
            (4, 3),
            "missing field 'type'",
        ),
    ];
    for (src, (line, col), want) in cases {
        let e = fail(&src);
        assert_eq!((e.line, e.col), (Some(line), Some(col)), "{src}\n{e}");
        assert!(
            e.msg.starts_with("node 'orders-db': ") && e.msg.contains(want),
            "{src}\n{e}"
        );
    }
}

#[test]
fn comments_crlf_and_bom_do_not_move_positions() {
    let lf = format!("{HEAD}    bogus: 1\n");
    let base = fail(&lf);
    let commented = fail(&lf.replace("nodes:\n", "nodes:\n  # the shop\n"));
    assert_eq!(
        (commented.line, commented.col),
        (Some(8), Some(5)),
        "{commented}"
    );
    let windows = fail(&format!("\u{feff}{}", lf.replace('\n', "\r\n")));
    assert_eq!(windows, base, "CRLF + BOM must give the same error as LF");
}

#[test]
fn errors_outside_nodes_keep_serdes_position() {
    // serde stops at `port`; the bad node below must not steal the report.
    let e =
        fail("version: 1\nseed: 1\nport: notanumber\nnodes:\n  app: { type: service, bogus: 1 }\n");
    assert_eq!(e.line, Some(3), "{e}");
}

#[test]
fn anchors_and_merge_keys_fall_back_cleanly() {
    // serde has no merge keys: both parsers see `<<` as an unknown field of `b`.
    let e = fail(
        "version: 1\nnodes:\n  a: &base { type: service }\n  b: { <<: *base, type: service }\n",
    );
    assert_eq!((e.line, e.col), (Some(4), Some(8)), "{e}");
    assert!(e.msg.contains("<<"), "{e}");
    // An undefined alias: an error either way, and no panic.
    let _ = fail("version: 1\nnodes:\n  a: *nope\n");
}

#[test]
fn nodes_named_like_yaml_keywords_are_found() {
    for name in ["true", "null", "yes", "on", "1"] {
        let e = fail(&format!(
            "version: 1\nnodes:\n  a: {{ type: service }}\n  {name}: {{ type: service, bogus: 1 }}\n"
        ));
        assert_eq!(e.line, Some(4), "{name}: {e}");
        assert!(
            e.msg
                .starts_with(&format!("node '{name}': unknown field 'bogus'")),
            "{name}: {e}"
        );
    }
}

#[test]
fn graph_errors_point_at_the_edge_or_node() {
    let nodes = "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service }\n  app2: { type: service }\n  db: { type: database, flavor: postgres }\nedges:\n";
    // Forbidden edge: at the edge entry (line 9).
    let e = located(&format!(
        "{nodes}  - {{ from: gw, to: app }}\n  - {{ from: gw, to: db }}\n"
    ));
    assert_eq!((e.line, e.col), (Some(9), Some(5)), "{e}");
    assert!(e.msg.contains("cannot call"), "{e}");
    // Cycle: at the edge that closes it.
    let e = located(&format!(
        "{nodes}  - {{ from: app, to: app2 }}\n  - {{ from: app2, to: app }}\n"
    ));
    assert_eq!(e.line, Some(9), "{e}");
    assert!(e.msg.starts_with("cycle: "), "{e}");
    // Unknown node: at the name.
    let e = located(&format!("{nodes}  - {{ from: app, to: redis }}\n"));
    assert_eq!((e.line, e.col), (Some(8), Some(22)), "{e}");
    // Second gateway: at its key. Empty topology: at `nodes`.
    let e = located("version: 1\nnodes:\n  a: { type: gateway }\n  b: { type: gateway }\n");
    assert_eq!((e.line, e.col), (Some(4), Some(3)), "{e}");
    let e = located("version: 1\nnodes: {}\n");
    assert_eq!((e.line, e.col), (Some(2), Some(1)), "{e}");
}

#[test]
fn located_without_a_place_has_no_position() {
    let e = Located::new(Vec::new(), "port in use").in_source("version: 1\n");
    assert_eq!((e.line, e.col, e.msg.as_str()), (None, None, "port in use"));
    let e = Located::new(keys(&["nodes", "missing"]), "x").in_source("version: 1\nnodes: {}\n");
    assert_eq!(e.line, None);
    let e = Located::new(vec![PathSeg::Key("version".into())], "v").in_source("version: 1\n");
    assert_eq!((e.line, e.col), (Some(1), Some(1)));
    // The version check is positioned too.
    assert_eq!(fail("version: 2\nnodes: {}\n").line, Some(1));
}

/// A valid fixture for the mutation sweep.
const VALID: &str = "version: 1\nnodes:\n  gw: { type: gateway, flavor: nginx }\n  app: { type: service }\n  db:\n    type: database\n    flavor: postgres\nedges:\n  - { from: gw, to: app }\n  - { from: app, to: db }\n";

#[test]
fn explain_never_panics_or_flips_the_verdict() {
    assert!(Config::from_yaml(VALID).is_ok());
    let mut variants = Vec::new();
    let lines: Vec<&str> = VALID.split_inclusive('\n').collect();
    for i in 0..lines.len() {
        // every one-line deletion
        variants.push(
            lines
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .map(|(_, l)| *l)
                .collect::<String>(),
        );
        // truncation at each line break
        variants.push(lines[..i].concat());
        // duplicate a line, indent one line by 3, swap `:` for `;`
        let mut dup = lines.clone();
        dup.insert(i, lines[i]);
        variants.push(dup.concat());
        let mut ind = lines.clone();
        let indented = format!("   {}", lines[i]);
        ind[i] = &indented;
        variants.push(ind.concat());
        variants.push(
            lines
                .iter()
                .enumerate()
                .map(|(j, l)| {
                    if j == i {
                        l.replacen(':', ";", 1)
                    } else {
                        (*l).to_string()
                    }
                })
                .collect(),
        );
    }
    // odd shapes
    for odd in [
        "",
        "- a\n- b\n",
        "nodes: 5\n",
        "nodes:\n  a: [1, 2]\n",
        "nodes:\n  a: *nope\n",
        "nodes:\n  a: 1\n  a: 2\n",
        "\u{feff}",
        "version: 1\nnodes:\n  a: {type: service}\n---\nx: 1\n",
    ] {
        variants.push(odd.to_string());
    }
    for v in variants {
        let plain = serde_yaml_ng::from_str::<Config>(v.strip_prefix('\u{feff}').unwrap_or(&v));
        let got = Config::from_yaml(&v);
        if plain.is_ok() && !matches!(&got, Err(e) if e.msg.contains("unsupported config version"))
        {
            assert!(
                got.is_ok(),
                "accepted by serde, rejected by from_yaml:\n{v}"
            );
        }
        if plain.is_err() {
            let e = got.expect_err(&format!("serde rejects, from_yaml must too:\n{v}"));
            if let Some(line) = e.line {
                assert!(
                    (1..=v.lines().count() + 1).contains(&line),
                    "line {line} out of range:\n{v}\n{e}"
                );
            }
            if let Some(col) = e.col {
                assert!(col >= 1, "col {col}:\n{v}\n{e}");
            }
        }
    }
}

#[test]
fn huge_source_falls_back_to_serdes_position() {
    let pad = format!("# {}\n", "x".repeat(1 << 20));
    let e = fail(&format!("{HEAD}    bogus: 1\n{pad}"));
    assert!(e.line.is_some() && e.msg.contains("bogus"), "{e}");
}

#[test]
fn deep_block_nesting_does_not_overflow_the_stack() {
    // 100k nested sequences on one line: serde refuses at its own depth limit.
    let deep = format!("version: 1\nnodes: {}x\n", "- ".repeat(500_000));
    let _ = fail(&deep);
    let deep = format!("version: 1\nnodes:\n  a: {}x\n", "- ".repeat(500_000));
    let _ = fail(&deep);
}

#[test]
fn scalars_serde_reads_as_strings_do_not_hide_the_real_error() {
    // `password: 12345` is a valid string to serde; the bad node below is the real error.
    let e = fail(
        "version: 1\nnodes:\n  a: { type: auth, users: [{ username: bob, password: 12345 }] }\n  b: { type: service, bogus: 1 }\n",
    );
    assert_eq!(e.line, Some(4), "{e}");
    assert!(e.msg.starts_with("node 'b': unknown field 'bogus'"), "{e}");
}

#[test]
fn nesting_under_the_other_guards_falls_back_instead_of_overflowing() {
    // 2000 levels on one 4 KB line: under the line guard, past serde's depth limit.
    let _ = fail(&format!(
        "version: 1\nnodes:\n  a:\n    {}x\n",
        "- ".repeat(2000)
    ));
    // 600 levels by indentation, every line short.
    let mut deep = String::from("version: 1\nnodes:\n");
    for i in 0..600 {
        deep.push_str(&" ".repeat(i + 2));
        deep.push_str("a:\n");
    }
    let _ = fail(&deep);
}

#[test]
fn duplicate_field_in_a_later_node_names_that_node() {
    let e = fail(
        "version: 1\nnodes:\n  a: { type: service }\n  b:\n    type: service\n    proc: 1ms\n    proc: 2ms\n",
    );
    assert_eq!((e.line, e.col), (Some(7), Some(5)), "{e}");
    assert!(e.msg.starts_with("node 'b': duplicate field 'proc'"), "{e}");
}

#[test]
fn duplicate_node_names_are_rejected_at_the_second() {
    let e = fail("version: 1\nnodes:\n  app: { type: service }\n  app: { type: service }\n");
    assert_eq!((e.line, e.col), (Some(4), Some(3)), "{e}");
    assert!(e.msg.contains("duplicate node 'app'"), "{e}");
}

#[test]
fn bad_edge_values_are_placed_at_the_field() {
    let nodes = "version: 1\nnodes:\n  a: { type: service }\n  b: { type: service }\nedges:\n";
    let e = fail(&format!(
        "{nodes}  - {{ from: a, to: b, net: {{ p50: 9ms, p99: 1ms }} }}\n"
    ));
    assert_eq!((e.line, e.col), (Some(6), Some(28)), "{e}");
    assert!(
        e.msg.starts_with("edge #0: field 'net': ") && e.msg.contains("above p99"),
        "{e}"
    );
    let e = fail(&format!(
        "{nodes}  - {{ from: a, to: b }}\n  - {{ from: b, to: a, calls: 0 }}\n"
    ));
    assert_eq!(e.line, Some(7), "{e}");
    assert!(e.msg.starts_with("edge #1: field 'calls'"), "{e}");
}

#[test]
fn located_points_at_values_and_keeps_known_positions() {
    let src = "version: 1\nendpoints: missing.yaml\n";
    let e = Located::at_value(keys(&["endpoints"]), "missing.yaml: not found").in_source(src);
    assert_eq!((e.line, e.col), (Some(2), Some(12)), "{e}");
    let e = Located::new(keys(&["endpoints"]), "x").in_source(src);
    assert_eq!((e.line, e.col), (Some(2), Some(1)), "{e}");
    let known = ConfigError {
        line: Some(3),
        col: Some(7),
        msg: "bad".into(),
    };
    let parsed = Located::parsed(PathBuf::from("eps.yaml"), known.clone());
    assert_eq!(parsed.file.as_deref(), Some(Path::new("eps.yaml")));
    assert_eq!(parsed.in_source("anything at all"), known);
    let f = Located::new(keys(&["GET /x"]), "y").in_file(PathBuf::from("eps.yaml"));
    assert_eq!(f.file.as_deref(), Some(Path::new("eps.yaml")));
    assert_eq!(f.in_source("\"GET /x\": {}\n").line, Some(1));
}

#[test]
fn node_chain_errors_are_placed_at_the_chains_field() {
    // `chains` sits inside a tagged node, so serde loses the place; the A1 serde probe finds it.
    let e = fail(
        "version: 1\nnodes:\n  app: { type: service, chains: { r: [ { cache: c, op: read } ] } }\n",
    );
    assert_eq!((e.line, e.col), (Some(3), Some(33)), "{e}");
    assert!(
        e.msg.starts_with("node 'app': field 'chains': ")
            && e.msg
                .contains("a cache step takes no other fields (got 'op')"),
        "{e}"
    );
}

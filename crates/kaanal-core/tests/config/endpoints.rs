//! The `endpoints:` map: every step shape and its positioned errors.

use kaanal_core::config::{
    Branch, CallStep, Determinism, EndpointsConfig, ErrorMapping, Override, SeedSource, StepConfig,
    parse_endpoints,
};
use kaanal_core::{Config, Op};
use serde_json::json;

#[test]
fn routes_key_is_replaced_by_endpoints() {
    let err = Config::from_yaml("version: 1\nnodes: {}\nroutes:\n  \"GET /health\": { body: 1 }\n")
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "3:1: routes: was replaced by endpoints: (see docs)"
    );
}

pub(crate) const ENDPOINTS: &str = r#"
version: 1
determinism: realistic
nodes:
  app: { type: service, chains: { reserve: [ db, { call: db, op: write } ] } }
  db: { type: database, flavor: postgres }
endpoints:
  "GET /orders/{id}":
    op: read
    chain:
      - cache: c
      - { call: svc, chain: [ { call: db, op: read, repeat: 2 } ] }
      - { call: svc, with: reserve }
      - { call: svc, via: "POST /orders" }
      - parallel: [ audit, [ pay, { call: db } ] ]
    override: { status: 201, body: { ok: true } }
    error_map: { db: 503, "5xx": 502, pay: { status: 502, body: { error: upstream } } }
"#;

#[test]
fn endpoints_parse_inline_with_every_step_shape() {
    let cfg = Config::from_yaml(ENDPOINTS).unwrap();
    assert_eq!(cfg.determinism, Determinism::Realistic);
    assert_eq!(cfg.seed_source(), SeedSource::Entropy);
    let Some(EndpointsConfig::Inline(eps)) = &cfg.endpoints else {
        panic!("inline: {:?}", cfg.endpoints)
    };
    let e = &eps["GET /orders/{id}"];
    assert_eq!(e.op, Some(Op::Read));
    let call = |node: &str| CallStep {
        node: node.into(),
        ..CallStep::default()
    };
    let bare = |node: &str| {
        StepConfig::Call(CallStep {
            bare: true,
            ..call(node)
        })
    };
    let want = vec![
        StepConfig::Cache { node: "c".into() },
        StepConfig::Call(CallStep {
            chain: Some(vec![StepConfig::Call(CallStep {
                op: Some(Op::Read),
                repeat: Some(2),
                ..call("db")
            })]),
            ..call("svc")
        }),
        StepConfig::Call(CallStep {
            with: Some("reserve".into()),
            ..call("svc")
        }),
        StepConfig::Call(CallStep {
            via: Some("POST /orders".into()),
            ..call("svc")
        }),
        StepConfig::Parallel(vec![
            Branch {
                steps: vec![bare("audit")],
                list: false,
            },
            Branch {
                steps: vec![bare("pay"), StepConfig::Call(call("db"))],
                list: true,
            },
        ]),
    ];
    assert_eq!(e.chain.as_ref(), Some(&want));
    assert_eq!(
        e.response,
        Some(Override {
            status: 201,
            body: json!({ "ok": true }),
            headers: std::collections::BTreeMap::default(),
            content_type: None,
            encoding: None,
        })
    );
    assert_eq!(
        e.error_map["db"],
        ErrorMapping {
            status: 503,
            body: None
        }
    );
    assert_eq!(
        e.error_map["5xx"],
        ErrorMapping {
            status: 502,
            body: None
        }
    );
    assert_eq!(
        e.error_map["pay"],
        ErrorMapping {
            status: 502,
            body: Some(json!({ "error": "upstream" }))
        }
    );
    assert_eq!(
        cfg.nodes["app"].chains().unwrap()["reserve"],
        vec![
            bare("db"),
            StepConfig::Call(CallStep {
                op: Some(Op::Write),
                ..call("db")
            })
        ]
    );
    assert!(cfg.nodes["db"].chains().is_none());
}

#[test]
fn step_shape_errors_are_placed_at_the_step() {
    // Lines 1–5; each case is line 6. Positions come from serde's own marks (D1).
    let head = "version: 1\nnodes: {}\nendpoints:\n  \"GET /x\":\n    chain:\n";
    for (step, (line, col), want) in [
        (
            "      - { cache: c, repeat: 2 }\n",
            (6, 9),
            "a cache step takes no other fields (got 'repeat')",
        ),
        (
            "      - { parallel: [a, b], op: read }\n",
            (6, 9),
            "a parallel step takes no other fields (got 'op')",
        ),
        (
            "      - { op: read }\n",
            (6, 9),
            "a step needs one of call, cache, parallel",
        ),
        (
            "      - { call: a, bogus: 1 }\n",
            (6, 9),
            "unknown step field 'bogus'",
        ),
        (
            "      - { call: a, call: b }\n",
            (6, 9),
            "duplicate step field 'call'",
        ),
        ("      - { call: a, repeat: -1 }\n", (6, 28), "-1"),
        ("      - [a, b]\n", (6, 9), "expected a step"),
    ] {
        let err = Config::from_yaml(&format!("{head}{step}")).unwrap_err();
        assert_eq!((err.line, err.col), (Some(line), Some(col)), "{step}{err}");
        assert!(err.msg.contains(want), "{step}want '{want}' in: {err}");
    }
}

#[test]
fn endpoint_entries_are_strict() {
    let head = "version: 1\nnodes: {}\nendpoints:\n";
    let err = Config::from_yaml(&format!("{head}  \"GET /x\": {{ bogus: 1 }}\n")).unwrap_err();
    assert!(err.msg.contains("unknown field `bogus`"), "{err}");
    assert_eq!(err.line, Some(4), "{err}");
    let err =
        Config::from_yaml(&format!("{head}  \"GET /x\": {{}}\n  \"GET /x\": {{}}\n")).unwrap_err();
    assert!(err.msg.contains("duplicate endpoint 'GET /x'"), "{err}");
    assert_eq!(
        err.line,
        Some(4),
        "D19: placed at the start of the map: {err}"
    );
    let err = Config::from_yaml("version: 1\nnodes: {}\nendpoints: 5\n").unwrap_err();
    assert!(
        err.msg
            .contains("a path to an endpoints file or a map of endpoints"),
        "{err}"
    );
    assert_eq!((err.line, err.col), (Some(3), Some(12)), "{err}");
    let err = Config::from_yaml(&format!(
        "{head}  \"GET /x\": {{ error_map: {{ db: {{ status: 503, extra: 1 }} }} }}\n"
    ))
    .unwrap_err();
    assert!(err.msg.contains("unknown field `extra`"), "{err}");
    let err = Config::from_yaml("version: 1\nnodes: {}\ndeterminism: random\n").unwrap_err();
    assert!(
        err.msg.contains("controlled") && err.msg.contains("realistic"),
        "{err}"
    );
}

#[test]
fn chains_belong_to_services_and_monoliths() {
    Config::from_yaml("version: 1\nnodes:\n  m: { type: monolith, chains: { cron: [ x ] } }\n")
        .unwrap();
    let err =
        Config::from_yaml("version: 1\nnodes:\n  d: { type: database, chains: {} }\n").unwrap_err();
    assert!(err.msg.contains("unknown field 'chains'"), "{err}");
}

#[test]
fn endpoints_file_parses_and_places_shape_errors() {
    let m = parse_endpoints("# entries\n\"GET /x\": { op: read }\n").unwrap();
    assert_eq!(m.keys().collect::<Vec<_>>(), ["GET /x"]);
    for empty in ["", "# only a comment\n", "\u{feff}"] {
        assert!(parse_endpoints(empty).unwrap().is_empty(), "D19: {empty:?}");
    }
    let bad = "\"GET /x\":\n  chain:\n    - { cache: c, op: read }\n";
    let e = parse_endpoints(bad).unwrap_err();
    assert_eq!((e.line, e.col), (Some(3), Some(7)), "{e}");
    assert!(
        e.msg
            .contains("a cache step takes no other fields (got 'op')"),
        "{e}"
    );
    let windows = parse_endpoints(&format!("\u{feff}{}", bad.replace('\n', "\r\n"))).unwrap_err();
    assert_eq!(
        (windows.line, windows.col),
        (e.line, e.col),
        "R6.4: {windows}"
    );
    let top = parse_endpoints("- a\n").unwrap_err();
    assert!(top.msg.contains("map of endpoints"), "{top}");
}

#[test]
fn serde_errors_do_not_repeat_their_position_or_offer_routes() {
    // The position is in the error's own line/col; the message must not say it again.
    let e = Config::from_yaml("version: 1\nfoo: 2\n").unwrap_err();
    assert!(!e.msg.contains("at line"), "{}", e.msg);
    assert!(
        e.msg.contains("`endpoints`") && !e.msg.contains("routes"),
        "routes: is removed, so it is not a valid key to suggest: {}",
        e.msg
    );
    let e = parse_endpoints("\"GET /a\":\n  chain:\n    - { cache: c, op: read }\n").unwrap_err();
    assert_eq!((e.line, e.col), (Some(3), Some(7)), "{e}");
    assert!(!e.msg.contains("at line"), "{}", e.msg);
}

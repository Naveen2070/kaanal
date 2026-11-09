//! `override:` headers and the one check shared by a file and a live edit (step 8A).

use kaanal_core::Config;
use kaanal_core::config::{EndpointsConfig, Override};
use serde_json::{Value, json};

fn ov(status: u16, body: Value, h: &[(&str, &str)]) -> Override {
    Override {
        status,
        body,
        headers: h
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect(),
    }
}

fn yaml(over: &str) -> String {
    format!(
        "version: 1\nnodes:\n  g: {{ type: gateway }}\nendpoints:\n  \"GET /x\": {{ override: {over} }}\n"
    )
}

#[test]
fn headers_default_to_empty_and_load_when_given() {
    let c = Config::from_yaml(&yaml("{ status: 201, body: {} }")).unwrap();
    let Some(EndpointsConfig::Inline(m)) = &c.endpoints else {
        panic!("inline endpoints")
    };
    assert!(m["GET /x"].response.as_ref().unwrap().headers.is_empty());

    let c = Config::from_yaml(&yaml("{ body: {}, headers: { x-demo: \"1\" } }")).unwrap();
    let Some(EndpointsConfig::Inline(m)) = &c.endpoints else {
        panic!("inline endpoints")
    };
    assert_eq!(
        m["GET /x"].response.as_ref().unwrap().headers["x-demo"],
        "1"
    );
}

#[test]
fn check_accepts_a_plain_override() {
    assert!(
        ov(
            503,
            json!({"e": 1}),
            &[("x-demo", "1"), ("Retry-After", "5")]
        )
        .check()
        .is_ok()
    );
    assert!(ov(204, Value::Null, &[]).check().is_ok());
}

#[test]
fn check_rejects_by_field() {
    let long = "a".repeat(Override::MAX_BODY);
    let long_value = "v".repeat(Override::MAX_HEADER_VALUE + 1);
    let many: Vec<(String, String)> = (0..33).map(|i| (format!("x-h{i}"), "v".into())).collect();
    let many: Vec<(&str, &str)> = many.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let cases: Vec<(Override, &str)> = vec![
        (ov(99, json!({}), &[]), "status"),
        (ov(199, json!({}), &[]), "status"),
        (ov(600, json!({}), &[]), "status"),
        (ov(204, json!({"a": 1}), &[]), "status"),
        (ov(304, json!({"a": 1}), &[]), "status"),
        (ov(200, json!(long), &[]), "body"),
        (ov(200, json!({}), &many), "headers"),
        (ov(200, json!({}), &[("content-length", "1")]), "headers"),
        (
            ov(200, json!({}), &[("Content-Type", "text/html")]),
            "headers",
        ),
        (
            ov(200, json!({}), &[("Transfer-Encoding", "chunked")]),
            "headers",
        ),
        (ov(200, json!({}), &[("connection", "close")]), "headers"),
        (
            ov(200, json!({}), &[("Content-Encoding", "gzip")]),
            "headers",
        ),
        (
            ov(200, json!({}), &[("x-a", long_value.as_str())]),
            "headers",
        ),
        (
            ov(200, json!({}), &[("Access-Control-Allow-Origin", "*")]),
            "headers",
        ),
        (ov(200, json!({}), &[("x-request-id", "a")]), "headers"),
        (ov(200, json!({}), &[("X-Kaanal-Trace", "a")]), "headers"),
        (ov(200, json!({}), &[("bad name", "a")]), "headers"),
        (ov(200, json!({}), &[("", "a")]), "headers"),
        (ov(200, json!({}), &[("x-a", "a\r\nx-b: 1")]), "headers"),
    ];
    for (o, field) in cases {
        let e = o.check().unwrap_err();
        assert!(e.starts_with(field), "{o:?} -> {e}");
    }
}

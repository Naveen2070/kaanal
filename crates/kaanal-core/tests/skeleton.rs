//! `skeleton_openapi`: a generated `OpenAPI` file for endpoints with no spec.

use indexmap::IndexMap;
use kaanal_core::{EndpointConfig, skeleton_openapi};
use serde_json::Value;
use std::fmt::Write as _;

fn entries(keys: &[&str]) -> IndexMap<String, EndpointConfig> {
    keys.iter()
        .map(|k| ((*k).to_string(), EndpointConfig::default()))
        .collect()
}

fn doc(keys: &[&str]) -> Value {
    serde_yaml_ng::from_str(&skeleton_openapi(&entries(keys), "t").unwrap()).unwrap()
}

#[test]
fn every_endpoint_becomes_an_operation_with_a_response() {
    let d = doc(&["GET /users", "post /users", "GET /users/{id}"]);
    assert_eq!(d["openapi"], "3.0.3");
    assert_eq!(d["x-kaanal-skeleton"], true);
    assert!(d["paths"]["/users"]["get"]["responses"]["200"].is_object());
    assert!(
        d["paths"]["/users"]["post"].is_object(),
        "the method is normalized"
    );
    assert_eq!(
        d["paths"]["/users"]["get"]["responses"]["200"]["content"]["application/json"]["schema"]["type"],
        "object"
    );
}

#[test]
fn path_parameters_are_declared_required() {
    let d = doc(&["GET /users/{id}/orders/{*rest}"]);
    // the path is written as it is: `{*rest}` stays a catch-all for the router
    let op = &d["paths"]["/users/{id}/orders/{*rest}"]["get"];
    let ps = op["parameters"].as_array().unwrap();
    assert_eq!(ps.len(), 2);
    for p in ps {
        assert_eq!(p["in"], "path");
        assert_eq!(p["required"], true);
    }
    assert_eq!(ps[1]["name"], "rest");
}

#[test]
fn operation_ids_are_unique() {
    let d = doc(&["GET /a/{id}", "GET /a", "GET /a/{x}"]);
    let mut ids: Vec<String> = d["paths"]
        .as_object()
        .unwrap()
        .values()
        .flat_map(|item| item.as_object().unwrap().values())
        .map(|op| op["operationId"].as_str().unwrap().to_string())
        .collect();
    ids.sort();
    let n = ids.len();
    ids.dedup();
    assert_eq!(ids.len(), n, "{ids:?}");
}

#[test]
fn an_override_status_is_the_success_response_and_204_has_no_body() {
    let e: IndexMap<String, EndpointConfig> = serde_yaml_ng::from_str(
        "'POST /orders': { override: { status: 201, body: {} } }\n'DELETE /orders/{id}': { override: { status: 204 } }\n",
    )
    .unwrap();
    let d: Value = serde_yaml_ng::from_str(&skeleton_openapi(&e, "t").unwrap()).unwrap();
    assert!(d["paths"]["/orders"]["post"]["responses"]["201"]["content"].is_object());
    let del = &d["paths"]["/orders/{id}"]["delete"]["responses"]["204"];
    assert!(del.is_object() && del.get("content").is_none());
}

#[test]
fn nothing_to_write_and_bad_keys_are_clear_errors() {
    assert!(
        skeleton_openapi(&IndexMap::new(), "t")
            .unwrap_err()
            .contains("no endpoints")
    );
    let bad = skeleton_openapi(&entries(&["nonsense"]), "t").unwrap_err();
    assert!(bad.contains("invalid endpoint"), "{bad}");
    let trace = skeleton_openapi(&entries(&["TRACE /x"]), "t").unwrap_err();
    assert!(trace.contains("no operation for TRACE"), "{trace}");
    let twice = skeleton_openapi(&entries(&["GET /x", "get /x"]), "t").unwrap_err();
    assert!(twice.contains("appears twice"), "{twice}");
}

#[test]
fn repeated_parameters_are_declared_once_and_bad_ones_are_refused() {
    let d = doc(&["GET /a/{id}/b/{id}"]);
    let ps = d["paths"]["/a/{id}/b/{id}"]["get"]["parameters"]
        .as_array()
        .unwrap();
    assert_eq!(ps.len(), 1);
    for bad in ["GET /a/{id", "GET /a/id}", "GET /a/{}", "GET /a/{*}"] {
        let e = skeleton_openapi(&entries(&[bad]), "t").unwrap_err();
        assert!(e.contains(bad) && e.contains("endpoint"), "{bad}: {e}");
    }
}

#[test]
fn colliding_operation_ids_stay_unique_and_cheap() {
    // all of these spell the same words once the non-alphanumerics become separators
    let keys: Vec<String> = (0..3000).map(|i| format!("GET /a?{i}")).collect();
    let refs: Vec<&str> = keys.iter().map(String::as_str).collect();
    let d = doc(&refs);
    let mut ids: Vec<String> = d["paths"]
        .as_object()
        .unwrap()
        .values()
        .map(|item| item["get"]["operationId"].as_str().unwrap().to_string())
        .collect();
    let n = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), n);
    assert_eq!(n, 3000);
}

#[test]
fn too_many_endpoints_are_refused() {
    let keys: Vec<String> = (0..=kaanal_core::spec_skeleton::MAX_ENDPOINTS)
        .map(|i| format!("GET /e{i}"))
        .collect();
    let refs: Vec<&str> = keys.iter().map(String::as_str).collect();
    let e = skeleton_openapi(&entries(&refs), "t").unwrap_err();
    assert!(e.contains("at most"), "{e}");
}

#[test]
fn keys_must_be_short_plain_ascii_and_parameters_are_bounded() {
    let e = skeleton_openapi(&entries(&["GET /caf\u{e9}"]), "t").unwrap_err();
    assert!(e.contains("plain ASCII"), "{e}");
    let bidi = skeleton_openapi(&entries(&["GET /a\u{202e}b"]), "t").unwrap_err();
    assert!(bidi.contains("plain ASCII"), "{bidi}");
    let long = format!("GET /{}", "a".repeat(3000));
    assert!(skeleton_openapi(&entries(&[&long]), "t").is_err());
    // many distinct parameters in one path: bounded work, a clear error
    let mut many = String::new();
    for i in 0..5000 {
        write!(many, "{{p{i}}}").unwrap();
    }
    let e = skeleton_openapi(&entries(&[&format!("GET /{many}")]), "t").unwrap_err();
    assert!(e.contains("2048") || e.contains("parameters"), "{e}");
}

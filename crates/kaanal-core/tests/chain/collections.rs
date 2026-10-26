//! Collection routes: operation shapes, names and the compiled entry.

use crate::common::*;
use kaanal_core::{CollectionOp, Config, collection_op};

#[test]
fn collection_op_maps_the_four_shapes_and_rejects_the_rest() {
    use CollectionOp::*;
    for (m, p, want) in [
        ("GET", "/x", Some(List)),
        ("GET", "/x/{p}", Some(Get)),
        ("POST", "/x", Some(Add)),
        ("DELETE", "/x/{p}", Some(Remove)),
        ("PUT", "/x/{p}", None),
        ("PATCH", "/x/{p}", None),
        ("POST", "/x/{id}", None),
        ("GET", "/x/{a}/{b}", None),
        ("GET", "/x/y", None),
        ("GET", "/x/{}", None),
        ("DELETE", "/x/", None),
        ("GET", "/x/{a}-{b}", None),
        ("DELETE", "/x/{a}{b}", None),
        ("GET", "/x/{a}b", None),
        ("GET", "/x/a{b}", None),
        ("DELETE", "/x", None),
        ("GET", "/", None),
        ("GET", "", None),
        ("get", "/x", None),
    ] {
        assert_eq!(collection_op(m, p), want, "{m} {p}");
    }
}

#[test]
fn collection_name_must_match_a_slug() {
    for name in ["a", "todos", "my-list_2", &"a".repeat(32)] {
        ok(&format!("  \"GET /x\": {{ collection: {name} }}\n"));
    }
    for name in ["\"\"", "Todos", "1a", "-a", "a b", "a.b", &"a".repeat(33)] {
        let e = err(&format!("{BASE}  \"GET /x\": {{ collection: {name} }}\n"));
        assert_eq!((e.line, e.col), (Some(23), Some(3)), "{name}");
        assert!(
            e.msg.starts_with("endpoint 'GET /x': collection: name"),
            "{name}: {}",
            e.msg
        );
    }
}

#[test]
fn collection_with_a_bad_shape_is_a_located_error_at_the_entry() {
    let e = err(&format!("{BASE}  \"PUT /x/{{id}}\": {{ collection: x }}\n"));
    assert_eq!(
        (e.line, e.col, e.msg.as_str()),
        (
            Some(23),
            Some(3),
            "endpoint 'PUT /x/{id}': collection: needs GET /x, GET /x/{id}, POST /x or DELETE /x/{id}"
        )
    );
}

#[test]
fn collection_with_override_is_an_error() {
    let e = err(&format!(
        "{BASE}  \"GET /x\": {{ collection: x, override: {{ status: 200 }} }}\n"
    ));
    assert_eq!(
        (e.line, e.col, e.msg.as_str()),
        (
            Some(23),
            Some(3),
            "endpoint 'GET /x': collection: cannot be used with override (override replaces the responder)"
        )
    );
}

#[test]
fn a_collection_entry_compiles_and_exposes_its_name() {
    let src = format!(
        "{BASE}  \"GET /notes\": {{ collection: notes }}\n  \"POST /notes\": {{ collection: notes }}\n"
    );
    let routes = [("GET /notes", "app"), ("POST /notes", "app"), ROUTES[0]];
    let eps = build(&src, &routes).1.unwrap();
    assert_eq!(
        eps.get("GET /notes").unwrap().collection.as_deref(),
        Some("notes")
    );
    assert_eq!(
        eps.get("POST /notes").unwrap().collection.as_deref(),
        Some("notes")
    );
    assert_eq!(eps.get("GET /users/{id}").unwrap().collection, None);
}

#[test]
fn total_is_parsed_and_compiled() {
    let src = format!(
        "{BASE}  \"GET /x\": {{ total: 250 }}\n  \"GET /y\": {{ total: 0 }}\n  \"GET /z\": {{ total: 10000 }}\n"
    );
    let routes = [
        ("GET /x", "app"),
        ("GET /y", "app"),
        ("GET /z", "app"),
        ROUTES[0],
    ];
    let eps = build(&src, &routes).1.unwrap();
    assert_eq!(eps.get("GET /x").unwrap().total, Some(250));
    assert_eq!(eps.get("GET /y").unwrap().total, Some(0));
    assert_eq!(eps.get("GET /z").unwrap().total, Some(10000));
    assert_eq!(eps.get("GET /users/{id}").unwrap().total, None);
}

#[test]
fn total_over_10000_is_a_positioned_error() {
    for v in ["10001", "-1", "1.5", "abc"] {
        let e = Config::from_yaml(&format!("{BASE}  \"GET /x\": {{ total: {v} }}\n")).unwrap_err();
        assert_eq!((e.line, e.col), (Some(23), Some(22)), "{v}: {}", e.msg);
        assert!(e.msg.contains("total must be 0 to 10000"), "{v}: {}", e.msg);
    }
}

#[test]
fn total_with_override_is_an_error() {
    let e = err(&format!(
        "{BASE}  \"GET /x\": {{ total: 5, override: {{ status: 200 }} }}\n"
    ));
    assert_eq!(
        (e.line, e.col, e.msg.as_str()),
        (
            Some(23),
            Some(3),
            "endpoint 'GET /x': `total` cannot be combined with `override`"
        )
    );
}

#[test]
fn total_with_collection_is_an_error() {
    let e = err(&format!(
        "{BASE}  \"GET /x\": {{ total: 5, collection: x }}\n"
    ));
    assert_eq!(
        (e.line, e.col, e.msg.as_str()),
        (
            Some(23),
            Some(3),
            "endpoint 'GET /x': `total` cannot be combined with `collection`"
        )
    );
}

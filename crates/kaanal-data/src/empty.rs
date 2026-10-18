//! Empty response bodies: the "no data" shape of a response, for UI-state pins.
//!
//! Arrays become `[]`, pagination counters become `0` / `false` / `null`, nested objects
//! recurse and every other scalar is kept (value form) or generated (schema form).

use serde_json::{Map, Value, json};

use crate::generate::generate;

/// **Private.** The fixed value for a pagination-style property name, matched
/// case-insensitively and exactly. ponytail: a name table, not a model; extend it when a real API misses.
/// Number names apply only to `num` properties, bool names only to `boolean` ones, cursors to any scalar.
fn rule(name: &str, num: bool, boolean: bool) -> Option<Value> {
    Some(match name.to_ascii_lowercase().as_str() {
        "total" | "count" | "totalcount" | "total_count" | "totalitems" | "total_items"
        | "totalelements" | "numfound" | "size"
            if num =>
        {
            json!(0)
        }
        "hasnext" | "has_next" | "hasmore" | "has_more" if boolean => json!(false),
        "next" | "nextcursor" | "next_cursor" | "nextpage" | "next_page" => Value::Null,
        _ => return None,
    })
}

/// The empty form of a response body described by `schema` (`$ref`s already inlined), or
/// `None` when it has none (a bare scalar, or a top-level `oneOf`/`anyOf`/`allOf`).
/// Scalars without a name rule come from [`generate`] with `seed`.
#[must_use]
pub fn empty(schema: &Value, seed: u64) -> Option<Value> {
    if ["oneOf", "anyOf", "allOf"]
        .iter()
        .any(|k| schema.get(k).is_some())
    {
        return None;
    }
    if is_type(schema, "array") {
        Some(json!([]))
    } else if is_type(schema, "object")
        || schema.get("type").is_none() && schema.get("properties").is_some()
    {
        Some(object_of_schema(schema, seed))
    } else {
        None
    }
}

/// **Private.** The object rule over a schema's `properties`.
fn object_of_schema(schema: &Value, seed: u64) -> Value {
    let mut out = Map::new();
    for (name, ps) in schema
        .get("properties")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        let is = |t| is_type(ps, t);
        let v = if is("array") {
            json!([])
        } else if is("object") {
            object_of_schema(ps, seed)
        } else {
            let (num, boolean) = (is("integer") || is("number"), is("boolean"));
            rule(name, num, boolean).unwrap_or_else(|| generate(ps, Some(name), seed))
        };
        out.insert(name.clone(), v);
    }
    Value::Object(out)
}

/// The empty form of a concrete body: arrays emptied, pagination names reset, objects
/// recursed, other scalars kept. `None` for a bare scalar or `null`.
#[must_use]
pub fn empty_of_value(v: &Value) -> Option<Value> {
    match v {
        Value::Array(_) => Some(json!([])),
        Value::Object(m) => Some(Value::Object(
            m.iter()
                .map(|(k, x)| {
                    let e = match x {
                        Value::Array(_) | Value::Object(_) => empty_of_value(x),
                        _ => rule(k, x.is_number(), x.is_boolean()),
                    };
                    (k.clone(), e.unwrap_or_else(|| x.clone()))
                })
                .collect(),
        )),
        _ => None,
    }
}

/// **Private.** A schema "is" `t` when its `type` equals `t` or is an array containing it (`["integer","null"]`).
fn is_type(schema: &Value, t: &str) -> bool {
    match schema.get("type") {
        Some(Value::String(s)) => s == t,
        Some(Value::Array(a)) => a.iter().any(|x| x.as_str() == Some(t)),
        _ => false,
    }
}

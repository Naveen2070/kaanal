//! A generated skeleton `OpenAPI` file for a system that has endpoints but no spec.
//!
//! An `endpoints:` entry cannot create a route on its own: routes are owned by a node through
//! its spec (R1.32). So Studio writes this file, registers it as a node's `spec:`, and the
//! entries then apply to real routes. The skeleton declares each operation, its path
//! parameters and a success response with an empty object schema; payload sizes come from the
//! traffic profile, not from the skeleton.

use std::collections::{HashMap, HashSet};

use indexmap::IndexMap;
use serde_json::{Map, Value, json};

use crate::config::EndpointConfig;
use crate::scenario::endpoint_key;

/// Methods `OpenAPI` 3.0 can describe.
const METHODS: [&str; 7] = ["GET", "PUT", "POST", "DELETE", "PATCH", "HEAD", "OPTIONS"];

/// The distinct `{name}` parts of a path template, in order; a catch-all `{*rest}` is named
/// `rest`. The path itself is written unchanged: the loader hands spec paths to the router as
/// they are, so `{*rest}` must stay a catch-all.
///
/// # Errors
/// An unbalanced brace or an empty name.
fn params(path: &str) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    let mut have: HashSet<&str> = HashSet::new();
    let mut rest = path;
    while let Some(open) = rest.find(['{', '}']) {
        if !rest[open..].starts_with('{') {
            return Err("an unmatched '}'".into());
        }
        let after = &rest[open + 1..];
        let close = after.find('}').ok_or("an unclosed '{'")?;
        let name = after[..close].trim_start_matches('*');
        if name.is_empty() || name.contains('{') {
            return Err("an empty or malformed parameter name".into());
        }
        if have.insert(name) {
            if out.len() == MAX_PARAMS {
                return Err(format!("more than {MAX_PARAMS} path parameters"));
            }
            out.push(name.to_string());
        }
        rest = &after[close + 1..];
    }
    Ok(out)
}

/// `get_users_id` for `GET /users/{id}`; made unique with a numeric suffix. `seen` counts how
/// often each base has been used, so a thousand colliding keys cost a thousand steps, not a
/// million.
fn operation_id(method: &str, path: &str, seen: &mut HashMap<String, usize>) -> String {
    let words: String = format!("{} {path}", method.to_ascii_lowercase())
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("_");
    let n = seen.entry(words.clone()).or_insert(0);
    *n += 1;
    if *n == 1 {
        words
    } else {
        format!("{words}_{n}")
    }
}

/// Longest endpoint key a skeleton accepts, in bytes.
const MAX_KEY_BYTES: usize = 2048;
/// Most distinct path parameters in one path.
const MAX_PARAMS: usize = 32;

/// Most endpoints one skeleton covers; a bound on the work and on the size of the answer.
pub const MAX_ENDPOINTS: usize = 10_000;

/// The skeleton for `entries`, as `OpenAPI` 3.0 YAML titled `title`.
///
/// # Errors
/// No entries, more than [`MAX_ENDPOINTS`], a key that is not `METHOD /path`, a path with a
/// malformed `{parameter}`, or a method `OpenAPI` 3.0 has no operation for.
pub fn skeleton_openapi(
    entries: &IndexMap<String, EndpointConfig>,
    title: &str,
) -> Result<String, String> {
    if entries.is_empty() {
        return Err("there are no endpoints to write a skeleton for".into());
    }
    if entries.len() > MAX_ENDPOINTS {
        return Err(format!(
            "there are {} endpoints; a skeleton covers at most {MAX_ENDPOINTS}",
            entries.len()
        ));
    }
    let mut paths: Map<String, Value> = Map::new();
    let mut seen = HashMap::new();
    for (raw, entry) in entries {
        // plain ASCII only: a generated file must show what it says (no bidi or invisible
        // characters), and a long key is not a path
        if raw.len() > MAX_KEY_BYTES || !raw.chars().all(|c| c.is_ascii_graphic() || c == ' ') {
            return Err("an endpoint key must be at most 2048 plain ASCII characters (percent-encode anything else)".into());
        }
        let key = endpoint_key(raw)?;
        let (method, path) = key.split_once(' ').ok_or("invalid endpoint")?;
        if !METHODS.contains(&method) {
            return Err(format!(
                "endpoint '{key}': OpenAPI 3.0 has no operation for {method}"
            ));
        }
        let status = entry
            .response
            .as_ref()
            .map_or(200, |o| o.status)
            .clamp(100, 599);
        let mut response = json!({ "description": "Skeleton response" });
        if status != 204 && method != "HEAD" {
            response["content"] = json!({ "application/json": { "schema": { "type": "object" } } });
        }
        let parameters: Vec<Value> = params(path)
            .map_err(|m| format!("endpoint '{key}': {m}"))?
            .into_iter()
            .map(|n| json!({ "name": n, "in": "path", "required": true, "schema": { "type": "string" } }))
            .collect();
        let mut op = json!({
            "operationId": operation_id(method, path, &mut seen),
            "responses": { status.to_string(): response },
        });
        if !parameters.is_empty() {
            op["parameters"] = Value::Array(parameters);
        }
        let item = paths.entry(path.to_string()).or_insert_with(|| json!({}));
        if item.get(method.to_ascii_lowercase()).is_some() {
            return Err(format!("endpoint '{key}' appears twice"));
        }
        item[method.to_ascii_lowercase()] = op;
    }
    let doc = json!({
        "openapi": "3.0.3",
        "info": { "title": title, "version": "1.0.0" },
        "x-kaanal-skeleton": true,
        "paths": paths,
    });
    serde_yaml_ng::to_string(&doc).map_err(|e| e.to_string())
}

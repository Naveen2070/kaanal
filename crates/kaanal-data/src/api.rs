//! `OpenAPI` 3.0.x → [`ApiModel`]: the list of endpoints the server will mock.
//!
//! One [`Endpoint`] per `path × method`, with the success response's JSON schema
//! fully `$ref`-inlined so downstream code never resolves references. Phase one
//! supports 3.0.x only; 3.1 is rejected with a clear message.

use serde_json::{Map, Value};

/// One mockable endpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct Endpoint {
    /// Uppercase HTTP method.
    pub method: String,
    /// `OpenAPI` path template; also valid matchit 0.8 syntax (`/users/{id}`).
    pub path: String,
    /// Chosen success status: the lowest 2xx declared, else 200.
    pub status: u16,
    /// application/json response schema with every local `$ref` inlined. `None` = no JSON body.
    pub schema: Option<Value>,
}

/// A parsed `OpenAPI` document.
#[derive(Debug, Clone)]
pub struct ApiModel {
    /// Every endpoint found in `paths`, in document order.
    pub endpoints: Vec<Endpoint>,
}

/// **Private.** HTTP methods recognised under a path item.
const METHODS: [&str; 7] = ["get", "put", "post", "delete", "patch", "head", "options"];
/// **Private.** Max `$ref` expansions per response schema. Acyclic refs can still
/// fan out exponentially (A -> 2x B -> 2x C ...); this caps time and memory.
const MAX_REF_EXPANSIONS: usize = 10_000;

impl ApiModel {
    /// Parses an `OpenAPI` 3.0.x document (YAML or JSON).
    ///
    /// Accepts an unquoted `openapi: 3.0` (which YAML reads as a number). Every
    /// path must start with `/`; every local `$ref` in a response schema is
    /// inlined.
    ///
    /// # Errors
    /// Returns a message for non-3.0.x versions, missing `paths`, bad paths,
    /// external or recursive `$ref`s, dangling refs, or ref-expansion bombs.
    pub fn from_openapi(src: &str) -> Result<ApiModel, String> {
        let doc: Value =
            serde_yaml_ng::from_str(src).map_err(|e| format!("invalid YAML/JSON: {e}"))?;
        // `openapi: 3.0` (unquoted) parses as a YAML number, not a string.
        let version = match doc.get("openapi") {
            Some(Value::String(s)) => s.clone(),
            Some(other) => other.to_string(),
            None => String::new(),
        };
        if !version.starts_with("3.0") {
            return Err(format!(
                "unsupported OpenAPI version '{version}' (phase one supports 3.0.x)"
            ));
        }
        let paths = doc
            .get("paths")
            .and_then(Value::as_object)
            .ok_or_else(|| String::from("missing 'paths'"))?;
        let mut endpoints = Vec::new();
        for (path, item) in paths {
            if !path.starts_with('/') {
                return Err(format!("path '{path}' must start with '/'"));
            }
            let item_ptr = format!("/paths/{}", path.replace('~', "~0").replace('/', "~1"));
            // A path-item `$ref` is followed (external ones fail), never dropped (R6.10e).
            let item = deref(item, &doc, &item_ptr)?;
            for m in METHODS {
                let Some(op) = item.get(m) else { continue };
                let ptr = format!("{item_ptr}/{m}");
                let (status, schema) = pick_response(op, &doc, &ptr)?;
                endpoints.push(Endpoint {
                    method: m.to_uppercase(),
                    path: path.clone(),
                    status,
                    schema,
                });
            }
        }
        Ok(ApiModel { endpoints })
    }
}

/// **Private.** Lowest 2xx response, and its application/json schema if it has one.
fn pick_response(op: &Value, doc: &Value, ptr: &str) -> Result<(u16, Option<Value>), String> {
    let responses = op
        .get("responses")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{ptr}: missing 'responses'"))?;
    let mut codes: Vec<(u16, &Value)> = responses
        .iter()
        .filter_map(|(k, v)| {
            k.parse::<u16>()
                .ok()
                .filter(|c| (200..300).contains(c))
                .map(|c| (c, v))
        })
        .collect();
    codes.sort_by_key(|(c, _)| *c);
    let Some(&(status, resp)) = codes.first() else {
        return Ok((200, None));
    };
    let resp = deref(resp, doc, ptr)?;
    let schema = match resp.pointer("/content/application~1json/schema") {
        Some(s) => {
            let mut budget = MAX_REF_EXPANSIONS;
            Some(inline(s, doc, ptr, &mut Vec::new(), &mut budget)?)
        }
        None => None,
    };
    Ok((status, schema))
}

/// **Private.** Inline every local `$ref`. `stack` holds the refs being expanded on
/// the current path, so any cycle fails on first revisit; `budget` caps total
/// expansions.
fn inline<'d>(
    v: &'d Value,
    doc: &'d Value,
    ptr: &str,
    stack: &mut Vec<&'d str>,
    budget: &mut usize,
) -> Result<Value, String> {
    match v {
        Value::Object(map) => {
            if let Some(r) = map.get("$ref").and_then(Value::as_str) {
                if stack.contains(&r) {
                    return Err(format!("{ptr}: recursive $ref '{r}' is not supported"));
                }
                if *budget == 0 {
                    return Err(format!(
                        "{ptr}: too many $ref expansions (> {MAX_REF_EXPANSIONS})"
                    ));
                }
                *budget -= 1;
                stack.push(r);
                let out = inline(resolve_ref(r, doc, ptr)?, doc, ptr, stack, budget);
                stack.pop();
                return out;
            }
            let mut out = Map::new();
            for (k, x) in map {
                out.insert(k.clone(), inline(x, doc, ptr, stack, budget)?);
            }
            Ok(Value::Object(out))
        }
        Value::Array(items) => Ok(Value::Array(
            items
                .iter()
                .map(|x| inline(x, doc, ptr, stack, budget))
                .collect::<Result<_, _>>()?,
        )),
        other => Ok(other.clone()),
    }
}

/// **Private.** Follows a `$ref` chain (`A -> B -> target`) to a value that is not a ref.
/// A ref seen twice is a cycle and fails, so `A -> B -> A` cannot hang (R6.10e).
fn deref<'d>(mut v: &'d Value, doc: &'d Value, ptr: &str) -> Result<&'d Value, String> {
    let mut seen: Vec<&str> = Vec::new();
    while let Some(r) = v.get("$ref").and_then(Value::as_str) {
        if seen.contains(&r) {
            return Err(format!("{ptr}: recursive $ref '{r}' is not supported"));
        }
        seen.push(r);
        v = resolve_ref(r, doc, ptr)?;
    }
    Ok(v)
}

/// **Private.** Resolves a local (`#/...`) JSON-pointer ref; external refs are rejected.
fn resolve_ref<'d>(r: &str, doc: &'d Value, ptr: &str) -> Result<&'d Value, String> {
    let Some(pointer) = r.strip_prefix('#') else {
        return Err(format!("{ptr}: external $ref '{r}' is not supported"));
    };
    doc.pointer(pointer)
        .ok_or_else(|| format!("{ptr}: $ref '{r}' not found"))
}

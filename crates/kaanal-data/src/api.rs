use std::fmt;

use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq)]
pub struct Endpoint {
    /// Uppercase HTTP method.
    pub method: String,
    /// OpenAPI path template; also valid matchit 0.8 syntax (`/users/{id}`).
    pub path: String,
    pub status: u16,
    /// application/json response schema with every local `$ref` inlined. `None` = no JSON body.
    pub schema: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct ApiModel {
    pub endpoints: Vec<Endpoint>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApiError(pub String);

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ApiError {}

const METHODS: [&str; 7] = ["get", "put", "post", "delete", "patch", "head", "options"];
/// Max `$ref` expansions per response schema. Acyclic refs can still fan out
/// exponentially (A -> 2x B -> 2x C ...); this caps time and memory.
const MAX_REF_EXPANSIONS: usize = 10_000;

impl ApiModel {
    pub fn from_openapi(src: &str) -> Result<ApiModel, ApiError> {
        let doc: Value = serde_yaml_ng::from_str(src)
            .map_err(|e| ApiError(format!("invalid YAML/JSON: {e}")))?;
        // `openapi: 3.0` (unquoted) parses as a YAML number, not a string.
        let version = match doc.get("openapi") {
            Some(Value::String(s)) => s.clone(),
            Some(other) => other.to_string(),
            None => String::new(),
        };
        if !version.starts_with("3.0") {
            return Err(ApiError(format!(
                "unsupported OpenAPI version '{version}' (phase one supports 3.0.x)"
            )));
        }
        let paths = doc
            .get("paths")
            .and_then(Value::as_object)
            .ok_or_else(|| ApiError("missing 'paths'".into()))?;
        let mut endpoints = Vec::new();
        for (path, item) in paths {
            if !path.starts_with('/') {
                return Err(ApiError(format!("path '{path}' must start with '/'")));
            }
            for m in METHODS {
                let Some(op) = item.get(m) else { continue };
                let ptr = format!("/paths/{}/{m}", path.replace('~', "~0").replace('/', "~1"));
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

/// Lowest 2xx response, and its application/json schema if it has one.
fn pick_response(op: &Value, doc: &Value, ptr: &str) -> Result<(u16, Option<Value>), ApiError> {
    let responses = op
        .get("responses")
        .and_then(Value::as_object)
        .ok_or_else(|| ApiError(format!("{ptr}: missing 'responses'")))?;
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
    let resp = match resp.get("$ref").and_then(Value::as_str) {
        Some(r) => resolve_ref(r, doc, ptr)?,
        None => resp,
    };
    let schema = match resp.pointer("/content/application~1json/schema") {
        Some(s) => {
            let mut budget = MAX_REF_EXPANSIONS;
            Some(inline(s, doc, ptr, &mut Vec::new(), &mut budget)?)
        }
        None => None,
    };
    Ok((status, schema))
}

/// Inline every local `$ref`. `stack` holds the refs being expanded on the current path,
/// so any cycle fails on first revisit; `budget` caps total expansions.
fn inline<'d>(
    v: &'d Value,
    doc: &'d Value,
    ptr: &str,
    stack: &mut Vec<&'d str>,
    budget: &mut usize,
) -> Result<Value, ApiError> {
    match v {
        Value::Object(map) => {
            if let Some(r) = map.get("$ref").and_then(Value::as_str) {
                if stack.contains(&r) {
                    return Err(ApiError(format!(
                        "{ptr}: recursive $ref '{r}' is not supported"
                    )));
                }
                if *budget == 0 {
                    return Err(ApiError(format!(
                        "{ptr}: too many $ref expansions (> {MAX_REF_EXPANSIONS})"
                    )));
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

fn resolve_ref<'d>(r: &str, doc: &'d Value, ptr: &str) -> Result<&'d Value, ApiError> {
    let Some(pointer) = r.strip_prefix('#') else {
        return Err(ApiError(format!(
            "{ptr}: external $ref '{r}' is not supported"
        )));
    };
    doc.pointer(pointer)
        .ok_or_else(|| ApiError(format!("{ptr}: $ref '{r}' not found")))
}

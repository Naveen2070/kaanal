//! Self-validation (FR-DATA-05, D2 spec section 7): generate a few samples of each endpoint's
//! response and validate them against the schema the generator saw. A miss is usually Kaanal's
//! bug (or a spec that contradicts itself, such as an `example` over its own `maximum`), so
//! findings are warnings; the repo-wide test makes them hard failures.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::api::{ApiModel, Endpoint};
use crate::dataset::Options;
use crate::generate::{child_seed, generate};
use crate::page::{Envelope, PageSpec, generate_page};

/// Samples per endpoint at `validate` and `up` (a smoke check; the repo-wide test uses far more).
pub const SAMPLES: usize = 5;

/// One schema violation in one generated sample.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// `GET /orders`.
    pub endpoint: String,
    /// JSON Pointer of the offending value in the sample, e.g. `/items/0/total`.
    pub path: String,
    /// The failed schema keyword, e.g. `maximum`.
    pub keyword: String,
    /// The value is `null` because the body budget or depth limit cut it, not a schema miss.
    pub budget: bool,
    /// Index of the generated sample (0-based) the finding came from.
    pub sample: usize,
}

/// Validates `samples` generated responses of `e` (seeds derived from `base_seed`) with `opts`.
/// An endpoint without a response schema has no findings. A schema the validator cannot build
/// yields one finding with keyword `schema`.
#[must_use]
pub fn check_endpoint(
    e: &Endpoint,
    opts: &Options,
    samples: usize,
    base_seed: u64,
) -> Vec<Finding> {
    let Some(schema) = &e.schema else {
        return Vec::new();
    };
    let name = format!("{} {}", e.method, e.path);
    let page = PageSpec::detect(e);
    let mut prepared = schema.clone();
    prepare(&mut prepared);
    if let Some(p) = &page {
        // a page holds at most one page size of records, however large `minItems` is
        strip_min_items(&mut prepared, &p.envelope);
    }
    let validator = match jsonschema::draft4::options()
        .should_validate_formats(true)
        .build(&prepared)
    {
        Ok(v) => v,
        Err(err) => {
            return vec![Finding {
                endpoint: name,
                path: err.to_string(),
                keyword: "schema".into(),
                budget: false,
                sample: 0,
            }];
        }
    };
    // a pattern the generator cannot honour already has its own startup warning
    let skip_pattern = has_unmet_pattern(schema);
    let mut out = Vec::new();
    for k in 0..samples {
        let seed = child_seed(base_seed, "selfcheck", k);
        let body = match page
            .as_ref()
            .and_then(|p| Some((p, p.window(&[], seed, None).ok()?)))
        {
            Some((p, w)) => generate_page(schema, p, &w, seed, opts, &e.path, &[]),
            None => generate(schema, None, seed, opts),
        };
        let start = out.len();
        for err in validator.iter_errors(&body) {
            if skip_pattern && err.kind().keyword() == "pattern" {
                continue;
            }
            let path = err.instance_path().to_string();
            out.push(Finding {
                endpoint: name.clone(),
                budget: body.pointer(&path).is_some_and(Value::is_null),
                path,
                keyword: err.kind().keyword().to_string(),
                sample: k,
            });
        }
        // An array cut short by the budget breaks its own `minItems`; when this sample also has a
        // budget null, that miss is the same cause.
        if out[start..].iter().any(|f| f.budget) {
            for f in &mut out[start..] {
                f.budget |= f.keyword == "minItems";
            }
        }
    }
    out
}

/// Checks every endpoint of `model` and returns deduplicated warning lines (one per endpoint
/// for violations, one for budget cuts). `opts_for` gives the endpoint's own options (its
/// node's locale pack and the project dictionary change what is generated).
pub fn check_model(
    model: &ApiModel,
    opts_for: &dyn Fn(&Endpoint) -> Options,
    samples: usize,
) -> Vec<String> {
    let mut lines = Vec::new();
    for e in &model.endpoints {
        let base = child_seed(0, &format!("{} {}", e.method, e.path), 0);
        lines.extend(lines_for(
            &check_endpoint(e, &opts_for(e), samples, base),
            samples,
        ));
    }
    lines
}

/// **Private.** Warning lines for one endpoint's findings.
fn lines_for(findings: &[Finding], samples: usize) -> Vec<String> {
    let Some(first) = findings.first() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for budget in [false, true] {
        let mine: Vec<&Finding> = findings.iter().filter(|f| f.budget == budget).collect();
        if mine.is_empty() {
            continue;
        }
        let kinds: BTreeMap<String, usize> = mine.iter().fold(BTreeMap::new(), |mut m, f| {
            *m.entry(format!("{}: {}", generalise(&f.path), f.keyword))
                .or_default() += 1;
            m
        });
        let shown: Vec<&str> = kinds.keys().take(3).map(String::as_str).collect();
        let more = kinds.len().saturating_sub(3);
        let tail = if more > 0 {
            format!(" (and {more} more)")
        } else {
            String::new()
        };
        let mut hits: Vec<usize> = mine.iter().map(|f| f.sample).collect();
        hits.sort_unstable();
        hits.dedup();
        let hit = hits.len();
        out.push(if budget {
            format!(
                "{}: the body budget or depth limit turned values into null in {hit} of {samples} generated samples ({}){tail}; a smaller minItems or a shallower schema keeps the body whole",
                first.endpoint,
                shown.join(", ")
            )
        } else {
            format!(
                "{}: {hit} of {samples} generated samples violate the response schema: {}{tail}",
                first.endpoint,
                shown.join(", ")
            )
        });
    }
    out
}

/// **Private.** `/items/3/total` -> `/items/*/total`, so one miss in a list is one line.
fn generalise(path: &str) -> String {
    path.split('/')
        .map(|s| {
            if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) {
                "*"
            } else {
                s
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// **Private.** Rewrites a schema copy into what the generator actually honours, so the
/// validator reports misses and not deliberate differences: `writeOnly` properties are never
/// returned (R6.10b); `nullable` becomes a type union; empty `required`/`enum`/`allOf`/`anyOf`/
/// `oneOf` are dropped (the generator ignores them and Draft 4's meta-schema rejects them);
/// a boolean `exclusive*` without its bound is dropped; keywords the generator ignores (and
/// `multipleOf`/`uniqueItems` where it cannot honour them) have their own startup warning, so
/// they are dropped too.
fn prepare(s: &mut Value) {
    let drop_multiple = crate::generate::multiple::problem(s).is_some();
    let drop_unique = crate::generate::unique::problem(s)
        .is_some_and(|w| w.starts_with("uniqueItems is only supported"));
    let Value::Object(m) = s else { return };
    for k in crate::warn::IGNORED {
        m.remove(k);
    }
    if drop_multiple {
        m.remove("multipleOf");
    }
    if drop_unique {
        m.remove("uniqueItems");
    }
    let mut write_only: Vec<String> = Vec::new();
    if let Some(Value::Object(props)) = m.get_mut("properties") {
        props.retain(|name, p| {
            let w = p.get("writeOnly").and_then(Value::as_bool) == Some(true);
            if w {
                write_only.push(name.clone());
            }
            !w
        });
        props.values_mut().for_each(prepare);
    }
    if let Some(Value::Array(req)) = m.get_mut("required") {
        req.retain(|r| {
            r.as_str()
                .is_none_or(|r| !write_only.iter().any(|w| w == r))
        });
    }
    for k in ["required", "enum", "allOf", "anyOf", "oneOf"] {
        if m.get(k)
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
        {
            m.remove(k);
        }
    }
    for (excl, bound) in [
        ("exclusiveMinimum", "minimum"),
        ("exclusiveMaximum", "maximum"),
    ] {
        if m.get(excl).is_some_and(Value::is_boolean) && !m.contains_key(bound) {
            m.remove(excl);
        }
    }
    if m.remove("nullable") == Some(Value::Bool(true)) {
        match m.get_mut("type") {
            Some(Value::String(t)) => {
                let t = std::mem::take(t);
                m.insert("type".into(), Value::Array(vec![t.into(), "null".into()]));
            }
            Some(Value::Array(a)) => a.push("null".into()),
            _ => {}
        }
        if let Some(Value::Array(e)) = m.get_mut("enum") {
            e.push(Value::Null);
        }
    }
    for k in ["items", "additionalProperties"] {
        if let Some(v) = m.get_mut(k) {
            prepare(v);
        }
    }
    for k in ["allOf", "anyOf", "oneOf"] {
        if let Some(Value::Array(a)) = m.get_mut(k) {
            a.iter_mut().for_each(prepare);
        }
    }
}

/// **Private.** Removes `minItems` from the array a paged route pages.
fn strip_min_items(schema: &mut Value, envelope: &Envelope) {
    let target = match envelope {
        Envelope::Array => Some(schema),
        Envelope::Object { items } => schema
            .get_mut("properties")
            .and_then(|props| props.get_mut(items.as_str())),
    };
    if let Some(Value::Object(m)) = target {
        m.remove("minItems");
    }
}

/// **Private.** Whether some schema in `s` has a `pattern` that already has a startup warning
/// (unsupported, or no candidate fits its length bounds). Coarse on purpose: the validator's
/// error does not say which pattern failed. Only schema positions are walked, never `example`
/// or `enum` values.
fn has_unmet_pattern(s: &Value) -> bool {
    let Value::Object(m) = s else { return false };
    if let Some(p) = m.get("pattern").and_then(Value::as_str)
        && crate::generate::pattern::problem(p, s).is_some()
    {
        return true;
    }
    let subs = ["items", "additionalProperties"]
        .iter()
        .filter_map(|k| m.get(*k));
    let lists = ["allOf", "anyOf", "oneOf"]
        .iter()
        .filter_map(|k| m.get(*k).and_then(Value::as_array))
        .flatten();
    let props = m
        .get("properties")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|p| p.values());
    subs.chain(lists).chain(props).any(has_unmet_pattern)
}

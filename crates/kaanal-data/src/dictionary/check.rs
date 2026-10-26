//! Parameter checks for generators and the startup warnings for project entries that do not fit.

use serde_json::Value;

use super::Dictionary;
use super::generator::Gen;
use super::generator::template_key;
use crate::ApiModel;
use crate::dataset::Options;
use crate::generate::{MAX_DEPTH, merge_all_of, scalar_type};
use crate::pack::fill;

/// **Private.** Largest `days` window of a `timestamp` or `date` entry: 100 years.
const MAX_DAYS: u32 = 36_500;
/// **Private.** Oldest age a `date` entry's `years` may reach.
const MAX_AGE: u32 = 150;

/// **Private.** Checks a generator's parameters beyond their types.
pub(super) fn check(g: &Gen) -> Result<(), String> {
    let ordered = |lo: Option<f64>, hi: Option<f64>| match (lo, hi) {
        _ if lo.into_iter().chain(hi).any(|x| !x.is_finite()) => {
            Err("min and max must be finite".to_string())
        }
        (Some(a), Some(b)) if a > b => Err(format!("min {a} is above max {b}")),
        _ => Ok(()),
    };
    match g {
        #[expect(
            clippy::cast_precision_loss,
            reason = "only compared, never used as a value"
        )]
        Gen::Integer { min, max, .. } => ordered(min.map(|v| v as f64), max.map(|v| v as f64)),
        Gen::Money { min, max, .. } | Gen::Latitude { min, max } | Gen::Longitude { min, max } => {
            ordered(*min, *max)
        }
        Gen::Pick {
            weights: Some(w), ..
        } if !w.0.values().any(|n| *n > 0) => {
            Err("weights: at least one weight must be above 0".to_string())
        }
        Gen::Timestamp { days: Some(d) } | Gen::Date { days: Some(d), .. }
            if !(1..=MAX_DAYS).contains(d) =>
        {
            Err(format!("days must be between 1 and {MAX_DAYS} (100 years)"))
        }
        Gen::Date {
            years: Some([lo, hi]),
            ..
        } if lo > hi || *hi > MAX_AGE => Err(format!(
            "years [{lo}, {hi}] must be ordered and at most {MAX_AGE}"
        )),
        Gen::Template { value } => fill(value, |k| template_key(k).map(|_| String::new()))
            .map(|_| ())
            .ok_or_else(|| {
                format!(
                    "template `{value}` has an unclosed brace or an unknown placeholder; use {{digits:n}} or {{letters:n}}, n from 1 to 20"
                )
            }),
        _ => Ok(()),
    }
}

/// Startup warnings for project dictionary entries that match a response field but cannot
/// fit its type or bounds, so they are skipped (design 11a rule 2). One line per field,
/// e.g. `GET /users .age: ...`. Built-in entries that do not fit are skipped silently.
pub fn dictionary_warnings(model: &ApiModel, opts: &Options) -> Vec<String> {
    let Some(project) = opts.dictionary_project.as_deref() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in &model.endpoints {
        if let Some(s) = &e.schema {
            walk(
                s,
                None,
                &format!("{} {}", e.method, e.path),
                project,
                &mut out,
                0,
            );
        }
    }
    out
}

/// **Private.** Schema walk for [`dictionary_warnings`], following what the generator visits.
fn walk(
    s: &Value,
    field: Option<&str>,
    at: &str,
    d: &Dictionary,
    out: &mut Vec<String>,
    depth: usize,
) {
    if depth > MAX_DEPTH {
        return;
    }
    if s.get("allOf").is_some() {
        return walk(&merge_all_of(s).0, field, at, d, out, depth);
    }
    let Value::Object(m) = s else { return };
    // as the generator: examples, const and enums win, writeOnly properties are never returned
    let non_empty = |k| {
        m.get(k)
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty())
    };
    let examples = non_empty("examples")
        || m.get("examples")
            .and_then(Value::as_object)
            .is_some_and(|o| o.values().any(|e| e.get("value").is_some()));
    if m.contains_key("example")
        || examples
        || m.contains_key("const")
        || non_empty("enum")
        || m.get("writeOnly").and_then(Value::as_bool) == Some(true)
    {
        return;
    }
    // the generator takes a branch of the first non-empty `oneOf`/`anyOf`, never the schema around it
    if let Some(parts) = ["oneOf", "anyOf"].iter().find_map(|k| {
        m.get(*k)
            .and_then(Value::as_array)
            .filter(|a| !a.is_empty())
    }) {
        for part in parts {
            walk(part, field, at, d, out, depth + 1);
        }
        return;
    }
    match scalar_type(s) {
        "object" => {
            for (name, p) in m
                .get("properties")
                .and_then(Value::as_object)
                .into_iter()
                .flatten()
            {
                walk(p, Some(name), &format!("{at} .{name}"), d, out, depth + 1);
            }
        }
        "array" => {
            if let Some(items) = m.get("items") {
                walk(items, None, at, d, out, depth + 1);
            }
        }
        ty => {
            let Some(f) = field else { return };
            let Some(gens) = d.lookup(f) else { return };
            // as the generator: the first entry that fits the type applies
            let msg = match gens.iter().find(|g| g.fits(s, ty)) {
                None => format!(
                    "{at}: project dictionary entry for `{f}` does not fit type `{ty}` or its bounds; skipped"
                ),
                Some(chosen) if !chosen.length_fits(s, ty) => format!(
                    "{at}: project dictionary value for `{f}` can fall outside its minLength/maxLength; it is sent as written"
                ),
                Some(_) => return,
            };
            if !out.contains(&msg) {
                out.push(msg);
            }
        }
    }
}

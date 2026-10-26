//! The reconcile pass (D2 spec section 3): after an object's properties are final, rules make
//! related fields agree (time order, age and birth date, money sums).
//!
//! Invariant: no shared rng stream. A rule's own values come from seeds derived from the field
//! itself (`child_seed`), or are pure functions of values already there, so adding an unrelated
//! property never changes another field. A rule only rewrites *free* fields: a property is
//! pinned (the developer wrote the value) when its schema has `example`, `examples`, `const` or
//! `enum`, or a project dictionary entry for it is a `static` or `pick`.

mod money;
mod person;
mod time;

use serde_json::{Map, Value};

use crate::dataset::Options;
use crate::dictionary::{Gen, norm, value_fits};
use crate::generate::pattern::cached;
use crate::generate::{count, multiple, scalar_type};

/// Makes the fields of one finished object agree. `schema` is the object's schema (its
/// `properties` give each field's schema); `seed` is the object's seed. Idempotent.
pub(crate) fn reconcile(obj: &mut Map<String, Value>, schema: &Value, seed: u64, opts: &Options) {
    let Some(props) = schema.get("properties").and_then(Value::as_object) else {
        return;
    };
    // One pass decides which rules can apply at all, so an object that names none of the fields
    // they look for (the common case) costs one normalised name per key and nothing more.
    let (mut chain, mut age, mut dob) = (false, false, false);
    for k in obj.keys() {
        let n = norm(k);
        chain |= time::is_chain_name(&n, opts);
        age |= n == "age";
        dob |= person::is_dob_name(&n);
    }
    if chain {
        time::chains(obj, props, seed, opts);
    }
    if age && dob {
        person::age_dob(obj, props, seed, opts);
    }
    if obj
        .values()
        .any(|v| v.as_array().is_some_and(|a| a.iter().any(Value::is_object)))
    {
        money::sums(obj, props, seed, opts);
    }
}

/// Whether the value of property `name` (schema `ps`) was written by the developer, so no rule
/// may change it. `name` is the property's own spelling: the dictionary matches suffix keys on
/// its words (`createdAt` -> `created`, `at`).
pub(crate) fn is_pinned(ps: &Value, name: &str, opts: &Options) -> bool {
    let has = |k: &str| ps.get(k).is_some_and(|v| !v.is_null());
    let listed = |k: &str| {
        ps.get(k).is_some_and(|v| match v {
            Value::Array(a) => !a.is_empty(),
            Value::Object(o) => !o.is_empty(),
            _ => true,
        })
    };
    if has("example") || has("const") || listed("examples") || listed("enum") {
        return true;
    }
    // the generator the field actually got: the first entry that fits its type and bounds
    let ty = scalar_type(ps);
    opts.dictionary_project
        .as_deref()
        .and_then(|d| d.lookup(name))
        .and_then(|gens| gens.iter().find(|g| g.fits(ps, ty)))
        .is_some_and(|g| matches!(g, Gen::Static { .. } | Gen::Pick { .. }))
}

/// Whether `v` may be written to a property with schema `ps`: its type and bounds, a string's
/// lengths and `pattern`, and `multipleOf`. A rule that would break one leaves the generated
/// value alone (spec 4.1).
pub(crate) fn can_hold(ps: &Value, v: &Value) -> bool {
    let ty = scalar_type(ps);
    if !value_fits(v, ps, ty) {
        return false;
    }
    if let Some(text) = v.as_str() {
        let n = u64::try_from(text.chars().count()).unwrap_or(u64::MAX);
        if count(ps, "minLength", true).is_some_and(|m| n < m)
            || count(ps, "maxLength", false).is_some_and(|m| n > m)
        {
            return false;
        }
        if let Some(p) = ps.get("pattern").and_then(Value::as_str) {
            let entry = cached(p);
            if entry.as_ref().as_ref().is_ok_and(|c| !c.is_match(text)) {
                return false;
            }
        }
    }
    !v.is_number() || multiple::snap(v.clone(), ty, ps) == *v
}

/// The key of `obj` whose normalised name is `want`, if any. Compares without allocating: this
/// runs for every name of every rule on every generated object.
pub(crate) fn key_for(obj: &Map<String, Value>, want: &str) -> Option<String> {
    obj.keys().find(|k| norm_eq(k, want)).cloned()
}

/// **Private.** Whether `norm(field) == want`, without building the normalised string.
fn norm_eq(field: &str, want: &str) -> bool {
    field
        .chars()
        .filter(|c| *c != '_' && *c != '-')
        .flat_map(char::to_lowercase)
        .eq(want.chars())
}

#[cfg(test)]
mod tests {
    use super::reconcile;
    use crate::dataset::Options;
    use crate::generate::generate;
    use serde_json::{Value, json};

    #[test]
    fn reconcile_is_idempotent() {
        let s = json!({ "type": "object", "properties": {
            "createdAt": { "type": "string", "format": "date-time" },
            "updatedAt": { "type": "string", "format": "date-time" },
            "deletedAt": { "type": "string", "format": "date-time" }
        } });
        let opts = Options::default();
        for seed in 0..500 {
            let once = generate(&s, None, seed, &opts);
            let Value::Object(mut again) = once.clone() else {
                unreachable!("an object schema gives an object")
            };
            reconcile(&mut again, &s, seed, &opts);
            assert_eq!(once, Value::Object(again));
        }
    }

    /// An object `{ <lists>, subtotal: 0.0, total: 0.0 }` whose lists hold `qty`/`price` lines.
    fn with_lists(lists: &[(&str, Value)]) -> (serde_json::Map<String, Value>, Value) {
        let mut obj = serde_json::Map::new();
        let mut props = serde_json::Map::new();
        for (name, v) in lists {
            obj.insert((*name).into(), v.clone());
            props.insert(
                (*name).into(),
                json!({ "type": "array", "items": { "type": "object", "properties": {
                    "price": { "type": "number" }, "quantity": { "type": "integer" } } } }),
            );
        }
        for k in ["subtotal", "total"] {
            obj.insert(k.into(), json!(0.0));
            props.insert(k.into(), json!({ "type": "number" }));
        }
        (obj, json!({ "type": "object", "properties": props }))
    }

    fn lines() -> Value {
        json!([{ "price": 2.5, "quantity": 2 }, { "price": 1.25, "quantity": 4 }])
    }

    #[test]
    fn a_budget_null_element_skips_the_sum() {
        let (mut obj, schema) =
            with_lists(&[("items", json!([{ "price": 2.5, "quantity": 2 }, null]))]);
        let before = obj.clone();
        reconcile(&mut obj, &schema, 1, &Options::default());
        assert_eq!(obj, before);
    }

    #[test]
    fn several_arrays_use_the_preferred_name_or_skip() {
        let (mut obj, schema) = with_lists(&[("items", lines()), ("history", lines())]);
        reconcile(&mut obj, &schema, 1, &Options::default());
        assert_eq!(obj["subtotal"], json!(10.0));
        let (mut obj, schema) = with_lists(&[("alpha", lines()), ("beta", lines())]);
        let before = obj.clone();
        reconcile(&mut obj, &schema, 1, &Options::default());
        assert_eq!(obj, before);
    }

    #[test]
    fn an_empty_array_is_not_summed() {
        let (mut obj, schema) = with_lists(&[("items", json!([]))]);
        let before = obj.clone();
        reconcile(&mut obj, &schema, 1, &Options::default());
        assert_eq!(obj, before);
    }

    #[test]
    fn money_sums_are_idempotent() {
        let (mut obj, schema) = with_lists(&[("items", lines())]);
        reconcile(&mut obj, &schema, 7, &Options::default());
        let once = obj.clone();
        reconcile(&mut obj, &schema, 7, &Options::default());
        assert_eq!(obj, once);
        assert_eq!(obj["subtotal"], json!(10.0));
    }
}

//! Envelope detection and page generation (spec section 2.4): which array of a response is paged,
//! and the page body with its meta siblings filled consistently with the window.

use super::{PageSpec, Style, Window, encode_cursor, normalise};
use crate::dataset::Options;
use crate::generate::{MAX_VALUES, child_seed, generate, merge_all_of, scalar_type, value_bound};
use serde_json::{Map, Value, json};
use std::borrow::Cow;

/// Where the records of a paged response go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Envelope {
    /// The response is the array itself; metadata goes in headers.
    Array,
    /// The response is an object whose property `items` holds the array.
    Object {
        /// Name of the paged array property, as the spec spells it.
        items: String,
    },
}

/// Array property names that win over key order (normalised).
const NAMED: &[&str] = &["items", "data", "results", "content", "records"];
/// Sibling names that carry the total item count (normalised).
const TOTAL: &[&str] = &[
    "total",
    "totalcount",
    "totalitems",
    "totalelements",
    "count",
];
/// Sibling names that carry the previous page (normalised); exact, so `preview` is not one.
const PREV: &[&str] = &[
    "prev",
    "previous",
    "prevcursor",
    "previouscursor",
    "prevpage",
    "previouspage",
    "prevpagetoken",
];

/// `s` with its `allOf` merged, the same view the generator takes.
fn merged(s: &Value) -> Cow<'_, Value> {
    if s.get("allOf").is_some() {
        Cow::Owned(merge_all_of(s).0)
    } else {
        Cow::Borrowed(s)
    }
}

/// The array properties of an object schema (`allOf` merged), in key order.
pub(crate) fn array_props(schema: &Value) -> Vec<String> {
    let s = merged(schema);
    s.get("properties")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter(|(_, ps)| scalar_type(&merged(ps)) == "array")
        .map(|(k, _)| k.clone())
        .collect()
}

/// The seed of record `i` of a paged collection (R4.4): `child_seed(seed, "item", i)`.
#[must_use]
pub fn item_seed(seed: u64, i: u32) -> u64 {
    child_seed(seed, "item", i as usize)
}

impl Envelope {
    /// The envelope of a response schema; `None` when it has no array to page.
    ///
    /// An object pages its first array property named `items|data|results|content|records`,
    /// else its first array property. "First" is in key order: `serde_json` keeps object keys
    /// sorted (no `preserve_order`), not in the order the spec writes them.
    #[must_use]
    pub fn detect(schema: &Value) -> Option<Envelope> {
        let s = merged(schema);
        match scalar_type(&s) {
            "array" => Some(Envelope::Array),
            "object" => {
                let arrays = array_props(&s);
                let key = arrays
                    .iter()
                    .find(|k| NAMED.contains(&normalise(k).as_str()))
                    .or_else(|| arrays.first())?;
                Some(Envelope::Object { items: key.clone() })
            }
            _ => None,
        }
    }

    /// The schema of the array this envelope pages in `schema` (`allOf` merged).
    pub(crate) fn array_of(&self, schema: &Value) -> Value {
        let s = merged(schema);
        match self {
            Envelope::Array => s.into_owned(),
            Envelope::Object { items } => s
                .get("properties")
                .and_then(|p| p.get(items))
                .map_or(Value::Null, |a| merged(a).into_owned()),
        }
    }

    /// Most records a page of `schema` may hold so its body stays within `MAX_VALUES`: what an
    /// object envelope itself may spend comes off first, then each record's worst case
    /// ([`value_bound`]). At least 1, so a single record over the cap is still served.
    pub(crate) fn record_cap(&self, schema: &Value) -> u32 {
        let array = self.array_of(schema);
        let per = value_bound(array.get("items").unwrap_or(&json!({})), 0).max(1);
        let own = match self {
            Envelope::Array => 0,
            Envelope::Object { items } => value_bound(&siblings(schema, items), 0),
        };
        u32::try_from(MAX_VALUES.saturating_sub(own) / per)
            .unwrap_or(u32::MAX)
            .max(1)
    }
}

/// An object envelope with its paged array `key` emptied (`maxItems: 0`): what is generated
/// around the records, so a large `minItems` costs nothing.
fn siblings(schema: &Value, key: &str) -> Value {
    let mut rest = merged(schema).into_owned();
    if let Some(Value::Object(props)) = rest.get_mut("properties") {
        props.insert(key.to_string(), json!({"type": "array", "maxItems": 0}));
    }
    rest
}

/// The body of one page: records `[offset, min(offset + size, total))`, each generated alone
/// from [`item_seed`], so a record never depends on the page that holds it.
///
/// An object envelope is generated as today with its paged array emptied ([`siblings`]), then
/// the paged array and the meta siblings (spec section 2.4, matched by normalised name and JSON type)
/// are overwritten. `path` and `query` are the request's: a string `next`/`prev` of the page
/// and offset styles is the URL of that page.
#[must_use]
pub fn generate_page(
    schema: &Value,
    spec: &PageSpec,
    w: &Window,
    seed: u64,
    opts: &Options,
    path: &str,
    query: &[(String, String)],
) -> Value {
    let array = spec.envelope.array_of(schema);
    let items = array.get("items").cloned().unwrap_or_else(|| json!({}));
    let end = w.total.min(w.offset.saturating_add(w.size));
    // Each record has its own value budget (generate's), so a record is the same alone and in
    // any page; the window's size is already shrunk to `record_cap`, so the records together
    // stay within one body's `MAX_VALUES` (a single record may still use the whole budget).
    let records: Vec<Value> = (w.offset..end)
        .map(|i| generate(&items, None, item_seed(seed, i), opts))
        .collect();
    let Envelope::Object { items: key } = &spec.envelope else {
        return Value::Array(records);
    };
    let mut body = generate(&siblings(schema, key), None, seed, opts);
    let s = merged(schema);
    if let (Value::Object(out), Some(props)) =
        (&mut body, s.get("properties").and_then(Value::as_object))
    {
        let url = |o| spec.page_url(path, query, o, w.size);
        fill_meta(out, props, key, w, spec, &url);
        out.insert(key.clone(), Value::Array(records));
    }
    body
}

/// Overwrite the known meta siblings of `out` with values consistent with `w`; `url` gives the
/// URL of the page at an offset.
fn fill_meta(
    out: &mut Map<String, Value>,
    props: &Map<String, Value>,
    key: &str,
    w: &Window,
    spec: &PageSpec,
    url: &dyn Fn(u32) -> String,
) {
    let (total, offset, size) = (w.total, w.offset, w.size.max(1));
    let base = spec.page_base;
    let next = offset.checked_add(size).filter(|n| *n < total);
    // ponytail: prev of a page past the end is still past the end; nobody walks backwards there.
    let prev = (offset > 0).then(|| offset.saturating_sub(size));
    // A next/prev field: a cursor (cursor style) or that page's URL when it is a string, a page
    // number when it is a number, `null` when there is no such page; `None` (left as generated)
    // for any other type.
    let link = |ty: &str, at: Option<u32>| {
        let v = |o: u32| match ty {
            "string" if spec.style == Style::Cursor => json!(encode_cursor(o)),
            "string" => json!(url(o)),
            _ => json!((o / size).saturating_add(base)),
        };
        matches!(ty, "string" | "integer" | "number").then(|| at.map_or(Value::Null, v))
    };
    for (name, ps) in props {
        // the array itself, and fields the generator left out (`writeOnly`)
        if name == key || !out.contains_key(name) {
            continue;
        }
        let ty = scalar_type(&merged(ps));
        let int = matches!(ty, "integer" | "number");
        let n = normalise(name);
        let v = match n.as_str() {
            t if int && TOTAL.contains(&t) => json!(total),
            "page" | "pagenumber" if int => json!((offset / size).saturating_add(base)),
            "limit" | "size" | "pagesize" | "perpage" if int => json!(size),
            "offset" if int => json!(offset),
            "totalpages" | "pagecount" | "pages" if int => json!(total.div_ceil(size)),
            "hasmore" | "hasnext" | "hasnextpage" if ty == "boolean" => json!(next.is_some()),
            "nextcursor" | "next" | "nextpagetoken" | "nextpage" => match link(ty, next) {
                Some(v) => v,
                None => continue,
            },
            t if PREV.contains(&t) => match link(ty, prev) {
                Some(v) => v,
                None => continue,
            },
            _ => continue,
        };
        out.insert(name.clone(), v);
    }
}

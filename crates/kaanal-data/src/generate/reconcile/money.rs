//! Money sums (D2 spec section 4.3): an order's `subtotal` and `total` add up its lines.
//!
//! Applies only to real line items: an element with a unit price (`price`/`unitPrice`) and a
//! quantity (`quantity`/`qty`) or a line-amount field (`lineTotal`/`amount`/`subtotal`). A plain
//! listing `{items: [{name, price}], total: 3}` has no quantity, so its `total` stays a count; a
//! bare `total` takes part only next to another money field (`subtotal`, `tax`, `shipping`,
//! `discount`) or when its schema type is `number`.
//! Arithmetic is exact: all-integer money uses checked `i128` integers; otherwise cents, rounded
//! half-even. Mixed schema types or an overflow skip the object. Never rescaled. Everything is
//! worked out before anything is written, so an object is reconciled whole or not at all.

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde_json::{Map, Number, Value, json};

use super::{can_hold, is_pinned, key_for};
use crate::dataset::Options;
use crate::generate::{child_seed, merge_all_of, scalar_type};
use crate::page::is_list_name;

const PRICE: [&str; 2] = ["price", "unitprice"];
const QTY: [&str; 2] = ["quantity", "qty"];
const AMOUNT: [&str; 3] = ["linetotal", "amount", "subtotal"];
const TAX: [&str; 2] = ["tax", "taxamount"];
const SHIPPING: [&str; 2] = ["shipping", "shippingcost"];
const DISCOUNT: [&str; 2] = ["discount", "discountamount"];
const TOTAL: [&str; 3] = ["total", "ordertotal", "grandtotal"];
/// **Private.** Flat shipping bands in cents, picked by the field's own seed.
const SHIPPING_CENTS: [i128; 4] = [0, 499, 999, 1499];
/// **Private.** Most tax, in permille of the subtotal.
const MAX_TAX_PERMILLE: i128 = 200;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Whole units, checked integer arithmetic.
    Int,
    /// Two decimals, held as integer cents.
    Cents,
}

/// **Private.** Value of a money field in the mode's integer unit.
#[expect(
    clippy::cast_possible_truncation,
    reason = "bounded by the 1e12 check just above"
)]
fn read(value: &Value, mode: Mode) -> Option<i128> {
    match mode {
        Mode::Int => value.as_i64().map(i128::from),
        Mode::Cents => {
            let x = value.as_f64()?;
            (x.is_finite() && x.abs() < 1e12).then(|| (x * 100.0).round_ties_even() as i128)
        }
    }
}

/// **Private.** The JSON value for `n` in the mode's unit.
#[expect(
    clippy::cast_precision_loss,
    reason = "money far below 2^53 cents; the read side caps at 1e12"
)]
fn put(n: i128, mode: Mode) -> Option<Value> {
    match mode {
        Mode::Int => i64::try_from(n).ok().map(|x| json!(x)),
        Mode::Cents => Number::from_f64(n as f64 / 100.0).map(Value::Number),
    }
}

fn first(m: &Map<String, Value>, names: &[&str]) -> Option<String> {
    names.iter().find_map(|n| key_for(m, n))
}

fn is_line_item(e: &Map<String, Value>) -> bool {
    first(e, &PRICE).is_some() && (first(e, &QTY).is_some() || first(e, &AMOUNT).is_some())
}

/// Makes the line amounts, `subtotal`, `tax`, `shipping`, `discount` and `total` of `obj` add up.
/// See the module comment. Idempotent; an object without line items is untouched.
pub(super) fn sums(
    obj: &mut Map<String, Value>,
    props: &Map<String, Value>,
    seed: u64,
    opts: &Options,
) {
    let _ = run(obj, props, seed, opts);
}

/// **Private.** [`sums`] with `?` for the "skip the object" exits.
fn run(
    obj: &mut Map<String, Value>,
    props: &Map<String, Value>,
    seed: u64,
    opts: &Options,
) -> Option<()> {
    let list = line_array(obj)?;
    let item_schema = {
        let items = props.get(&list)?.get("items")?;
        if items.get("allOf").is_some() {
            merge_all_of(items).0
        } else {
            items.clone()
        }
    };
    let item_props = item_schema.get("properties").and_then(Value::as_object);
    let totals = total_keys(obj, props);
    let mode = mode_of(obj, props, item_props, &list, &totals)?;

    // lines, read only: where a line amount may be written, and what the subtotal adds up
    let mut line_writes: Vec<(usize, String, i128)> = Vec::new();
    let mut sub: i128 = 0;
    for (i, e) in obj.get(&list)?.as_array()?.iter().enumerate() {
        let e = e.as_object()?;
        let price = read(e.get(&first(e, &PRICE)?)?, mode)?;
        let quantity = match first(e, &QTY) {
            Some(k) => i128::from(e.get(&k)?.as_i64().filter(|q| *q >= 0)?),
            None => 1,
        };
        let line = price.checked_mul(quantity)?;
        let mut counted = line;
        if let Some(a) = first(e, &AMOUNT) {
            // a line the mode cannot represent skips the whole object
            let rendered = put(line, mode)?;
            let ps = item_props.and_then(|ps| ps.get(&a));
            let free = ps.is_none_or(|p| !is_pinned(p, &a, opts));
            if free && ps.is_none_or(|p| can_hold(p, &rendered)) {
                line_writes.push((i, a, line));
            } else {
                counted = read(e.get(&a)?, mode)?;
            }
        }
        sub = sub.checked_add(counted)?;
    }

    let parent = parent_writes(obj, props, (sub, mode), &totals, seed, opts)?;
    for (i, key, line) in line_writes {
        if let Some(Value::Object(e)) = obj.get_mut(&list)?.as_array_mut()?.get_mut(i) {
            e.insert(key, put(line, mode)?);
        }
    }
    for (key, n) in parent {
        obj.insert(key, put(n, mode)?);
    }
    Some(())
}

/// **Private.** The total fields that take part: `total` only next to another money field or
/// typed `number` (otherwise it is an item count), `orderTotal` and `grandTotal` always.
fn total_keys(obj: &Map<String, Value>, props: &Map<String, Value>) -> Vec<String> {
    let money_fields = [&["subtotal"][..], &TAX, &SHIPPING, &DISCOUNT];
    let money_sibling = money_fields.iter().any(|names| first(obj, names).is_some());
    TOTAL
        .iter()
        .filter_map(|n| key_for(obj, n).map(|k| (*n, k)))
        .filter(|(n, k)| {
            *n != "total"
                || money_sibling
                || props.get(k).is_some_and(|p| scalar_type(p) == "number")
        })
        .map(|(_, k)| k)
        .collect()
}

/// **Private.** Integer math when every money field is integer-typed, cents when none is, `None`
/// (skip the object) when they are mixed. A schema silent on a field falls back to the value.
fn mode_of(
    obj: &Map<String, Value>,
    props: &Map<String, Value>,
    item_props: Option<&Map<String, Value>>,
    list: &str,
    totals: &[String],
) -> Option<Mode> {
    let first_el = obj.get(list)?.as_array()?.first()?.as_object()?;
    let (price_k, amt_k) = (first(first_el, &PRICE)?, first(first_el, &AMOUNT));
    let kind = |ps: Option<&Value>, v: Option<&Value>| match (ps, v) {
        (Some(p), _) => scalar_type(p) == "integer",
        (None, Some(v)) => v.is_i64() || v.is_u64(),
        (None, None) => false,
    };
    let mut kinds = vec![kind(
        item_props.and_then(|p| p.get(&price_k)),
        first_el.get(&price_k),
    )];
    if let Some(a) = &amt_k {
        kinds.push(kind(item_props.and_then(|p| p.get(a)), first_el.get(a)));
    }
    let money_fields = [&["subtotal"][..], &TAX, &SHIPPING, &DISCOUNT];
    let others = money_fields
        .iter()
        .filter_map(|names| first(obj, names))
        .chain(totals.iter().cloned());
    kinds.extend(others.map(|k| kind(props.get(&k), obj.get(&k))));
    if kinds.iter().all(|k| *k) {
        Some(Mode::Int)
    } else if kinds.iter().any(|k| *k) {
        None
    } else {
        Some(Mode::Cents)
    }
}

/// **Private.** The parent fields once the lines add up to `sub`, as `(key, value)` writes:
/// `subtotal`, derived `tax` and `shipping`, a capped `discount`, and the totals. Only free
/// fields are written, and only a value the field can hold. Reads everything first and returns
/// `None` (skip the object) before anything is written if a field is not a number.
fn parent_writes(
    obj: &Map<String, Value>,
    props: &Map<String, Value>,
    (sub, mode): (i128, Mode),
    totals: &[String],
    seed: u64,
    opts: &Options,
) -> Option<Vec<(String, i128)>> {
    let free = |k: &str| props.get(k).is_none_or(|p| !is_pinned(p, k, opts));
    let own_rng =
        |k: &str, tag: &str| ChaCha8Rng::seed_from_u64(child_seed(child_seed(seed, k, 0), tag, 0));
    let current = |k: &Option<String>| -> Option<i128> {
        k.as_ref().map_or(Some(0), |k| read(obj.get(k)?, mode))
    };
    let mut writes: Vec<(String, i128)> = Vec::new();
    // a write is proposed only when the field can hold it; the caller then uses the new value
    let mut propose = |k: &str, n: i128| -> bool {
        let fits = put(n, mode).is_some_and(|v| props.get(k).is_none_or(|p| can_hold(p, &v)));
        if fits {
            writes.push((k.to_string(), n));
        }
        fits
    };

    if let Some(k) = key_for(obj, "subtotal").filter(|k| free(k)) {
        propose(&k, sub);
    }
    let (tax_k, ship_k, disc_k) = (
        first(obj, &TAX),
        first(obj, &SHIPPING),
        first(obj, &DISCOUNT),
    );
    let mut tax = current(&tax_k)?;
    if let Some(k) = tax_k.as_ref().filter(|k| free(k)) {
        let permille = own_rng(k, "tax").random_range(0..=MAX_TAX_PERMILLE);
        let want = sub.checked_mul(permille)? / 1000;
        if propose(k, want) {
            tax = want;
        }
    }
    let mut shipping = current(&ship_k)?;
    if mode == Mode::Cents
        && let Some(k) = ship_k.as_ref().filter(|k| free(k))
    {
        let want = SHIPPING_CENTS[own_rng(k, "shipping").random_range(0..SHIPPING_CENTS.len())];
        if propose(k, want) {
            shipping = want;
        }
    }
    // never more than the subtotal, so a total is never negative
    let cap = sub.max(0);
    let mut discount = current(&disc_k)?;
    if discount > cap {
        discount = cap;
        if let Some(k) = disc_k.as_ref().filter(|k| free(k)) {
            propose(k, cap);
        }
    }
    let total = sub
        .checked_add(tax)?
        .checked_add(shipping)?
        .checked_sub(discount)?;
    for k in totals.iter().filter(|k| free(k)) {
        propose(k, total);
    }
    Some(writes)
}

/// **Private.** The array of line items to sum, or `None` to skip the object: no array holds
/// them, an element was nulled by the body budget, or several arrays qualify and none is the
/// obvious list (paging's list names: `items`, `data`, `results`, `content`, `records`).
fn line_array(obj: &Map<String, Value>) -> Option<String> {
    let mut found: Vec<&String> = Vec::new();
    for (k, v) in obj {
        let Value::Array(a) = v else { continue };
        if a.is_empty() {
            continue;
        }
        if a.iter().any(Value::is_null) {
            return None;
        }
        if a.iter().all(|e| e.as_object().is_some_and(is_line_item)) {
            found.push(k);
        }
    }
    if let [one] = found.as_slice() {
        return Some((*one).clone());
    }
    let named: Vec<&&String> = found.iter().filter(|k| is_list_name(k)).collect();
    if let [one] = named.as_slice() {
        Some((**one).clone())
    } else {
        None
    }
}

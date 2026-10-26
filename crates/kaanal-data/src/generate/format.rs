//! `format` strings and dictionary values: picking an entry that fits the type and bounds.

use super::text::DEFAULT_DAYS;
use super::{
    ChaCha8Rng, Dictionary, Gen, Pack, Rng, Value, base64, date_time, email, fit_length, int_for,
    json, number_for, run, uuid, words,
};

/// **Private.** A value for a known string `format`; `None` for an unknown one,
/// which then falls through to the dictionary and lengths.
pub(super) fn format_value(
    s: &Value,
    rng: &mut ChaCha8Rng,
    group: u64,
    pack: &Pack,
) -> Option<String> {
    Some(match s.get("format").and_then(Value::as_str)? {
        "email" => email(rng, group, pack),
        "uuid" => uuid(rng),
        "date-time" => date_time(rng, DEFAULT_DAYS),
        "date" => date_time(rng, DEFAULT_DAYS)[..10].to_string(),
        "uri" | "url" => format!("https://example.com/{}", words::pick(words::WORDS, rng)),
        // RFC 5737 documentation ranges, never a real host
        "ipv4" => format!(
            "{}.{}",
            ["198.51.100", "203.0.113"][rng.random_range(0..2)],
            rng.random_range(1..=254)
        ),
        // RFC 3849 documentation prefix
        "ipv6" => format!(
            "2001:db8::{:x}:{:x}",
            rng.random_range(0..=0xffff),
            rng.random_range(1..=0xffff)
        ),
        "hostname" => format!("{}.example.com", words::pick(words::WORDS, rng)),
        "byte" => base64(&rng.random::<[u8; 12]>()),
        // formats beat lengths, but a password is plain text: fit it
        "password" => {
            const ALNUM: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
            let p = (0..16)
                .map(|_| char::from(ALNUM[rng.random_range(0..ALNUM.len())]))
                .collect();
            fit_length(p, s)
        }
        "duration" => format!(
            "P{}DT{}H{}M",
            rng.random_range(0..=30),
            rng.random_range(0..24),
            rng.random_range(0..60)
        ),
        _ => return None,
    })
}

/// **Private.** The first generator of `field`'s entry in `d` that fits `ty` and the
/// schema's bounds, run; `None` when there is no entry or none fits (design 11a rule 2).
/// A `builtin` entry must also meet the string lengths (its values are never cut or
/// padded); a project one is sent as written and warned at startup instead.
#[expect(
    clippy::too_many_arguments,
    reason = "private helper, one call site per layer"
)]
pub(super) fn from_dictionary<'d>(
    d: Option<&'d Dictionary>,
    builtin: bool,
    s: &Value,
    field: Option<&str>,
    ty: &str,
    rng: &mut ChaCha8Rng,
    group: u64,
    pack: &Pack,
) -> Option<(Value, &'d Gen)> {
    let f = field?;
    let g = d?
        .lookup(f)?
        .iter()
        .find(|g| g.fits(s, ty) && (!builtin || g.length_fits(s, ty)))?;
    Some((run(g, s, f, ty, rng, group, pack), g))
}

/// **Private.** `(min, max)` from optional entry bounds and a default range: a lone bound
/// keeps the default's other end when it can, else extends by `step` (keeps `lo <= hi`).
pub(super) fn entry_range<T: PartialOrd + Copy>(
    min: Option<T>,
    max: Option<T>,
    (dlo, dhi): (T, T),
    up: impl Fn(T) -> T,
    down: impl Fn(T) -> T,
) -> (T, T) {
    match (min, max) {
        (Some(a), Some(b)) => (a, b),
        (Some(a), None) => (a, if a <= dhi { dhi } else { up(a) }),
        (None, Some(b)) => (if dlo <= b { dlo } else { down(b) }, b),
        (None, None) => (dlo, dhi),
    }
}

/// **Private.** A float range into an `integer` field (rounded, so `lo <= hi` holds) or a
/// `number` field (`places` decimals); either way clamped to the schema's bounds.
#[expect(
    clippy::cast_possible_truncation,
    reason = "f64 -> i64 `as` saturates, which is the wanted clamp"
)]
pub(super) fn numeric(
    s: &Value,
    ty: &str,
    rng: &mut ChaCha8Rng,
    (lo, hi): (f64, f64),
    places: i32,
) -> Value {
    if ty == "integer" {
        json!(int_for(s, rng, (lo.round() as i64, hi.round() as i64)))
    } else {
        number_for(s, rng, (lo, hi), places)
    }
}

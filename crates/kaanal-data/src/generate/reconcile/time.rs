//! Time chains (D2 spec section 4.1): `createdAt` before `updatedAt` before `deletedAt`, and so on.
//!
//! The first free member of a chain keeps its generated value. Each later member is the previous
//! one plus a fraction of the time left before `BASE_EPOCH`, the fraction drawn from that member's
//! own seed (integer permille, skewed small so updates cluster near creation). It cannot
//! overflow, stays before `BASE_EPOCH`, and adding a later member never moves an earlier one.

use std::collections::HashSet;

use rand::{RngExt, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde_json::{Map, Value};

use super::{can_hold, is_pinned, key_for};
use crate::dataset::Options;
use crate::generate::text::{civil_from_days, days_from_civil};
use crate::generate::{BASE_EPOCH, child_seed};

/// **Private.** Built-in chains, earliest first; each slot lists the names that can fill it.
const BUILTIN: &[&[&[&str]]] = &[
    &[&["createdat"], &["updatedat", "modifiedat"], &["deletedat"]],
    &[
        &["startat", "startdate", "startsat"],
        &["endat", "enddate", "endsat"],
    ],
    &[
        &["orderedat"],
        &["paidat"],
        &["shippedat"],
        &["deliveredat"],
    ],
    &[&["issuedat", "issuedate"], &["dueat", "duedate"]],
];

/// **Private.** A chain member found in the object.
struct Member {
    key: String,
    epoch: i64,
    date_only: bool,
    pinned: bool,
}

/// Whether the normalised field name `n` is a member of any chain (built-in or the project's).
pub(super) fn is_chain_name(n: &str, opts: &Options) -> bool {
    BUILTIN
        .iter()
        .flat_map(|c| c.iter())
        .flat_map(|slot| slot.iter())
        .any(|name| *name == n)
        || opts
            .dictionary_project
            .as_deref()
            .is_some_and(|d| d.chains().iter().flatten().any(|name| name == n))
}

/// Applies every chain to `obj`. Built-in chains the project turned off are skipped; project
/// chains are added. See the module comment for the rule.
pub(super) fn chains(
    obj: &mut Map<String, Value>,
    props: &Map<String, Value>,
    seed: u64,
    opts: &Options,
) {
    let project = opts.dictionary_project.as_deref();
    let off = project.map_or(&[][..], |d| d.chains_off());
    // fields an earlier chain already placed: a later (project) chain treats them as fixed
    let mut handled: HashSet<String> = HashSet::new();
    for chain in BUILTIN {
        let named = |n: &&str| off.iter().any(|o| o == n);
        if !chain.iter().flat_map(|s| s.iter()).any(named) {
            apply(obj, props, seed, opts, chain, &mut handled);
        }
    }
    for chain in project.map_or(&[][..], |d| d.chains()) {
        let one: Vec<[&str; 1]> = chain.iter().map(|n| [n.as_str()]).collect();
        let slots: Vec<&[&str]> = one.iter().map(<[&str; 1]>::as_slice).collect();
        apply(obj, props, seed, opts, &slots, &mut handled);
    }
}

/// **Private.** One chain: `slots[i]` are the names that fill position `i`.
fn apply(
    obj: &mut Map<String, Value>,
    props: &Map<String, Value>,
    seed: u64,
    opts: &Options,
    slots: &[&[&str]],
    handled: &mut HashSet<String>,
) {
    // most objects have no chain member: find them without allocating
    let found: Vec<String> = slots
        .iter()
        .filter_map(|slot| slot.iter().find_map(|n| key_for(obj, n)))
        .collect();
    if found.len() < 2 {
        return;
    }
    let mut members: Vec<Member> = Vec::new();
    for key in found {
        let Some((epoch, date_only)) = obj.get(&key).and_then(Value::as_str).and_then(parse) else {
            return;
        };
        let pinned =
            handled.contains(&key) || props.get(&key).is_some_and(|ps| is_pinned(ps, &key, opts));
        members.push(Member {
            key,
            epoch,
            date_only,
            pinned,
        });
    }
    // members of different shapes (a date and a date-time) are not comparable: leave them
    if members.iter().any(|m| m.date_only != members[0].date_only) {
        return;
    }
    let mut prev: Option<i64> = None;
    for m in &members {
        let mut epoch = m.epoch;
        if let Some(p) = prev
            && !m.pinned
            && p < BASE_EPOCH
        {
            let mut rng =
                ChaCha8Rng::seed_from_u64(child_seed(child_seed(seed, &m.key, 0), "chain", 0));
            let r = rng.random_range(0..1000i64);
            // quadratic skew toward small fractions, integer only
            let permille = r * r / 1000;
            let t = p + (BASE_EPOCH - p) * permille / 1000;
            let text = Value::String(format(t, m.date_only));
            // a value the field cannot hold (a pattern, a length) leaves it as generated
            if props.get(&m.key).is_none_or(|ps| can_hold(ps, &text)) {
                obj.insert(m.key.clone(), text);
                handled.insert(m.key.clone());
                epoch = t;
            }
        }
        prev = Some(epoch);
    }
}

/// **Private.** `2025-03-04T05:06:07Z` or `2025-03-04` -> (seconds since the epoch, date only?).
pub(super) fn parse(s: &str) -> Option<(i64, bool)> {
    let b = s.as_bytes();
    let num = |a: usize, z: usize| s.get(a..z)?.parse::<i64>().ok();
    if b.len() < 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let (month, day) = (num(5, 7)?, num(8, 10)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let days = days_from_civil(
        num(0, 4)?,
        u32::try_from(month).ok()?,
        u32::try_from(day).ok()?,
    );
    if b.len() == 10 {
        return Some((days * 86_400, true));
    }
    if b.len() != 20 || b[10] != b'T' || b[13] != b':' || b[16] != b':' || b[19] != b'Z' {
        return None;
    }
    let (hour, minute, second) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    Some((days * 86_400 + hour * 3600 + minute * 60 + second, false))
}

/// **Private.** The inverse of [`parse`].
pub(super) fn format(t: i64, date_only: bool) -> String {
    let (days, secs) = (t.div_euclid(86_400), t.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(days);
    if date_only {
        return format!("{y:04}-{m:02}-{d:02}");
    }
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

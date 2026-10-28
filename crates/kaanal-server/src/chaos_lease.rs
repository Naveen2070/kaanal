//! Chaos leases: live chaos set with a `ttl` (spec L1-L6).
//!
//! A lease belongs to one key and one **unit** of that key's live entry. A `PUT` with `ttl`
//! leases every unit it sets; a `PUT` without `ttl` makes every unit it sets permanent again;
//! units it does not mention keep their lease. [`prune`] runs inside `commit`, so a lease never
//! outlives its field, whatever write removed it. Pure functions; the handlers in `control`
//! hold the lock.

use std::collections::HashMap;
use std::time::Duration;

use kaanal_core::{ChaosKey, ChaosLayer, ChaosState};
use serde_json::{Map, Value, json};
use tokio::time::Instant;

use crate::control::remaining_ms;

/// The part of a live entry a lease covers; it expires as a whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Unit {
    /// `latency_ms`.
    Latency,
    /// `error_rate`, `error_kind` and `error_status` together (one fault, S7).
    Error,
    /// `down`.
    Down,
    /// `jitter_ms` (link only): its own lease, so it never clobbers a `latency_ms` lease.
    Jitter,
    /// `bandwidth_cap` (link only).
    Bandwidth,
    /// The `mode` switch; its expiry goes back to merge and touches no other unit.
    Mode,
}

impl Unit {
    /// `latency`, `error`, `down` or `mode`: the key in `expires_in_ms`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Unit::Latency => "latency",
            Unit::Error => "error",
            Unit::Down => "down",
            Unit::Jitter => "jitter",
            Unit::Bandwidth => "bandwidth",
            Unit::Mode => "mode",
        }
    }
}

/// Every chaos lease: (key, unit) to its deadline.
pub type Leases = HashMap<(ChaosKey, Unit), Instant>;

/// The units a sparse layer sets (a patch, or a stored live entry), in [`Unit`] order.
#[must_use]
pub fn units_set(p: &ChaosLayer) -> Vec<Unit> {
    [
        (Unit::Latency, p.latency_add.is_some()),
        (Unit::Jitter, p.jitter.is_some()),
        (
            Unit::Error,
            p.error_rate.is_some() || p.error_kind.is_some() || p.error_status.is_some(),
        ),
        (Unit::Down, p.down.is_some()),
        (Unit::Bandwidth, p.bandwidth_cap.is_some()),
        (Unit::Mode, p.replace.is_some()),
    ]
    .into_iter()
    .filter_map(|(u, set)| set.then_some(u))
    .collect()
}

/// With `Some(ttl)` leases each of `units` on `key` until `now + ttl` (replacing an older
/// deadline); with `None` removes their leases, so the fields are permanent.
pub fn apply(
    leases: &mut Leases,
    key: &ChaosKey,
    units: &[Unit],
    ttl: Option<Duration>,
    now: Instant,
) {
    for &u in units {
        let k = (key.clone(), u);
        match ttl {
            Some(t) => {
                leases.insert(k, now + t);
            }
            None => {
                leases.remove(&k);
            }
        }
    }
}

/// Drops every lease whose key has no live entry or whose unit that entry no longer sets.
pub fn prune(leases: &mut Leases, live: &ChaosState) {
    leases.retain(|(k, u), _| live.layer(k).is_some_and(|l| units_set(l).contains(u)));
}

/// How many stored fields the leases cover: one each for `latency` and `down`, each of
/// `error_rate`, `error_kind` and `error_status` an `error` lease's entry holds, none for `mode`
/// (a scenario file holds no mode). For the `save` warning.
#[must_use]
pub fn leased_fields(leases: &Leases, live: &ChaosState) -> usize {
    leases
        .keys()
        .filter_map(|(k, u)| {
            let l = live.layer(k)?;
            Some(match u {
                Unit::Latency => usize::from(l.latency_add.is_some()),
                Unit::Down | Unit::Jitter | Unit::Bandwidth => 1,
                Unit::Error => [
                    l.error_rate.is_some(),
                    l.error_kind.is_some(),
                    l.error_status.is_some(),
                ]
                .into_iter()
                .filter(|set| *set)
                .count(),
                Unit::Mode => 0,
            })
        })
        .sum()
}

/// Removes one unit's fields from a layer (what an expiry does). `Mode` unsets `replace`,
/// which is merge.
pub fn clear_units(layer: &mut ChaosLayer, unit: Unit) {
    match unit {
        Unit::Latency => layer.latency_add = None,
        Unit::Jitter => layer.jitter = None,
        Unit::Bandwidth => layer.bandwidth_cap = None,
        Unit::Error => {
            layer.error_rate = None;
            layer.error_kind = None;
            layer.error_status = None;
        }
        Unit::Down => layer.down = None,
        Unit::Mode => layer.replace = None,
    }
}

/// `{unit: ms}` for the leases of `key`, rounded up; empty when it has none.
#[must_use]
pub fn remaining(leases: &Leases, key: &ChaosKey, now: Instant) -> Map<String, Value> {
    leases
        .iter()
        .filter(|((k, _), _)| k == key)
        .map(|((_, u), d)| (u.as_str().to_string(), json!(remaining_ms(*d, now))))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kaanal_core::ErrorKind;

    fn layer(f: impl FnOnce(&mut ChaosLayer)) -> ChaosLayer {
        let mut l = ChaosLayer::default();
        f(&mut l);
        l
    }

    #[test]
    fn units_set_table() {
        use Unit::{Down, Error, Latency, Mode};
        let ms = Some(Duration::from_millis(5));
        let cases: [(ChaosLayer, &[Unit]); 8] = [
            (ChaosLayer::default(), &[]),
            (layer(|l| l.latency_add = ms), &[Latency]),
            (layer(|l| l.error_rate = Some(0.5)), &[Error]),
            (layer(|l| l.error_kind = Some(ErrorKind::Reset)), &[Error]),
            (layer(|l| l.error_status = Some(502)), &[Error]),
            (layer(|l| l.down = Some(true)), &[Down]),
            (layer(|l| l.replace = Some(false)), &[Mode]),
            (
                layer(|l| {
                    l.latency_add = ms;
                    l.error_rate = Some(0.1);
                    l.down = Some(true);
                    l.replace = Some(true);
                }),
                &[Latency, Error, Down, Mode],
            ),
        ];
        for (l, want) in cases {
            assert_eq!(units_set(&l), want, "{l:?}");
        }
    }

    #[test]
    fn apply_some_then_none_removes_only_the_named_units() {
        let now = Instant::now();
        let k = ChaosKey::Node(0);
        let other = ChaosKey::Node(1);
        let mut ls = Leases::new();
        apply(
            &mut ls,
            &k,
            &[Unit::Latency, Unit::Down],
            Some(Duration::from_secs(5)),
            now,
        );
        apply(
            &mut ls,
            &other,
            &[Unit::Down],
            Some(Duration::from_secs(9)),
            now,
        );
        assert_eq!(ls[&(k.clone(), Unit::Down)], now + Duration::from_secs(5));
        // A new ttl replaces the deadline.
        apply(
            &mut ls,
            &k,
            &[Unit::Down],
            Some(Duration::from_secs(7)),
            now,
        );
        assert_eq!(ls[&(k.clone(), Unit::Down)], now + Duration::from_secs(7));
        apply(&mut ls, &k, &[Unit::Down], None, now);
        let mut left: Vec<_> = ls.keys().cloned().collect();
        left.sort_by_key(|(k, u)| (format!("{k:?}"), *u));
        assert_eq!(left, [(k, Unit::Latency), (other, Unit::Down)]);
    }

    #[test]
    fn prune_drops_a_lease_when_the_entry_or_the_field_is_gone() {
        let now = Instant::now();
        let (a, b) = (ChaosKey::Node(0), ChaosKey::Node(1));
        let mut live = ChaosState::default();
        live.set(
            a.clone(),
            layer(|l| {
                l.latency_add = Some(Duration::from_millis(1));
                l.down = Some(true);
            }),
        );
        live.set(b.clone(), layer(|l| l.down = Some(true)));
        let mut ls = Leases::new();
        let ttl = Some(Duration::from_secs(1));
        apply(&mut ls, &a, &[Unit::Latency, Unit::Down], ttl, now);
        apply(&mut ls, &b, &[Unit::Down], ttl, now);
        prune(&mut ls, &live);
        assert_eq!(ls.len(), 3);
        live.set(
            a.clone(),
            layer(|l| l.latency_add = Some(Duration::from_millis(1))),
        );
        live.remove(&b);
        prune(&mut ls, &live);
        assert_eq!(ls.keys().cloned().collect::<Vec<_>>(), [(a, Unit::Latency)]);
    }

    #[test]
    fn jitter_has_its_own_lease_apart_from_latency() {
        let l = layer(|l| {
            l.latency_add = Some(Duration::from_millis(1));
            l.jitter = Some(Duration::from_millis(1));
        });
        assert_eq!(units_set(&l), [Unit::Latency, Unit::Jitter]);
        let mut l = l;
        clear_units(&mut l, Unit::Jitter);
        assert!(l.jitter.is_none() && l.latency_add.is_some());
    }

    #[test]
    fn clear_units_per_unit() {
        let full = layer(|l| {
            l.latency_add = Some(Duration::from_millis(1));
            l.error_rate = Some(0.5);
            l.error_kind = Some(ErrorKind::Hang);
            l.error_status = Some(504);
            l.down = Some(true);
            l.replace = Some(true);
        });
        for u in [Unit::Latency, Unit::Error, Unit::Down, Unit::Mode] {
            let mut l = full.clone();
            clear_units(&mut l, u);
            let left = units_set(&l);
            assert!(!left.contains(&u), "{u:?}");
            assert_eq!(left.len(), 3, "{u:?} cleared more than its unit");
        }
    }

    #[test]
    fn remaining_rounds_up_and_lists_only_the_key() {
        let now = Instant::now();
        let k = ChaosKey::Endpoint("GET /x".into());
        let mut ls = Leases::new();
        ls.insert((k.clone(), Unit::Error), now + Duration::from_micros(1_500));
        ls.insert((k.clone(), Unit::Mode), now + Duration::from_secs(2));
        ls.insert(
            (ChaosKey::Node(0), Unit::Down),
            now + Duration::from_secs(9),
        );
        let m = remaining(&ls, &k, now);
        assert_eq!(Value::Object(m), json!({ "error": 2, "mode": 2000 }));
        assert!(remaining(&ls, &ChaosKey::Node(3), now).is_empty());
    }
}

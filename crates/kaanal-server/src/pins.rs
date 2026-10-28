//! UI-state pins on the server side: the one decision of what an `empty` pin answers
//! ([`empty_form`]) and the two adapters that let `resolve_scenario` ask it ([`empty_ok`] at
//! load, [`Generation::empty_ok_for`] at runtime), so load time and request time cannot disagree.

use std::collections::BTreeMap;

use kaanal_core::{Endpoint, stable_hash};
use kaanal_data::Options;
use serde_json::Value;

use crate::generation::Generation;

/// The seed of a route's generated data (`seed:` keys the data, not the chain draws): the same
/// hash `generated_body` uses.
pub fn route_seed(seed: u64, owner: &str, path: &str) -> u64 {
    stable_hash(&[&seed.to_le_bytes(), owner.as_bytes(), path.as_bytes()])
}

/// The body an `empty` pin answers with, or `None` when the endpoint has no empty form.
/// In order: `states.empty.body`; none for a built-in responder without `override:`; the emptied
/// `override:` body; the emptied response schema, its scalars generated with `opts` (the
/// owning node's locale and dictionaries).
pub fn empty_form(
    ep: &Endpoint,
    schema: Option<&Value>,
    seed: u64,
    opts: &Options,
) -> Option<Value> {
    if let Some(b) = ep.states.empty.as_ref().and_then(|o| o.body.as_ref()) {
        return Some(b.clone());
    }
    match &ep.response {
        Some(ov) => kaanal_data::empty_of_value(&ov.body),
        None if ep.responder.is_some() => None,
        None => schema.and_then(|s| kaanal_data::empty(s, seed, opts)),
    }
}

/// **Private.** A route that normally answers 204 or 304 has no body, so it always has an empty
/// form ("no body", status unchanged); otherwise [`empty_form`] decides. Whether a form exists
/// does not depend on the locale, so the default options do.
fn has_form(ep: &Endpoint, normal_status: u16, schema: Option<&Value>, seed: u64) -> bool {
    ((ep.responder.is_none() || ep.response.is_some()) && matches!(normal_status, 204 | 304))
        || empty_form(ep, schema, seed, &Options::default()).is_some()
}

/// The load-time `empty_ok`: reads the spec table (`key` to status and schema) because the
/// routes do not exist yet. A `HEAD` with no entry of its own reads its `GET`'s, as the route
/// loop does.
pub fn empty_ok(
    info: &BTreeMap<String, (u16, Option<Value>)>,
    seed: u64,
) -> impl Fn(&Endpoint) -> bool + '_ {
    move |ep| {
        let spec = info.get(&ep.key).or_else(|| {
            let path = ep.key.strip_prefix("HEAD ")?;
            info.get(&format!("GET {path}"))
        });
        let status = ep
            .response
            .as_ref()
            .map_or(spec.map_or(200, |s| s.0), |o| o.status);
        has_form(ep, status, spec.and_then(|s| s.1.as_ref()), seed)
    }
}

impl Generation {
    /// The runtime `empty_ok`, from the built route (control API: `PUT /scenarios`, pins, cycle).
    #[must_use]
    pub fn empty_ok_for(&self, ep: &Endpoint) -> bool {
        let (method, path) = ep.key.split_once(' ').unwrap_or((ep.key.as_str(), ""));
        let Ok(m) = self.paths.at(path) else {
            return false;
        };
        let Some(route) = m.value.iter().find(|r| r.method == method) else {
            return false;
        };
        let owner = ep.owner.map_or("", |id| self.topo.nodes[id].name.as_str());
        let status = ep.response.as_ref().map_or(route.status, |o| o.status);
        has_form(
            ep,
            status,
            route.schema.as_ref(),
            route_seed(self.seed, owner, path),
        )
    }
}

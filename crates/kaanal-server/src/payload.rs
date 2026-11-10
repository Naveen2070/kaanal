//! Live response edits (step 8A, RULES 108): a table of `Override`s by route key, copy on write so
//! a request reads one snapshot. [`crate::mock`] applies an edit below chaos and pins and above a
//! file's `override:`; a collection route and a built-in responder refuse one.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::body::Bytes;
use kaanal_core::Endpoint;
use kaanal_core::config::Override;
use kaanal_core::scenario::endpoint_key;

use crate::generation::Generation;
use crate::state::AppState;

/// One live edit: the checked [`Override`], its body already serialized (a request clones a
/// refcount, not a JSON tree, and a size is a `len()`), and an optional one-shot flag.
#[derive(Debug)]
pub struct Edit {
    /// What the route answers.
    pub ov: Override,
    /// The body as sent: JSON text, or the text itself when `ov.content_type` is set. Empty for a
    /// `null` body.
    pub body: Bytes,
    /// Answers one request, then is gone.
    pub once: bool,
    spent: AtomicBool,
}

impl Edit {
    /// An edit from a checked override.
    #[must_use]
    pub fn new(ov: Override, once: bool) -> Self {
        let body = match (&ov.content_type, &ov.body) {
            (_, serde_json::Value::Null) => Bytes::new(),
            // text or base64 bytes, checked before an edit is stored
            (Some(_), serde_json::Value::String(_)) => {
                Bytes::from(ov.raw_bytes().unwrap_or_default())
            }
            (_, v) => Bytes::from(serde_json::to_vec(v).unwrap_or_default()),
        };
        Self {
            ov,
            body,
            once,
            spent: AtomicBool::new(false),
        }
    }

    /// Takes the edit for one request: always for a lasting edit, for a one-shot only the first
    /// caller (atomic, so two requests at once never both get it).
    #[must_use]
    pub fn take(&self) -> bool {
        !self.once || !self.spent.swap(true, Ordering::AcqRel)
    }

    /// Whether a one-shot edit has been used.
    #[must_use]
    pub fn spent(&self) -> bool {
        self.spent.load(Ordering::Acquire)
    }
}

/// The live edits, by normalized route key (`GET /users/{id}`).
#[derive(Debug, Default, Clone)]
pub struct Payloads(BTreeMap<String, Arc<Edit>>);

impl Payloads {
    /// The edit for `ep`; a synthesized `HEAD` follows its `GET`, as pins do.
    #[must_use]
    pub fn get_for(&self, ep: &Endpoint) -> Option<&Arc<Edit>> {
        if self.0.is_empty() {
            return None;
        }
        let key: Cow<str> = if ep.synthesized {
            ep.key.replacen("HEAD ", "GET ", 1).into()
        } else {
            Cow::Borrowed(&ep.key)
        };
        self.0.get(key.as_ref())
    }

    /// The edits in route order; a spent one-shot is gone.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &Arc<Edit>)> {
        self.0.iter().filter(|(_, e)| !e.spent())
    }

    /// Whether there is none.
    #[cfg(test)]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Why an edit was refused; each holds the message to show.
#[derive(Debug, PartialEq, Eq)]
pub enum PayloadError {
    /// No such route.
    NoRoute(String),
    /// A `HEAD` made from its `GET`.
    Synthesized(String),
    /// A `collection:` route: an edit would freeze live data.
    Collection(String),
    /// A built-in responder (login, JWKS) with no `override:`.
    BuiltIn(String),
    /// The edit itself is not valid ([`Override::check`]) or the key is malformed.
    Invalid(String),
}

/// The normalized key of `raw` when the route takes an edit in `g`.
///
/// # Errors
/// [`PayloadError`]: a malformed key, an unknown route, a `HEAD` made from its `GET`, a collection
/// route or a built-in responder with no `override:`.
pub fn check_route(g: &Generation, raw: &str) -> Result<String, PayloadError> {
    let key = endpoint_key(raw).map_err(PayloadError::Invalid)?;
    let ep = g
        .endpoints
        .get(&key)
        .ok_or_else(|| PayloadError::NoRoute(format!("unknown endpoint '{key}'")))?;
    if ep.synthesized {
        return Err(PayloadError::Synthesized(format!(
            "'{key}' is a HEAD made from its GET; edit the GET route"
        )));
    }
    if ep.collection.is_some() {
        return Err(PayloadError::Collection(format!(
            "'{key}' answers from a collection, which an edit would freeze"
        )));
    }
    if ep.responder.is_some() && ep.response.is_none() {
        return Err(PayloadError::BuiltIn(format!(
            "'{key}' is a built-in responder (login or JWKS)"
        )));
    }
    Ok(key)
}

impl AppState {
    /// One snapshot of the edits.
    #[must_use]
    pub fn payloads(&self) -> Arc<Payloads> {
        Arc::clone(&self.payloads.read().expect("payload lock poisoned"))
    }

    fn swap_payloads(&self, f: impl FnOnce(&mut Payloads)) {
        let mut w = self.payloads.write().expect("payload lock poisoned");
        let mut next = (**w).clone();
        f(&mut next);
        *w = Arc::new(next);
    }

    /// Sets the edit of a route and returns its normalized key.
    ///
    /// # Errors
    /// [`PayloadError`], with nothing stored.
    #[cfg(test)]
    pub fn set_payload(
        &self,
        g: &Generation,
        raw: &str,
        ov: Override,
    ) -> Result<String, PayloadError> {
        self.set_payload_with(g, raw, ov, false)
    }

    /// [`Self::set_payload`], and `once` makes the edit answer one request only.
    ///
    /// # Errors
    /// [`PayloadError`], with nothing stored.
    pub fn set_payload_with(
        &self,
        g: &Generation,
        raw: &str,
        ov: Override,
        once: bool,
    ) -> Result<String, PayloadError> {
        let key = check_route(g, raw)?;
        ov.check().map_err(PayloadError::Invalid)?;
        self.swap_payloads(|p| {
            p.0.insert(key.clone(), Arc::new(Edit::new(ov, once)));
        });
        Ok(key)
    }

    /// Removes the edit of a route; whether there was one.
    pub fn clear_payload(&self, raw: &str) -> bool {
        let Ok(key) = endpoint_key(raw) else {
            return false;
        };
        if !self.payloads().0.contains_key(&key) {
            return false;
        }
        self.swap_payloads(|p| {
            p.0.remove(&key);
        });
        true
    }

    /// Removes every edit (`POST /reset`).
    pub fn clear_payloads(&self) {
        self.swap_payloads(|p| p.0.clear());
    }

    /// Drops the edits whose route is gone or no longer takes one in `g`; the keys dropped.
    pub fn prune_payloads(&self, g: &Generation) -> Vec<String> {
        let mut dropped = Vec::new();
        self.swap_payloads(|p| {
            p.0.retain(|k, _| {
                let ok = g.endpoints.get(k).is_some_and(|ep| {
                    !ep.synthesized
                        && ep.collection.is_none()
                        && !(ep.responder.is_some() && ep.response.is_none())
                });
                if !ok {
                    dropped.push(k.clone());
                }
                ok
            });
        });
        dropped
    }
}

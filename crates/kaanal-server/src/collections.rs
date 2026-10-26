//! `collection:` routes: small in-memory lists of JSON objects (F, spec section 5).
//!
//! One store per running server, shared by every scope and every user (C14), emptied by
//! `POST /__kaanal/reset` and on restart. A route still runs its chain; the operation is applied
//! only when the request succeeds (see [`Call::apply`] and [`Call::preview`] in `mock.rs`).

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use axum::extract::Request;
use kaanal_core::{CollectionOp, collection_op};
use serde_json::{Value, json};

use crate::state::Route;

/// The most items one collection holds; one more add is a 409.
pub const MAX_ITEMS: usize = 1000;
/// The largest item, as serialized JSON; a bigger one is a 413.
pub const MAX_ITEM_BYTES: usize = 64 * 1024;

/// **Private.** One collection: its items in insertion order and the last id handed out.
#[derive(Default)]
struct List {
    last: u64,
    items: Vec<Value>,
}

/// Every collection of a running server, by name.
// ponytail: one lock for every collection; per-collection locks if a load test ever shows contention.
// A list clones the whole collection under the lock (up to about 64 MB at 1000 x 64 KiB); store
// `Arc<Value>` items, or serialize under the lock, if that memory or lock time ever matters.
#[derive(Default)]
pub struct Collections {
    lists: Mutex<HashMap<String, List>>,
}

impl Collections {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs `op` on the collection `name` and returns the status and body to send.
    ///
    /// - `List`: 200 and every item, oldest first.
    /// - `Get`: 200 and the item `id`, else 404 `{"error":"not found"}`.
    /// - `Add`: 201 and `item` with its new `id` (a client-sent `id` is overwritten); 400 when
    ///   `item` is missing or not an object, 413 over [`MAX_ITEM_BYTES`], 409 at [`MAX_ITEMS`].
    /// - `Remove`: 204 and `null`, else 404.
    ///
    /// Ids count from 1 per collection and are never handed out twice (until [`Collections::clear`]).
    pub fn apply(
        &self,
        name: &str,
        op: CollectionOp,
        id: Option<&str>,
        item: Option<Value>,
    ) -> (u16, Value) {
        self.run(name, op, id, item, true)
    }

    /// Empties every collection and restarts every id at 1.
    pub fn clear(&self) {
        self.lock().clear();
    }

    /// **Private.** [`Collections::apply`], or with `commit` false what it would answer, changing nothing.
    fn run(
        &self,
        name: &str,
        op: CollectionOp,
        id: Option<&str>,
        item: Option<Value>,
        commit: bool,
    ) -> (u16, Value) {
        let not_found = || (404, json!({ "error": "not found" }));
        // Ids are plain integers in canonical form; anything else (huge, signed, `007`, `+7`)
        // names no item, so one item has one URL.
        let want = id.and_then(|s| s.parse::<u64>().ok().filter(|n| n.to_string() == s));
        // An add is checked before the lock: its serialization is the only costly step.
        let item = match (op, item) {
            (CollectionOp::Add, Some(Value::Object(obj))) => {
                // Measured as sent, before the id: the limit is on what the client controls.
                let size = serde_json::to_vec(&obj).map_or(usize::MAX, |b| b.len());
                if size > MAX_ITEM_BYTES {
                    return (413, json!({ "error": "item too large" }));
                }
                Some(obj)
            }
            (CollectionOp::Add, _) => {
                return (400, json!({ "error": "item must be a JSON object" }));
            }
            _ => None,
        };
        let mut lists = self.lock();
        let at =
            |l: &List| want.and_then(|w| l.items.iter().position(|v| v["id"].as_u64() == Some(w)));
        match op {
            CollectionOp::List => {
                let items = lists.get(name).map(|l| l.items.clone()).unwrap_or_default();
                (200, Value::Array(items))
            }
            CollectionOp::Get => match lists.get(name).and_then(|l| at(l).map(|i| &l.items[i])) {
                Some(v) => (200, v.clone()),
                None => not_found(),
            },
            CollectionOp::Remove => {
                let Some((l, i)) = lists.get_mut(name).and_then(|l| at(l).map(|i| (l, i))) else {
                    return not_found();
                };
                if commit {
                    l.items.remove(i);
                }
                (204, Value::Null)
            }
            CollectionOp::Add => {
                let mut obj = item.expect("checked before the lock");
                let (len, last) = lists.get(name).map_or((0, 0), |l| (l.items.len(), l.last));
                if len >= MAX_ITEMS {
                    return (409, json!({ "error": "collection full" }));
                }
                obj.insert("id".into(), json!(last + 1));
                let v = Value::Object(obj);
                if commit {
                    let l = lists.entry(name.to_string()).or_default();
                    l.last = last + 1;
                    l.items.push(v.clone());
                }
                (201, v)
            }
        }
    }

    /// **Private.** The lock; never held across an `.await`. A poisoned lock is used as is (plain data).
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, List>> {
        self.lists.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The collection operation of a route, `None` for a route without `collection:`. A synthesized
/// `HEAD` answers like its `GET` (a read; it never writes).
pub(crate) fn op_of(route: &Route) -> Option<CollectionOp> {
    route.endpoint.collection.as_ref()?;
    let (_, path) = route.key.split_once(' ')?;
    let method = if route.method == "HEAD" {
        "GET"
    } else {
        route.method.as_str()
    };
    collection_op(method, path)
}

/// What an add sent: the item, or the status and body of its rejection, answered once the chain
/// succeeded (as a login body is). The body is read with a hard cap, whatever its `Content-Length`.
pub(crate) type Item = Result<Value, (u16, Value)>;

/// Reads and parses an add's body: over [`MAX_ITEM_BYTES`] is a 413, anything that is not JSON
/// (invalid UTF-8 and nesting past serde's recursion limit included) a 400. The shape is checked
/// by [`Collections::apply`].
pub(crate) async fn read_item(req: Request) -> Item {
    let too_large = || (413, json!({ "error": "item too large" }));
    // ponytail: any body read error counts as too large, as for a login body.
    let body = axum::body::to_bytes(req.into_body(), MAX_ITEM_BYTES + 1)
        .await
        .map_err(|_| too_large())?;
    if body.len() > MAX_ITEM_BYTES {
        return Err(too_large());
    }
    serde_json::from_slice(&body)
        .map_err(|_| (400, json!({ "error": "item must be a JSON object" })))
}

/// One collection operation of a request, built before its chain runs. Taken by value when
/// applied, so a request can change its collection at most once.
pub(crate) struct Call<'a> {
    pub(crate) name: &'a str,
    pub(crate) op: CollectionOp,
    pub(crate) id: Option<&'a str>,
    /// `Some` for an add only.
    pub(crate) item: Option<Item>,
}

impl Call<'_> {
    /// Applies the operation: the request succeeded.
    pub(crate) fn apply(self, c: &Collections) -> (u16, Value) {
        match self.item {
            Some(Err(rejected)) => rejected,
            item => c.run(self.name, self.op, self.id, item.and_then(Result::ok), true),
        }
    }

    /// What [`Call::apply`] would answer, changing nothing (the body a wire failure breaks).
    pub(crate) fn preview(&self, c: &Collections) -> (u16, Value) {
        match &self.item {
            Some(Err(rejected)) => rejected.clone(),
            item => {
                let item = item.as_ref().and_then(|r| r.as_ref().ok()).cloned();
                c.run(self.name, self.op, self.id, item, false)
            }
        }
    }
}

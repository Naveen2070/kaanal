//! A sample response body for an endpoint, made by the same generator and seeds the mock uses,
//! so what Studio shows is what a frontend would get. Also the size of one such body, which a
//! traffic profile uses as the default `resp_bytes`.

use serde_json::Value;

use crate::generation::Generation;
use crate::pins;
use crate::state::Route;

/// One sampled answer.
pub(crate) struct SampleOut {
    /// The route's success status.
    pub status: u16,
    /// The body (`null` when the route has no schema).
    pub body: Value,
    /// The body serialized, in bytes.
    pub bytes: usize,
}

/// A concrete path for `template`: every `{name}` or `{*name}` becomes `kaanal-x`, a segment no
/// static sibling route is likely to spell out.
fn concrete(template: &str) -> String {
    let mut out = String::with_capacity(template.len());
    let mut in_param = false;
    for c in template.chars() {
        match (c, in_param) {
            ('{', _) => {
                in_param = true;
                out.push_str("kaanal-x");
            }
            ('}', _) => in_param = false,
            (_, true) => {}
            (c, false) => out.push(c),
        }
    }
    out
}

/// A writer that only counts, so a body's size is known without building its text.
struct Count(usize);

impl std::io::Write for Count {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0 += buf.len();
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The length of `v` as compact JSON.
fn json_len(v: &Value) -> usize {
    let mut c = Count(0);
    let _ = serde_json::to_writer(&mut c, v);
    c.0
}

/// The compiled route for an endpoint key (`"GET /users/{id}"`), if there is one.
pub(crate) fn find_route<'a>(g: &'a Generation, key: &str) -> Option<&'a Route> {
    let (_, path) = key.split_once(' ')?;
    let at = concrete(path);
    let found = g.paths.at(&at).ok()?;
    found.value.iter().find(|r| r.key == key)
}

/// The sampled answer of `key`, generated with `seed` (default the config's `seed:`); `None` when
/// the endpoint is not a route. An `override` body is returned as written; a generated route's
/// body comes from the response schema. A paged route returns the whole schema body, not a page.
pub(crate) fn sample(g: &Generation, key: &str, seed: Option<u64>) -> Option<SampleOut> {
    let route = find_route(g, key)?;
    let ep = &route.endpoint;
    let (status, body) = if let Some(ov) = &ep.response {
        (ov.status, ov.body.clone())
    } else if let Some(schema) = &route.schema {
        let owner = ep.owner.map_or("", |id| g.topo.nodes[id].name.as_str());
        let path = concrete(key.split_once(' ').map_or("", |(_, p)| p));
        let s = pins::route_seed(seed.unwrap_or(g.seed), owner, &path);
        (
            route.status,
            kaanal_data::generate(schema, None, s, g.options_for_owner(ep.owner)),
        )
    } else {
        (route.status, Value::Null)
    };
    let bytes = if body.is_null() { 0 } else { json_len(&body) };
    Some(SampleOut {
        status,
        body,
        bytes,
    })
}

/// The size in bytes the mock charges for the response of `key`: one sampled body, or 0 where there is
/// nothing to size here: a `HEAD` (hyper strips its body), a `collection:` route (a run starts with
/// no stored items; the live mock sizes what the collection holds) and a built-in responder without
/// an `override` (the live mock sizes the JWKS key set). `sample` itself still returns the body for
/// a preview.
pub(crate) fn sized(g: &Generation, key: &str, seed: u64) -> usize {
    let Some(route) = find_route(g, key) else {
        return 0;
    };
    let ep = &route.endpoint;
    if key.starts_with("HEAD ")
        || ep.collection.is_some()
        || (ep.responder.is_some() && ep.response.is_none())
    {
        return 0;
    }
    sample(g, key, Some(seed)).map_or(0, |s| s.bytes)
}

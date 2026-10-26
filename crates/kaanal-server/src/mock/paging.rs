//! D3 paging of a generated list route: one page of its records (or the `400` for a bad paging
//! parameter) and the `X-Total-Count` / `Link` headers of a page that answered normally.
//!
//! The window is resolved from the request's query and the route's data seed in both places;
//! resolving it is cheap (no records), so the body and the headers never disagree.

use kaanal_data::{PageError, PageSpec, Window};

use super::{AppState, Ask, HeaderValue, Response, Route, Value, pins};

/// **Private.** The window `ask` addresses on `route`, with the data seed it was resolved with.
fn window(
    st: &AppState,
    route: &Route,
    spec: &PageSpec,
    owner: &str,
    ask: &Ask,
) -> (u64, Result<Window, PageError>) {
    // The same seed `generated_body` uses: `seed:` keys the data (NFR-10, D14).
    let seed = pins::route_seed(st.seed, owner, ask.path);
    (seed, spec.window(ask.query, seed, route.total))
}

/// **Private.** The success answer of a paged route: its status and one page with the path
/// parameters applied, or `400` and the error body for a bad paging parameter.
pub(super) fn page(
    st: &AppState,
    route: &Route,
    spec: &PageSpec,
    owner: &str,
    ask: &Ask,
) -> (u16, Value) {
    let Some(schema) = &route.schema else {
        return (route.status, Value::Null);
    };
    match window(st, route, spec, owner, ask) {
        (_, Err(e)) => (400, e.body()),
        (seed, Ok(w)) => {
            let opts = st.options_for_owner(route.endpoint.owner);
            let mut v =
                kaanal_data::generate_page(schema, spec, &w, seed, opts, ask.path, ask.query);
            kaanal_data::apply_params(&mut v, ask.params);
            (route.status, v)
        }
    }
}

/// **Private.** Adds `x-total-count` and `link` to the response of a paged route; nothing for an
/// unpaged route or a query that does not resolve. Links use the raw (still encoded) path.
pub(super) fn add_headers(st: &AppState, route: &Route, ask: &Ask, resp: &mut Response) {
    let Some(spec) = &route.paging else { return };
    let owner = route
        .endpoint
        .owner
        .map_or("", |id| st.topo.nodes[id].name.as_str());
    let (_, Ok(w)) = window(st, route, spec, owner, ask) else {
        return;
    };
    for (name, value) in w.headers(spec, ask.path, ask.query) {
        if let Ok(v) = HeaderValue::from_str(&value) {
            resp.headers_mut().insert(name, v);
        }
    }
}

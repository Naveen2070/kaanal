//! `GET /__kaanal/studio` and `GET|HEAD /__kaanal/studio/assets/{name}` (R1.106). The bundle
//! (`index.html` plus the files the Vite manifest references) comes from `AppState::studio`:
//! the one `build.rs` compiled in, or none, in which case Studio is the status page and a plain
//! `cargo build` still works.

use super::{
    AppState, Arc, HeaderValue, IntoResponse, Response, State, StatusCode, assets, header, reject,
};
use axum::http::{HeaderMap, Method, Uri};

/// The page may load only same-origin files. `style-src 'unsafe-inline'` stays because Radix
/// scroll-lock and the graph canvas set inline styles; there is no inline script.
const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
                   font-src 'self'; img-src 'self' data:; connect-src 'self'; base-uri 'none'; \
                   form-action 'none'; frame-ancestors 'none'";

/// `GET /__kaanal/studio`: the bundle's `index.html` with the page CSP and `no-referrer` (the
/// URL fragment holds the token), or the status page when there is no bundle. The guard marks
/// it `no-store`.
pub async fn studio(State(st): State<Arc<AppState>>) -> Response {
    let Some(b) = st.studio else {
        return super::admin::ui().await.into_response();
    };
    let mut r = (StatusCode::OK, b.page).into_response();
    let h = r.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    r
}

/// Every path under `/__kaanal/studio/`, any method: only `assets/{name}`, an exact table hit on
/// `GET`/`HEAD` with no query, is served (the same [`assets::hit`] test the guard exempts);
/// everything else is 404 (after the guard's 401 without a token).
/// Headers are keyed by file: `Content-Type`, `Cache-Control` (`immutable` for hashed names,
/// `no-cache` otherwise), `Vary: Accept-Encoding` on every variant, a per-encoding `ETag`, and 304
/// for a matching `If-None-Match`. No range support: `Range` is ignored.
pub async fn studio_asset(
    State(st): State<Arc<AppState>>,
    method: Method,
    uri: Uri,
    req: HeaderMap,
) -> Response {
    let Some(a) = assets::hit(st.studio, &method, &uri) else {
        return reject(StatusCode::NOT_FOUND, "no such Studio asset").into_response();
    };
    let text = |name| req.get(name).and_then(|v: &HeaderValue| v.to_str().ok());
    let (enc, body) = assets::negotiate(text(header::ACCEPT_ENCODING), a);
    let etag = assets::etag_for(a, enc);
    let fresh = text(header::IF_NONE_MATCH).is_some_and(|v| assets::etag_matches(v, &etag));
    let mut r = if fresh {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        (StatusCode::OK, body).into_response()
    };
    let h = r.headers_mut();
    if !fresh {
        h.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static(a.content_type),
        );
        // An SVG opened directly is a document; it may not run script or load anything.
        if a.content_type == "image/svg+xml" {
            h.insert(
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static("default-src 'none'; style-src 'unsafe-inline'"),
            );
        }
        if let Some(ce) = enc.header() {
            h.insert(header::CONTENT_ENCODING, HeaderValue::from_static(ce));
        }
    }
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(assets::cache_control(a)),
    );
    h.insert(header::VARY, HeaderValue::from_static("Accept-Encoding"));
    if let Ok(v) = HeaderValue::from_str(&etag) {
        h.insert(header::ETAG, v);
    }
    r
}

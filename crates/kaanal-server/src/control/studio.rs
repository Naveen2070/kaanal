//! `GET /__kaanal/studio` and its two fixed assets. The bundle (`ui-dist/index.html`, `app.js`,
//! `app.css`, built from `ui/`) is compiled in with `include_bytes!` when `build.rs` found it
//! (`cfg(kaanal_ui)`); without it Studio is the status page, so a plain `cargo build` still works.

use axum::http::StatusCode;
#[cfg(kaanal_ui)]
use axum::http::{HeaderValue, header};
use axum::response::IntoResponse;
#[cfg(kaanal_ui)]
use axum::response::Response;

/// The bundle's own page may load only its two same-origin files. Inline styles stay allowed
/// because the graph canvas positions nodes with them; there is no inline script.
#[cfg(kaanal_ui)]
const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
                   img-src 'self' data:; font-src 'self'; connect-src 'self'; base-uri 'none'; \
                   form-action 'none'; frame-ancestors 'none'";

/// `GET /__kaanal/studio`: the bundle's `index.html`, or the status page.
#[cfg(kaanal_ui)]
pub async fn studio() -> impl IntoResponse {
    asset(
        "text/html; charset=utf-8",
        include_bytes!("../../ui-dist/index.html"),
        true,
    )
}

/// `GET /__kaanal/studio`: no bundle was built in, so this is the status page.
#[cfg(not(kaanal_ui))]
pub async fn studio() -> impl IntoResponse {
    super::admin::ui().await
}

/// `GET /__kaanal/studio/assets/app.js`.
#[cfg(kaanal_ui)]
pub async fn app_js() -> impl IntoResponse {
    asset(
        "text/javascript; charset=utf-8",
        include_bytes!("../../ui-dist/app.js"),
        false,
    )
}

/// `GET /__kaanal/studio/assets/app.css`.
#[cfg(kaanal_ui)]
pub async fn app_css() -> impl IntoResponse {
    asset(
        "text/css; charset=utf-8",
        include_bytes!("../../ui-dist/app.css"),
        false,
    )
}

/// `GET /__kaanal/studio/assets/app.js`: no bundle, nothing to serve.
#[cfg(not(kaanal_ui))]
pub async fn app_js() -> impl IntoResponse {
    StatusCode::NOT_FOUND
}

/// `GET /__kaanal/studio/assets/app.css`: no bundle, nothing to serve.
#[cfg(not(kaanal_ui))]
pub async fn app_css() -> impl IntoResponse {
    StatusCode::NOT_FOUND
}

/// **Private.** One embedded file. `page` adds the page-only headers (CSP, no referrer: the URL
/// fragment holds the token).
#[cfg(kaanal_ui)]
fn asset(content_type: &'static str, body: &'static [u8], page: bool) -> Response {
    let mut r = (StatusCode::OK, body).into_response();
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    if page {
        h.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(CSP),
        );
        h.insert(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        );
    }
    r
}

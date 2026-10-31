//! The security guard in front of every `/__kaanal/*` route: Host check, bearer token, constant-time compare.

use super::{
    AppState, Arc, HeaderValue, IntoResponse, Next, Request, Response, State, StatusCode, assets,
    header, reject,
};

/// Runs in front of every `/__kaanal/*` route, before any extractor parses a body or query,
/// so unauthenticated callers never reach handler code:
/// 1. `Host` must be loopback with our port (DNS-rebinding guard), for the whole namespace.
/// 2. Everything except `health`, `ui`, `studio` and a Studio asset needs the bearer token (the
///    page must load before it has the token). An asset is exempt only when [`assets::hit`]
///    says so: `GET`/`HEAD`, no query, and the raw path is `studio/assets/` plus a name in the
///    table, exactly; every other method, miss or variant falls through to the token check
///    (R1.106). `/events` alone may take the token as `?token=`, because the browser
///    `EventSource` cannot set headers.
/// 3. Responses are marked uncacheable (unless the handler set its own cache policy) and non-sniffable.
pub async fn guard(State(st): State<Arc<AppState>>, req: Request, next: Next) -> Response {
    let headers = req.headers();
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    if !host_allowed(host, st.port) {
        return reject(StatusCode::FORBIDDEN, "bad Host header").into_response();
    }
    let path = req.uri().path();
    let exempt = matches!(
        path,
        "/__kaanal/health" | "/__kaanal/ui" | "/__kaanal/studio"
    ) || assets::hit(st.studio, req.method(), req.uri()).is_some();
    if !exempt {
        let bearer = headers
            .get(header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.strip_prefix("Bearer "));
        let query = if path == "/__kaanal/events" {
            query_token(req.uri().query())
        } else {
            None
        };
        match bearer.or(query) {
            Some(t) if !st.token.is_empty() && ct_eq(t.as_bytes(), st.token.as_bytes()) => {}
            _ => {
                return reject(StatusCode::UNAUTHORIZED, "missing or bad token").into_response();
            }
        }
    }
    let mut resp = next.run(req).await;
    mark_response(resp.headers_mut());
    resp
}

/// **Private.** Marks a response non-sniffable, and uncacheable unless the handler chose its own
/// cache policy (the Studio assets send `immutable` for hashed files; overwriting it with
/// `no-store` would defeat their caching).
pub(super) fn mark_response(h: &mut axum::http::HeaderMap) {
    if !h.contains_key(header::CACHE_CONTROL) {
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
}

/// **Private.** Extracts `token=` from a query string; `xtoken=` or a bare `token` do not match.
pub(super) fn query_token(query: Option<&str>) -> Option<&str> {
    query?.split('&').find_map(|kv| kv.strip_prefix("token="))
}

// limitation: by design (RULES R5.3). Even when `--host 0.0.0.0` exposes the server on the LAN,
// control calls must use `localhost`, `127.0.0.1` or `[::1]` as Host with the bound port. Reaching the
// control API by LAN IP or through a port-forward (container, SSH tunnel on another port) gets a 403.
// Add an explicit allowed-hosts setting if that is ever needed.
/// **Private.** True iff `host` is `localhost`/`127.0.0.1`/`[::1]` with no port or
/// the bound port. Case-insensitive; rejects `localhost.evil.example` and
/// `evil.example:8080@localhost`.
pub(super) fn host_allowed(host: &str, port: u16) -> bool {
    let host = host.to_ascii_lowercase();
    let (name, p) = match host.strip_prefix('[') {
        Some(rest) => match rest.split_once(']') {
            Some((h, tail)) => (format!("[{h}]"), tail.strip_prefix(':')),
            None => return false,
        },
        None => match host.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), Some(p)),
            None => (host.clone(), None),
        },
    };
    matches!(name.as_str(), "localhost" | "127.0.0.1" | "[::1]")
        && p.is_none_or(|p| p.parse::<u16>().ok() == Some(port))
}

/// Constant-time comparison so the token can't be guessed byte by byte from timing.
pub(super) fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

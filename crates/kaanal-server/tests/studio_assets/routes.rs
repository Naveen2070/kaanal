//! `GET|HEAD /__kaanal/studio/assets/{name}`: the exemption covers only a table hit on GET/HEAD;
//! headers are keyed by file; encodings, `ETag`s and 304 (spec section 11, Review Focus 4).

use crate::{CSS, ICON, JS, asset, start};
use reqwest::{Method, StatusCode};

const IMMUTABLE: &str = "public, max-age=31536000, immutable";

/// (asset, `Accept-Encoding`, `Content-Encoding`, `ETag`, body)
type Case = (
    &'static str,
    Option<&'static str>,
    Option<&'static str>,
    &'static str,
    &'static [u8],
);

#[tokio::test]
async fn table_hit_needs_no_token_get_and_head_only() {
    let s = start().await;
    let get = s.get(&asset(JS)).await;
    assert_eq!(get.status(), StatusCode::OK);
    let head = s.req(Method::HEAD, &asset(JS), &[]).await;
    assert_eq!(head.status(), StatusCode::OK);
    assert_eq!(
        head.headers()["content-type"],
        "text/javascript; charset=utf-8"
    );
    assert_eq!(head.headers()["cache-control"], IMMUTABLE);
    assert!(head.bytes().await.unwrap().is_empty(), "HEAD has no body");
    for m in [
        Method::POST,
        Method::OPTIONS,
        Method::PUT,
        Method::DELETE,
        Method::PATCH,
    ] {
        let r = s.req(m.clone(), &asset(JS), &[]).await;
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED, "{m}");
        assert!(
            r.headers().get("access-control-allow-origin").is_none(),
            "{m}"
        );
        // with the token the method is still not served
        assert_eq!(s.authed(m.clone(), &asset(JS)).await.status(), 404, "{m}");
    }
}

/// HEAD reports the length GET would send, for each encoding.
#[tokio::test]
async fn head_content_length_matches_get() {
    let s = start().await;
    for ae in [None, Some("br")] {
        let h: Vec<(&str, &str)> = ae.map(|v| ("Accept-Encoding", v)).into_iter().collect();
        let get = s.req(Method::GET, &asset(JS), &h).await;
        let len = get.headers()["content-length"].clone();
        let body = get.bytes().await.unwrap();
        assert_eq!(len.to_str().unwrap(), body.len().to_string(), "{ae:?}");
        let head = s.req(Method::HEAD, &asset(JS), &h).await;
        assert_eq!(head.headers()["content-length"], len, "{ae:?}");
    }
}

/// An SVG opened directly cannot run script or load anything (defence in depth); other types
/// carry no CSP.
#[tokio::test]
async fn svg_carries_its_own_csp() {
    let s = start().await;
    let r = s.get(&asset(ICON)).await;
    assert_eq!(
        r.headers()["content-security-policy"],
        "default-src 'none'; style-src 'unsafe-inline'"
    );
    let r = s.get(&asset(JS)).await;
    assert!(r.headers().get("content-security-policy").is_none());
}

#[tokio::test]
async fn miss_is_401_without_token_404_with_it() {
    let s = start().await;
    for name in ["nope-Ab12Cd34.js", "app.js", "app.css", "index.html"] {
        assert_eq!(s.get(&asset(name)).await.status(), 401, "{name}");
        assert_eq!(
            s.authed(Method::GET, &asset(name)).await.status(),
            404,
            "{name}"
        );
        assert_eq!(
            s.authed(Method::HEAD, &asset(name)).await.status(),
            404,
            "{name}"
        );
    }
}

#[tokio::test]
async fn hashed_asset_is_immutable_and_vary_set() {
    let s = start().await;

    let cases: [Case; 9] = [
        (
            JS,
            Some("gzip, deflate, br"),
            Some("br"),
            "\"0011223344556677-br\"",
            b"BR-BYTES",
        ),
        (
            JS,
            Some("gzip"),
            Some("gzip"),
            "\"0011223344556677-gz\"",
            b"GZ-BYTES",
        ),
        (
            JS,
            Some("br;q=0, gzip"),
            Some("gzip"),
            "\"0011223344556677-gz\"",
            b"GZ-BYTES",
        ),
        (
            JS,
            Some("br;q=0"),
            None,
            "\"0011223344556677\"",
            b"console.log('identity')",
        ),
        (
            JS,
            Some("identity;q=0, gzip"),
            Some("gzip"),
            "\"0011223344556677-gz\"",
            b"GZ-BYTES",
        ),
        (
            JS,
            None,
            None,
            "\"0011223344556677\"",
            b"console.log('identity')",
        ),
        (
            CSS,
            Some("br, gzip"),
            None,
            "\"8899aabbccddeeff\"",
            b"body{margin:0}",
        ),
        (ICON, Some("br"), None, "\"0123456789abcdef\"", b"<svg/>"),
        (ICON, None, None, "\"0123456789abcdef\"", b"<svg/>"),
    ];
    for (name, ae, ce, etag, body) in cases {
        let headers: Vec<(&str, &str)> = ae.map(|v| ("Accept-Encoding", v)).into_iter().collect();
        let r = s.req(Method::GET, &asset(name), &headers).await;
        let ctx = format!("{name} {ae:?}");
        assert_eq!(r.status(), StatusCode::OK, "{ctx}");
        let h = r.headers();
        let cache = if name == ICON { "no-cache" } else { IMMUTABLE };
        assert_eq!(
            h["cache-control"], cache,
            "{ctx}: the guard keeps the handler's header"
        );
        assert_eq!(h["vary"], "Accept-Encoding", "{ctx}");
        assert_eq!(h["etag"], etag, "{ctx}");
        assert_eq!(h["x-content-type-options"], "nosniff", "{ctx}");
        assert_eq!(
            h.get("content-encoding").map(|v| v.to_str().unwrap()),
            ce,
            "{ctx}"
        );
        assert!(h.get("accept-ranges").is_none_or(|v| v == "none"), "{ctx}");
        assert_eq!(&r.bytes().await.unwrap()[..], body, "{ctx}");
    }
    // no range support: a Range header still gets the whole file
    let r = s
        .req(Method::GET, &asset(CSS), &[("Range", "bytes=0-1")])
        .await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(&r.bytes().await.unwrap()[..], b"body{margin:0}");
}

#[tokio::test]
async fn if_none_match_gives_304() {
    let s = start().await;
    let id = "\"0011223344556677\"";
    let br = "\"0011223344556677-br\"";
    let r = s
        .req(Method::GET, &asset(JS), &[("If-None-Match", id)])
        .await;
    assert_eq!(r.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(r.headers()["etag"], id);
    assert_eq!(r.headers()["vary"], "Accept-Encoding");
    assert_eq!(r.headers()["cache-control"], IMMUTABLE);
    assert!(r.bytes().await.unwrap().is_empty());
    let r = s
        .req(
            Method::GET,
            &asset(JS),
            &[("If-None-Match", br), ("Accept-Encoding", "br")],
        )
        .await;
    assert_eq!(r.status(), StatusCode::NOT_MODIFIED);
    let r = s
        .req(
            Method::HEAD,
            &asset(JS),
            &[("If-None-Match", &format!("W/{id}"))],
        )
        .await;
    assert_eq!(
        r.status(),
        StatusCode::NOT_MODIFIED,
        "weak compare, HEAD too"
    );
    // a stale tag, or the tag of another encoding, gets the full body
    for (inm, ae) in [("\"stale\"", None), (br, None), (id, Some("br"))] {
        let mut h = vec![("If-None-Match", inm)];
        h.extend(ae.map(|v| ("Accept-Encoding", v)));
        let r = s.req(Method::GET, &asset(JS), &h).await;
        assert_eq!(r.status(), StatusCode::OK, "{inm} {ae:?}");
        assert!(!r.bytes().await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn index_html_and_map_and_manifest_not_served_from_assets() {
    let s = start().await;
    for name in [
        "index.html",
        &format!("{JS}.map"),
        "manifest.json",
        ".vite/manifest.json",
        ".vite%2Fmanifest.json",
        &format!("{JS}.br"),
        &format!("{JS}.gz"),
    ] {
        assert_eq!(
            s.raw_status("GET", &asset(name), false).await,
            401,
            "{name}"
        );
        assert_eq!(s.raw_status("GET", &asset(name), true).await, 404, "{name}");
    }
}

/// Review Focus 4: every variant of a table name falls through to the token check, sent raw so
/// no client normalises it first.
#[tokio::test]
async fn asset_path_variants_fall_through_to_auth() {
    let s = start().await;
    let p = "/__kaanal/studio/assets";
    let variants = [
        format!("{p}/%2e%2e/assets/{JS}"),
        format!("{p}/%2E%2E/{JS}"),
        format!("{p}/../assets/{JS}"),
        format!("{p}/x/../{JS}"),
        format!("{p}/./{JS}"),
        format!("{p}/%2F{JS}"),
        format!("{p}/{}", JS.replace('.', "%2E")),
        format!("{p}/{}", JS.replace('.', "%2e")),
        format!("{p}/{}", JS.to_uppercase()),
        format!("{p}/{}", "Index-B7PI925R.js"),
        format!("{p}/{JS}/"),
        format!("{p}//{JS}"),
        format!("{p}/{JS}?x=1"),
        format!("{p}/{JS}?"),
        format!("{p}/{JS}%00"),
        format!("{p}/{JS};x"),
        format!("{p}/{JS}%20"),
        // an encoded slash in place of the `/` after `assets`
        format!("{p}%2F{JS}"),
        format!("{p}%2f{JS}"),
    ];
    for v in &variants {
        assert_eq!(s.raw_status("GET", v, false).await, 401, "no token: {v}");
        assert_eq!(
            s.raw_status("HEAD", v, false).await,
            401,
            "no token, HEAD: {v}"
        );
        assert_eq!(s.raw_status("GET", v, true).await, 404, "token: {v}");
    }
    // the plain name sent the same raw way is a hit, so the variants above are not refused by accident
    assert_eq!(s.raw_status("GET", &format!("{p}/{JS}"), false).await, 200);
}

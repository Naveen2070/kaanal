//! The page at `/__kaanal/studio`, the no-bundle fallback, and the exemption basics (migrated
//! from the old three-fixed-files `site_ui.rs`).

use crate::{ASSETS, CSS, ICON, JS, PAGE, asset, start, start_embedded, start_with};
use reqwest::{Method, StatusCode};

const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
                   font-src 'self'; img-src 'self' data:; connect-src 'self'; base-uri 'none'; \
                   form-action 'none'; frame-ancestors 'none'";

/// Spec section 11: the binary-served page carries the CSP and `no-referrer` (the fragment holds
/// the token); the guard keeps it uncacheable.
#[tokio::test]
async fn page_has_csp_and_no_referrer() {
    let s = start().await;
    let r = s.get("/__kaanal/studio").await;
    assert_eq!(r.status(), StatusCode::OK);
    let h = r.headers();
    assert_eq!(h["content-type"], "text/html; charset=utf-8");
    assert_eq!(h["content-security-policy"], CSP);
    assert_eq!(h["referrer-policy"], "no-referrer");
    assert_eq!(h["cache-control"], "no-store");
    assert_eq!(h["x-content-type-options"], "nosniff");
    assert_eq!(&r.bytes().await.unwrap()[..], PAGE);
}

/// No bundle: Studio is the status page, and no asset name is exempt (every one is a miss).
#[tokio::test]
async fn build_without_bundle_serves_status_page() {
    let s = start_with(None).await;
    let studio = s.get("/__kaanal/studio").await;
    assert_eq!(studio.status(), StatusCode::OK);
    assert!(
        studio.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );
    let studio = studio.text().await.unwrap();
    let status = s.get("/__kaanal/ui").await.text().await.unwrap();
    assert_eq!(studio, status, "no bundle: Studio is the status page");
    for name in [JS, CSS, ICON, "app.js", "app.css"] {
        assert_eq!(s.get(&asset(name)).await.status(), 401, "{name}");
        assert_eq!(
            s.authed(Method::GET, &asset(name)).await.status(),
            404,
            "{name}"
        );
    }
    // what plain `serve` uses: the status page when no bundle was compiled in
    let s = start_embedded().await;
    let page = s.get("/__kaanal/studio").await.text().await.unwrap();
    match kaanal_server::Bundle::embedded() {
        None => assert_eq!(page, status),
        // built with a bundle (`cfg(kaanal_ui)`): its page and every table file, with no token
        Some(b) => {
            assert_eq!(page.as_bytes(), b.page);
            for a in b.assets {
                let r = s.get(&asset(a.name)).await;
                assert_eq!(r.status(), StatusCode::OK, "{}", a.name);
                assert_eq!(r.headers()["content-type"], a.content_type, "{}", a.name);
            }
        }
    }
}

/// Every table file is served with its own content type; the page names the hashed script.
#[tokio::test]
async fn studio_serves_the_bundle_with_the_right_content_types() {
    let s = start().await;
    for a in ASSETS {
        let r = s.get(&asset(a.name)).await;
        assert_eq!(r.status(), StatusCode::OK, "{}", a.name);
        assert_eq!(r.headers()["content-type"], a.content_type, "{}", a.name);
        assert_eq!(&r.bytes().await.unwrap()[..], a.identity, "{}", a.name);
    }
    let page = s.get("/__kaanal/studio").await.text().await.unwrap();
    assert!(
        page.contains(&asset(JS)),
        "the page loads the hashed script"
    );
}

/// R5.2 exemption: the page and its table need no token; the API behind them still does.
#[tokio::test]
async fn the_assets_need_no_token_but_the_api_still_does() {
    let s = start().await;
    assert_eq!(s.get("/__kaanal/studio").await.status(), StatusCode::OK);
    for a in ASSETS {
        assert_eq!(
            s.get(&asset(a.name)).await.status(),
            StatusCode::OK,
            "{}",
            a.name
        );
    }
    let r = s.get("/__kaanal/topology").await;
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
}

/// The exemption is exact names, not a prefix: every other path under `studio/` is 401 without
/// the token and 404 with it; `studio2` is no Studio path at all (the namespace's 404 either way).
#[tokio::test]
async fn a_sibling_of_an_exempt_path_still_needs_the_token() {
    let s = start().await;
    for path in [
        "/__kaanal/studio/".to_string(),
        "/__kaanal/studio/x".to_string(),
        asset("other.js"),
        asset(&format!("{JS}.map")),
        "/__kaanal/studio/assets".to_string(),
        "/__kaanal/studio/assets/".to_string(),
    ] {
        assert_eq!(s.get(&path).await.status(), 401, "no token: {path}");
        assert_eq!(
            s.authed(Method::GET, &path).await.status(),
            404,
            "token: {path}"
        );
    }
    assert_eq!(s.get("/__kaanal/studio2").await.status(), 404);
    assert_eq!(
        s.authed(Method::GET, "/__kaanal/studio2").await.status(),
        404
    );
    // the page itself is still the page
    assert_eq!(s.get("/__kaanal/studio").await.status(), StatusCode::OK);
}

/// The Host check covers the exempt paths too (DNS rebinding must not reach the page).
#[tokio::test]
async fn the_exempt_paths_still_refuse_a_bad_host() {
    let s = start().await;
    let mut paths = vec!["/__kaanal/studio".to_string()];
    paths.extend(ASSETS.iter().map(|a| asset(a.name)));
    for path in paths {
        let r = s.req(Method::GET, &path, &[("Host", "evil.example")]).await;
        assert_eq!(r.status(), StatusCode::FORBIDDEN, "{path}");
    }
}

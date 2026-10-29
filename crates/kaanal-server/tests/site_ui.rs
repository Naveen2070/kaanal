//! Studio's own page: `GET /__kaanal/studio` and its two fixed assets. Without a built bundle the page
//! is the status page; with one (`cfg(kaanal_ui)`, set by `build.rs`) it is the bundle.

use kaanal_core::Config;
use kaanal_server::{BoundServer, ServeOpts};
use reqwest::StatusCode;
use std::path::{Path, PathBuf};

async fn start() -> (String, BoundServer) {
    let dir: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let opts = ServeOpts {
        host: "127.0.0.1".parse().unwrap(),
        port: 0,
        token: kaanal_server::new_token(),
        auth_key: None,
        draft: None,
    };
    let yaml = "version: 1\nnodes:\n  gw: { type: gateway }\n";
    let s = kaanal_server::serve(&Config::from_yaml(yaml).unwrap(), &dir, opts)
        .await
        .unwrap();
    (format!("http://127.0.0.1:{}", s.addr.port()), s)
}

#[cfg(not(kaanal_ui))]
#[tokio::test]
async fn studio_serves_the_fallback_page_without_a_bundle() {
    let (base, _s) = start().await;
    let studio = reqwest::get(format!("{base}/__kaanal/studio"))
        .await
        .unwrap();
    assert_eq!(studio.status(), StatusCode::OK);
    assert!(
        studio.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );
    let studio = studio.text().await.unwrap();
    let status = reqwest::get(format!("{base}/__kaanal/ui"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_eq!(studio, status, "no bundle: Studio is the status page");
    let js = reqwest::get(format!("{base}/__kaanal/studio/assets/app.js"))
        .await
        .unwrap();
    assert_eq!(js.status(), StatusCode::NOT_FOUND);
}

#[cfg(kaanal_ui)]
#[tokio::test]
async fn studio_serves_the_bundle_with_the_right_content_types() {
    let (base, _s) = start().await;
    for (path, ct) in [
        ("/__kaanal/studio", "text/html"),
        ("/__kaanal/studio/assets/app.js", "text/javascript"),
        ("/__kaanal/studio/assets/app.css", "text/css"),
    ] {
        let r = reqwest::get(format!("{base}{path}")).await.unwrap();
        assert_eq!(r.status(), StatusCode::OK, "{path}");
        assert!(
            r.headers()["content-type"]
                .to_str()
                .unwrap()
                .starts_with(ct),
            "{path}"
        );
        assert!(!r.bytes().await.unwrap().is_empty(), "{path}");
    }
    let page = reqwest::get(format!("{base}/__kaanal/studio"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        page.contains("assets/app.js"),
        "the page loads the fixed bundle name"
    );
}

/// R5.2 exemption: the page must load before it has the token; the API behind it still needs one.
#[tokio::test]
async fn the_assets_need_no_token_but_the_api_still_does() {
    let (base, _s) = start().await;
    for path in [
        "/__kaanal/studio",
        "/__kaanal/studio/assets/app.js",
        "/__kaanal/studio/assets/app.css",
    ] {
        let r = reqwest::get(format!("{base}{path}")).await.unwrap();
        assert_ne!(r.status(), StatusCode::UNAUTHORIZED, "{path}");
    }
    let r = reqwest::get(format!("{base}/__kaanal/topology"))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
}

/// The exemption is exact paths, not a prefix: a sibling under `studio/` never gets a 2xx without the token.
#[tokio::test]
async fn a_sibling_of_an_exempt_path_still_needs_the_token() {
    let (base, _s) = start().await;
    for path in [
        "/__kaanal/studio/",
        "/__kaanal/studio/assets/other.js",
        "/__kaanal/studio/assets/app.js.map",
        "/__kaanal/studio/assets",
        "/__kaanal/studio2",
    ] {
        let r = reqwest::get(format!("{base}{path}")).await.unwrap();
        // a path with no route at all is a 404; what must never happen is a 2xx without a token
        assert!(
            matches!(r.status(), StatusCode::UNAUTHORIZED | StatusCode::NOT_FOUND),
            "{path}: {}",
            r.status()
        );
    }
}

/// The Host check covers the exempt paths too (DNS rebinding must not reach the page).
#[tokio::test]
async fn the_exempt_paths_still_refuse_a_bad_host() {
    let (base, _s) = start().await;
    let c = reqwest::Client::new();
    for path in [
        "/__kaanal/studio",
        "/__kaanal/studio/assets/app.js",
        "/__kaanal/studio/assets/app.css",
    ] {
        let r = c
            .get(format!("{base}{path}"))
            .header("Host", "evil.example")
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::FORBIDDEN, "{path}");
    }
}

//! The embedded template: it validates, the page and its assets are served, and the page keeps to its rules.

use crate::common::*;
use crate::demo;

#[test]
fn the_template_validates() {
    let dir = project("validates");
    let _clean = Cleanup(dir.clone());
    let o = run(&dir, &["validate"]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
}

#[test]
fn every_pinned_route_and_scenario_exists() {
    let dir = project("pinned");
    let (_up, port) = up(&dir);
    // Every route has an explicit chain, so each story lands on the same requests every time.
    let out = run(&dir, &["endpoints"]);
    let routes = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(routes.starts_with("nothing to add"), "{routes}");
    let login = agent()
        .post(&format!("http://127.0.0.1:{port}/login"))
        .send_json(serde_json::json!({"username": "demo@example.com", "password": "demo"}))
        .unwrap();
    assert_eq!(login.status().as_u16(), 200);
    let scenarios = run(&dir, &["scenario", "list"]);
    let list = String::from_utf8_lossy(&scenarios.stdout).into_owned();
    for name in STORIES {
        assert!(list.contains(name), "missing {name}: {list}");
    }
    // The pinned values, as whole lines of the files (so 800 never matches 8000).
    for (name, line) in [
        ("slow-db", "  - { node: db, latency_ms: 800 }"),
        ("cache-down", "  - { node: cache, down: true }"),
        (
            "auth-errors",
            "  - { node: auth, error_rate: 0.5, error_status: 503 }",
        ),
        ("gateway-down", "  - { node: nginx, down: true }"),
        (
            "black-friday",
            "  - { endpoint: \"POST /cart\", state: error, status: 429 }",
        ),
        ("payment-slow", "  - { node: payments, latency_ms: 1800 }"),
        (
            "payment-flaky",
            "  - { node: payments, error_rate: 0.4, error_status: 502 }",
        ),
        (
            "payment-down",
            "  - { node: payments, error_rate: 1.0, error_status: 503 }",
        ),
        (
            "payment-timeout",
            "  - { node: payments, error_rate: 1.0, error_kind: hang }",
        ),
        (
            "payment-declined",
            "  - { endpoint: \"POST /checkout\", state: error, status: 402 }",
        ),
    ] {
        let text = std::fs::read_to_string(dir.join(format!("scenarios/{name}.yaml"))).unwrap();
        assert!(text.lines().any(|l| l == line), "{name}: {text}");
    }
}

#[test]
fn cart_adds_lists_and_removes_over_http() {
    let dir = project("cart");
    let (_up, port) = up(&dir);
    let base = format!("http://127.0.0.1:{port}");
    let http = agent();
    let cart = format!("{base}/cart");
    assert_eq!(get_json(&http, &cart), serde_json::json!([]));
    let mut added = http
        .post(&cart)
        .send_json(serde_json::json!({"productId": 1, "qty": 2}))
        .unwrap();
    assert_eq!(added.status().as_u16(), 201);
    let item: serde_json::Value = added.body_mut().read_json().unwrap();
    let id = item["id"].as_u64().unwrap();
    assert_eq!(get_json(&http, &cart).as_array().unwrap().len(), 1);
    let removed = http.delete(&format!("{cart}/{id}")).call().unwrap();
    assert_eq!(removed.status().as_u16(), 204);
    assert_eq!(get_json(&http, &cart), serde_json::json!([]));
    // Products are store-shaped and the page is served.
    let products = get_json(&http, &format!("{base}/products"));
    let items = products.as_array().unwrap();
    let ids: Vec<u64> = items.iter().map(|p| p["id"].as_u64().unwrap()).collect();
    assert_eq!(ids, (1..=24).collect::<Vec<u64>>());
    let mut names: Vec<&str> = items.iter().map(|p| p["name"].as_str().unwrap()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), 24, "product names repeat");
    assert!(
        items
            .iter()
            .all(|p| p["price"].is_number() && p["image"].is_string())
    );
    assert_eq!(
        http.get(&format!("{base}/"))
            .call()
            .unwrap()
            .status()
            .as_u16(),
        200
    );
}

#[test]
fn the_template_has_no_real_names_or_brands() {
    // A denylist is overkill: every email in the template must be an example.com address.
    for (path, text) in demo::FILES {
        for word in text.split(|c: char| c.is_whitespace() || "\"',:[]{}()<>".contains(c)) {
            if word.contains('@') && !word.starts_with('@') && word.len() > 1 {
                assert!(word.ends_with("@example.com"), "{path}: {word}");
            }
        }
    }
}

#[test]
fn the_page_and_its_assets_are_served_from_root() {
    let dir = project("served");
    let (_up, port) = up(&dir);
    let http = agent();
    for (path, file, kind) in [
        ("/", "index.html", "text/html"),
        ("/app.js", "app.js", "text/javascript"),
        ("/style.css", "style.css", "text/css"),
        ("/img/product.svg", "img/product.svg", "image/svg+xml"),
    ] {
        let mut r = http
            .get(&format!("http://127.0.0.1:{port}{path}"))
            .call()
            .unwrap();
        assert_eq!(r.status().as_u16(), 200, "{path}");
        let ct = r.headers()["content-type"].to_str().unwrap().to_string();
        assert!(ct.starts_with(kind), "{path}: {ct}");
        assert_eq!(r.body_mut().read_to_string().unwrap(), site_file(file));
    }
}

#[test]
fn every_script_style_and_image_the_page_references_is_embedded() {
    let html = site_file("index.html");
    let js = site_file("app.js");
    let mut refs: Vec<&str> = values_after(html, "src=\"");
    refs.extend(
        values_after(html, "href=\"")
            .into_iter()
            .filter(|h| !h.starts_with('#')),
    );
    // Images the page builds or the API hands it, as quoted absolute paths.
    let api = demo::FILES
        .iter()
        .find(|(p, _)| *p == "openapi.yaml")
        .unwrap()
        .1;
    for text in [js, site_file("style.css"), api] {
        for (i, _) in text.match_indices("/img/") {
            let end = text[i..].find(|c: char| "\"' ,)}\n".contains(c)).unwrap();
            refs.push(&text[i..i + end]);
        }
    }
    assert!(refs.iter().any(|r| r.ends_with("app.js")), "{refs:?}");
    assert!(refs.iter().any(|r| r.ends_with("style.css")), "{refs:?}");
    for r in refs {
        let path = r.trim_start_matches("./").trim_start_matches('/');
        site_file(path);
    }
}

#[test]
fn the_page_makes_no_external_requests() {
    for (path, text) in demo::FILES.iter().filter(|(p, _)| p.starts_with("site/")) {
        for scheme in ["http://", "https://", "ws://", "wss://"] {
            for (i, _) in text.match_indices(scheme) {
                let host = &text[i + scheme.len()..];
                // A local host must end there (`localhost.evil.com` is not local).
                let local = |name: &str| {
                    host.strip_prefix(name)
                        .is_some_and(|rest| rest.is_empty() || rest.starts_with([':', '/']))
                };
                // The SVG namespace is an identifier, never fetched.
                assert!(
                    (scheme == "http://" && host.starts_with("www.w3.org/2000/svg\""))
                        || local("localhost")
                        || local("127.0.0.1"),
                    "{path}: {}",
                    &text[i..(i + 60).min(text.len())]
                );
            }
        }
        // Protocol-relative URLs, in any quoting.
        for bad in ["\"//", "'//", "`//", "url(//", "@import"] {
            assert!(!text.contains(bad), "{path}: {bad}");
        }
    }
    // Every fetch goes to this origin: the storefront's own `api(method, "/...")` helper, or a
    // template literal starting with `/`.
    let js = site_file("app.js");
    for (i, _) in js.match_indices("fetch(") {
        let arg = &js[i + "fetch(".len()..];
        assert!(
            arg.starts_with("path,") || arg.starts_with("`/__kaanal"),
            "fetch target: {}",
            &arg[..40.min(arg.len())]
        );
    }
    for (i, _) in js.match_indices("api(\"") {
        let rest = &js[i + "api(\"".len()..];
        let target = rest.split_once("\", ").map_or_default(|(_, t)| t);
        assert!(
            (target.starts_with("\"/") || target.starts_with("`/"))
                && !target[1..].starts_with("//"),
            "api target: {}",
            &rest[..40.min(rest.len())]
        );
    }
}

#[test]
fn the_script_builds_no_html_and_keeps_the_token_in_session_storage() {
    let js = site_file("app.js");
    for bad in [
        "innerHTML",
        "outerHTML",
        "insertAdjacentHTML",
        "eval(",
        "new Function",
        "document.write",
        "localStorage",
    ] {
        assert!(!js.contains(bad), "app.js uses {bad}");
    }
    // One storage write in the whole script, in `store`, to sessionStorage; the token goes
    // through it and nowhere else.
    assert_eq!(js.matches("setItem(").count(), 1);
    assert_eq!(
        js.matches("sessionStorage.setItem(").count(),
        1,
        "the only write is to sessionStorage"
    );
    assert!(js.contains("const TOKEN_KEY = \"kaanal.token\";"));
    assert_eq!(js.matches("store(TOKEN_KEY, t)").count(), 1);
    // The token only rides in the Authorization header of /__kaanal calls.
    for (i, _) in js.match_indices("${token}") {
        let line_start = js[..i].rfind('\n').unwrap_or(0);
        let line = &js[line_start..js[i..].find('\n').map_or(js.len(), |e| i + e)];
        assert!(line.contains("authorization: `Bearer ${token}`"), "{line}");
    }
}

#[test]
fn every_id_the_script_looks_up_is_in_the_page() {
    let html = site_file("index.html");
    let js = site_file("app.js");
    let ids: Vec<&str> = values_after(html, " id=\"");
    let mut looked_up = values_after(js, "$(\"");
    looked_up.extend(values_after(js, "getElementById(\""));
    assert!(looked_up.len() > 10, "{looked_up:?}");
    for id in looked_up {
        assert!(
            ids.contains(&id),
            "app.js looks up #{id}, index.html has {ids:?}"
        );
    }
    // `$` is only ever called with a literal id, so the check above sees every lookup.
    assert_eq!(
        js.matches("$(").count(),
        js.matches("$(\"").count(),
        "a $() call with a computed id"
    );
}

#[test]
fn asset_budget() {
    let total: usize = demo::FILES.iter().map(|(_, t)| t.len()).sum();
    assert!(total < 300 * 1024, "embedded demo is {total} bytes");
}

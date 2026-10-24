//! The embedded demo project, written out and run with plain `kaanal`.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

#[path = "../src/demo_files.rs"]
mod demo;

fn bin() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_kaanal"));
    c.env_remove("KAANAL_JSON");
    c.env_remove("KAANAL_ENDPOINT");
    c.env_remove("KAANAL_TOKEN");
    c.stdin(Stdio::null());
    c
}

/// A fresh folder holding the template.
fn project(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("kaanal-demo-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    for (path, text) in demo::FILES {
        let p = d.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    d
}

fn run(dir: &Path, args: &[&str]) -> Output {
    bin().args(args).current_dir(dir).output().unwrap()
}

/// A `kaanal up` child, killed on drop, then its folder removed.
struct Up(Child, PathBuf);

impl Drop for Up {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
        let _ = std::fs::remove_dir_all(&self.1);
    }
}

fn up(dir: &Path) -> (Up, u16) {
    let child = bin()
        .args(["up", "--port", "0"])
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(dir.join("up.err")).unwrap())
        .current_dir(dir)
        .spawn()
        .unwrap();
    let mut guard = Up(child, dir.to_path_buf());
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(p) = std::fs::read_to_string(dir.join(".kaanal/run.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|j| j["port"].as_u64())
        {
            return (guard, u16::try_from(p).unwrap());
        }
        let exited = guard.0.try_wait().unwrap();
        assert!(
            exited.is_none() && Instant::now() < deadline,
            "kaanal up did not start ({exited:?}): {}",
            std::fs::read_to_string(dir.join("up.err")).unwrap_or_default()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into()
}

/// Removes a folder on drop, so a failing test still cleans up.
struct Cleanup(PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

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

fn get_json(agent: &ureq::Agent, url: &str) -> serde_json::Value {
    agent
        .get(url)
        .call()
        .unwrap()
        .body_mut()
        .read_json()
        .unwrap()
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

/// The embedded text of a site file (`site/` + `path`).
fn site_file(path: &str) -> &'static str {
    let want = format!("site/{path}");
    demo::FILES
        .iter()
        .find(|(p, _)| *p == want)
        .map_or_else(|| panic!("{want} is not embedded"), |(_, t)| *t)
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

/// The quoted values after each `key` (`src="`, `scenario: "`, ...) in `text`.
fn values_after<'a>(text: &'a str, key: &str) -> Vec<&'a str> {
    text.match_indices(key)
        .filter_map(|(i, _)| {
            let rest = &text[i + key.len()..];
            rest.find('"').map(|end| &rest[..end])
        })
        .collect()
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
        let target = rest.split_once("\", ").map(|(_, t)| t).unwrap_or_default();
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

/// The stories (scenario files) the template ships, in the order `kaanal` lists them.
const STORIES: [&str; 15] = [
    "auth-errors",
    "black-friday",
    "brownout",
    "cache-down",
    "cart-write-errors",
    "db-timeout",
    "flaky-network",
    "gateway-down",
    "meltdown",
    "payment-declined",
    "payment-down",
    "payment-flaky",
    "payment-slow",
    "payment-timeout",
    "slow-db",
];

/// The scenario names embedded as `scenarios/NAME.yaml`, sorted.
fn scenario_files() -> Vec<&'static str> {
    let mut v: Vec<&str> = demo::FILES
        .iter()
        .filter_map(|(p, _)| p.strip_prefix("scenarios/")?.strip_suffix(".yaml"))
        .collect();
    v.sort_unstable();
    v
}

/// What the chaos bar sends, read from `app.js`: (ttl, pinned endpoint, pin states).
fn chaos_bar() -> (String, String, Vec<String>) {
    let js = site_file("app.js");
    let one = |key: &str| {
        let v = values_after(js, key);
        assert_eq!(v.len(), 1, "{key}: {v:?}");
        v[0].to_string()
    };
    let all = |key: &str| {
        values_after(js, key)
            .into_iter()
            .map(String::from)
            .collect()
    };
    (
        one("const TTL = \""),
        one("const PIN_ENDPOINT = \""),
        all("pin: \""),
    )
}

#[test]
fn the_chaos_bar_names_what_the_project_has() {
    let (ttl, endpoint, pins) = chaos_bar();
    assert_eq!(ttl, "30s");
    assert_eq!(endpoint, "GET /products");
    assert_eq!(pins, ["empty", "error", "loading"]);
    // Stories are read from the control API, never hard-coded in the page.
    let js = site_file("app.js");
    assert!(js.contains("control(\"GET\", \"/scenarios\")"));
    assert!(!js.contains("scenario: \""));
    assert_eq!(scenario_files(), STORIES);
    // Each story has a one-line description: the widget shows it as the subtitle.
    for (path, text) in demo::FILES
        .iter()
        .filter(|(p, _)| p.starts_with("scenarios/"))
    {
        let first = text.lines().next().unwrap_or_default();
        let d = first
            .strip_prefix("description: \"")
            .and_then(|d| d.strip_suffix('"'));
        assert!(
            d.is_some_and(|d| !d.is_empty() && d.len() <= 100),
            "{path}: {first}"
        );
        assert_eq!(text.matches("description:").count(), 1, "{path}");
    }
}

#[test]
fn the_chaos_bar_requests_are_accepted_by_a_live_up() {
    let dir = project("chaosbar");
    let (_up, port) = up(&dir);
    let run: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join(".kaanal/run.json")).unwrap())
            .unwrap();
    let auth = format!("Bearer {}", run["token"].as_str().unwrap());
    let base = format!("http://127.0.0.1:{port}");
    let ctl = format!("{base}/__kaanal");
    let http = agent();
    let (ttl, endpoint, pins) = chaos_bar();

    // Without the token the bar gets the control API's 401.
    let r = http.get(&format!("{ctl}/scenarios")).call().unwrap();
    assert_eq!(r.status().as_u16(), 401);

    // The widget lists the stories from GET /scenarios, each with its description.
    let mut list = http
        .get(&format!("{ctl}/scenarios"))
        .header("authorization", &auth)
        .call()
        .unwrap();
    let list: serde_json::Value = list.body_mut().read_json().unwrap();
    let rows = list.as_array().unwrap();
    let scenarios: Vec<String> = rows
        .iter()
        .map(|r| r["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(scenarios, STORIES, "{list}");
    assert!(
        rows.iter()
            .all(|r| r["description"].as_str().is_some_and(|d| !d.is_empty())
                && r["active"] == false),
        "{list}"
    );

    // Scenarios: activate with a lease, see it in the poll, deactivate.
    for s in &scenarios {
        let mut r = http
            .post(&format!("{ctl}/scenarios/{s}/activate"))
            .header("authorization", &auth)
            .send_json(serde_json::json!({ "ttl": ttl }))
            .unwrap();
        assert_eq!(r.status().as_u16(), 200, "{s}");
        let body: serde_json::Value = r.body_mut().read_json().unwrap();
        let left = body["expires_in_ms"][s.as_str()].as_u64().unwrap();
        assert!(left > 25_000 && left <= 30_000, "{s}: {left}");
        let mut list = http
            .get(&format!("{ctl}/scenarios"))
            .header("authorization", &auth)
            .call()
            .unwrap();
        let list: serde_json::Value = list.body_mut().read_json().unwrap();
        let row = list
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["name"] == s.as_str())
            .unwrap();
        assert!(
            row["active"] == true && row["expires_in_ms"].is_u64(),
            "{row}"
        );
        let r = http
            .post(&format!("{ctl}/scenarios/{s}/deactivate"))
            .header("authorization", &auth)
            .send_empty()
            .unwrap();
        assert_eq!(r.status().as_u16(), 204);
    }

    // Pins on the products list: set, observe, read back, clear (the page's encoded query).
    let products = format!("{base}/products");
    let clear = format!(
        "{ctl}/pins?endpoint={}",
        endpoint.replace(' ', "%20").replace('/', "%2F")
    );
    for state in &pins {
        let r = http
            .put(&format!("{ctl}/pins"))
            .header("authorization", &auth)
            .send_json(serde_json::json!({ "endpoint": endpoint, "state": state }))
            .unwrap();
        assert_eq!(r.status().as_u16(), 200, "{state}");
        let mut live = http
            .get(&format!("{ctl}/pins"))
            .header("authorization", &auth)
            .call()
            .unwrap();
        let live: serde_json::Value = live.body_mut().read_json().unwrap();
        assert_eq!(live["live"][0]["state"], state.as_str(), "{live}");
        match state.as_str() {
            "empty" => assert_eq!(get_json(&http, &products), serde_json::json!([])),
            "error" => assert_eq!(http.get(&products).call().unwrap().status().as_u16(), 500),
            _ => {} // loading answers after 10 s; not worth the wait here.
        }
        let r = http
            .delete(&clear)
            .header("authorization", &auth)
            .call()
            .unwrap();
        assert_eq!(r.status().as_u16(), 204);
        assert_eq!(get_json(&http, &products).as_array().unwrap().len(), 24);
    }
}

/// The string items of a one-line JS array, `const NAME = ["a", "b"];`.
fn js_list(js: &str, decl: &str) -> Vec<String> {
    let start = js.find(decl).unwrap_or_else(|| panic!("{decl}")) + decl.len();
    let end = start + js[start..].find(']').unwrap();
    js[start..end]
        .split(',')
        .map(|s| s.trim().trim_matches('"').to_string())
        .collect()
}

#[test]
fn the_chaos_sliders_and_reset_are_accepted_by_a_live_up() {
    let js = site_file("app.js");
    let nodes = js_list(js, "const CHAOS_NODES = [");
    let resets = js_list(js, "const RESET_ALL = [");
    let max: u64 = js
        .split("const LATENCY_MAX = ")
        .nth(1)
        .and_then(|r| r.split(';').next())
        .and_then(|n| n.parse().ok())
        .expect("LATENCY_MAX");
    assert_eq!(nodes, ["nginx", "shop", "auth", "db", "cache", "payments"]);
    assert_eq!(resets, ["/chaos", "/scenarios", "/pins"]);
    assert_eq!(max, 2000);
    let (ttl, endpoint, pins) = chaos_bar();

    let dir = project("sliders");
    let (_up, port) = up(&dir);
    let run: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join(".kaanal/run.json")).unwrap())
            .unwrap();
    let auth = format!("Bearer {}", run["token"].as_str().unwrap());
    let base = format!("http://127.0.0.1:{port}");
    let ctl = format!("{base}/__kaanal");
    let products = format!("{base}/products");
    let http = agent();
    let put = |node: &str, latency: u64, rate: f64| {
        let mut r = http
            .put(&format!("{ctl}/chaos/{node}"))
            .header("authorization", &auth)
            .send_json(serde_json::json!({ "latency_ms": latency, "error_rate": rate, "ttl": ttl }))
            .unwrap();
        let status = r.status().as_u16();
        let body: serde_json::Value = r.body_mut().read_json().unwrap();
        assert_eq!(status, 200, "{node} {latency} {rate}: {body}");
        assert!(body["expires_in_ms"]["latency"].is_u64(), "{body}");
    };
    let timed = || {
        let t = Instant::now();
        let status = http.get(&products).call().unwrap().status().as_u16();
        (status, t.elapsed())
    };

    // What the sliders send: each end of both ranges, one at zero with the other set, and a
    // DELETE once both are back at zero.
    for node in &nodes {
        put(node, max, 1.0);
        put(node, 0, 0.5);
        put(node, 50, 0.0);
        let r = http
            .delete(&format!("{ctl}/chaos/{node}"))
            .header("authorization", &auth)
            .call()
            .unwrap();
        assert_eq!(r.status().as_u16(), 204, "{node}");
    }

    // The gateway is on every request, so its sliders show on the product list.
    put("nginx", max, 0.0);
    let (status, took) = timed();
    assert_eq!(status, 200);
    assert!(took >= Duration::from_millis(max), "{took:?}");
    put("nginx", 0, 1.0);
    assert_eq!(timed().0, 503);

    // Everything on at once, then "Reset all chaos".
    put("nginx", max, 0.0);
    let r = http
        .post(&format!("{ctl}/scenarios/{}/activate", STORIES[0]))
        .header("authorization", &auth)
        .send_json(serde_json::json!({ "ttl": ttl }))
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let r = http
        .put(&format!("{ctl}/pins"))
        .header("authorization", &auth)
        .send_json(serde_json::json!({ "endpoint": endpoint, "state": pins[1] }))
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    for path in &resets {
        let r = http
            .delete(&format!("{ctl}{path}"))
            .header("authorization", &auth)
            .call()
            .unwrap();
        assert_eq!(r.status().as_u16(), 204, "{path}");
    }
    let (status, took) = timed();
    assert_eq!(status, 200);
    assert!(took < Duration::from_millis(max * 3 / 4), "{took:?}");
    assert_eq!(get_json(&http, &products).as_array().unwrap().len(), 24);
    let mut chaos = http
        .get(&format!("{ctl}/chaos"))
        .header("authorization", &auth)
        .call()
        .unwrap();
    let chaos: serde_json::Value = chaos.body_mut().read_json().unwrap();
    assert_eq!(
        chaos["effective"]["nodes"],
        serde_json::json!({}),
        "{chaos}"
    );
}

/// What one request met: its status (`None` when the connection broke) and how long it took.
fn hit(http: &ureq::Agent, method: &str, url: &str) -> (Option<u16>, Duration) {
    let t = Instant::now();
    let r = match method {
        "GET" => http.get(url).call(),
        "POST" => http
            .post(url)
            .send_json(serde_json::json!({"productId": 1, "qty": 1, "username": "demo@example.com", "password": "demo"})),
        _ => http.delete(url).call(),
    };
    // A reset can land after the status line, so the body is read too.
    let status = r.ok().and_then(|mut r| {
        let s = r.status().as_u16();
        r.body_mut().read_to_string().ok().map(|_| s)
    });
    (status, t.elapsed())
}

#[test]
#[expect(clippy::too_many_lines, reason = "one walk through every story")]
fn every_story_has_a_visible_effect() {
    let dir = project("stories");
    let (_up, port) = up(&dir);
    let run: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join(".kaanal/run.json")).unwrap())
            .unwrap();
    let auth = format!("Bearer {}", run["token"].as_str().unwrap());
    let base = format!("http://127.0.0.1:{port}");
    let http = agent();
    let only = |name: &str| {
        let r = http
            .put(&format!("{base}/__kaanal/scenarios/active"))
            .header("authorization", &auth)
            .send_json(serde_json::json!([name]))
            .unwrap();
        assert_eq!(r.status().as_u16(), 200, "{name}");
    };
    let list = format!("{base}/products");
    let detail = format!("{base}/products/1");
    let cart = format!("{base}/cart");
    let login = format!("{base}/login");
    let ms = |d: Duration| d.as_millis();
    // Ten tries: (failures, successes), and the slowest success.
    let tally = |method: &str, url: &str, bad: &[Option<u16>]| {
        let (mut fail, mut ok, mut slow) = (0, 0, Duration::ZERO);
        for _ in 0..10 {
            let (s, took) = hit(&http, method, url);
            if bad.contains(&s) {
                fail += 1;
            } else {
                assert!(s.is_some_and(|s| s < 300), "{method} {url}: {s:?}");
                ok += 1;
                slow = slow.max(took);
            }
        }
        (fail, ok, slow)
    };

    // Calm: the list reads the db, the product page hits the warm cache.
    let (s, calm) = hit(&http, "GET", &list);
    assert_eq!(s, Some(200));
    assert!(ms(calm) < 800, "{calm:?}");

    only("slow-db");
    let (s, took) = hit(&http, "GET", &list);
    assert!(s == Some(200) && ms(took) >= 800, "{s:?} {took:?}");
    let (s, took) = hit(&http, "GET", &detail);
    assert!(
        s == Some(200) && ms(took) < 800,
        "the cache keeps product pages fast: {took:?}"
    );
    assert!(ms(hit(&http, "GET", &cart).1) >= 800);

    only("cache-down");
    let (s, took) = hit(&http, "GET", &detail);
    assert!(s == Some(200) && ms(took) >= 250, "{s:?} {took:?}");
    assert!(ms(hit(&http, "GET", &list).1) >= 250);

    only("auth-errors");
    let (fail, ok, _) = tally("POST", &login, &[Some(503)]);
    assert!(fail > 0 && ok > 0, "{fail} failed, {ok} ok");

    only("black-friday");
    let r = http
        .post(&cart)
        .send_json(serde_json::json!({"productId": 1, "qty": 1}))
        .unwrap();
    assert_eq!(r.status().as_u16(), 429);
    let (s, took) = hit(&http, "GET", &list);
    assert!(s == Some(200) && ms(took) >= 600, "{s:?} {took:?}");

    only("flaky-network");
    let (fail, ok, slow) = tally("GET", &list, &[None]);
    assert!(
        fail > 0 && ok > 0 && ms(slow) >= 300,
        "{fail} {ok} {slow:?}"
    );

    only("gateway-down");
    assert_eq!(hit(&http, "GET", &list).0, Some(503));
    assert_eq!(hit(&http, "GET", &detail).0, Some(503));
    assert_eq!(
        hit(&http, "GET", &format!("{base}/")).0,
        Some(200),
        "the page still loads"
    );

    only("db-timeout");
    let (s, took) = hit(&http, "GET", &list);
    assert!(s == Some(504) && ms(took) >= 3000, "{s:?} {took:?}");
    assert_eq!(
        hit(&http, "GET", &detail).0,
        Some(200),
        "served from the cache"
    );

    only("cart-write-errors");
    let (fail, ok, _) = tally("POST", &cart, &[Some(503)]);
    assert!(fail > 0 && ok > 0, "{fail} failed, {ok} ok");
    assert_eq!(hit(&http, "GET", &list).0, Some(200));

    only("brownout");
    let (fail, ok, slow) = tally("GET", &list, &[Some(503)]);
    assert!(
        fail > 0 && ok > 0 && ms(slow) >= 400,
        "{fail} {ok} {slow:?}"
    );

    only("meltdown");
    assert_eq!(hit(&http, "POST", &login).0, Some(503));
    let (fail, ok, slow) = tally("GET", &detail, &[Some(504)]);
    assert!(ok > 0 && ms(slow) >= 1200, "{fail} {ok} {slow:?}");

    // Payments: browsing is untouched, checkout is not.
    let checkout = format!("{base}/checkout");
    only("payment-slow");
    let (s, took) = hit(&http, "POST", &checkout);
    assert!(s == Some(201) && ms(took) >= 1800, "{s:?} {took:?}");
    assert_eq!(hit(&http, "GET", &list).0, Some(200));
    assert!(ms(hit(&http, "GET", &list).1) < 1000);

    only("payment-flaky");
    let (fail, ok, _) = tally("POST", &checkout, &[Some(502)]);
    assert!(fail > 0 && ok > 0, "{fail} failed, {ok} ok");

    only("payment-down");
    assert_eq!(hit(&http, "POST", &checkout).0, Some(503));

    only("payment-timeout");
    let (s, took) = hit(&http, "POST", &checkout);
    assert!(s == Some(504) && ms(took) >= 3000, "{s:?} {took:?}");

    only("payment-declined");
    assert_eq!(hit(&http, "POST", &checkout).0, Some(402));
    assert_eq!(hit(&http, "GET", &list).0, Some(200));
}

#[test]
fn checkout_stores_an_order_only_when_the_payment_goes_through() {
    let dir = project("checkout");
    let (_up, port) = up(&dir);
    let run: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join(".kaanal/run.json")).unwrap())
            .unwrap();
    let auth = format!("Bearer {}", run["token"].as_str().unwrap());
    let base = format!("http://127.0.0.1:{port}");
    let http = agent();
    let orders = format!("{base}/orders");
    let order = serde_json::json!({
        "lines": [{"productId": 1, "name": "Canvas Backpack", "qty": 2, "price": 2499}],
        "total": 4998, "method": "card", "last4": "1111"
    });
    let pay = |expect: u16| {
        let r = http
            .post(&format!("{base}/checkout"))
            .send_json(&order)
            .unwrap();
        assert_eq!(r.status().as_u16(), expect);
    };
    let only = |name: &str| {
        let r = http
            .put(&format!("{base}/__kaanal/scenarios/active"))
            .header("authorization", &auth)
            .send_json(serde_json::json!([name]))
            .unwrap();
        assert_eq!(r.status().as_u16(), 200, "{name}");
    };
    assert_eq!(get_json(&http, &orders), serde_json::json!([]));
    pay(201);
    let list = get_json(&http, &orders);
    assert_eq!(list.as_array().unwrap().len(), 1, "{list}");
    assert_eq!(list[0]["id"], 1);
    assert_eq!(list[0]["total"], 4998);
    assert_eq!(get_json(&http, &format!("{orders}/1"))["last4"], "1111");
    // A refused, failed or declined payment stores nothing.
    for story in ["payment-down", "payment-declined"] {
        only(story);
        pay(if story == "payment-down" { 503 } else { 402 });
        assert_eq!(
            get_json(&http, &orders).as_array().unwrap().len(),
            1,
            "{story}"
        );
    }
    let missing = http.get(&format!("{orders}/99")).call().unwrap();
    assert_eq!(missing.status().as_u16(), 404);
}

// ---- `kaanal demo` itself ----

/// An empty folder to run `kaanal demo` in; `name` may hold spaces and non-ASCII.
fn empty(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("kaanal-demo-run-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// The names in `dir`, sorted.
fn entries(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

/// A running `kaanal demo`: its banner (stdout up to the `Stop:` line, or the JSON object) and
/// its port. Killed on drop, then its folder and its stderr file removed.
struct Demo {
    child: Child,
    dir: PathBuf,
    err: PathBuf,
    out: String,
    port: u16,
}

impl Drop for Demo {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
        let _ = std::fs::remove_file(&self.err);
    }
}

impl Demo {
    fn stderr(&self) -> String {
        std::fs::read_to_string(&self.err).unwrap_or_default()
    }

    /// `kaanal down` from the same folder; the demo then exits 0.
    fn down(&mut self) {
        let o = run(&self.dir, &["down"]);
        assert_eq!(
            o.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&o.stderr)
        );
        let status = self.child.wait().unwrap();
        assert_eq!(status.code(), Some(0), "{}", self.stderr());
    }
}

/// Starts `kaanal [pre...] demo [args...]` in `dir` and reads its banner. Its stderr goes to a
/// file beside `dir`, not in it.
fn demo_with(dir: &Path, pre: &[&str], args: &[&str]) -> Demo {
    use std::io::BufRead;
    let err = PathBuf::from(format!("{}.err", dir.display()));
    let mut child = bin()
        .args(pre)
        .arg("demo")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(std::fs::File::create(&err).unwrap())
        .current_dir(dir)
        .spawn()
        .unwrap();
    let mut lines = std::io::BufReader::new(child.stdout.take().unwrap()).lines();
    let mut d = Demo {
        child,
        dir: dir.to_path_buf(),
        err,
        out: String::new(),
        port: 0,
    };
    for line in lines.by_ref() {
        let line = line.unwrap();
        d.out.push_str(&line);
        d.out.push('\n');
        if let Some(rest) = line.strip_prefix("Demo:") {
            let rest = rest.trim().strip_prefix("http://localhost:").unwrap();
            d.port = rest[..rest.find('/').unwrap()].parse().unwrap();
        }
        if line.starts_with('{') {
            let j: serde_json::Value = serde_json::from_str(&line).unwrap();
            d.port = u16::try_from(j["port"].as_u64().unwrap()).unwrap();
            break;
        }
        if line.starts_with("Stop:") {
            break;
        }
    }
    assert!(d.port != 0, "demo did not start: {}{}", d.out, d.stderr());
    // Keep the pipe drained so the demo never blocks on a write.
    std::thread::spawn(move || lines.for_each(drop));
    d
}

fn demo_in(dir: &Path, args: &[&str]) -> Demo {
    demo_with(dir, &[], args)
}

fn status_of(url: &str) -> u16 {
    agent().get(url).call().unwrap().status().as_u16()
}

/// The token of the instance running in `dir`.
fn run_token(dir: &Path) -> String {
    let run: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join(".kaanal/run.json")).unwrap())
            .unwrap();
    run["token"].as_str().unwrap().to_string()
}

#[test]
fn demo_serves_the_storefront_and_leaves_only_dot_kaanal() {
    let mut d = demo_in(&empty("serves"), &["--port", "0"]);
    let token = run_token(&d.dir);
    // One URL with the token in the fragment, the seed and locale, how to stop, one story.
    let out = d.out.clone();
    assert!(
        out.contains(&format!(
            "Demo:  http://localhost:{}/#token={token}",
            d.port
        )),
        "{out}"
    );
    assert!(out.contains("Seed:  7") && out.contains("en-IN"), "{out}");
    assert!(
        out.contains("Ctrl-C") && out.contains("kaanal down"),
        "{out}"
    );
    assert!(out.contains("kaanal scenario activate slow-db"), "{out}");
    let base = format!("http://127.0.0.1:{}", d.port);
    let page = agent()
        .get(&format!("{base}/"))
        .call()
        .unwrap()
        .body_mut()
        .read_to_string()
        .unwrap();
    assert_eq!(page, site_file("index.html"));
    assert_eq!(status_of(&format!("{base}/products")), 200);
    d.down();
    assert_eq!(entries(&d.dir), [".kaanal"]);
    assert!(d.dir.join(".kaanal/demo/kaanal.yaml").exists());
}

#[test]
fn run_json_is_removed_after_stop() {
    let mut d = demo_with(&empty("json"), &["--json"], &["--port", "0"]);
    let token = run_token(&d.dir);
    let line = d.out.lines().last().unwrap().to_string();
    let j: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert!(
        !line.contains(&token),
        "the object never holds the token: {line}"
    );
    assert_eq!(j["locale"], "en-IN", "{j}");
    assert_eq!(j["url"], format!("http://localhost:{}/", d.port), "{j}");
    assert_eq!(j["seed"], 7, "{j}");
    assert!(d.dir.join(".kaanal/run.json").exists());
    d.down();
    assert!(!d.dir.join(".kaanal/run.json").exists());
}

#[test]
fn a_second_demo_exits_64_and_leaves_the_files_alone() {
    let d = demo_in(&empty("twice"), &["--port", "0"]);
    let page = d.dir.join(".kaanal/demo/site/index.html");
    std::fs::write(&page, "edited").unwrap();
    let o = run(&d.dir, &["demo", "--port", "0", "--locale", "de-DE"]);
    let err = String::from_utf8_lossy(&o.stderr);
    assert_eq!(o.status.code(), Some(64), "{err}");
    assert!(err.contains("already running"), "{err}");
    assert_eq!(std::fs::read_to_string(&page).unwrap(), "edited");
    let cfg = std::fs::read_to_string(d.dir.join(".kaanal/demo/kaanal.yaml")).unwrap();
    assert!(cfg.contains("locale: en-IN"), "{cfg}");
    assert_eq!(
        status_of(&format!("http://127.0.0.1:{}/products", d.port)),
        200
    );
}

#[test]
fn demo_next_to_a_kaanal_yaml_notes_it_and_works() {
    let dir = empty("beside");
    let mine = "not: [a, valid, project\n";
    std::fs::write(dir.join("kaanal.yaml"), mine).unwrap();
    let d = demo_in(&dir, &["--port", "0"]);
    assert_eq!(
        status_of(&format!("http://127.0.0.1:{}/products", d.port)),
        200
    );
    assert!(d.stderr().contains("kaanal.yaml"), "{}", d.stderr());
    assert_eq!(
        std::fs::read_to_string(dir.join("kaanal.yaml")).unwrap(),
        mine
    );
}

#[test]
fn kaanal_chaos_from_the_same_folder_changes_the_demo() {
    let d = demo_in(&empty("chaos"), &["--port", "0"]);
    let products = format!("http://127.0.0.1:{}/products", d.port);
    assert_eq!(status_of(&products), 200);
    let o = run(&d.dir, &["chaos", "set", "nginx", "--down"]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert_eq!(status_of(&products), 503);
}

#[test]
fn locale_flag_is_written_into_the_demo_config() {
    // The products are fixed examples, so the locale shows in the config and the banner.
    let d = demo_in(&empty("locale"), &["--port", "0", "--locale", "de-DE"]);
    let cfg = std::fs::read_to_string(d.dir.join(".kaanal/demo/kaanal.yaml")).unwrap();
    assert!(cfg.lines().any(|l| l == "locale: de-DE"), "{cfg}");
    assert!(!cfg.contains("en-IN"), "{cfg}");
    assert!(d.out.contains("de-DE"), "{}", d.out);
}

#[test]
fn an_unknown_locale_is_a_usage_error_64() {
    let dir = empty("badlocale");
    let _clean = Cleanup(dir.clone());
    for bad in ["xx-XX", "../x", "en-us"] {
        let o = run(&dir, &["demo", "--port", "0", "--locale", bad]);
        let err = String::from_utf8_lossy(&o.stderr);
        assert_eq!(o.status.code(), Some(64), "{bad}: {err}");
        assert!(err.contains("en-US"), "lists the packs: {err}");
        assert!(entries(&dir).is_empty(), "{bad}: wrote {:?}", entries(&dir));
    }
}

#[test]
fn a_busy_default_port_falls_back_and_prints_the_real_url() {
    // Whoever holds 8080 (this test, or something else on the machine), the demo moves on.
    let _hold = std::net::TcpListener::bind("127.0.0.1:8080");
    let d = demo_in(&empty("busy"), &[]);
    assert_ne!(d.port, 8080, "{}", d.out);
    assert_eq!(status_of(&format!("http://127.0.0.1:{}/", d.port)), 200);
}

#[test]
fn demo_works_in_a_folder_with_spaces_and_non_ascii() {
    let mut d = demo_in(&empty("sp\u{e4}ce \u{65e5}\u{672c}"), &["--port", "0"]);
    let base = format!("http://127.0.0.1:{}", d.port);
    assert_eq!(status_of(&format!("{base}/")), 200);
    assert_eq!(status_of(&format!("{base}/cart")), 200);
    d.down();
    assert_eq!(entries(&d.dir), [".kaanal"]);
}

#[test]
fn demo_has_no_host_or_detach_flag() {
    let dir = empty("flags");
    let _clean = Cleanup(dir.clone());
    for flags in [
        &["--host", "0.0.0.0"][..],
        &["-d"],
        &["--detach"],
        &["--detached"],
        &["--wait", "1s"],
    ] {
        let o = bin()
            .arg("demo")
            .args(flags)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert_eq!(o.status.code(), Some(64), "{flags:?}");
        assert!(entries(&dir).is_empty(), "{flags:?}");
    }
    let help = run(&dir, &["demo", "--help"]);
    let help = String::from_utf8_lossy(&help.stdout);
    assert!(
        help.contains("--port") && help.contains("--locale") && help.contains("--open"),
        "{help}"
    );
    assert!(!help.contains("--host"), "{help}");
}

/// A link from `link` to `target`: a symlink on Unix, a junction on Windows (no privilege needed).
fn link_dir(link: &Path, target: &Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).is_ok()
    }
    #[cfg(windows)]
    {
        Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .is_ok_and(|o| o.status.success())
    }
}

#[test]
fn demo_refuses_to_write_through_a_linked_folder() {
    for (name, linked) in [("linkdemo", "demo"), ("linkstate", "")] {
        let dir = empty(name);
        let outside = empty(&format!("{name}-outside"));
        std::fs::write(outside.join("keep.txt"), "mine").unwrap();
        let ok = if linked.is_empty() {
            link_dir(&dir.join(".kaanal"), &outside)
        } else {
            std::fs::create_dir_all(dir.join(".kaanal")).unwrap();
            link_dir(&dir.join(".kaanal").join(linked), &outside)
        };
        if !ok {
            continue; // links cannot be made here
        }
        let o = run(&dir, &["demo", "--port", "0"]);
        let err = String::from_utf8_lossy(&o.stderr);
        assert_eq!(o.status.code(), Some(1), "{name}: {err}");
        assert!(err.contains("symbolic link"), "{name}: {err}");
        let mut left: Vec<_> = std::fs::read_dir(&outside)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        left.sort();
        assert_eq!(left, ["keep.txt"], "{name}: wrote through the link");
    }
}

// ---- `kaanal demo --eject` ----

/// The relative paths of every file under `dir`, sorted.
fn files_under(dir: &Path) -> Vec<String> {
    fn walk(base: &Path, at: &Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(at).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(base, &p, out);
            } else {
                let rel = p.strip_prefix(base).unwrap();
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let mut v = Vec::new();
    walk(dir, dir, &mut v);
    v.sort();
    v
}

fn template_paths() -> Vec<String> {
    let mut v: Vec<String> = demo::FILES.iter().map(|(p, _)| (*p).to_string()).collect();
    v.sort();
    v
}

/// What `--eject` leaves: the template plus the `.gitignore` entry for `.kaanal/`.
fn ejected_paths() -> Vec<String> {
    let mut v = template_paths();
    v.push(".gitignore".to_string());
    v.sort();
    v
}

#[test]
fn eject_writes_the_project_and_starts_no_server() {
    let work = empty("eject-basic");
    let _clean = Cleanup(work.clone());
    let o = run(&work, &["demo", "--eject"]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let dir = work.join("kaanal-demo");
    assert_eq!(files_under(&dir), ejected_paths());
    let ignore = std::fs::read_to_string(dir.join(".gitignore")).unwrap();
    assert!(ignore.lines().any(|l| l == ".kaanal/"), "{ignore}");
    for (path, text) in demo::FILES {
        let got = std::fs::read_to_string(dir.join(path)).unwrap();
        assert_eq!(got, *text, "{path}");
    }
    // It returned (a server would still be running) and left no state beside the project.
    assert_eq!(entries(&work), ["kaanal-demo"]);
    assert_eq!(run(&dir, &["validate"]).status.code(), Some(0));
}

#[test]
fn eject_refuses_a_non_empty_directory_without_force() {
    let work = empty("eject-busy");
    let _clean = Cleanup(work.clone());
    let dir = work.join("mine");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("notes.txt"), "keep").unwrap();
    let o = run(&work, &["demo", "--eject", "mine"]);
    let err = String::from_utf8_lossy(&o.stderr);
    assert_eq!(o.status.code(), Some(64), "{err}");
    assert!(
        err.contains("not empty") && err.contains("--force"),
        "{err}"
    );
    assert_eq!(entries(&dir), ["notes.txt"]);
}

#[test]
fn eject_into_an_empty_existing_directory_works() {
    let work = empty("eject-emptydir");
    let _clean = Cleanup(work.clone());
    std::fs::create_dir_all(work.join("out")).unwrap();
    let o = run(&work, &["demo", "--eject", "out"]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert_eq!(files_under(&work.join("out")), ejected_paths());
}

#[test]
fn ejected_project_serves_the_same_storefront_under_plain_up() {
    let work = empty("eject-up");
    let _clean = Cleanup(work.clone());
    let o = run(&work, &["demo", "--eject", "shop"]);
    assert_eq!(o.status.code(), Some(0));
    let dir = work.join("shop");
    let (_up, port) = up(&dir);
    let base = format!("http://127.0.0.1:{port}");
    let http = agent();
    let page = http
        .get(&format!("{base}/"))
        .call()
        .unwrap()
        .body_mut()
        .read_to_string()
        .unwrap();
    assert_eq!(page, site_file("index.html"));
    let products = get_json(&http, &format!("{base}/products"));
    assert_eq!(products.as_array().unwrap().len(), 24);
    let cart = format!("{base}/cart");
    let mut added = http
        .post(&cart)
        .send_json(serde_json::json!({"productId": 1, "qty": 2}))
        .unwrap();
    assert_eq!(added.status().as_u16(), 201);
    let item: serde_json::Value = added.body_mut().read_json().unwrap();
    assert_eq!(get_json(&http, &cart).as_array().unwrap().len(), 1);
    let id = item["id"].as_u64().unwrap();
    let removed = http.delete(&format!("{cart}/{id}")).call().unwrap();
    assert_eq!(removed.status().as_u16(), 204);
    let mut list = http
        .get(&format!("{base}/__kaanal/scenarios"))
        .header("authorization", &format!("Bearer {}", run_token(&dir)))
        .call()
        .unwrap();
    let list: serde_json::Value = list.body_mut().read_json().unwrap();
    let names: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, STORIES);
}

#[test]
fn eject_writes_the_chosen_locale() {
    let work = empty("eject-locale");
    let _clean = Cleanup(work.clone());
    let o = run(&work, &["demo", "--eject", "de", "--locale", "de-DE"]);
    assert_eq!(o.status.code(), Some(0));
    let cfg = std::fs::read_to_string(work.join("de/kaanal.yaml")).unwrap();
    assert!(cfg.lines().any(|l| l == "locale: de-DE"), "{cfg}");
    assert!(!cfg.contains("en-IN"), "{cfg}");
    // Without --locale the template's en-IN stays.
    run(&work, &["demo", "--eject", "en"]);
    let cfg = std::fs::read_to_string(work.join("en/kaanal.yaml")).unwrap();
    assert!(cfg.lines().any(|l| l == "locale: en-IN"), "{cfg}");
    // An unknown tag is a usage error and creates nothing.
    let o = run(&work, &["demo", "--eject", "bad", "--locale", "xx-XX"]);
    assert_eq!(o.status.code(), Some(64));
    assert!(!work.join("bad").exists());
}

#[test]
fn eject_prints_the_next_command() {
    let work = empty("eject-next");
    let _clean = Cleanup(work.clone());
    let o = run(&work, &["demo", "--eject"]);
    let out = String::from_utf8_lossy(&o.stdout);
    let root = std::path::absolute(work.join("kaanal-demo")).unwrap();
    assert!(out.contains(&root.display().to_string()), "{out}");
    assert!(out.contains("cd kaanal-demo && kaanal up"), "{out}");
    // A directory with spaces is quoted.
    let o = run(&work, &["demo", "--eject", "my shop"]);
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(out.contains("cd \"my shop\" && kaanal up"), "{out}");
    // --json: one object with the root, the files and the command.
    let o = run(&work, &["--json", "demo", "--eject", "j"]);
    let j: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(
        j["root"],
        std::path::absolute(work.join("j"))
            .unwrap()
            .display()
            .to_string()
    );
    assert_eq!(j["next"], "cd j && kaanal up");
    let mut written: Vec<String> = j["written"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    written.sort();
    assert_eq!(written, ejected_paths());
}

#[test]
fn eject_while_a_demo_runs_works() {
    let d = demo_in(&empty("eject-running"), &["--port", "0"]);
    let o = run(&d.dir, &["demo", "--eject", "copy"]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert_eq!(files_under(&d.dir.join("copy")), ejected_paths());
    assert_eq!(
        status_of(&format!("http://127.0.0.1:{}/products", d.port)),
        200
    );
}

#[test]
fn force_without_eject_is_usage_error_64() {
    let work = empty("eject-force-alone");
    let _clean = Cleanup(work.clone());
    let o = run(&work, &["demo", "--force"]);
    assert_eq!(o.status.code(), Some(64));
    // --port and --open make no sense when nothing runs.
    for flag in [&["--port", "0"][..], &["--open"]] {
        let mut args = vec!["demo", "--eject"];
        args.extend_from_slice(flag);
        let o = run(&work, &args);
        assert_eq!(o.status.code(), Some(64), "{flag:?}");
    }
    assert_eq!(std::fs::read_dir(&work).unwrap().count(), 0);
}

#[test]
fn eject_does_not_create_dot_kaanal() {
    let work = empty("eject-nostate");
    let _clean = Cleanup(work.clone());
    run(&work, &["demo", "--eject", "x"]);
    assert_eq!(entries(&work), ["x"]);
    assert!(!work.join("x/.kaanal").exists());
}

#[test]
fn eject_force_keeps_foreign_files_and_overwrites_template_files() {
    let work = empty("eject-force");
    let _clean = Cleanup(work.clone());
    let dir = work.join("p");
    std::fs::create_dir_all(dir.join("site")).unwrap();
    std::fs::write(dir.join("mine.txt"), "mine").unwrap();
    std::fs::write(dir.join("site/extra.js"), "extra").unwrap();
    std::fs::write(dir.join("kaanal.yaml"), "edited").unwrap();
    let o = run(&work, &["demo", "--eject", "p", "--force"]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("mine.txt")).unwrap(),
        "mine"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("site/extra.js")).unwrap(),
        "extra"
    );
    let cfg = std::fs::read_to_string(dir.join("kaanal.yaml")).unwrap();
    assert!(cfg.contains("locale: en-IN"), "{cfg}");
    let mut want = ejected_paths();
    want.extend(["mine.txt".to_string(), "site/extra.js".to_string()]);
    want.sort();
    assert_eq!(files_under(&dir), want);
}

#[test]
fn eject_through_a_linked_dir_is_refused() {
    let work = empty("eject-link");
    let _clean = Cleanup(work.clone());
    let outside = empty("eject-link-outside");
    let _clean2 = Cleanup(outside.clone());
    if !link_dir(&work.join("l"), &outside) {
        return; // links cannot be made here
    }
    for force in [&[][..], &["--force"]] {
        let mut args = vec!["demo", "--eject", "l"];
        args.extend_from_slice(force);
        let o = run(&work, &args);
        let err = String::from_utf8_lossy(&o.stderr);
        assert_eq!(o.status.code(), Some(1), "{err}");
        assert!(err.contains("symbolic link"), "{err}");
        assert!(entries(&outside).is_empty(), "wrote through the link");
    }
}

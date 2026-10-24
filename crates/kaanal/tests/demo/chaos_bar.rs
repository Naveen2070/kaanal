//! The chaos bar: what it sends is accepted by a live `kaanal up`.

use crate::common::*;
use crate::demo;

use std::time::{Duration, Instant};

/// The scenario names embedded as `scenarios/NAME.yaml`, sorted.
pub(crate) fn scenario_files() -> Vec<&'static str> {
    let mut v: Vec<&str> = demo::FILES
        .iter()
        .filter_map(|(p, _)| p.strip_prefix("scenarios/")?.strip_suffix(".yaml"))
        .collect();
    v.sort_unstable();
    v
}

/// What the chaos bar sends, read from `app.js`: (ttl, pinned endpoint, pin states).
pub(crate) fn chaos_bar() -> (String, String, Vec<String>) {
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
pub(crate) fn js_list(js: &str, decl: &str) -> Vec<String> {
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

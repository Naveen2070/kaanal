//! Every story has a visible effect, and checkout stores an order only when the payment goes through.

use crate::common::*;

use std::time::{Duration, Instant};

/// What one request met: its status (`None` when the connection broke) and how long it took.
pub(crate) fn hit(http: &ureq::Agent, method: &str, url: &str) -> (Option<u16>, Duration) {
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

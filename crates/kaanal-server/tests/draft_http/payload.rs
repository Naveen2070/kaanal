//! `/__kaanal/payload`: live response edits (step 8A, RULES 108).

use crate::common::*;
use reqwest::Method;
use serde_json::{Value, json};

const PAYLOAD: &str = "/__kaanal/payload";
const USER_ID: &str = "/__kaanal/payload?route=GET%20%2Fusers%2F%7Bid%7D";

async fn put(s: &Srv, body: Value) -> reqwest::Response {
    s.send(Method::PUT, PAYLOAD, Some(body)).await
}

async fn mock(s: &Srv, path: &str) -> reqwest::Response {
    s.client
        .get(format!("{}{path}", s.base))
        .send()
        .await
        .unwrap()
}

fn with_collection() -> String {
    format!(
        "{}endpoints:\n  \"GET /users\": {{ collection: people }}\n",
        system("1ms")
    )
}

#[tokio::test]
async fn put_then_the_route_answers_the_edit_and_get_lists_it() {
    let s = start(&system("1ms")).await;
    let r = put(
        &s,
        json!({ "route": "GET /users/{id}", "status": 503, "body": { "down": true }, "headers": { "x-demo": "1" } }),
    )
    .await;
    assert_eq!(r.status(), 204);
    let r = mock(&s, "/users/1").await;
    assert_eq!(r.status(), 503);
    assert_eq!(r.headers()["x-demo"], "1");
    assert_eq!(r.json::<Value>().await.unwrap(), json!({ "down": true }));
    let list: Value = s.get(PAYLOAD).await.json().await.unwrap();
    assert_eq!(
        list,
        json!({ "edits": [ { "route": "GET /users/{id}", "status": 503, "body": { "down": true }, "headers": { "x-demo": "1" }, "content_type": null, "encoding": null, "once": false } ] })
    );
}

#[tokio::test]
async fn a_text_one_shot_edit_is_sent_as_is_once_and_then_leaves_the_list() {
    let s = start(&system("1ms")).await;
    let r = put(
        &s,
        json!({ "route": "GET /users/{id}", "body": "<h1>hi</h1>", "content_type": "text/html", "once": true }),
    )
    .await;
    assert_eq!(r.status(), 204);
    let r = mock(&s, "/users/1").await;
    assert_eq!(r.headers()["content-type"], "text/html");
    assert_eq!(r.text().await.unwrap(), "<h1>hi</h1>");
    let again = mock(&s, "/users/1").await;
    assert_eq!(again.status(), 200);
    assert_ne!(again.headers()["content-type"], "text/html");
    let list: Value = s.get(PAYLOAD).await.json().await.unwrap();
    assert_eq!(list, json!({ "edits": [] }));
    let r = put(
        &s,
        json!({ "route": "GET /users/{id}", "body": { "a": 1 }, "content_type": "text/html" }),
    )
    .await;
    assert_eq!(r.status(), 422);
    let body: Value = r.json().await.unwrap();
    assert_eq!(
        body["field"], "body",
        "a body-type problem is said under the body"
    );
}

#[tokio::test]
async fn an_unknown_route_is_404() {
    let s = start(&system("1ms")).await;
    let r = put(&s, json!({ "route": "GET /nope", "status": 200 })).await;
    assert_eq!(r.status(), 404);
}

#[tokio::test]
async fn a_collection_route_is_422_naming_the_route_field() {
    let s = start(&with_collection()).await;
    let r = put(&s, json!({ "route": "GET /users", "status": 200 })).await;
    assert_eq!(r.status(), 422);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["field"], "route");
    assert!(v["error"].as_str().unwrap().contains("collection"));
}

#[tokio::test]
async fn bad_values_are_422_naming_the_field() {
    let s = start(&system("1ms")).await;
    let cases = [
        (
            json!({ "route": "GET /users/{id}", "status": 99 }),
            "status",
        ),
        (
            json!({ "route": "GET /users/{id}", "status": 204, "body": { "a": 1 } }),
            "status",
        ),
        (
            json!({ "route": "GET /users/{id}", "headers": { "content-length": "1" } }),
            "headers",
        ),
        (
            json!({ "route": "GET /users/{id}", "headers": { "x-a": "a\r\nx-b: 1" } }),
            "headers",
        ),
    ];
    for (body, field) in cases {
        let r = put(&s, body.clone()).await;
        assert_eq!(r.status(), 422, "{body}");
        let v: Value = r.json().await.unwrap();
        assert_eq!(v["field"], field, "{body}");
    }
    let list: Value = s.get(PAYLOAD).await.json().await.unwrap();
    assert_eq!(list, json!({ "edits": [] }));
}

#[tokio::test]
async fn a_body_over_the_cap_is_413_and_malformed_json_is_400() {
    let s = start(&system("1ms")).await;
    let big = format!(
        "{{\"route\":\"GET /users/{{id}}\",\"body\":\"{}\"}}",
        "a".repeat((1 << 20) + 8192)
    );
    let r = s
        .client
        .put(format!("{}{PAYLOAD}", s.base))
        .bearer_auth(&s.token)
        .header("content-type", "application/json")
        .body(big)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 413);
    let r = s
        .client
        .put(format!("{}{PAYLOAD}", s.base))
        .bearer_auth(&s.token)
        .body("not json")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
}

#[tokio::test]
async fn delete_removes_the_edit_then_404() {
    let s = start(&system("1ms")).await;
    put(&s, json!({ "route": "GET /users/{id}", "status": 503 })).await;
    assert_eq!(s.send(Method::DELETE, USER_ID, None).await.status(), 204);
    assert_eq!(mock(&s, "/users/1").await.status(), 200);
    assert_eq!(s.send(Method::DELETE, USER_ID, None).await.status(), 404);
}

#[tokio::test]
async fn preview_is_what_the_route_answers_without_the_edit() {
    let s = start(&system("1ms")).await;
    let q = "/__kaanal/payload/preview?route=GET%20%2Fusers%2F%7Bid%7D";
    let before: Value = s.get(q).await.json().await.unwrap();
    assert_eq!(before["status"], 200);
    assert!(before["body"].is_object());
    put(
        &s,
        json!({ "route": "GET /users/{id}", "status": 503, "body": 1 }),
    )
    .await;
    let after: Value = s.get(q).await.json().await.unwrap();
    assert_eq!(before, after, "an edit never shows in the preview");
    let gone = s.get("/__kaanal/payload/preview?route=GET%20%2Fnope").await;
    assert_eq!(gone.status(), 404);
}

#[tokio::test]
async fn the_routes_need_the_token() {
    let s = start(&system("1ms")).await;
    for (m, p) in [
        (Method::GET, "/__kaanal/payload"),
        (Method::PUT, "/__kaanal/payload"),
        (Method::DELETE, "/__kaanal/payload?route=GET%20%2Fx"),
        (Method::GET, "/__kaanal/payload/preview?route=GET%20%2Fx"),
    ] {
        let r = s
            .client
            .request(m.clone(), format!("{}{p}", s.base))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 401, "{m} {p}");
    }
}

#[tokio::test]
async fn each_put_and_delete_marks_the_live_timeline() {
    let s = start(&system("1ms")).await;
    let mut stream = s
        .client
        .get(format!(
            "{}/__kaanal/live?fresh=1&token={}",
            s.base, s.token
        ))
        .send()
        .await
        .unwrap();
    let mut seen = String::new();
    let mut read_until = async |needle: &str| {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while let Some(chunk) = stream.chunk().await.unwrap() {
                seen.push_str(&String::from_utf8_lossy(&chunk));
                if seen.contains(needle) {
                    return true;
                }
            }
            false
        })
        .await
        .unwrap_or(false)
    };
    assert!(read_until("event: hello").await, "no hello");
    put(&s, json!({ "route": "GET /users/{id}", "status": 503 })).await;
    assert!(read_until("GET /users/{id} answers 503").await, "no marker");
    s.send(Method::DELETE, USER_ID, None).await;
    assert!(
        read_until("GET /users/{id} back to normal").await,
        "no marker"
    );
}

#[tokio::test]
async fn apply_prunes_an_edit_of_a_route_that_became_a_collection_and_says_so() {
    let s = start(&system("1ms")).await;
    put(&s, json!({ "route": "GET /users", "status": 503 })).await;
    let text = with_collection();
    assert_eq!(
        s.put("/__kaanal/draft", json!({ "text": text }))
            .await
            .status(),
        200
    );
    let r = s.send(Method::POST, "/__kaanal/draft/apply", None).await;
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["payloads_dropped"], json!(["GET /users"]));
    let list: Value = s.get(PAYLOAD).await.json().await.unwrap();
    assert_eq!(list, json!({ "edits": [] }));
}

/// A server whose `endpoints:` is the file `routes.yaml` (with a comment the splice must keep).
async fn start_with_endpoints_file(routes: &str) -> Srv {
    let dir = new_dir();
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/openapi.yaml");
    std::fs::copy(fixture, dir.join("openapi.yaml")).unwrap();
    std::fs::write(dir.join("routes.yaml"), routes).unwrap();
    let text = format!("{}endpoints: routes.yaml\n", system("1ms"));
    start_at(dir.clone(), &dir, &text, &text).await
}

#[tokio::test]
async fn stage_file_writes_the_edit_into_the_endpoints_file_and_keeps_its_comments() {
    let routes = "# the routes\n\"GET /users/{id}\": { chain: [ db ] } # users\n";
    let s = start_with_endpoints_file(routes).await;
    let r = s
        .send(
            Method::POST,
            "/__kaanal/payload/stage-file",
            Some(json!({ "route": "get /users/{id}", "status": 503, "body": { "down": true } })),
        )
        .await;
    assert_eq!(r.status(), 200, "{:?}", r.text().await);
    let on_disk = std::fs::read_to_string(s.dir.join("routes.yaml")).unwrap();
    assert!(
        on_disk.contains("# the routes") && on_disk.contains("# users"),
        "{on_disk}"
    );
    assert!(
        on_disk.contains("override") && on_disk.contains("503"),
        "{on_disk}"
    );
    // an Apply reads the file: the route now answers the staged edit with no live edit at all
    let r = s.send(Method::POST, "/__kaanal/draft/apply", None).await;
    assert_eq!(r.status(), 200, "{:?}", r.text().await);
    let r = mock(&s, "/users/1").await;
    assert_eq!(r.status(), 503);
    // a second stage replaces the override instead of adding another
    let r = s
        .send(
            Method::POST,
            "/__kaanal/payload/stage-file",
            Some(json!({ "route": "GET /users/{id}", "status": 202, "content_type": "text/plain", "body": "ok" })),
        )
        .await;
    assert_eq!(r.status(), 200);
    let on_disk = std::fs::read_to_string(s.dir.join("routes.yaml")).unwrap();
    assert_eq!(on_disk.matches("override").count(), 1, "{on_disk}");
    assert!(
        on_disk.contains("202") && !on_disk.contains("503"),
        "{on_disk}"
    );
}

#[tokio::test]
async fn stage_file_adds_a_missing_entry_and_refuses_inline_endpoints_and_bad_edits() {
    let s = start_with_endpoints_file("\"GET /other\": { chain: [ db ] }\n").await;
    let r = s
        .send(
            Method::POST,
            "/__kaanal/payload/stage-file",
            Some(json!({ "route": "GET /users/{id}", "status": 200 })),
        )
        .await;
    assert_eq!(r.status(), 200, "{:?}", r.text().await);
    let on_disk = std::fs::read_to_string(s.dir.join("routes.yaml")).unwrap();
    assert!(
        on_disk.contains("GET /other") && on_disk.contains("GET /users/{id}"),
        "{on_disk}"
    );
    // a refused value writes nothing
    let before = on_disk;
    let r = s
        .send(
            Method::POST,
            "/__kaanal/payload/stage-file",
            Some(json!({ "route": "GET /users/{id}", "status": 99 })),
        )
        .await;
    assert_eq!(r.status(), 422);
    assert_eq!(
        std::fs::read_to_string(s.dir.join("routes.yaml")).unwrap(),
        before
    );
    let r = s
        .send(
            Method::POST,
            "/__kaanal/payload/stage-file",
            Some(json!({ "route": "GET /nope" })),
        )
        .await;
    assert_eq!(r.status(), 404);
    // inline endpoints: stage through the draft, not here
    let inline = start(&system("1ms")).await;
    let r = inline
        .send(
            Method::POST,
            "/__kaanal/payload/stage-file",
            Some(json!({ "route": "GET /users/{id}" })),
        )
        .await;
    assert_eq!(r.status(), 409);
}

#[tokio::test]
async fn the_instance_table_is_served_whole_and_sorted() {
    let s = start(&system("1ms")).await;
    let r = s.get("/__kaanal/instances").await;
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    let types = v["types"].as_array().unwrap();
    assert!(types.len() > 3000, "{} types", types.len());
    assert_eq!(types.len(), kaanal_core::instances::all().len());
    let m5 = types.iter().find(|t| t[0] == "aws/m5.large").unwrap();
    assert_eq!(m5, &json!(["aws/m5.large", 2, 8192, 10000, 96_000]));
    let names: Vec<&str> = types.iter().map(|t| t[0].as_str().unwrap()).collect();
    assert!(names.windows(2).all(|w| w[0] < w[1]));
}

#[tokio::test]
async fn price_regions_and_one_regions_prices_are_served() {
    let s = start(&system("1ms")).await;
    let v: Value = s.get("/__kaanal/prices").await.json().await.unwrap();
    assert_eq!(
        v["defaults"],
        json!({ "aws": "us-east-1", "azure": "us-east", "gcp": "us-central1" })
    );
    for p in ["aws", "azure", "gcp"] {
        assert!(v["regions"][p].as_array().unwrap().len() >= 20, "{p}");
    }
    let r = s.get("/__kaanal/prices/aws/eu-west-1").await;
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    let m5 = v["prices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t[0] == "aws/m5.large")
        .unwrap();
    assert_eq!(m5, &json!(["aws/m5.large", 107_000]));
    assert_eq!(s.get("/__kaanal/prices/aws/nowhere").await.status(), 404);
    assert_eq!(
        s.get("/__kaanal/prices/oracle/us-east-1").await.status(),
        404
    );
}

#[tokio::test]
async fn object_store_prices_and_the_managed_class_lists_are_served() {
    let s = start(&system("1ms")).await;
    let v: Value = s
        .get("/__kaanal/storage-prices")
        .await
        .json()
        .await
        .unwrap();
    let rows = v["rows"].as_array().unwrap();
    // S3 Standard in us-east-1: $0.023 a GB-month, $0.005 per 1,000 writes, $0.0004 per 1,000 reads
    assert!(rows.contains(&json!(["aws", "us-east-1", 23_000, 5_000, 400])));
    assert!(rows.iter().any(|r| r[0] == "azure" && r[1] == "us-east"));
    assert!(rows.contains(&json!(["gcp", "us-central1", 20_000, 5_000, 400])));
    // a managed database's list is served like any region's
    let r = s.get("/__kaanal/prices/aws-rds-postgresql/us-east-1").await;
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    assert!(
        v["prices"]
            .as_array()
            .unwrap()
            .contains(&json!(["aws/db.m5.large", 178_000]))
    );
    let r = s.get("/__kaanal/prices/aws-elasticache/us-east-1").await;
    let v: Value = r.json().await.unwrap();
    assert!(
        v["prices"]
            .as_array()
            .unwrap()
            .contains(&json!(["aws/cache.m5.large", 156_000]))
    );
    // Valkey has its own list, 20% below Redis OSS
    let r = s
        .get("/__kaanal/prices/aws-elasticache-valkey/us-east-1")
        .await;
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    assert!(
        v["prices"]
            .as_array()
            .unwrap()
            .contains(&json!(["aws/cache.m5.large", 124_800]))
    );
}

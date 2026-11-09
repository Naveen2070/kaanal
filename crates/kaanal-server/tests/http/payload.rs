//! A built-in responder takes no live edit (step 8A).

use crate::common::*;
use serde_json::{Value, json};

#[tokio::test]
async fn the_login_route_refuses_an_edit_as_422_on_the_route_field() {
    let s = start_yaml(AUTH).await;
    let r = s
        .client
        .put(format!("{}/__kaanal/payload", s.base))
        .bearer_auth(&s.token)
        .json(&json!({ "route": "POST /login", "status": 200 }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["field"], "route");
    assert!(v["error"].as_str().unwrap().contains("built-in"));
}

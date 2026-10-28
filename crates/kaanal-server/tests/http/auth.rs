//! The auth node: login (JSON and form), JWKS, error cases, CORS and secrecy.

use crate::common::*;

use kaanal_core::Config;

use kaanal_server::ServeOpts;

use serde_json::{Value, json};

use std::time::Duration;

#[tokio::test]
async fn serve_without_a_signing_key_fails_closed() {
    let cfg = Config::from_yaml("version: 1\nnodes:\n  login: { type: auth }\n").unwrap();
    let opts = ServeOpts {
        host: "127.0.0.1".parse().unwrap(),
        port: 0,
        token: "t".into(),
        auth_key: None,
        draft: None,
    };
    let e = kaanal_server::serve(&cfg, &fixtures(), opts)
        .await
        .err()
        .unwrap();
    assert_eq!(
        e.msg,
        "auth node 'login' needs a signing key (kaanal up provides one)"
    );
}

pub(crate) const ALICE: &str = r#"{"username":"alice","password":"s3cr3t-pw"}"#;

impl Srv {
    /// `POST /login` with `body` and, if given, a `Content-Type`.
    pub(crate) async fn login(
        &self,
        content_type: Option<&str>,
        body: impl Into<reqwest::Body>,
    ) -> reqwest::Response {
        let mut rb = self.client.post(format!("{}/login", self.base)).body(body);
        if let Some(ct) = content_type {
            rb = rb.header("content-type", ct);
        }
        rb.send().await.unwrap()
    }

    pub(crate) fn port(&self) -> u16 {
        self.base.rsplit(':').next().unwrap().parse().unwrap()
    }
}

/// Verifies `token` against the server's own JWKS with an independent library (X13) and
/// returns its claims.
pub(crate) async fn verify(s: &Srv, token: &str) -> Value {
    let set: jsonwebtoken::jwk::JwkSet =
        s.get("/.well-known/jwks.json").await.json().await.unwrap();
    let kid = jsonwebtoken::decode_header(token).unwrap().kid.unwrap();
    let key = jsonwebtoken::DecodingKey::from_jwk(set.find(&kid).unwrap()).unwrap();
    let mut v = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::RS256);
    v.set_issuer(&[format!("http://localhost:{}", s.port())]);
    v.validate_aud = false;
    jsonwebtoken::decode::<Value>(token, &key, &v)
        .unwrap()
        .claims
}

#[tokio::test]
async fn login_with_json_returns_a_token_that_verifies_against_the_served_jwks() {
    let s = start_yaml(AUTH).await;
    let r = s
        .login(
            Some("application/json; charset=UTF-8"),
            r#"{"username":"alice","password":"s3cr3t-pw","grant_type":"password"}"#,
        )
        .await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers()["content-type"], "application/json");
    assert_eq!(trace_of(&r), "nginx.proc=1.0ms,net=1.0ms,login.proc=50.0ms");
    let b: Value = r.json().await.unwrap();
    assert_eq!(b["token_type"], "Bearer");
    assert_eq!(b["expires_in"], 7200);
    assert_eq!(b["scope"], "read write");
    let c = verify(&s, b["access_token"].as_str().unwrap()).await;
    assert_eq!(c["sub"], "alice");
    assert_eq!(c["roles"], json!(["admin"]));
    assert_eq!(c["scope"], "read write");
    assert_eq!(
        (c["aud"].clone(), c["tier"].clone()),
        (json!("my-app"), json!("pro"))
    );
    assert_eq!(
        c["exp"].as_u64().unwrap() - c["iat"].as_u64().unwrap(),
        7200
    );
}

#[tokio::test]
async fn login_with_form_data_works_and_omits_scope_without_scopes() {
    let s = start_yaml(AUTH).await;
    let r = s
        .login(
            Some("Application/X-WWW-Form-Urlencoded"),
            "username=bob&password=pw",
        )
        .await;
    assert_eq!(r.status(), 200);
    let b: Value = r.json().await.unwrap();
    assert!(b.get("scope").is_none(), "{b}");
    let c = verify(&s, b["access_token"].as_str().unwrap()).await;
    assert_eq!(
        (c["sub"].clone(), c["tier"].clone()),
        (json!("bob"), json!("free"))
    );
    assert!(c.get("scope").is_none(), "{c}");
    // Percent escapes are decoded.
    let r = s
        .login(
            Some("application/x-www-form-urlencoded"),
            "username=alice&password=s3cr3t%2Dpw",
        )
        .await;
    assert_eq!(r.status(), 200);
}

#[tokio::test]
async fn wrong_or_unknown_credentials_are_401_after_the_chain_latency() {
    let s = start_yaml(AUTH).await;
    for body in [
        r#"{"username":"alice","password":"nope"}"#,
        r#"{"username":"mallory","password":"s3cr3t-pw"}"#,
    ] {
        let t = std::time::Instant::now();
        let r = s.login(Some("application/json"), body).await;
        assert!(
            t.elapsed() >= Duration::from_millis(50),
            "{:?}",
            t.elapsed()
        );
        assert_eq!(r.status(), 401, "{body}");
        assert_eq!(
            r.json::<Value>().await.unwrap(),
            json!({ "error": "invalid_credentials" })
        );
    }
}

#[tokio::test]
async fn missing_or_text_plain_content_type_is_415() {
    let s = start_yaml(AUTH).await;
    // `fetch(url, { method: "POST", body: JSON.stringify(x) })` sends text/plain.
    for ct in [
        None,
        Some("text/plain;charset=UTF-8"),
        Some("application/jsonx"),
    ] {
        let t = std::time::Instant::now();
        let r = s.login(ct, ALICE).await;
        assert!(
            t.elapsed() >= Duration::from_millis(50),
            "{:?}",
            t.elapsed()
        );
        assert_eq!(r.status(), 415, "{ct:?}");
        assert_eq!(
            r.json::<Value>().await.unwrap(),
            json!({ "error": "unsupported_media_type" })
        );
    }
}

#[tokio::test]
async fn malformed_duplicate_or_non_string_fields_are_400_without_echo() {
    let s = start_yaml(AUTH).await;
    let cases = [
        (
            "application/json",
            r#"{"username":"alice","password":"s3cr3t-pw","password":"s3cr3t-pw"}"#,
        ),
        ("application/json", r#"{"username":"alice","password":123}"#),
        ("application/json", r#"{"username":"alice"}"#),
        (
            "application/json",
            r#"{"username":"alice","password":"s3cr3t-pw""#,
        ),
        ("application/json", "[]"),
        (
            "application/x-www-form-urlencoded",
            "username=alice&password=s3cr3t-pw&password=s3cr3t-pw",
        ),
        ("application/x-www-form-urlencoded", "username=alice"),
    ];
    for (ct, body) in cases {
        let r = s.login(Some(ct), body).await;
        assert_eq!(r.status(), 400, "{body}");
        // Exactly this text: parser messages (which can quote the password) never leak.
        assert_eq!(
            r.text().await.unwrap(),
            r#"{"error":"invalid_request"}"#,
            "{body}"
        );
    }
}

#[tokio::test]
async fn oversized_login_body_is_413() {
    let s = start_yaml(AUTH).await;
    let with_pad = |n: usize| {
        format!(
            r#"{{"username":"alice","password":"s3cr3t-pw","pad":"{}"}}"#,
            "x".repeat(n)
        )
    };
    let at_cap = with_pad(16 * 1024 - with_pad(0).len());
    assert_eq!(at_cap.len(), 16 * 1024);
    assert_eq!(
        s.login(Some("application/json"), at_cap).await.status(),
        200
    );
    let r = s.login(Some("application/json"), with_pad(16 * 1024)).await;
    assert_eq!(r.status(), 413);
    assert_eq!(
        r.json::<Value>().await.unwrap(),
        json!({ "error": "request_too_large" })
    );
}

#[tokio::test]
async fn every_login_response_is_no_store() {
    let s = start_yaml(AUTH).await;
    let cases = [
        (Some("application/json"), ALICE.to_string(), 200),
        (
            Some("application/json"),
            r#"{"username":"alice","password":"x"}"#.to_string(),
            401,
        ),
        (Some("application/json"), "{".to_string(), 400),
        (Some("application/json"), "x".repeat(16 * 1024 + 1), 413),
        (None, ALICE.to_string(), 415),
    ];
    for (ct, body, status) in cases {
        let r = s.login(ct, body).await;
        assert_eq!(r.status(), status);
        assert_eq!(r.headers()["cache-control"], "no-store", "{status}");
        assert_eq!(r.headers()["pragma"], "no-cache", "{status}");
    }
    // The JWKS is a public document: no such header.
    let jwks = s.get("/.well-known/jwks.json").await;
    assert!(jwks.headers().get("cache-control").is_none());
}

#[tokio::test]
async fn chain_failure_beats_body_errors_and_goes_through_error_map() {
    let s = start_yaml(AUTH).await;
    s.put_chaos("login", json!({ "down": true })).await;
    for (ct, body) in [
        (Some("application/json"), ALICE.to_string()),
        (Some("application/json"), "x".repeat(16 * 1024 + 1)),
        (None, "x".to_string()),
        (Some("application/json"), "{".to_string()),
    ] {
        let r = s.login(ct, body).await;
        assert_eq!(r.status(), 503);
        assert_eq!(trace_of(&r), "nginx.proc=1.0ms,net=1.0ms,login.error=503");
        assert_eq!(
            r.json::<Value>().await.unwrap(),
            json!({ "error": "chaos", "node": "login" })
        );
    }
    assert_eq!(s.get("/.well-known/jwks.json").await.status(), 503);
    // An `error_map` on the built-in route's entry shapes the failure (X9).
    let s = start_yaml(&format!(
        "{AUTH}  \"POST /login\": {{ error_map: {{ login: {{ status: 502, body: {{ e: 1 }} }} }} }}\n"
    ))
    .await;
    s.put_chaos("login", json!({ "error_rate": 1.0, "error_status": 500 }))
        .await;
    let r = s.login(Some("application/json"), ALICE).await;
    assert_eq!(r.status(), 502);
    assert_eq!(r.json::<Value>().await.unwrap(), json!({ "e": 1 }));
}

#[tokio::test]
async fn auth_latency_chaos_slows_login_and_jwks() {
    let s = start_yaml(AUTH).await;
    s.put_chaos("login", json!({ "latency_ms": 200 })).await;
    let t = std::time::Instant::now();
    assert_eq!(s.login(Some("application/json"), ALICE).await.status(), 200);
    assert!(
        t.elapsed() >= Duration::from_millis(250),
        "{:?}",
        t.elapsed()
    );
    let t = std::time::Instant::now();
    assert_eq!(s.get("/.well-known/jwks.json").await.status(), 200);
    assert!(
        t.elapsed() >= Duration::from_millis(250),
        "{:?}",
        t.elapsed()
    );
}

#[tokio::test]
async fn override_replaces_the_login_responder() {
    let s = start_yaml(&format!(
        "{AUTH}  \"POST /login\": {{ override: {{ body: {{ access_token: fixed }} }} }}\n"
    ))
    .await;
    // No Content-Type and no credentials: the override answers anyway (it is never read).
    let r = s.login(None, "").await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers()["cache-control"], "no-store");
    assert_eq!(
        r.json::<Value>().await.unwrap(),
        json!({ "access_token": "fixed" })
    );
}

#[tokio::test]
async fn login_and_jwks_send_cors_and_answer_preflight() {
    let s = start_yaml(AUTH).await;
    let pre = s
        .client
        .request(reqwest::Method::OPTIONS, format!("{}/login", s.base))
        .header("origin", "http://localhost:3000")
        .header("access-control-request-method", "POST")
        .header("access-control-request-headers", "content-type")
        .send()
        .await
        .unwrap();
    assert_eq!(pre.status(), 204);
    assert_eq!(pre.headers()["access-control-allow-origin"], "*");
    let r = s.login(Some("application/json"), ALICE).await;
    assert_eq!(r.headers()["access-control-allow-origin"], "*");
    let jwks = s.get("/.well-known/jwks.json").await;
    assert_eq!(jwks.headers()["access-control-allow-origin"], "*");
}

#[tokio::test]
async fn passwords_never_reach_trace_events_or_control_api() {
    let s = start_yaml(AUTH).await;
    let mut stream = s
        .client
        .get(format!("{}/__kaanal/events?token={}", s.base, s.token))
        .send()
        .await
        .unwrap();
    let ok = s.login(Some("application/json"), ALICE).await;
    let wrong = s
        .login(
            Some("application/json"),
            r#"{"username":"alice","password":"zz-wrong-zz"}"#,
        )
        .await;
    let mut seen = String::new();
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(chunk) = stream.chunk().await.unwrap() {
            seen.push_str(&String::from_utf8_lossy(&chunk));
            if seen.matches("\"path\":\"/login\"").count() >= 2 {
                break;
            }
        }
    })
    .await
    .unwrap();
    assert!(seen.contains("\"status\":401"), "{seen}");
    let topo = s
        .get_authed("/__kaanal/topology")
        .await
        .text()
        .await
        .unwrap();
    let chaos = s.get_authed("/__kaanal/chaos").await.text().await.unwrap();
    for text in [trace_of(&ok), trace_of(&wrong), seen, topo, chaos] {
        assert!(
            !text.contains("s3cr3t") && !text.contains("zz-wrong"),
            "{text}"
        );
    }
}

#[tokio::test]
async fn jwks_head_trace_and_route_listing() {
    let s = start_yaml(AUTH).await;
    let get = s.get("/.well-known/jwks.json").await;
    assert_eq!(get.status(), 200);
    assert_eq!(get.headers()["content-type"], "application/json");
    assert_eq!(
        trace_of(&get),
        "nginx.proc=1.0ms,net=1.0ms,login.proc=50.0ms"
    );
    let jwks: Value = get.json().await.unwrap();
    assert_eq!(jwks["keys"][0]["alg"], "RS256");
    let head = s
        .client
        .head(format!("{}/.well-known/jwks.json", s.base))
        .send()
        .await
        .unwrap();
    assert_eq!(head.status(), 200);
    assert!(head.bytes().await.unwrap().is_empty());
    let t: Value = s
        .get_authed("/__kaanal/topology")
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(
        t["routes"],
        json!(["GET /.well-known/jwks.json", "POST /login"])
    );
}

#[tokio::test]
async fn an_auth_only_config_serves_login() {
    // X11 / success criterion 7: a frontend and a login mock, nothing else.
    let s = start_yaml(
        "version: 1\nnodes:\n  login: { type: auth, proc: 1ms, users: [ { username: a, password: b } ] }\n",
    )
    .await;
    let r = s
        .login(
            Some("application/json"),
            r#"{"username":"a","password":"b"}"#,
        )
        .await;
    assert_eq!(r.status(), 200);
    assert_eq!(trace_of(&r), "login.proc=1.0ms");
    let token = r.json::<Value>().await.unwrap()["access_token"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(verify(&s, &token).await["sub"], "a");
}

#[tokio::test]
async fn get_on_login_is_405() {
    let s = start_yaml(AUTH).await;
    assert_eq!(s.get("/login").await.status(), 405);
}

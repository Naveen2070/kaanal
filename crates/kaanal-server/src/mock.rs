use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use kaanal_core::{ChainRequest, ChainResult, HopKind, execute, stable_hash};
use serde_json::{Value, json};

use crate::state::{AppState, Body, Event};

pub async fn handle(State(st): State<Arc<AppState>>, req: Request) -> Response {
    let path = req.uri().path().to_string();
    // The control namespace never gets CORS (RULES R5.4), not even on a 404 or a preflight.
    let control = path == "/__kaanal" || path.starts_with("/__kaanal/");
    let cors = |r: Response| if control { r } else { with_cors(r) };

    if req.method() == Method::OPTIONS {
        return cors(StatusCode::NO_CONTENT.into_response());
    }
    let method = req.method().as_str().to_string();

    let Ok(matched) = st.paths.at(&path) else {
        return cors(
            (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "no route", "path": path })),
            )
                .into_response(),
        );
    };
    // HEAD answers like GET unless the spec defines HEAD itself (hyper strips the body).
    let route = matched
        .value
        .iter()
        .find(|r| r.method == method)
        .or_else(|| {
            (method == "HEAD")
                .then(|| matched.value.iter().find(|r| r.method == "GET"))
                .flatten()
        });
    let Some(route) = route else {
        let body = Json(json!({ "error": "method not allowed", "path": path }));
        return cors((StatusCode::METHOD_NOT_ALLOWED, body).into_response());
    };
    let params: Vec<(String, String)> = matched
        .params
        .iter()
        .map(|(k, v)| (k.to_string(), percent_decode(v)))
        .collect();

    let started = Instant::now();
    let n = route.counter.fetch_add(1, Ordering::Relaxed);
    let outcome = {
        let chaos = st.chaos.read().expect("chaos lock poisoned");
        let req = ChainRequest {
            entry: route.owner,
            op: route.op,
            endpoint_key: &route.key,
            n,
            seed: st.seed,
        };
        execute(&st.topo, &chaos, &req)
    };
    // ponytail: limitation: on Windows the OS timer ticks about every 15 ms, so a sleep shorter than that
    // rounds up. Measured latency (the `ms` in events) can exceed the simulated sum shown in the trace.
    // Linux and macOS are exact to about 1 ms. Raise the timer resolution if sub-15 ms accuracy matters.
    tokio::time::sleep(outcome.total()).await;

    let name = |id: usize| st.topo.nodes[id].name.as_str();
    // One decimal: real hops are often under 1 ms and must not show as `0ms`.
    let fmt_ms = |d: std::time::Duration| format!("{:.1}ms", d.as_secs_f64() * 1000.0);
    let mut parts: Vec<String> = outcome
        .hops
        .iter()
        .map(|h| match h.kind {
            HopKind::Net => format!("net={}", fmt_ms(h.dur)),
            HopKind::Proc => format!("{}.proc={}", name(h.node), fmt_ms(h.dur)),
            HopKind::Op(op) => format!("{}.{}={}", name(h.node), op.name(), fmt_ms(h.dur)),
        })
        .collect();
    // The walk stops at the first failing node, so say which one and why.
    if let ChainResult::Err { node, status } = &outcome.result {
        parts.push(format!("{}.error={status}", name(*node)));
    }
    let trace = parts.join(",");

    let (status, body, failed) = match outcome.result {
        ChainResult::Err { node, status } => (
            status,
            json!({ "error": "chaos", "node": name(node) }),
            Some(name(node).to_string()),
        ),
        ChainResult::Ok => {
            let body = match &route.body {
                Body::Static(v) => v.clone(),
                Body::Generated {
                    schema: Some(schema),
                } => {
                    let owner = route.owner.map_or("", name);
                    let mut v = kaanal_data::generate(
                        schema,
                        None,
                        stable_hash(&[owner.as_bytes(), path.as_bytes()]),
                    );
                    kaanal_data::apply_params(&mut v, &params);
                    v
                }
                Body::Generated { schema: None } => Value::Null,
            };
            (route.status, body, None)
        }
    };

    let code = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut resp = if body.is_null() {
        code.into_response()
    } else {
        (code, Json(body)).into_response()
    };
    if let Ok(v) = HeaderValue::from_str(&trace) {
        resp.headers_mut().insert("x-kaanal-trace", v);
    }

    let mut nodes: Vec<String> = Vec::new();
    for h in &outcome.hops {
        let n = name(h.node);
        if !nodes.iter().any(|x| x == n) {
            nodes.push(n.to_string());
        }
    }
    if let Some(f) = &failed
        && !nodes.contains(f)
    {
        nodes.push(f.clone());
    }
    let ts_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
    // Measured, not simulated: shows what a client really waited (timer granularity, load).
    // The trace header keeps the simulated per-hop times.
    let ms = started.elapsed().as_millis() as u64;
    // No subscribers is fine: send only errors when nobody listens.
    let _ = st.events.send(Event {
        ts_ms,
        method,
        path,
        status,
        ms,
        nodes,
        failed_node: failed,
    });

    with_cors(resp)
}

/// `a%20b` -> `a b`. Invalid escapes are kept as typed; invalid UTF-8 becomes U+FFFD.
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        match (
            b[i],
            b.get(i + 1).and_then(|c| hex(*c)),
            b.get(i + 2).and_then(|c| hex(*c)),
        ) {
            (b'%', Some(h), Some(l)) => {
                out.push((h * 16 + l) as u8);
                i += 3;
            }
            (c, _, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn with_cors(mut r: Response) -> Response {
    let h = r.headers_mut();
    h.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    h.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, HEAD, POST, PUT, PATCH, DELETE, OPTIONS"),
    );
    h.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("*"),
    );
    h.insert(
        header::ACCESS_CONTROL_EXPOSE_HEADERS,
        HeaderValue::from_static("x-kaanal-trace"),
    );
    r
}

#[cfg(test)]
mod tests {
    use super::percent_decode;

    #[test]
    fn decodes_escapes_and_keeps_bad_ones() {
        assert_eq!(percent_decode("a%20b"), "a b");
        assert_eq!(percent_decode("%34%32"), "42");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
        assert_eq!(percent_decode("%C3%A9"), "é");
        assert_eq!(percent_decode("%ff"), "\u{fffd}");
    }
}

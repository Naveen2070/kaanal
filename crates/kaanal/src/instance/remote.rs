//! The thin HTTP client for the control API: one agent, one `call`, `status` and the shutdown reply.

use crate::{
    Duration, Fail, Value, active_names, chaos_text, clean, err, json, lifecycle, output,
    pin_extra_ms, source_text, target,
};

/// **Private.** An HTTP agent for the control API: no proxy (it could forward the token), no
/// redirects, short timeouts.
pub(crate) fn control_agent(total: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .proxy(None)
        .max_redirects(0)
        .timeout_connect(Some(Duration::from_secs(2)))
        .timeout_global(Some(total))
        .build()
        .into()
}

/// **Private.** The answer to `POST /__kaanal/shutdown`.
pub(crate) struct ShutdownReply {
    pub(crate) status: u16,
    /// The drain time left, from the 202 body (a drain that was already running reports its own).
    pub(crate) left: Option<Duration>,
}

/// **Private.** `POST /__kaanal/shutdown` with a drain budget such as `5s`: the answer, or `None`
/// when there was no answer at all.
pub(crate) fn post_shutdown(run: &lifecycle::Run, budget: &str) -> Option<ShutdownReply> {
    let mut r = control_agent(Duration::from_secs(5))
        .post(&format!("http://{}/__kaanal/shutdown", run.addr))
        .header("Host", &format!("localhost:{}", run.addr.port()))
        .header("Authorization", &format!("Bearer {}", run.token))
        .send_json(json!({ "timeout": budget }))
        .ok()?;
    let status = r.status().as_u16();
    let left = r
        .body_mut()
        .read_json::<Value>()
        .ok()
        .and_then(|v| v["timeout_ms"].as_u64())
        .map(|ms| lifecycle::server_wait(Some(ms), 0));
    Some(ShutdownReply { status, left })
}

/// **Private.** Calls the instance [`target::resolve`] picks: `--endpoint` / `KAANAL_ENDPOINT`, else
/// the one found via `.kaanal/run.json`. Locally exit 2 if it isn't running or its state is stale,
/// exit 1 if it is alive but silent (spec M9, M16); remotely no answer is exit 1 (E5). The client
/// never follows redirects and never uses a proxy — either could forward the token.
pub(crate) fn call(method: &str, path: &str, body: Option<Value>) -> Result<Value, Fail> {
    call_for(method, path, body, Duration::from_secs(10))
}

/// **Private.** [`call`] with its own total time budget, for a request that waits on purpose
/// (`draft apply` drains requests in flight).
pub(crate) fn call_for(
    method: &str,
    path: &str,
    body: Option<Value>,
    total: Duration,
) -> Result<Value, Fail> {
    let target = target::resolve()?;
    let auth = target.auth();
    let url = target.url(path);
    // The control API only accepts a loopback Host with the bound port (R5.3), even when
    // we connect to a non-loopback or remote address.
    let host = target.host_header();
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        // Never send the token through an HTTP(S)_PROXY from the environment.
        .proxy(None)
        // The server never redirects; following one could forward the token to another host.
        .max_redirects(0)
        .timeout_connect(Some(Duration::from_secs(2)))
        .timeout_global(Some(total))
        .build()
        .into();
    let res = match (method, body) {
        ("PUT", Some(b)) => agent
            .put(&url)
            .header("Host", &host)
            .header("Authorization", &auth)
            .send_json(&b),
        ("POST", Some(b)) => agent
            .post(&url)
            .header("Host", &host)
            .header("Authorization", &auth)
            .send_json(&b),
        ("POST", None) => agent
            .post(&url)
            .header("Host", &host)
            .header("Authorization", &auth)
            .send_empty(),
        ("DELETE", _) => agent
            .delete(&url)
            .header("Host", &host)
            .header("Authorization", &auth)
            .call(),
        _ => agent
            .get(&url)
            .header("Host", &host)
            .header("Authorization", &auth)
            .call(),
    };
    let mut resp = res.map_err(|e| {
        // the server took the connection but did not answer in time: it may still be working
        // (a drain), which is not the same as nobody answering
        if matches!(e, ureq::Error::Timeout(ureq::Timeout::Global)) {
            return err(format!(
                "{method} {path}: no answer within {}s; the server may still be working (check `kaanal draft show` or `kaanal status`)",
                total.as_secs()
            ));
        }
        target.unreachable(&e.to_string())
    })?;
    let status = resp.status().as_u16();
    let text = resp.body_mut().read_to_string().unwrap_or_default();
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if status >= 300 {
        let msg = v["error"].as_str().unwrap_or(&text);
        if let Some(f) = target.refused(status, msg) {
            return Err(f);
        }
        let detail = v["detail"]
            .as_str()
            .map_or(String::new(), |d| format!(" ({d})"));
        return Err(err(clean(&format!(
            "{method} {path}: HTTP {status}: {msg}{detail}"
        ))));
    }
    Ok(v)
}

/// **Private.** Prints the node table with each node's effective chaos (scenarios plus live)
/// and the active scenarios.
pub(crate) fn status() -> Result<(), Fail> {
    let topo = call("GET", "/__kaanal/topology", None)?;
    // One call, so the stack and the chaos compiled from it come from the same snapshot.
    let active = call("GET", "/__kaanal/scenarios/active", None)?;
    let eff = &active["effective"];
    // Only the text table shows pins; fetched before any output so a failure prints nothing.
    let pins = if output::json_on() {
        Value::Null
    } else {
        call("GET", "/__kaanal/pins", None)?
    };
    let live = call("GET", "/__kaanal/chaos", None)?;
    let rows = output::node_rows(&topo, &active, &live);
    output::emit(&Value::Array(rows.clone()), || {
        output::print_node_table(&rows);
        // Endpoint-wide and node-at-endpoint chaos belong to no single row.
        for (key, c) in eff["endpoints"].as_object().into_iter().flatten() {
            println!("{}", clean(&format!("endpoint {key}: {}", chaos_text(c))));
        }
        for c in eff["node_at"].as_array().into_iter().flatten() {
            let (n, e) = (
                c["node"].as_str().unwrap_or(""),
                c["endpoint"].as_str().unwrap_or(""),
            );
            println!("{}", clean(&format!("{n} @ {e}: {}", chaos_text(c))));
        }
        for c in eff["edges"].as_array().into_iter().flatten() {
            let (a, b) = (
                c["edge"]["from"].as_str().unwrap_or(""),
                c["edge"]["to"].as_str().unwrap_or(""),
            );
            println!("{}", clean(&format!("{a} -> {b}: {}", chaos_text(c))));
        }
        for (key, p) in pins["effective"].as_object().into_iter().flatten() {
            let line = format!(
                "pin {key}: {}{} ({})",
                p["state"].as_str().unwrap_or(""),
                pin_extra_ms(p),
                source_text(&p["source"])
            );
            println!("{}", clean(&line));
        }
        let names = active_names(&active);
        println!(
            "active scenarios: {}",
            if names.is_empty() {
                "-".to_string()
            } else {
                clean(&names.join(", "))
            }
        );
    });
    Ok(())
}

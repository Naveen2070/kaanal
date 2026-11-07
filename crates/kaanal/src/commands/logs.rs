//! `kaanal logs` and `trace`: one-shot and `--follow` reading of the request log, and the output helpers.

use crate::{
    Arc, AtomicU64, Duration, Fail, LogsArgs, Path, STATE_DIR, Value, call, clean, encode_segment,
    err, lifecycle, logview, output, target,
};

/// **Private.** `kaanal logs`: checks the flags and turns them into the server's query pairs.
pub(crate) fn logs_cmd(a: &LogsArgs) -> Result<(), Fail> {
    if !(1..=1000).contains(&a.n) {
        return Err(Fail::usage("-n must be from 1 to 1000"));
    }
    let params: Vec<(String, String)> = [
        Some(("limit", a.n.to_string())),
        a.since.clone().map(|v| ("since", v)),
        a.route.clone().map(|v| ("endpoint", v)),
        a.status.clone().map(|v| ("status", v)),
        a.failed.then(|| ("failed", "true".to_string())),
        a.scope.clone().map(|v| ("scope", v)),
    ]
    .into_iter()
    .flatten()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    // A bad filter is a usage error in both modes, before any request or run-file read.
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    let filter = logview::filter_of(&params, now_ms).map_err(Fail::usage)?;
    if a.follow {
        return follow_logs(&params, &filter, output::json_on());
    }
    logs(&params, output::json_on())
}

/// **Private.** Prints one line to stdout. A closed pipe (`kaanal logs | head`) ends the process
/// quietly with exit 0, which is how every well-behaved filter tool treats it.
pub(crate) fn out_line(text: &str) {
    out_bytes(
        format!(
            "{text}
"
        )
        .as_bytes(),
    );
}

/// **Private.** Writes raw bytes to stdout with the same closed-pipe rule as [`out_line`].
pub(crate) fn out_bytes(bytes: &[u8]) {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    if let Err(e) = out.write_all(bytes).and_then(|()| out.flush()) {
        if e.kind() == std::io::ErrorKind::BrokenPipe {
            std::process::exit(0);
        }
        eprintln!("kaanal: cannot write to stdout: {e}");
        std::process::exit(1);
    }
}

/// **Private.** `kaanal trace`: the waterfall of one request, or its entry as JSON.
pub(crate) fn trace(key: &str) -> Result<(), Fail> {
    let e = call(
        "GET",
        &format!("/__kaanal/trace/{}", encode_segment(key)),
        None,
    )
    // `last` is unknown only while nothing has finished yet
    .map_err(|f| match f {
        Fail(1, m) if key == "last" && m.contains("HTTP 404") => err("no requests recorded yet"),
        f => f,
    })?;
    out_line(&if output::json_on() {
        e.to_string()
    } else {
        logview::waterfall(&e)
    });
    Ok(())
}

/// **Private.** A status notice of `logs -f`. With `--json` stdout carries JSON Lines only, so
/// notices go to stderr then.
pub(crate) fn notice(json: bool, text: &str) {
    if json {
        eprintln!("{text}");
    } else {
        out_line(text);
    }
}

/// **Private.** One log line, or the JSON object with `--json`.
pub(crate) fn show(v: &Value, json: bool) {
    out_line(&if json {
        v.to_string()
    } else {
        logview::line(v)
    });
}

/// **Private.** `/__kaanal/logs?k=v&...` for the filter pairs.
pub(crate) fn logs_path(params: &[(String, String)]) -> String {
    let q: Vec<String> = params
        .iter()
        .map(|(k, v)| format!("{k}={}", encode_segment(v)))
        .collect();
    format!("/__kaanal/logs?{}", q.join("&"))
}

/// **Private.** `kaanal logs`: the tail, then (`-f`) every request as it finishes.
pub(crate) fn logs(params: &[(String, String)], json: bool) -> Result<(), Fail> {
    let page = call("GET", &logs_path(params), None)?;
    for e in page["entries"].as_array().into_iter().flatten() {
        show(e, json);
    }
    Ok(())
}

/// **Private.** Ctrl+C is a normal way to stop following: exit 0.
pub(crate) fn exit_on_ctrl_c() {
    std::thread::spawn(|| {
        if let Ok(rt) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            let _ = rt.block_on(tokio::signal::ctrl_c());
            std::process::exit(0);
        }
    });
}

/// **Private.** `logs -f`. Subscribe first and let the reader thread buffer events, then fetch the
/// tail, then print the tail and everything buffered or live that the tail did not already
/// contain: a request that finishes between the two is in both and printed once, one that
/// finishes late (slow) is printed when it finishes. Live events are filtered here, with the
/// server's own filter grammar.
pub(crate) fn follow_logs(
    params: &[(String, String)],
    filter: &kaanal_server::Filter,
    json: bool,
) -> Result<(), Fail> {
    let target = target::resolve()?;
    let gone = |why: String| target.unreachable(&why);
    exit_on_ctrl_c();
    // Its own agent: no overall timeout, the stream is meant to stay open.
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .proxy(None)
        .max_redirects(0)
        .timeout_connect(Some(Duration::from_secs(2)))
        .build()
        .into();
    let mut resp = agent
        .get(&target.url("/__kaanal/events"))
        .header("Host", &target.host_header())
        .header("Authorization", &target.auth())
        .call()
        .map_err(|e| gone(e.to_string()))?;
    let status = resp.status().as_u16();
    if status != 200 {
        let text = resp.body_mut().read_to_string().unwrap_or_default();
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        return Err(target
            .refused(status, v["error"].as_str().unwrap_or(""))
            .unwrap_or_else(|| err(format!("GET /__kaanal/events: HTTP {status}"))));
    }
    // Bounded: a follower that cannot keep up loses events visibly (a `lagged` notice), not memory.
    let (tx, rx) = std::sync::mpsc::sync_channel::<Option<(String, String)>>(logview::BACKLOG);
    let dropped_in = Arc::new(AtomicU64::new(0));
    let dropped = Arc::clone(&dropped_in);
    std::thread::spawn(move || {
        let mut r = std::io::BufReader::new(resp.into_body().into_reader());
        let (mut sse, mut buf) = (logview::Sse::default(), Vec::new());
        let dropped = Arc::clone(&dropped_in);
        loop {
            buf.clear();
            match logview::read_capped(&mut r, &mut buf, logview::MAX_MSG) {
                Ok(logview::Line::Eof) | Err(_) => break,
                Ok(logview::Line::TooLong) => sse.overflow(),
                Ok(logview::Line::Text) => {
                    if let Some(m) = sse.push(&logview::lossy_line(&buf))
                        && !logview::forward(&tx, &dropped, m)
                    {
                        return;
                    }
                }
            }
        }
        let _ = tx.send(None);
    });
    let mut seen = logview::Seen::new();
    let page = call("GET", &logs_path(params), None)?;
    for e in page["entries"].as_array().into_iter().flatten() {
        seen.insert(e["id"].as_str().unwrap_or(""));
        show(e, json);
    }
    // Ends only when the stream does (or the printer's queue is cut): the server is gone.
    let mut shutdown_ms: Option<Option<u64>> = None;
    logview::drain(
        &rx,
        &dropped,
        Duration::from_millis(500),
        &mut |s| notice(json, s),
        &mut |event, data| {
            if event == "shutdown" {
                // The server announces its drain; the time left says how long to wait for the exit.
                shutdown_ms = Some(
                    serde_json::from_str::<Value>(data)
                        .ok()
                        .and_then(|v| v["timeout_ms"].as_u64()),
                );
                return;
            }
            if event != "request" {
                return;
            }
            let Ok(mut v) = serde_json::from_str::<Value>(data) else {
                return;
            };
            if !seen.insert(v["id"].as_str().unwrap_or("")) || !logview::passes(filter, &v) {
                return;
            }
            // --json wants the whole entry; the event is only its summary.
            if json {
                let id = v["id"].as_str().unwrap_or("");
                match call(
                    "GET",
                    &format!("/__kaanal/trace/{}", encode_segment(id)),
                    None,
                ) {
                    Ok(full) => v = full,
                    Err(_) => notice(json, &format!("-- could not fetch {} --", clean(id))),
                }
            }
            show(&v, json);
        },
    );
    let drain = shutdown_ms.map(|ms| lifecycle::server_wait(ms, 5000));
    Err(stream_ended(&target, drain))
}

/// **Private.** Why `logs -f` stopped, once the event stream ended; `drain` is the shutdown the
/// server announced, if it did.
pub(crate) fn stream_ended(target: &target::Target, drain: Option<Duration>) -> Fail {
    // Remote: no lock to wait on, and exit 2 ("no local instance") is local-only (E5).
    if target.remote {
        return match drain {
            Some(_) => err(format!("{}: stopped (the server shut down)", target.base())),
            None => target.unreachable("the event stream closed"),
        };
    }
    let state = Path::new(STATE_DIR);
    if let Some(ms) = drain {
        // A `down` (or signal) drain: the process is still finishing its exit. That is a stop.
        lifecycle::wait_lock_free(state, ms + Duration::from_secs(2));
        return Fail(2, "kaanal stopped (the server shut down)".into());
    }
    // No shutdown message: killed, or cut by a tiny drain budget before it was written. Give the
    // exit a moment; a lock that is still held is then honestly "running but not answering".
    lifecycle::wait_lock_free(state, Duration::from_secs(2));
    target.unreachable("the event stream closed")
}

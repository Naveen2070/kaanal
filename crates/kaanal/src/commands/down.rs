//! `kaanal down`: stop a local instance (drain, then force) or ask a remote one to shut down.

use crate::{
    Duration, Fail, Instant, Path, STATE_DIR, ShutdownReply, Value, clean, control_agent,
    drain_wait, err, json, lifecycle, post_shutdown, target,
};

/// **Private.** `--timeout` of `down`: a duration the server's shutdown endpoint accepts. A bad
/// value is a clap error, which `main` turns into exit 64.
pub(crate) fn drain_budget(s: &str) -> Result<Duration, String> {
    humantime::parse_duration(s.trim())
        .ok()
        .filter(|d| (Duration::from_millis(1)..=Duration::from_secs(300)).contains(d))
        .ok_or_else(|| format!("bad timeout '{s}' (1ms to 300s, e.g. 5s)"))
}

/// **Private.** `kaanal down` (spec M7): ask the instance to drain, wait for its lock to go, and
/// only if that fails kill it, and only after checking the pid really is a kaanal process.
pub(crate) fn down(timeout: Duration) -> Result<(), Fail> {
    let state = Path::new(STATE_DIR);
    if let Some(msg) = lifecycle::clear_stale(state)? {
        println!("{}", clean(&msg));
        return Ok(());
    }
    // Lock held: an instance, possibly still starting. Never touch its files; if it vanishes
    // meanwhile, that is a stop, not a failure.
    let run = match lifecycle::run_or_gone(state, lifecycle::STARTUP_WAIT)? {
        lifecycle::Found::Gone(msg) => {
            println!("{}", clean(&msg));
            return Ok(());
        }
        lifecycle::Found::Run(r) => r,
    };
    let budget = humantime::format_duration(timeout).to_string();
    // 202, or no answer at all (refused, or cut by a tiny budget before the 202 left): the drain
    // is running or the process is going, so wait for the lock. Any other answer (401, 5xx) means
    // the API is up but unusable and no drain started: go straight to the kill.
    let reply = post_shutdown(&run, &budget);
    let draining = reply.as_ref().is_none_or(|r| r.status == 202);
    if draining && lifecycle::wait_lock_free(state, drain_wait(timeout, reply.as_ref())) {
        println!("stopped");
        return Ok(());
    }
    let Some(pid) = run.pid else {
        if matches!(lifecycle::lock_probe(state), Ok(lifecycle::Liveness::Free)) {
            println!("stopped");
            return Ok(());
        }
        return Err(err(
            "kaanal did not stop and run.json has no pid; not killing anything",
        ));
    };
    match lifecycle::force_stop(
        state,
        &run.token,
        pid,
        std::process::id(),
        lifecycle::verify_pid,
        lifecycle::kill_pid,
    )? {
        lifecycle::Forced::Stopped => println!("stopped"),
        lifecycle::Forced::Killed => println!("stopped (killed pid {pid} after {budget})"),
    }
    Ok(())
}

/// **Private.** `kaanal down --endpoint` (E6): the same `POST /__kaanal/shutdown`, then poll health
/// until nothing answers, for the budget plus 2 s. No run.json, lock or pid, and never a kill: a
/// remote instance that keeps answering is reported, not forced.
pub(crate) fn down_remote(t: &target::Target, timeout: Duration) -> Result<(), Fail> {
    let budget = humantime::format_duration(timeout).to_string();
    let agent = control_agent(Duration::from_secs(5));
    let mut r = agent
        .post(&t.url("/__kaanal/shutdown"))
        .header("Host", &t.host_header())
        .header("Authorization", &t.auth())
        .send_json(json!({ "timeout": budget }))
        .map_err(|e| t.unreachable(&clean(&e.to_string())))?;
    let status = r.status().as_u16();
    let text = r.body_mut().read_to_string().unwrap_or_default();
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if status != 202 {
        let msg = v["error"].as_str().unwrap_or(&text);
        return Err(t.refused(status, msg).unwrap_or_else(|| {
            err(clean(&format!(
                "POST /__kaanal/shutdown: HTTP {status}: {msg}"
            )))
        }));
    }
    // As local `down`: a drain already running with more time left is waited for.
    let left = v["timeout_ms"]
        .as_u64()
        .map(|ms| lifecycle::server_wait(Some(ms), 0));
    let wait = drain_wait(timeout, Some(&ShutdownReply { status, left }));
    let deadline = Instant::now() + wait;
    loop {
        // Only a refused or dropped connection means gone; a slow or odd answer means still up.
        let left = deadline.saturating_duration_since(Instant::now());
        // At least 2.5 s: with less left, ureq reports the global timeout before the 2 s connect
        // timeout, and a closed port on Windows would look like a live instance.
        let res = control_agent(left.clamp(Duration::from_millis(2500), Duration::from_secs(5)))
            .get(&t.url("/__kaanal/health"))
            .header("Host", &t.host_header())
            .header("Authorization", &t.auth())
            .call();
        // Windows reports a closed port as a connect timeout (its refusal takes longer than ours).
        let gone = match res {
            Err(ureq::Error::Io(e)) => matches!(
                e.kind(),
                std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
            ),
            Err(ureq::Error::Timeout(ureq::Timeout::Connect)) => true,
            _ => false,
        };
        if gone {
            println!("stopped");
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(err(format!(
                "{} is still answering after {}; not killing a remote instance",
                t.base(),
                humantime::format_duration(wait)
            )));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// **Private.** Prints a response's `warnings` to stderr.
pub(crate) fn warn_all(v: &Value) {
    for w in v["warnings"].as_array().into_iter().flatten() {
        eprintln!("warning: {}", clean(w.as_str().unwrap_or("")));
    }
}

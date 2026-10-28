//! `kaanal sim`, `kaanal sample` and `kaanal skeleton`: thin clients of the simulation routes
//! (`docs/web/simulation.md`). All the logic is the server's; this reads the answers aloud.

use std::fmt::Write as _;

use crate::cli::SimCmd;
use crate::{Duration, Fail, Path, Value, call, clean, err, json, output};

/// **Private.** How long `sim run` waits for a report before it says to look again later.
const WAIT: Duration = Duration::from_secs(300);

/// **Private.** `kaanal sim ...`.
pub(crate) fn sim(cmd: SimCmd) -> Result<(), Fail> {
    match cmd {
        SimCmd::Run {
            profile,
            file,
            duration,
            warmup,
            seed,
            scenario,
            no_wait,
        } => run(
            profile.as_deref(),
            file.as_deref(),
            duration.as_deref(),
            warmup.as_deref(),
            seed,
            scenario,
            no_wait,
        ),
        SimCmd::List => list(),
        SimCmd::Show { id } => show(&id),
    }
}

/// **Private.** A duration flag as seconds.
fn secs(flag: &str, text: &str) -> Result<f64, Fail> {
    humantime::parse_duration(text)
        .map(|d| d.as_secs_f64())
        .map_err(|_| Fail::usage(format!("bad {flag} '{}' (e.g. 60s, 2m)", clean(text))))
}

/// **Private.** `sim run`: starts a run and, unless told not to, waits for its report.
fn run(
    profile: Option<&str>,
    file: Option<&Path>,
    duration: Option<&str>,
    warmup: Option<&str>,
    seed: Option<u64>,
    scenario: Option<String>,
    no_wait: bool,
) -> Result<(), Fail> {
    let profile: Value = match (profile, file) {
        (Some(n), None) => json!(n),
        (None, Some(f)) => {
            let text = std::fs::read_to_string(f)
                .map_err(|e| err(format!("{}: {e}", clean(&f.display().to_string()))))?;
            serde_yaml_ng::from_str(&text).map_err(|e| {
                err(format!(
                    "{}: not a profile: {}",
                    clean(&f.display().to_string()),
                    clean(&e.to_string())
                ))
            })?
        }
        _ => {
            return Err(Fail::usage(
                "name a profile with --profile NAME, or give one with --file FILE",
            ));
        }
    };
    let mut body = json!({ "profile": profile });
    if let Some(d) = duration {
        body["duration_s"] = json!(secs("--duration", d)?);
    }
    if let Some(w) = warmup {
        body["warmup_s"] = json!(secs("--warmup", w)?);
    }
    if let Some(s) = seed {
        body["seed"] = json!(s);
    }
    if let Some(s) = scenario {
        body["scenario"] = json!(s);
    }
    let started = call("POST", "/__kaanal/sim/run", Some(body))?;
    let id = started["id"].as_str().unwrap_or_default().to_string();
    if no_wait {
        output::emit(&started, || println!("{}", clean(&id)));
        return Ok(());
    }
    let v = wait(&id)?;
    // a failed run is reported by the error below, not twice
    output::emit(&v, || {
        if v["error"].is_null() {
            print_run(&v);
        }
    });
    match v["error"].as_str() {
        Some(e) => Err(err(format!("run {}: {}", clean(&id), clean(e)))),
        None => Ok(()),
    }
}

/// **Private.** Polls run `id` until it is no longer running.
fn wait(id: &str) -> Result<Value, Fail> {
    let end = std::time::Instant::now() + WAIT;
    loop {
        let v = call(
            "GET",
            &format!("/__kaanal/sim/runs/{}", crate::encode_segment(id)),
            None,
        )?;
        if v["state"] != "running" {
            return Ok(v);
        }
        if std::time::Instant::now() > end {
            return Err(err(format!(
                "run {} is still going; look again with `kaanal sim show {}`",
                clean(id),
                clean(id)
            )));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// **Private.** `sim list`.
fn list() -> Result<(), Fail> {
    let v = call("GET", "/__kaanal/sim/runs", None)?;
    output::emit(&v, || {
        let runs = v["runs"].as_array().map_or(&[][..], Vec::as_slice);
        if runs.is_empty() {
            println!("no runs");
        }
        for r in runs {
            println!(
                "{:<5} {:<8} generation {} profile {}{}",
                clean(r["id"].as_str().unwrap_or("")),
                clean(r["state"].as_str().unwrap_or("")),
                r["generation"],
                clean(r["profile"].as_str().unwrap_or("")),
                r["scenario"]
                    .as_str()
                    .map_or(String::new(), |s| format!(" scenario {}", clean(s)))
            );
        }
    });
    Ok(())
}

/// **Private.** `sim show ID`.
fn show(id: &str) -> Result<(), Fail> {
    let v = call(
        "GET",
        &format!("/__kaanal/sim/runs/{}", crate::encode_segment(id)),
        None,
    )?;
    output::emit(&v, || print_run(&v));
    Ok(())
}

/// **Private.** A number from an answer, one decimal.
fn ms(v: &Value) -> String {
    format!("{:.1}", v.as_f64().unwrap_or(0.0))
}

/// **Private.** A load or share from an answer, two decimals; `-` when absent.
fn share(v: &Value) -> String {
    v.as_f64().map_or("-".into(), |x| format!("{x:.2}"))
}

/// **Private.** The report of a run as text: the label first, then latency, failures, the
/// busiest nodes and the warnings.
fn print_run(v: &Value) {
    let id = clean(v["id"].as_str().unwrap_or(""));
    let state = clean(v["state"].as_str().unwrap_or(""));
    println!("run {id} {state} (generation {})", v["generation"]);
    let s = &v["summary"];
    if !s.is_object() {
        if let Some(e) = v["error"].as_str() {
            println!("error: {}", clean(e));
        }
        return;
    }
    println!("{}", clean(s["label"].as_str().unwrap_or("")));
    let l = &s["latency"];
    println!(
        "requests {} (failed {})   latency p50 {} ms  p95 {} ms  p99 {} ms  max {} ms",
        s["requests"],
        s["failed"],
        ms(&l["p50_ms"]),
        ms(&l["p95_ms"]),
        ms(&l["p99_ms"]),
        ms(&l["max_ms"])
    );
    if let Some(errors) = s["errors"].as_object().filter(|e| !e.is_empty()) {
        let mut line = String::from("errors:");
        for (k, n) in errors {
            let _ = write!(line, " {} {n}", clean(k));
        }
        println!("{line}");
    }
    let nodes = s["nodes"].as_array().map_or(&[][..], Vec::as_slice);
    println!("busiest nodes (offered load, utilization):");
    for i in s["bottlenecks"].as_array().into_iter().flatten().take(5) {
        let Some(n) = i.as_u64().and_then(|i| nodes.get(usize::try_from(i).ok()?)) else {
            continue;
        };
        println!(
            "  {:<16} load {:>5}  util {:>5}  wait {:>7} ms  visits/s {} (expected {}){}",
            clean(n["name"].as_str().unwrap_or("")),
            share(&n["offered_load"]),
            share(&n["utilization"]),
            ms(&n["mean_wait_ms"]),
            ms(&n["visits_per_s"]),
            ms(&n["expected_visits_per_s"]),
            if n["saturated"] == true {
                "  SATURATED"
            } else {
                ""
            }
        );
    }
    for w in s["warnings"].as_array().into_iter().flatten() {
        println!("warning: {}", clean(w.as_str().unwrap_or("")));
    }
}

/// **Private.** `kaanal sample ROUTE`. Text mode prints the body to stdout, so it can be piped,
/// and the size and status to stderr; `--json` prints the whole answer.
pub(crate) fn sample(endpoint: &str, seed: Option<u64>) -> Result<(), Fail> {
    let mut body = json!({ "endpoint": endpoint });
    if let Some(s) = seed {
        body["seed"] = json!(s);
    }
    let v = call("POST", "/__kaanal/sample", Some(body))?;
    output::emit(&v, || {
        let pretty = serde_json::to_string_pretty(&v["body"]).unwrap_or_default();
        crate::out_line(&pretty);
        eprintln!("{} bytes, status {}", v["bytes"], v["status"]);
    });
    Ok(())
}

/// **Private.** `kaanal skeleton [-o FILE] [--force]`.
pub(crate) fn skeleton_cmd(output_file: Option<&Path>, force: bool) -> Result<(), Fail> {
    let v = call("GET", "/__kaanal/openapi/skeleton", None)?;
    // an answer without text (an unreadable body) must not become an empty file
    let text = v["text"]
        .as_str()
        .filter(|t| !t.is_empty())
        .ok_or_else(|| err("the server sent no skeleton text"))?;
    let Some(file) = output_file else {
        output::emit(&v, || crate::out_bytes(text.as_bytes()));
        return Ok(());
    };
    let shown = clean(&file.display().to_string());
    let mut open = std::fs::OpenOptions::new();
    open.write(true);
    if force {
        open.create(true).truncate(true);
    } else {
        // create_new: an existing file is an error here, not a race with a check
        open.create_new(true);
    }
    let mut f = open.open(file).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            err(format!("{shown} exists; --force overwrites it"))
        } else {
            err(format!("{shown}: {e}"))
        }
    })?;
    if let Err(e) = std::io::Write::write_all(&mut f, text.as_bytes()) {
        drop(f);
        if !force {
            // the file is ours (create_new): do not leave half of it behind
            let _ = std::fs::remove_file(file);
        }
        return Err(err(format!("{shown}: {e}")));
    }
    output::emit(
        &json!({ "written": shown, "endpoints": v["endpoints"] }),
        || {
            println!("wrote {shown} ({} endpoints)", v["endpoints"]);
        },
    );
    Ok(())
}

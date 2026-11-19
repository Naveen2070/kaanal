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
        SimCmd::Sweep {
            profile,
            file,
            p99,
            errors,
            max_factor,
            seed,
            scenario,
            no_wait,
        } => {
            // the limits first: a bad one is a usage error before anything is read or sent
            let body = limits(p99.as_deref(), errors.as_deref(), max_factor)?;
            sweep(
                body,
                profile.as_deref(),
                file.as_deref(),
                seed,
                scenario,
                no_wait,
            )
        }
        SimCmd::List => list(),
        SimCmd::Show { id } => show(&id),
    }
}

/// **Private.** A duration flag as seconds; `eg` names examples in the usage error.
fn secs(flag: &str, text: &str, eg: &str) -> Result<f64, Fail> {
    humantime::parse_duration(text)
        .map(|d| d.as_secs_f64())
        .map_err(|_| Fail::usage(format!("bad {flag} '{}' (e.g. {eg})", clean(text))))
}

/// **Private.** The `profile` of a body from `-p` or `-f`, and whether none was named.
fn profile_of(profile: Option<&str>, file: Option<&Path>) -> Result<(Value, bool), Fail> {
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
        // none given: an empty name, so the server says what the project has (RULES 58)
        _ => json!(""),
    };
    let unnamed = profile == json!("");
    Ok((profile, unnamed))
}

/// **Private.** Posts `body` (with `seed` and `scenario` when given) to `path` and, unless told
/// not to, waits for the record and prints it with `print`.
fn start(
    path: &str,
    mut body: Value,
    unnamed: bool,
    seed: Option<u64>,
    scenario: Option<String>,
    no_wait: bool,
    print: fn(&Value),
) -> Result<(), Fail> {
    if let Some(s) = seed {
        body["seed"] = json!(s);
    }
    if let Some(s) = scenario {
        body["scenario"] = json!(s);
    }
    let started = call("POST", path, Some(body)).map_err(|f| match f {
        // still a usage error (64), as when the CLI refused it before asking: only the text is new
        Fail(1, m) if unnamed && m.contains("HTTP 404: ") => {
            Fail::usage(m.split_once("HTTP 404: ").map_or(m.as_str(), |(_, t)| t))
        }
        f => f,
    })?;
    let id = started["id"].as_str().unwrap_or_default().to_string();
    if no_wait {
        output::emit(&started, || println!("{}", clean(&id)));
        return Ok(());
    }
    let v = wait(&id)?;
    // a failed run is reported by the error below, not twice
    output::emit(&v, || {
        if v["error"].is_null() {
            print(&v);
        }
    });
    match v["error"].as_str() {
        Some(e) => Err(err(format!(
            "{} {}: {}",
            clean(v["kind"].as_str().unwrap_or("run")),
            clean(&id),
            clean(e)
        ))),
        None => Ok(()),
    }
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
    let (profile, unnamed) = profile_of(profile, file)?;
    let mut body = json!({ "profile": profile });
    if let Some(d) = duration {
        body["duration_s"] = json!(secs("--duration", d, "60s, 2m")?);
    }
    if let Some(w) = warmup {
        body["warmup_s"] = json!(secs("--warmup", w, "5s, 1m")?);
    }
    start(
        "/__kaanal/sim/run",
        body,
        unnamed,
        seed,
        scenario,
        no_wait,
        print_run,
    )
}

/// **Private.** The limit of `sim sweep` as body fields: `--p99` a duration above zero, sent as
/// `p99_ms`; `--errors` a percentage, `1%` or `1`, 0 to 100, sent as `error_pct`. The server
/// checks `max_factor`.
fn limits(p99: Option<&str>, errors: Option<&str>, max_factor: Option<f64>) -> Result<Value, Fail> {
    let mut body = json!({});
    if let Some(p) = p99 {
        let s = secs("--p99", p, "500ms, 1s")?;
        if s <= 0.0 {
            return Err(Fail::usage(format!(
                "bad --p99 '{}' (above zero, e.g. 500ms, 1s)",
                clean(p)
            )));
        }
        body["p99_ms"] = json!(s * 1000.0);
    }
    if let Some(e) = errors {
        let t = e.trim();
        let pct = t
            .strip_suffix('%')
            .unwrap_or(t)
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|x| (0.0..=100.0).contains(x))
            .ok_or_else(|| {
                Fail::usage(format!(
                    "bad --errors '{}' (a percentage, 0 to 100, e.g. 1%)",
                    clean(e)
                ))
            })?;
        body["error_pct"] = json!(pct);
    }
    if let Some(m) = max_factor {
        // JSON has no NaN or infinity: `json!` would turn one into null and the server would use its default
        if !m.is_finite() {
            return Err(Fail::usage(format!(
                "bad --max-factor '{m}' (a number, 1 to 64)"
            )));
        }
        body["max_factor"] = json!(m);
    }
    Ok(body)
}

/// **Private.** `sim sweep`: starts a load sweep with the limit fields `body` and, unless told
/// not to, waits for its finding. A sweep has no log, so none is fetched.
fn sweep(
    mut body: Value,
    profile: Option<&str>,
    file: Option<&Path>,
    seed: Option<u64>,
    scenario: Option<String>,
    no_wait: bool,
) -> Result<(), Fail> {
    let (profile, unnamed) = profile_of(profile, file)?;
    body["profile"] = profile;
    start(
        "/__kaanal/sim/sweep",
        body,
        unnamed,
        seed,
        scenario,
        no_wait,
        print_sweep,
    )
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
                "{} is still going; look again with `kaanal sim show {}`",
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
                "{:<5} {:<6} {:<8} generation {} profile {}{}",
                clean(r["id"].as_str().unwrap_or("")),
                clean(r["kind"].as_str().unwrap_or("run")),
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
    output::emit(&v, || {
        if v["kind"] == "sweep" {
            print_sweep(&v);
        } else {
            print_run(&v);
        }
    });
    Ok(())
}

/// **Private.** A share (0 to 1) as a percentage, `1%`, `0.5%`; `<0.01%` for a share too small
/// for two decimals, so a row never says `0%` and over a limit of zero at once.
fn pct(v: &Value) -> String {
    let p = v.as_f64().unwrap_or(0.0) * 100.0;
    if p > 0.0 && p < 0.005 {
        return "<0.01%".into();
    }
    let s = format!("{p:.2}");
    format!("{}%", s.trim_end_matches('0').trim_end_matches('.'))
}

/// **Private.** The finding of a sweep as text: the verdict, the first node to saturate, the
/// dominant cause, the warnings (a cut ladder among them) and the label, then one row per point. Read from
/// `sweep` alone: its `points` are the full, sorted list once the sweep is done.
fn print_sweep(v: &Value) {
    let id = clean(v["id"].as_str().unwrap_or(""));
    let state = clean(v["state"].as_str().unwrap_or(""));
    println!("sweep {id} {state} (generation {})", v["generation"]);
    let s = &v["sweep"];
    if !s.is_object() {
        if let Some(e) = v["error"].as_str() {
            println!("error: {}", clean(e));
        }
        return;
    }
    let lim_ms = s["limit"]["p99_ms"].as_f64().unwrap_or(0.0);
    let lim_p99 = Duration::try_from_secs_f64(lim_ms / 1000.0).map_or_else(
        |_| format!("{lim_ms} ms"),
        |d| humantime::format_duration(d).to_string(),
    );
    // the record says which limit each point is over (`kaanal_sim::sweep::Limit::over`): no second copy of the rule here
    let p99_over = |p: &Value| p["over"]["p99"] == true;
    let err_over = |p: &Value| p["over"]["errors"] == true;
    let points = s["points"].as_array().map_or(&[][..], Vec::as_slice);
    let crossed = points.iter().find(|p| p99_over(p) || err_over(p)).map(|p| {
        let mut why = Vec::new();
        if p99_over(p) {
            why.push(format!("p99 over {lim_p99}"));
        }
        if err_over(p) {
            why.push(format!("errors over {}", pct(&s["limit"]["error_rate"])));
        }
        format!(": {}", why.join(" and "))
    });
    let vd = &s["verdict"];
    let at = format!(
        "{:.0} req/s ({:.2}x)",
        vd["req_s"].as_f64().unwrap_or(0.0),
        vd["k"].as_f64().unwrap_or(0.0)
    );
    let crossed = crossed.unwrap_or_default();
    match vd["kind"].as_str().unwrap_or("") {
        "knee" => println!("knee at {at}{crossed}"),
        "broken_at_lowest" => println!("over the limit already at the lowest point, {at}{crossed}"),
        // a cut ladder only held to the last point that ran: it does not claim the top
        "holds_to" if s["stopped_at"].is_number() => println!(
            "held to {at}, the last point run (the ladder stopped above it): no point over p99 {lim_p99} or errors {}",
            pct(&s["limit"]["error_rate"])
        ),
        "holds_to" => println!(
            "holds to {at}: no point over p99 {lim_p99} or errors {}",
            pct(&s["limit"]["error_rate"])
        ),
        k => println!("verdict {}", clean(k)),
    }
    let f = &s["first_to_saturate"];
    if f.is_object() {
        println!(
            "first to saturate: {} at {:.0} req/s ({:.2}x){}",
            clean(f["node"].as_str().unwrap_or("")),
            f["req_s"].as_f64().unwrap_or(0.0),
            f["k"].as_f64().unwrap_or(0.0),
            f["analytic_req_s"]
                .as_f64()
                .map_or(String::new(), |a| format!(", analytic {a:.0} req/s"))
        );
    }
    if let Some(c) = s["dominant_cause"].as_str() {
        println!("dominant cause: {}", clean(c));
    }
    // a cut ladder (`stopped_at`) is told by the sim's own warning, with its reason and req/s
    for w in s["warnings"].as_array().into_iter().flatten() {
        println!("warning: {}", clean(w.as_str().unwrap_or("")));
    }
    println!("{}", clean(s["label"].as_str().unwrap_or("")));
    println!(
        "{:>10}  {:>10}  {:>10}  {:>7}  ok",
        "load req/s", "goodput", "p99 ms", "errors"
    );
    for p in points {
        println!(
            "{:>10.1}  {:>10.1}  {:>10}  {:>7}  {}",
            p["req_s"].as_f64().unwrap_or(0.0),
            p["goodput_req_s"].as_f64().unwrap_or(0.0),
            ms(&p["p99_ms"]),
            pct(&p["error_rate"]),
            if p99_over(p) || err_over(p) {
                "no"
            } else {
                "yes"
            }
        );
    }
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
    print_cost(&v["cost"]);
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

/// **Private.** The estimated cost line of a run: dollars an hour of what has a price, how many
/// nodes that is, and the region each cloud was priced in. Silent when the record has no cost.
fn print_cost(c: &Value) {
    let Some(total) = c["total_micro_usd_hr"].as_u64() else {
        return;
    };
    let regions: Vec<String> = c["regions"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(p, r)| format!("{} {}", clean(p), clean(r.as_str().unwrap_or(""))))
        .collect();
    #[allow(clippy::cast_precision_loss)]
    let usd = total as f64 / 1e6;
    println!(
        "estimated cost: ${usd:.4} an hour ({} of {} nodes priced; {}) - an estimate, not a quote",
        c["priced"],
        c["priced"].as_u64().unwrap_or(0) + c["unpriced"].as_u64().unwrap_or(0),
        regions.join(", ")
    );
    // what the run sent out to the internet is already in the total; say how much, and when it was guessed
    for e in c["egress"].as_array().into_iter().flatten() {
        let Some(m) = e["micro_usd_hr"].as_u64() else {
            continue;
        };
        #[allow(clippy::cast_precision_loss)]
        let usd = m as f64 / 1e6;
        println!(
            "  of which sent out to the internet: ${usd:.4} an hour at {}'s rate{}",
            clean(e["provider"].as_str().unwrap_or("")),
            e["note"]
                .as_str()
                .map_or(String::new(), |n| format!(" ({})", clean(n)))
        );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tiny_share_is_not_printed_as_zero() {
        assert_eq!(pct(&json!(0.0)), "0%");
        assert_eq!(pct(&json!(0.00004)), "<0.01%");
        assert_eq!(pct(&json!(0.0001)), "0.01%");
        assert_eq!(pct(&json!(0.01)), "1%");
        assert_eq!(pct(&json!(0.005)), "0.5%");
    }
}

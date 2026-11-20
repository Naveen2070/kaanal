//! The text of a run and of a sweep as `sim run`, `sim sweep` and `sim show` print it, read from the record alone; the commands in `mod.rs` fetch, these print.

use std::fmt::Write as _;

use crate::{Duration, Value, clean};

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
pub(super) fn print_sweep(v: &Value) {
    let id = clean(v["id"].as_str().unwrap_or(""));
    let state = clean(v["state"].as_str().unwrap_or(""));
    println!("sweep {id} {state} (generation {})", v["generation"]);
    // what a re-run needs to reproduce it (the report carries both)
    if v["seed"].is_number() {
        println!("seed {}", v["seed"]);
    }
    if let Some(f) = v["fingerprint"].as_str() {
        println!("fingerprint {}", clean(f));
    }
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
pub(super) fn print_run(v: &Value) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json;

    #[test]
    fn a_tiny_share_is_not_printed_as_zero() {
        assert_eq!(pct(&json!(0.0)), "0%");
        assert_eq!(pct(&json!(0.00004)), "<0.01%");
        assert_eq!(pct(&json!(0.0001)), "0.01%");
        assert_eq!(pct(&json!(0.01)), "1%");
        assert_eq!(pct(&json!(0.005)), "0.5%");
    }
}

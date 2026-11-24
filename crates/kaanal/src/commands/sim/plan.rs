//! `kaanal sim plan`: starts a sizing plan, waits for it, and reads the record aloud (Step 9c-1).
//! The walk, the prices and the check are the server's; the text here is read from the record
//! alone and, like the report, calls the result a relative what-if, never a prediction.

use std::fmt::Write as _;

use super::print::{chaos_line, ms, pct};
use super::{limits, profile_of, report_text, start, write_file};
use crate::cli::PlanArgs;
use crate::{Duration, Fail, Value, call_text, clean, err, json};

/// **Private.** `sim plan`: the usage errors first (nothing is read or sent for a bad flag), then
/// a file that exists is refused before the plan runs, then post, wait, print and write the files.
pub(super) fn plan(a: PlanArgs) -> Result<(), Fail> {
    let mut body = limits(a.p99.as_deref(), a.errors.as_deref(), None)?;
    body["target"] = target(&a.target)?;
    if let Some(b) = a.budget.as_deref() {
        body["budget_per_hour"] = json!(budget(b)?);
    }
    if !a.keep.is_empty() {
        body["keep"] = json!(a.keep);
    }
    if let Some(n) = a.max_steps {
        body["max_steps"] = json!(n);
    }
    super::plan_cheapest::body(&a, &mut body)?;
    if a.force && a.report.is_none() && a.output.is_none() {
        return Err(Fail::usage("--force needs --report or -o"));
    }
    if a.report.is_some() && a.report == a.output {
        return Err(Fail::usage("--report and -o name the same file"));
    }
    for f in [&a.report, &a.output].into_iter().flatten() {
        if !a.force && f.exists() {
            return Err(err(format!(
                "{} exists; --force overwrites it",
                clean(&f.display().to_string())
            )));
        }
    }
    let (profile, unnamed) = profile_of(a.profile.as_deref(), a.file.as_deref())?;
    // none named: the server plans for the first profile of the design (the sweep asks for a name)
    if !unnamed {
        body["profile"] = profile;
    }
    let id = start(
        "/__kaanal/sim/plan",
        body,
        false,
        a.seed,
        a.scenario,
        a.no_wait,
        print_plan,
    )?;
    // a failed plan returned its error above: only a finished one has a report and a draft
    // both texts are fetched before either file is written, so a server answer that fails leaves
    // nothing behind (a disk error on the second write still can: the first file stays, as said)
    let report = a
        .report
        .as_ref()
        .map(|f| report_text(&id, false).map(|t| (f, t)))
        .transpose()?;
    let draft = a
        .output
        .as_ref()
        .map(|f| draft_text(&id).map(|t| (f, t)))
        .transpose()?;
    for (f, text) in report.into_iter().chain(draft) {
        write_file(f, &text, a.force)?;
        eprintln!("wrote {}", clean(&f.display().to_string()));
    }
    Ok(())
}

/// **Private.** The proposed draft of plan `id`.
fn draft_text(id: &str) -> Result<String, Fail> {
    let draft = call_text(&format!(
        "/__kaanal/sim/runs/{}/draft",
        crate::encode_segment(id)
    ))?;
    if draft.trim().is_empty() {
        return Err(err("the server sent no proposed draft"));
    }
    Ok(draft)
}

/// **Private.** `--target` as the body's `target`: `3x` a multiple, a bare number a req/s; both
/// finite and above zero (the server checks the range).
fn target(t: &str) -> Result<Value, Fail> {
    let s = t.trim();
    let (num, multiple) = s.strip_suffix(['x', 'X']).map_or((s, false), |n| (n, true));
    let n = num
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite() && *n > 0.0);
    match n {
        Some(n) if multiple => Ok(json!({ "multiple": n })),
        Some(n) => Ok(json!({ "req_s": n })),
        None => Err(Fail::usage(format!(
            "bad --target '{}' (a multiple like 3x, or a rate in req/s like 500, above zero)",
            clean(t)
        ))),
    }
}

/// **Private.** `--budget` as dollars an hour: a decimal, 0 or more, an optional `$` in front.
fn budget(b: &str) -> Result<f64, Fail> {
    let t = b.trim();
    t.strip_prefix('$')
        .unwrap_or(t)
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite() && *n >= 0.0)
        .ok_or_else(|| {
            Fail::usage(format!(
                "bad --budget '{}' (dollars an hour, 0 or more, e.g. 2.00)",
                clean(b)
            ))
        })
}

/// **Private.** Micro-USD an hour as dollars the way the report writes them (`report/plan.rs`,
/// `dollars`): integers only, the digits as they are, two decimals at least (`840000` is
/// `$0.84`, `3500` is `$0.0035`, `1005000` is `$1.005`), so the text and the report never
/// round apart.
pub(super) fn usd(micro: u64) -> String {
    let mut frac = format!("{:06}", micro % 1_000_000);
    while frac.len() > 2 && frac.ends_with('0') {
        frac.pop();
    }
    format!("${}.{frac}", micro / 1_000_000)
}

/// **Private.** A price change, signed.
fn delta(micro: i64) -> String {
    let sign = if micro < 0 { '-' } else { '+' };
    format!("{sign}{}", usd(micro.unsigned_abs()))
}

/// **Private.** A price in the record, or `price n/a`.
pub(super) fn price(v: &Value) -> String {
    v.as_u64()
        .map_or("price n/a".into(), |m| format!("{}/h", usd(m)))
}

/// **Private.** A number to two decimals, zeros trimmed (`3`, `2.5`).
fn num(x: f64) -> String {
    let s = format!("{x:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// **Private.** Milliseconds as the sweep prints them in text: seconds from one second up.
fn lat(v: &Value) -> String {
    let m = v.as_f64().unwrap_or(0.0);
    if m >= 1000.0 {
        format!("{:.1}s", m / 1000.0)
    } else {
        format!("{} ms", ms(v))
    }
}

/// **Private.** A req/s estimate, or `-`.
fn cap(v: &Value) -> String {
    v.as_f64().map_or("-".into(), |c| format!("{c:.0}"))
}

/// **Private.** `n unit` with the plural when `n` is not 1.
fn count(n: &Value, one: &str, many: &str) -> String {
    format!("{n} {}", if n.as_f64() == Some(1.0) { one } else { many })
}

/// **Private.** Bytes as GiB or MiB when exact.
fn bytes(v: &Value) -> String {
    let b = v.as_u64().unwrap_or(0);
    match (b % (1 << 30), b % (1 << 20)) {
        (0, _) if b > 0 => format!("{}GiB", b >> 30),
        (_, 0) if b > 0 => format!("{}MiB", b >> 20),
        _ => format!("{b} B"),
    }
}

/// **Private.** A move as `from -> to`.
fn mv(m: &Value) -> String {
    let k = &m["kind"];
    let (from, to) = (&k["from"], &k["to"]);
    match k["knob"].as_str().unwrap_or("") {
        "class" => format!(
            "{} -> {}",
            clean(from.as_str().unwrap_or("")),
            clean(to.as_str().unwrap_or(""))
        ),
        "cores" => format!(
            "{} -> {}",
            count(from, "core", "cores"),
            count(to, "core", "cores")
        ),
        "memory" => format!("memory {} -> {}", bytes(from), bytes(to)),
        "copies" => format!(
            "{} -> {}",
            count(from, "copy", "copies"),
            count(to, "copy", "copies")
        ),
        _ => "?".into(),
    }
}

/// **Private.** How a walk ended, in words, for the verdict line and a family answer.
fn verdict_words(v: &Value, req_s: f64, budget: Option<u64>, family: bool) -> String {
    match v["kind"].as_str().unwrap_or("") {
        "already_holds" if family => "already holds the target as it is, no step needed".into(),
        "already_holds" => "the design in force already holds the target; nothing to change".into(),
        "holds" => format!("holds {req_s:.0} req/s"),
        "cannot_hold" => {
            let node = v["node"]
                .as_str()
                .map_or(String::new(), |n| format!("{}: ", clean(n)));
            format!(
                "cannot hold it by sizing: {node}{}",
                clean(v["reason"].as_str().unwrap_or(""))
            )
        }
        "over_budget" => {
            let over = v["over_by_micro_usd_hr"].as_u64().unwrap_or(0);
            let b = budget.unwrap_or(0);
            if v["holds"].as_bool().unwrap_or(false) {
                format!(
                    "holds {req_s:.0} req/s but is over budget by {}/h (budget {}/h)",
                    usd(over),
                    usd(b)
                )
            } else {
                format!(
                    "no holding design was found within budget; the next step needed would cost {}/h, over budget by {}/h (budget {}/h)",
                    usd(b + over),
                    usd(over),
                    usd(b)
                )
            }
        }
        "no_progress" => format!(
            "stopped: two steps in a row gained nothing (the last on {})",
            clean(v["node"].as_str().unwrap_or(""))
        ),
        "step_cap" => "stopped at the step cap without holding the target".into(),
        k => format!("verdict {}", clean(k)),
    }
}

/// **Private.** The budget judged with per-use prices (the record's `budget_check`), as one line;
/// `None` without a budget.
fn budget_line(b: &Value) -> Option<String> {
    let budget = b["budget_micro_usd_hr"].as_u64()?;
    let cost = b["cost_micro_usd_hr"].as_u64()?;
    let against = if cost > budget {
        format!("over budget {}/h by {}/h", usd(budget), usd(cost - budget))
    } else {
        format!("within budget {}/h", usd(budget))
    };
    let mut s = format!(
        "budget check (per-use prices at the target run's visits): the plan shown costs {}/h, {against}",
        usd(cost)
    );
    if b["rejudged"].as_bool().unwrap_or(false) {
        s.push_str("; the walk's prices leave per-use prices out and found the plan within budget, so the verdict is over budget");
    }
    Some(s)
}

/// **Private.** The plan record as text: the request, the design in force, the steps and the menu
/// at each, the verdict, the re-check, the measured knee of the proposed design, and the database
/// family what-ifs; every figure is a simulation's relative what-if and every price an estimate.
/// A plan still running prints the steps taken so far; a failed one its error.
pub(super) fn print_plan(v: &Value) {
    print!("{}", plan_text(v));
}

/// **Private.** [`print_plan`]'s text.
fn plan_text(v: &Value) -> String {
    let mut out = String::new();
    let out = &mut out;
    text_of(v, out);
    std::mem::take(out)
}

/// **Private.** Writes the text of record `v` to `out`.
fn text_of(v: &Value, out: &mut String) {
    let id = clean(v["id"].as_str().unwrap_or(""));
    let _ = writeln!(
        out,
        "plan {id} {} (generation {})",
        clean(v["state"].as_str().unwrap_or("")),
        v["generation"]
    );
    if v["seed"].is_number() {
        let _ = writeln!(out, "seed {}", v["seed"]);
    }
    if let Some(f) = v["fingerprint"].as_str() {
        let _ = writeln!(out, "fingerprint {}", clean(f));
    }
    if let Some(c) = chaos_line(v) {
        let _ = writeln!(out, "chaos in force: {c}");
    }
    let p = &v["plan"];
    if !p.is_object() {
        if let Some(e) = v["error"].as_str() {
            let _ = writeln!(out, "error: {}", clean(e));
        } else if let Some(n) = v["plan_steps"].as_array() {
            let _ = writeln!(out, "{} steps so far", n.len());
        }
        return;
    }
    let r = &p["request"];
    let req_s = r["req_s"].as_f64().unwrap_or(0.0);
    let mut head = format!(
        "Target: {}x {} ({req_s:.0} req/s), p99 <= {}, errors <= {}",
        num(r["factor"].as_f64().unwrap_or(0.0)),
        clean(v["profile"].as_str().unwrap_or("")),
        secs_text(r["limit"]["p99_ms"].as_f64().unwrap_or(0.0)),
        pct(&r["limit"]["error_rate"]),
    );
    if let Some(b) = r["budget_micro_usd_hr"].as_u64() {
        let _ = write!(head, "    budget: {}/h", usd(b));
    }
    let _ = writeln!(out, "{head}");
    let s = &p["start"];
    let _ = writeln!(
        out,
        "Now: at the target p99 {}, errors {}, capacity est. {} req/s. {}",
        lat(&s["p99_ms"]),
        pct(&s["error_rate"]),
        cap(&s["capacity_req_s"]),
        price(&p["price_before"])
    );
    let vd = &p["verdict"];
    let budget = r["budget_micro_usd_hr"].as_u64();
    let steps = p["steps"].as_array().map_or(&[][..], Vec::as_slice);
    let cheapest_line = super::plan_cheapest::verdict_line(p);
    match vd["kind"].as_str().unwrap_or("") {
        _ if cheapest_line.is_some() => {
            let _ = writeln!(out, "{}", cheapest_line.unwrap_or_default());
        }
        "already_holds" => drop(writeln!(out, "{}", verdict_words(vd, req_s, budget, false))),
        "holds" => drop(writeln!(
            out,
            "Plan ({}):  {}",
            verdict_words(vd, req_s, budget, false),
            price(&p["price_after"])
        )),
        "over_budget" if vd["holds"].as_bool().unwrap_or(false) => drop(writeln!(
            out,
            "Plan ({}):  {}",
            verdict_words(vd, req_s, budget, false),
            price(&p["price_after"])
        )),
        _ => drop(writeln!(
            out,
            "Plan (does not hold the target; {}):  {}",
            verdict_words(vd, req_s, budget, false),
            price(&p["price_after"])
        )),
    }
    if let Some(line) = budget_line(&p["budget_check"]) {
        let _ = writeln!(out, "{line}");
    }
    print_steps(steps, out);
    if let Some(u) = p["unpriced_nodes"].as_array().filter(|u| !u.is_empty()) {
        let names: Vec<String> = u.iter().map(|n| clean(n.as_str().unwrap_or(""))).collect();
        let _ = writeln!(out, "not priced: {}", names.join(", "));
    }
    print_checked(p, v, out);
    print_families(p, req_s, out);
    let _ = writeln!(out, "{}", clean(p["label"].as_str().unwrap_or("")));
    super::plan_cheapest::print(p, out);
}

/// **Private.** One row per step, its note, then the menu of each step that had a choice.
fn print_steps(steps: &[Value], out: &mut String) {
    let w = steps
        .iter()
        .map(|s| {
            clean(s["mv"]["node"].as_str().unwrap_or(""))
                .chars()
                .count()
        })
        .max()
        .unwrap_or(0);
    for (i, s) in steps.iter().enumerate() {
        let d = s["price_delta_micro_usd_hr"]
            .as_i64()
            .map_or("price n/a".into(), |d| format!("{}/h", delta(d)));
        let _ = writeln!(
            out,
            "{:>2}  {:<w$}  {}   {d}   capacity est. {} -> {} req/s   p99 at target {} -> {}",
            i + 1,
            clean(s["mv"]["node"].as_str().unwrap_or("")),
            mv(&s["mv"]),
            cap(&s["before"]["capacity_req_s"]),
            cap(&s["after"]["capacity_req_s"]),
            lat(&s["before"]["p99_ms"]),
            lat(&s["after"]["p99_ms"]),
        );
        if let Some(n) = s["note"].as_str() {
            let _ = writeln!(out, "    note: {}", clean(n));
        }
    }
    for (i, s) in steps.iter().enumerate() {
        let menu: Vec<String> = s["menu"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|c| {
                format!(
                    "{} ({}, {} req/s est.{})",
                    mv(&c["mv"]),
                    price(&c["price_micro_usd_hr"]),
                    cap(&c["effect"]["capacity_req_s"]),
                    if c["holds"] == true { ", holds" } else { "" }
                )
            })
            .collect();
        if menu.len() > 1 {
            let _ = writeln!(out, " menu at step {}: {}", i + 1, menu.join(", "));
        }
    }
}

/// **Private.** A limit in milliseconds as a duration, `1s`, `500ms`.
fn secs_text(ms: f64) -> String {
    Duration::try_from_secs_f64(ms / 1000.0).map_or_else(
        |_| format!("{ms} ms"),
        |d| humantime::format_duration(d).to_string(),
    )
}

/// **Private.** The re-check of the proposed draft and the knee a sweep of it finds.
fn print_checked(plan: &Value, rec: &Value, out: &mut String) {
    let check = &plan["verify"];
    let _ = writeln!(
        out,
        "Checked: the proposed draft run again at the target {}: p99 {}, errors {}",
        if check["holds"] == true {
            "holds"
        } else {
            "does not hold"
        },
        lat(&check["p99_ms"]),
        pct(&check["error_rate"])
    );
    if let Some(n) = check["cost_note"].as_str() {
        let _ = writeln!(out, "  note: {}", clean(n));
    }
    let knee = &plan["measured_knee"];
    let at = format!(
        "{:.0} req/s ({}x)",
        knee["req_s"].as_f64().unwrap_or(0.0),
        num(knee["k"].as_f64().unwrap_or(0.0))
    );
    let id = clean(rec["id"].as_str().unwrap_or(""));
    match knee["kind"].as_str() {
        Some("knee") => drop(writeln!(
            out,
            "  a sweep of the proposed design finds its knee at {at} (see `kaanal sim report {id}`)"
        )),
        Some("broken_at_lowest") => drop(writeln!(
            out,
            "  a sweep of the proposed design is over the limit already at its lowest point, {at}"
        )),
        Some("holds_to") => drop(writeln!(
            out,
            "  a sweep of the proposed design finds no point over the limit up to {at}, the highest it ran"
        )),
        _ => match plan["sweep_error"].as_str() {
            Some(why) => drop(writeln!(
                out,
                "  the sweep of the proposed design did not run: {}",
                clean(why)
            )),
            None => drop(writeln!(out, "  no sweep of the proposed design was made")),
        },
    }
}

/// **Private.** The database-family what-ifs: each a walk on another flavor, labelled as a what-if
/// on preset figures, never a recommendation to migrate.
fn print_families(p: &Value, req_s: f64, out: &mut String) {
    for f in p["families"].as_array().into_iter().flatten() {
        if f["kind"] == "skipped" {
            let flavor = f["flavor"]
                .as_str()
                .map_or(String::new(), |x| format!(" as {}", clean(x)));
            let _ = writeln!(
                out,
                "If you changed the database family: {}{flavor} skipped: {}",
                clean(f["node"].as_str().unwrap_or("")),
                clean(f["reason"].as_str().unwrap_or(""))
            );
            continue;
        }
        let a = &f["plan"];
        let notes: Vec<String> = f["notes"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|n| clean(n.as_str().unwrap_or("")))
            .collect();
        let _ = writeln!(
            out,
            "If you changed the database family: {} as {}, in this what-if: {}{}{}",
            clean(f["node"].as_str().unwrap_or("")),
            clean(f["flavor"].as_str().unwrap_or("")),
            verdict_words(
                &a["verdict"],
                req_s,
                p["request"]["budget_micro_usd_hr"].as_u64(),
                true
            ),
            if a["verdict"]["kind"] == "already_holds" {
                String::new()
            } else {
                format!(" at {}", price(&a["price_after"]))
            },
            if notes.is_empty() {
                String::new()
            } else {
                format!(" ({})", notes.join("; "))
            }
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Cmd, SimCmd};
    use clap::Parser as _;

    #[test]
    fn money_is_exact_integer_dollars_as_the_report_writes_it() {
        // the report's rule (kaanal-server report/plan.rs): digits as they are, two decimals at least
        for (micro, want) in [
            (0, "$0.00"),
            (9_999, "$0.009999"),
            (10_000, "$0.01"),
            (3_500, "$0.0035"),
            (840_000, "$0.84"),
            (1_005_000, "$1.005"),
            (2_000_000, "$2.00"),
            (1_234_567, "$1.234567"),
        ] {
            assert_eq!(usd(micro), want, "{micro}");
        }
        assert_eq!(delta(-200_000), "-$0.20");
        assert_eq!(delta(580_000), "+$0.58");
        assert_eq!(delta(0), "+$0.00");
        assert_eq!(delta(i64::MIN), "-$9223372036854.775808");
    }

    /// A finished plan record written by hand: an over-budget walk with two steps (the second on a
    /// node whose name has a multi-byte letter, a price that falls), a menu, both family kinds.
    fn record() -> Value {
        let eff =
            |p99: f64, cap: f64| json!({ "p99_ms": p99, "error_rate": 0.0, "capacity_req_s": cap });
        json!({
            "id": "s3", "kind": "plan", "state": "done", "generation": 1, "profile": "peak",
            "plan": {
                "label": "LABEL",
                "request": { "factor": 3.0, "req_s": 150.0,
                    "limit": { "p99_ms": 1000.0, "error_rate": 0.01 }, "budget_micro_usd_hr": 2_000_000 },
                "start": eff(4100.0, 49.0),
                "verdict": { "kind": "over_budget", "over_by_micro_usd_hr": 300_000, "holds": false },
                "budget_check": { "budget_micro_usd_hr": 2_000_000, "cost_micro_usd_hr": 2_450_000, "rejudged": false },
                "price_before": 840_000, "price_after": 2_300_000, "unpriced_nodes": [],
                "steps": [
                    { "mv": { "node": "db", "kind": { "knob": "class", "from": "db.m5.large", "to": "db.m5.2xlarge" } },
                      "price_micro_usd_hr": 1_420_000, "price_delta_micro_usd_hr": 580_000,
                      "before": eff(4100.0, 49.0), "after": eff(600.0, 118.0),
                      "menu": [
                        { "mv": { "node": "db", "kind": { "knob": "class", "from": "db.m5.large", "to": "db.m5.2xlarge" } },
                          "price_micro_usd_hr": 1_420_000, "effect": eff(600.0, 118.0), "holds": false },
                        { "mv": { "node": "db", "kind": { "knob": "copies", "from": 1, "to": 2 } },
                          "price_micro_usd_hr": 1_130_000, "effect": eff(2000.0, 77.0), "holds": false } ],
                      "note": null },
                    { "mv": { "node": "café", "kind": { "knob": "copies", "from": 2, "to": 4 } },
                      "price_micro_usd_hr": 1_220_000, "price_delta_micro_usd_hr": -200_000,
                      "before": eff(600.0, 118.0), "after": eff(300.0, 251.0), "menu": [], "note": null } ],
                "families": [
                    { "kind": "answer", "node": "db", "flavor": "dynamodb",
                      "plan": { "verdict": { "kind": "holds" }, "price_after": 1_200_000 },
                      "notes": ["assumes your application can use this family; preset estimates"] },
                    { "kind": "answer", "node": "db", "flavor": "mongodb",
                      "plan": { "verdict": { "kind": "holds" }, "price_after": null }, "notes": [] },
                    { "kind": "answer", "node": "db", "flavor": "cassandra",
                      "plan": { "verdict": { "kind": "already_holds" }, "price_after": null }, "notes": [] },
                    { "kind": "skipped", "node": "cache", "flavor": null, "reason": "nothing to size" } ],
                "verify": { "holds": false, "p99_ms": 1500.0, "error_rate": 0.0, "cost_note": "understated" },
                "measured_knee": { "kind": "knee", "k": 4.0, "req_s": 262.0 }
            }
        })
    }

    #[test]
    fn an_over_budget_plan_that_holds_says_so() {
        let mut r = record();
        r["plan"]["verdict"] =
            json!({ "kind": "over_budget", "over_by_micro_usd_hr": 300_000, "holds": true });
        let t = plan_text(&r);
        assert!(
            t.contains("Plan (holds 150 req/s but is over budget by $0.30/h (budget $2.00/h))"),
            "{t}"
        );
        assert!(!t.contains("does not hold the target"), "{t}");
    }

    /// Per-use prices put a plan the walk found within the budget over it: the verdict is over
    /// budget and the budget line says why.
    #[test]
    fn a_plan_over_budget_only_with_per_use_prices_says_so() {
        let mut r = record();
        r["plan"]["verdict"] =
            json!({ "kind": "over_budget", "over_by_micro_usd_hr": 450_000, "holds": true });
        r["plan"]["budget_check"]["rejudged"] = json!(true);
        let t = plan_text(&r);
        assert!(t.contains("over budget $2.00/h by $0.45/h; the walk's prices leave per-use prices out and found the plan within budget, so the verdict is over budget"), "{t}");
        r["plan"]["budget_check"] = json!({ "budget_micro_usd_hr": 2_000_000, "cost_micro_usd_hr": 1_900_000, "rejudged": false });
        let t = plan_text(&r);
        assert!(
            t.contains("the plan shown costs $1.90/h, within budget $2.00/h"),
            "{t}"
        );
    }

    #[test]
    fn the_text_of_a_priced_over_budget_plan_with_families() {
        let t = plan_text(&record());
        for want in [
            "Target: 3x peak (150 req/s), p99 <= 1s, errors <= 1%    budget: $2.00/h",
            "Now: at the target p99 4.1s, errors 0%, capacity est. 49 req/s. $0.84/h",
            "no holding design was found within budget; the next step needed would cost $2.30/h, over budget by $0.30/h (budget $2.00/h)",
            "budget check (per-use prices at the target run's visits): the plan shown costs $2.45/h, over budget $2.00/h by $0.45/h",
            "):  $2.30/h",
            // the node column is padded by characters: "db" to the four letters of "café"
            " 1  db    db.m5.large -> db.m5.2xlarge   +$0.58/h   capacity est. 49 -> 118 req/s",
            " 2  café  2 copies -> 4 copies   -$0.20/h",
            " menu at step 1: db.m5.large -> db.m5.2xlarge ($1.42/h, 118 req/s est.), 1 copy -> 2 copies ($1.13/h, 77 req/s est.)",
            "Checked: the proposed draft run again at the target does not hold: p99 1.5s",
            "  note: understated",
            "its knee at 262 req/s (4x)",
            "If you changed the database family: db as dynamodb, in this what-if: holds 150 req/s at $1.20/h (assumes your application can use this family; preset estimates)",
            "db as mongodb, in this what-if: holds 150 req/s at price n/a",
            "db as cassandra, in this what-if: already holds the target as it is, no step needed",
            "If you changed the database family: cache skipped: nothing to size",
        ] {
            assert!(t.contains(want), "missing {want:?} in:\n{t}");
        }
        assert!(!t.contains("micro") && !t.contains("recommend"), "{t}");
        assert!(
            !t.contains("is within budget"),
            "never claims the shown plan is within: {t}"
        );
        assert!(t.trim_end().ends_with("LABEL"), "{t}");
    }

    /// The recipe the report prints, quoted as a shell would have it, goes through the real parser.
    #[test]
    fn the_recipe_with_quoted_values_parses_to_the_same_request() {
        let recipe = "kaanal sim plan --profile='peak load' --target=2.5x --p99 '1s 500ms' --errors 0.5% --budget=1.234567 --keep='a b' --keep=--help --keep='it'\\''s' --max-steps 6 --seed 4 --scenario=--help";
        let words = shell_words(recipe);
        let Cmd::Sim {
            cmd: SimCmd::Plan(a),
            ..
        } = Cli::try_parse_from(&words).unwrap().cmd
        else {
            panic!("not a sim plan: {words:?}");
        };
        assert_eq!(a.profile.as_deref(), Some("peak load"));
        assert_eq!(a.target, "2.5x");
        assert_eq!(a.p99.as_deref(), Some("1s 500ms"));
        assert_eq!(a.errors.as_deref(), Some("0.5%"));
        assert_eq!(a.budget.as_deref(), Some("1.234567"));
        assert_eq!(a.keep, ["a b", "--help", "it's"]);
        assert_eq!((a.max_steps, a.seed), (Some(6), Some(4)));
        assert_eq!(a.scenario.as_deref(), Some("--help"));
        // and the values mean what they say
        assert_eq!(
            limits(a.p99.as_deref(), a.errors.as_deref(), None).unwrap(),
            json!({ "p99_ms": 1500.0, "error_pct": 0.5 })
        );
        assert_eq!(target(&a.target).unwrap(), json!({ "multiple": 2.5 }));
        assert_eq!(budget("1.234567").unwrap(), 1.234_567);
    }

    /// POSIX words as the report's `shell` writes them: bare text, '...' groups, and `\'` (a quote
    /// outside the groups).
    fn shell_words(s: &str) -> Vec<String> {
        let (mut out, mut cur, mut quoted) = (Vec::new(), String::new(), false);
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            match c {
                '\'' => quoted = !quoted,
                '\\' if !quoted => cur.extend(chars.next()),
                c if c.is_whitespace() && !quoted => {
                    if !cur.is_empty() {
                        out.push(std::mem::take(&mut cur));
                    }
                }
                c => cur.push(c),
            }
        }
        if !cur.is_empty() {
            out.push(cur);
        }
        out
    }
}

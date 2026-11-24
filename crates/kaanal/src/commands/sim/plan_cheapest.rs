//! `kaanal sim plan --cheapest` (Step 9c-2): the flags as body fields, and the text of a cheapest
//! record read aloud. The search, the prices and the comparison data are the server's; the words
//! here follow the report's (`report/plan_cheapest.rs`) and, like it, never call a design
//! "cheapest" without the limit it was found within, never say "cannot hold" for a search a limit
//! cut, and never narrate the changes as a price sequence (each shows its own effect only).

use std::fmt::Write as _;

use super::plan::{price, usd};
use super::secs;
use crate::cli::PlanArgs;
use crate::{Fail, Value, clean, json};

/// The providers a section can be run for.
const PROVIDERS: [&str; 3] = ["aws", "gcp", "azure"];

/// **Private.** The cheapest flags as body fields: `--time` rounded up to whole seconds, 1 to 600;
/// `--run-cap` 1 to 2000; `--providers` from aws, gcp, azure, or `none` alone. Bounds are the
/// server's, checked here so a bad flag is a usage error before anything is posted.
pub(super) fn body(a: &PlanArgs, body: &mut Value) -> Result<(), Fail> {
    if !a.cheapest {
        return Ok(());
    }
    body["cheapest"] = json!(true);
    if let Some(t) = a.time.as_deref() {
        let s = secs("--time", t, "150s, 3m")?.ceil();
        if !(1.0..=600.0).contains(&s) {
            return Err(Fail::usage(format!(
                "bad --time '{}' (1 second to 10 minutes, e.g. 150s, 3m)",
                clean(t)
            )));
        }
        // ponytail: s is a whole number in 1..=600 here, so the cast is exact
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let whole = s as u32;
        body["time_limit_s"] = json!(whole);
    }
    if let Some(n) = a.run_cap {
        if !(1..=2000).contains(&n) {
            return Err(Fail::usage(format!("bad --run-cap '{n}' (1 to 2000)")));
        }
        body["run_cap"] = json!(n);
    }
    if let Some(p) = &a.providers {
        let ok =
            p == &["none"] || (!p.is_empty() && p.iter().all(|x| PROVIDERS.contains(&x.as_str())));
        if !ok {
            return Err(Fail::usage(format!(
                "bad --providers '{}' (aws, gcp, azure, comma separated, or none)",
                clean(&p.join(","))
            )));
        }
        body["providers"] = json!(p);
    }
    Ok(())
}

/// **Private.** A provider as a person writes it.
fn provider_name(p: &str) -> String {
    match p {
        "aws" => "AWS".into(),
        "gcp" => "GCP".into(),
        "azure" => "Azure".into(),
        o => clean(o),
    }
}

/// **Private.** What cut a search short; `None` when it ended by itself.
fn cut(info: &Value) -> Option<&'static str> {
    match info["stop"].as_str()? {
        "run_cap" => Some("run cap"),
        "time_limit" => Some("time limit"),
        _ => None,
    }
}

/// **Private.** Why a search stopped, in words that never claim a minimum.
fn stop_words(info: &Value) -> &'static str {
    match info["stop"].as_str().unwrap_or("") {
        "exhausted" => "it ran out of candidates to try",
        "run_cap" => "it reached the run cap, so cheaper designs may be left untried",
        "time_limit" => "its time limit passed, so cheaper designs may be left untried",
        _ => "of a reason this client does not know",
    }
}

/// **Private.** Whether a verdict's design holds the target (an over-budget one may).
fn holds(v: &Value) -> bool {
    match v["kind"].as_str().unwrap_or("") {
        "already_holds" | "holds" => true,
        "over_budget" => v["holds"].as_bool().unwrap_or(false),
        _ => false,
    }
}

/// **Private.** A verdict as a short table cell.
fn outcome(v: &Value, info: &Value) -> String {
    let over = v["over_by_micro_usd_hr"].as_u64().unwrap_or(0);
    match v["kind"].as_str().unwrap_or("") {
        "already_holds" => "holds, no change needed".into(),
        "holds" => "holds".into(),
        "over_budget" if holds(v) => format!("holds, over budget by {}/h", usd(over)),
        _ if cut(info).is_some() => "no holding design found within the limit".into(),
        "over_budget" => "no holding design within the budget".into(),
        "cannot_hold" => "cannot hold by sizing".into(),
        "no_progress" => "stopped: two steps gained nothing".into(),
        "step_cap" => "stopped at the step cap".into(),
        k => format!("verdict {}", clean(k)),
    }
}

/// **Private.** The verdict line of a cheapest plan where the 9c-1 words would mislead: a design
/// in force that holds (the search may still have changes to take off) and a search a limit cut
/// before a holding design was found. `None` leaves the 9c-1 line.
pub(super) fn verdict_line(p: &Value) -> Option<String> {
    let info = &p["search"];
    if !info.is_object() {
        return None;
    }
    let v = &p["verdict"];
    let what = cut(info);
    match (v["kind"].as_str().unwrap_or(""), what) {
        ("already_holds", None) => Some("Plan: the design in force already holds the target, and no cheaper design that holds was found within the limit.".into()),
        ("already_holds", Some(w)) => Some(format!("Plan: the design in force already holds the target. No cheaper design that holds was found within the limit: the search stopped at the {w} before it ran out of candidates, so one may exist.")),
        (_, Some(w)) if !holds(v) => Some(format!("Plan: no design that holds the target was found within the limit: the search stopped at the {w} before it could say more. That does not show that none exists.  {}", price(&p["price_after"]))),
        _ => None,
    }
}

/// **Private.** The search lines: why it stopped, runs, time, candidates refused for size.
fn search_lines(info: &Value, out: &mut String, indent: &str) {
    let _ = writeln!(out, "{indent}stopped because {}", stop_words(info));
    let _ = writeln!(
        out,
        "{indent}runs {} of {} allowed, time {} ms used of {} ms",
        info["runs"], info["run_cap"], info["elapsed_ms"], info["limit_ms"]
    );
    for n in info["notes"].as_array().into_iter().flatten() {
        let _ = writeln!(
            out,
            "{indent}refused for size: {}",
            clean(n.as_str().unwrap_or(""))
        );
    }
}

/// **Private.** The comparison the server built into the record (`comparison`, the same one the
/// report prints): a line per skipped provider, then the design in force and each provider answer,
/// holders first, and the sentence that names the winner or says why there is none. Nothing when no
/// provider section ran.
fn comparison(p: &Value, out: &mut String) {
    let c = &p["comparison"];
    let Some(rows) = c["rows"].as_array() else {
        return;
    };
    let _ = writeln!(
        out,
        "Comparison (price an hour with per-use prices at one run's visits, an estimate that leaves out the nodes with no price; designs that hold come first, cheapest first):"
    );
    // first, so a provider with no section is not missed under the table (defect 6 of the hand run)
    for n in c["not_compared"].as_array().into_iter().flatten() {
        let _ = writeln!(
            out,
            "Not compared: {}: {}",
            clean(n["provider"].as_str().unwrap_or("")),
            clean(n["reason"].as_str().unwrap_or("no answer"))
        );
    }
    for r in rows {
        let at = match (r["holds"].as_bool(), r["rank"].as_u64()) {
            (Some(true), Some(n)) => n.to_string(),
            (Some(true), None) => "n/c".to_string(),
            _ => "-".to_string(),
        };
        let nodes: Vec<String> = r["unpriced_nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|n| clean(n.as_str().unwrap_or("")))
            .collect();
        let which = if nodes.is_empty() {
            String::new()
        } else {
            format!(" ({})", nodes.join(", "))
        };
        let _ = writeln!(
            out,
            "  {at:>3}  {}: {}/h, not priced: {}{which}, {}",
            clean(r["name"].as_str().unwrap_or("")),
            usd(r["price_micro_usd_hr"].as_u64().unwrap_or(0)),
            r["unpriced"].as_u64().unwrap_or(0),
            clean(r["outcome"].as_str().unwrap_or(""))
        );
    }
    let _ = writeln!(out, "{}", clean(c["conclusion"].as_str().unwrap_or("")));
}

/// **Private.** The cheapest part of the plan text, after the plan's own label: the note, the
/// search, the comparison and a block per provider. Nothing for a plain plan.
pub(super) fn print(p: &Value, out: &mut String) {
    let info = &p["search"];
    if !info.is_object() {
        return;
    }
    if let Some(n) = p["cheapest_note"].as_str() {
        let _ = writeln!(out, "{}", clean(n));
    }
    let _ = writeln!(out, "Search:");
    search_lines(info, out, "  ");
    if let (Some(b), Some(a), true) = (
        p["price_before"].as_u64(),
        p["price_after"].as_u64(),
        info["design_in_force_holds"].as_bool().unwrap_or(false),
    ) && a < b
    {
        let _ = writeln!(
            out,
            "The design in force already holds the target and could cost less: {}/h now, {}/h with the changes, {}/h less.",
            usd(b),
            usd(a),
            usd(b - a)
        );
    }
    comparison(p, out);
    for s in p["providers"].as_array().into_iter().flatten() {
        let name = provider_name(s["provider"].as_str().unwrap_or(""));
        if s["kind"] != "answer" {
            continue;
        }
        let steps = s["plan"]["steps"].as_array().map_or(0, Vec::len);
        let _ = writeln!(
            out,
            "If it ran on {name}: {}; costs {}/h with per-use prices at one run's visits; {steps} changes. A what-if on the design translated to its classes.",
            outcome(&s["plan"]["verdict"], &s["info"]),
            usd(s["cost"]["total_micro_usd_hr"].as_u64().unwrap_or(0)),
        );
        search_lines(&s["info"], out, "  ");
        if let Some(n) = s["cost_note"].as_str() {
            let _ = writeln!(out, "  note: {}", clean(n));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Cmd, SimCmd};
    use clap::Parser as _;

    fn parse(recipe: &str) -> PlanArgs {
        let words: Vec<&str> = recipe.split_whitespace().collect();
        let Cmd::Sim {
            cmd: SimCmd::Plan(a),
            ..
        } = Cli::try_parse_from(&words).unwrap().cmd
        else {
            panic!("not a sim plan: {recipe}");
        };
        a
    }

    /// The recipe the report prints (`flags` in `report/plan_cheapest.rs`) goes through the real
    /// parser and back to the numbers it was written from.
    #[test]
    fn the_report_s_spelling_parses_to_the_same_request() {
        let a = parse(
            "kaanal sim plan --profile=peak --target=3x --p99 1s --errors 1% --max-steps 30 --cheapest --time=150s --run-cap=200 --providers=gcp,azure --seed 4",
        );
        let mut b = json!({});
        body(&a, &mut b).unwrap();
        assert_eq!(
            b,
            json!({ "cheapest": true, "time_limit_s": 150, "run_cap": 200, "providers": ["gcp", "azure"] })
        );
        let a =
            parse("kaanal sim plan --target=3x --cheapest --time=1s --run-cap=1 --providers=none");
        let mut b = json!({});
        body(&a, &mut b).unwrap();
        assert_eq!(b["providers"], json!(["none"]));
        // no --cheapest: the body is untouched
        let mut b = json!({});
        body(&parse("kaanal sim plan --target=3x"), &mut b).unwrap();
        assert_eq!(b, json!({}));
    }

    #[test]
    fn a_cut_search_never_says_cannot_hold() {
        let p = json!({ "search": { "stop": "run_cap" }, "verdict": { "kind": "cannot_hold" }, "price_after": 1_000_000 });
        let l = verdict_line(&p).unwrap();
        assert!(
            l.contains("within the limit") && l.contains("run cap") && !l.contains("cannot hold"),
            "{l}"
        );
        // an exhausted search keeps the 9c-1 words
        let p = json!({ "search": { "stop": "exhausted" }, "verdict": { "kind": "cannot_hold" } });
        assert_eq!(verdict_line(&p), None);
    }

    /// A skipped provider is the first thing under the comparison's header, with what is missing.
    #[test]
    fn a_skipped_provider_comes_first_with_its_reason() {
        let p = json!({ "comparison": {
            "not_compared": [ { "provider": "GCP", "reason": "db: no managed postgres class for gcp in the price table" } ],
            "rows": [ { "name": "the design in force", "price_micro_usd_hr": 1_000_000, "unpriced": 0, "unpriced_nodes": [], "holds": true, "outcome": "holds", "rank": 1 } ],
            "comparable": true,
            "conclusion": "No provider was compared, so no winner is named.",
        } });
        let mut out = String::new();
        comparison(&p, &mut out);
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines[0].starts_with("Comparison ("), "{out}");
        assert_eq!(
            lines[1], "Not compared: GCP: db: no managed postgres class for gcp in the price table",
            "{out}"
        );
        assert_eq!(
            lines[2], "    1  the design in force: $1.00/h, not priced: 0, holds",
            "{out}"
        );
        assert_eq!(
            lines[3], "No provider was compared, so no winner is named.",
            "{out}"
        );
    }
}

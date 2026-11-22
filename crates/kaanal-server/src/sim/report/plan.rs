//! The plan report (Step 9c-1): one finished plan record as a Markdown file with a fenced JSON block
//! holding the same data, made like the sweep report (same helpers, same hostile-name rules; see
//! [`super`]) so a person reads it cold and an agent parses it.
//!
//! Order: label, target, the design in force, the steps, the menu per step, the database-family
//! what-ifs, the verdict, the re-check and the measured knee, then the re-run recipe. Money is shown
//! from whole micro-USD with integer arithmetic, so no price goes through a float.

use std::fmt::Write as _;

use kaanal_core::resize::{Move, MoveKind};
use kaanal_sim::plan::{Candidate, Effect, PlanVerdict};
use kaanal_sim::sweep::Verdict;
use serde_json::json;

use super::{
    REPORT_VERSION, Recipe, ReportError, SweepReport, chaos_in_force, json_text, limit_pct,
    row_pct, shell, text,
};
use crate::sim::plan_draft::{BudgetCheck, FAMILY_MAX_STEPS, FamilySection, PlanDoc};
use crate::sim::store::RunRecord;

/// The report of the finished plan `rec`; `version` is the `kaanal` version the recipe names.
///
/// # Errors
/// [`ReportError`] for a record that is not a plan, a running or failed plan, or a plan record
/// missing its recipe.
pub(crate) fn plan_report(rec: &RunRecord, version: &str) -> Result<SweepReport, ReportError> {
    if rec.kind != "plan" {
        return Err(ReportError::NotAReport);
    }
    match rec.state {
        "running" => return Err(ReportError::Running),
        "failed" => return Err(ReportError::Failed(rec.error.clone().unwrap_or_default())),
        _ => {}
    }
    let p = rec.plan.as_ref().ok_or(ReportError::Internal("plan"))?;
    let seed = rec.seed.ok_or(ReportError::Internal("seed"))?;
    let fingerprint = rec
        .fingerprint
        .as_deref()
        .ok_or(ReportError::Internal("fingerprint"))?;
    let chaos = chaos_in_force(rec)?;
    let chaos_digest = rec
        .chaos_digest
        .as_deref()
        .ok_or(ReportError::Internal("chaos_digest"))?;
    let command = command(rec, p, seed)?;
    let json = json!({
        "report_version": REPORT_VERSION,
        "label": p.label,
        "id": rec.id,
        "plan": serde_json::to_value(p).map_err(|_| ReportError::Internal("plan"))?,
        "recipe": {
            "command": command,
            "seed": seed,
            "profile": rec.profile,
            "inline_profile": rec.inline_profile,
            "scenario": rec.scenario,
            "fingerprint": fingerprint,
            "chaos": chaos.kind(),
            "live_chaos": rec.live_chaos,
            "chaos_digest": chaos_digest,
            "runtime_scenarios": rec.runtime_scenarios,
            "generation": rec.generation,
            "kaanal_version": version,
        },
    });
    let mut md = String::new();
    // `write!` into a String cannot fail: the results are dropped
    let _ = write!(
        md,
        "# Sizing plan {}\n\n*{}*\n\nThis plan is not a prediction. It is what the model says about this design at the load below, and every price is an estimate. Kaanal does not apply it: the proposed draft is a text for you to read and apply yourself (`GET /__kaanal/sim/runs/{}/draft`; it is not in this report).\n\n",
        text(&rec.id),
        text(p.label),
        super::clean(&rec.id)
    );
    target(&mut md, rec, p);
    now(&mut md, p);
    steps(&mut md, p);
    menu(&mut md, p);
    families(&mut md, p);
    verdict(&mut md, p);
    checked(&mut md, p);
    super::rerun(
        &mut md,
        &Recipe {
            rec,
            command: &command,
            chaos: &chaos,
            seed,
            fingerprint,
            chaos_digest,
            version,
            seed_note: "every candidate draws the same random numbers, so the same seed on the same design repeats the plan",
        },
    );
    md.push_str(&json_text(&json));
    md.push_str("\n```\n");
    Ok(SweepReport { markdown: md, json })
}

/// `1234567` micro-USD as `1.234567`, at least two decimals, no float: `840000` is `0.84`, `3500` is `0.0035`.
fn dollars(micro: u64) -> String {
    let mut frac = format!("{:06}", micro % 1_000_000);
    while frac.len() > 2 && frac.ends_with('0') {
        frac.pop();
    }
    format!("{}.{frac}", micro / 1_000_000)
}

/// An hourly price, `$0.84/h`, or `not priced`.
fn price(micro: Option<u64>) -> String {
    micro.map_or("not priced".into(), |m| format!("${}/h", dollars(m)))
}

/// A figure with one decimal and no trailing zero: `250`, `118.4`.
fn n(x: f64) -> String {
    let s = format!("{x:.1}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// A capacity estimate, or why there is none.
fn capacity(c: Option<f64>) -> String {
    c.map_or("no estimate".into(), |c| format!("{} req/s", n(c)))
}

/// One run's figures in words.
fn effect(e: &Effect) -> String {
    format!(
        "p99 {} ms, errors {}, capacity est. {}",
        n(e.p99_ms),
        row_pct(e.error_rate),
        capacity(e.capacity_req_s)
    )
}

/// Bytes as the largest whole GiB or MiB, else bytes.
fn bytes(b: u64) -> String {
    const MIB: u64 = 1 << 20;
    const GIB: u64 = 1 << 30;
    if b >= GIB && b.is_multiple_of(GIB) {
        format!("{} GiB", b / GIB)
    } else if b >= MIB && b.is_multiple_of(MIB) {
        format!("{} MiB", b / MIB)
    } else {
        format!("{b} bytes")
    }
}

/// A move in words.
fn change(mv: &Move) -> String {
    match &mv.kind {
        MoveKind::Class { from, to } => {
            format!("instance class {} to {}", text(from), text(to))
        }
        MoveKind::Cores { from, to } => format!("cores {from} to {to}"),
        MoveKind::Memory { from, to } => {
            format!("memory limit {} to {}", bytes(*from), bytes(*to))
        }
        MoveKind::Copies { from, to } => format!("copies {from} to {to}"),
    }
}

/// The names of `xs` as one text, or `none`.
fn list(xs: &[String]) -> String {
    if xs.is_empty() {
        "none".into()
    } else {
        xs.iter().map(|x| text(x)).collect::<Vec<_>>().join(", ")
    }
}

/// The `## Target` section.
fn target(md: &mut String, rec: &RunRecord, p: &PlanDoc) {
    let r = &p.request;
    let p99 = humantime::format_duration(r.limit.p99).to_string();
    let budget = r.budget_micro_usd_hr.map_or("none".into(), |b| {
        format!("${}/h, counted over the priced nodes only", dollars(b))
    });
    let _ = write!(
        md,
        "## Target\n\n- Offered load: {} req/s, {:.2}x the rates of profile {}.\n- Limit: p99 {}, errors {}. A design holds the target only with a margin: p99 at most 90% of the p99 limit and errors at most half of the error limit.\n- Budget: {budget}.\n- Kept as they are: {}.\n- Most steps: {}.\n\n",
        n(r.req_s),
        r.factor,
        text(&rec.profile),
        text(&p99),
        limit_pct(r.limit.error_rate),
        list(&r.keep),
        r.max_steps
    );
}

/// The `## Now` section: the design in force at the target.
fn now(md: &mut String, p: &PlanDoc) {
    let _ = write!(
        md,
        "## Now\n\nThe design in force, at that load: {}. It costs {} (priced nodes only).\n\n",
        effect(&p.start),
        price(p.price_before)
    );
}

/// The `## Plan` section: the steps as a table, then the design they end in.
fn steps(md: &mut String, p: &PlanDoc) {
    md.push_str("## Plan\n\nEach step moves one size knob of one node (instance class, cores, memory limit or copies). The figures are what the model gives at the target load before and after the step. Capacity est. is the lowest closed-form saturation rate of any node: an estimate, not a measurement.\n\n");
    if p.steps.is_empty() {
        md.push_str("No steps.\n\n");
    } else {
        md.push_str("| step | node | change | price | capacity est. req/s | p99 at target ms | errors at target |\n| ---: | --- | --- | --- | --- | --- | --- |\n");
        let cap = |c: Option<f64>| c.map_or("no estimate".into(), n);
        for (i, s) in p.steps.iter().enumerate() {
            let cost = match (s.price_delta_micro_usd_hr, s.price_micro_usd_hr) {
                (Some(d), Some(total)) => format!(
                    "{}${}/h; {} in all",
                    if d < 0 { "-" } else { "+" },
                    dollars(d.unsigned_abs()),
                    price(Some(total))
                ),
                _ => "not priced".into(),
            };
            let _ = writeln!(
                md,
                "| {} | {} | {} | {cost} | {} to {} | {} to {} | {} to {} |",
                i + 1,
                text(&s.mv.node),
                change(&s.mv),
                cap(s.before.capacity_req_s),
                cap(s.after.capacity_req_s),
                n(s.before.p99_ms),
                n(s.after.p99_ms),
                row_pct(s.before.error_rate),
                row_pct(s.after.error_rate)
            );
        }
        md.push('\n');
        for (i, s) in p.steps.iter().enumerate() {
            if let Some(note) = &s.note {
                let _ = writeln!(md, "Note on step {}: {}\n", i + 1, text(note));
            }
        }
    }
    let _ = write!(
        md,
        "The design the plan ends in: {}; {} (the design in force costs {}).\n\n",
        effect(&p.end),
        price(p.price_after),
        price(p.price_before)
    );
}

/// One menu entry.
fn candidate(c: &Candidate, taken: bool) -> String {
    format!(
        "- {}{}: {}; {}; {}.",
        change(&c.mv),
        if taken { " (taken)" } else { "" },
        price(c.price_micro_usd_hr),
        effect(&c.effect),
        if c.holds {
            "holds the target"
        } else {
            "does not hold the target"
        }
    )
}

/// The `## Menu` section: what the walk tried at each step.
fn menu(md: &mut String, p: &PlanDoc) {
    md.push_str("## Menu\n\nAt each step the walk tried every move the bottleneck node offers and took the one marked taken. The price is the whole design's with that move; the figures are at the target load.\n\n");
    if p.steps.iter().all(|s| s.menu.is_empty()) {
        md.push_str("No step, so no menu.\n\n");
        return;
    }
    for (i, s) in p.steps.iter().enumerate() {
        if s.menu.is_empty() {
            continue;
        }
        let _ = write!(md, "Step {}, node {}:\n\n", i + 1, text(&s.mv.node));
        for c in &s.menu {
            let _ = writeln!(md, "{}", candidate(c, c.mv == s.mv));
        }
        md.push('\n');
    }
}

/// The `## If you changed the database family` section.
fn families(md: &mut String, p: &PlanDoc) {
    md.push_str("## If you changed the database family\n\n");
    if p.families.is_empty() {
        md.push_str("The design has no database to change.\n\n");
        return;
    }
    md.push_str("These are what-ifs, not a recommendation to migrate. Every answer assumes your application can use this family; preset estimates. The plan above does not depend on them.\n\n");
    for f in &p.families {
        match f {
            FamilySection::Answer(a) => {
                let k = a.plan.steps.len();
                let _ = writeln!(
                    md,
                    "- {} as {}:\n  - {}\n  - {k} {} taken, ending at: {}; {} (the changed design costs {} before any step).",
                    text(&a.node),
                    text(&a.flavor),
                    verdict_words(
                        &a.plan.verdict,
                        p.request.max_steps.min(FAMILY_MAX_STEPS),
                        true,
                        a.plan.price_after,
                        p.request.budget_micro_usd_hr
                    ),
                    if k == 1 { "step" } else { "steps" },
                    effect(&a.plan.end),
                    price(a.plan.price_after),
                    price(a.plan.price_before)
                );
                for note in &a.notes {
                    let _ = writeln!(md, "  - {}", text(note));
                }
            }
            FamilySection::Skipped {
                node,
                flavor,
                reason,
            } => {
                let of = flavor
                    .as_ref()
                    .map_or(String::new(), |f| format!(" as {}", text(f)));
                let _ = writeln!(md, "- {}{of}: not walked: {}", text(node), text(reason));
            }
        }
    }
    md.push('\n');
}

/// A walk's ending in plain words; `shown` is the plan's price and `budget` the request's ceiling.
fn verdict_words(
    v: &PlanVerdict,
    max_steps: u32,
    family: bool,
    shown: Option<u64>,
    budget: Option<u64>,
) -> String {
    match v {
        PlanVerdict::AlreadyHolds if family => {
            "The changed design already holds the target: no step needed.".into()
        }
        PlanVerdict::AlreadyHolds => {
            "The design in force already holds the target: nothing to change.".into()
        }
        PlanVerdict::Holds => "The plan holds the target.".into(),
        PlanVerdict::CannotHold { node, reason } => {
            let at = node
                .as_ref()
                .map_or(String::new(), |n| format!(" at '{}'", text(n)));
            format!(
                "The plan cannot hold the target by sizing{at}: {}. The design after the steps taken does not hold the target.",
                text(reason.trim_end_matches('.'))
            )
        }
        PlanVerdict::OverBudget {
            over_by_micro_usd_hr: over,
            holds: true,
        } => format!(
            "The plan holds the target but is over budget by ${}/h: the cheapest holding design found costs ${}/h against a budget of ${}/h.",
            dollars(*over),
            dollars(budget.unwrap_or(0) + over),
            dollars(budget.unwrap_or(0))
        ),
        PlanVerdict::OverBudget {
            over_by_micro_usd_hr: over,
            holds: false,
        } => format!(
            "No holding design was found within the budget. The plan shown costs {} without per-use prices and does not hold the target; the next step needed would cost ${}/h, over the budget of ${}/h by ${}/h, so the plan stops here.",
            price(shown),
            dollars(budget.unwrap_or(0) + over),
            dollars(budget.unwrap_or(0)),
            dollars(*over)
        ),
        // walk.rs counts consecutive no-gain steps across bottleneck changes: only the second
        // one's node is known, so nothing is claimed about sizing that node further
        PlanVerdict::NoProgress { node } => format!(
            "The plan stops: the last two steps the walk tried gained nothing (the second on '{}'), and they are not in the plan. The design where it stops does not hold the target.",
            text(node)
        ),
        PlanVerdict::StepCap => format!(
            "The plan stops after {max_steps} steps, the step cap, without holding the target."
        ),
    }
}

/// The `## Verdict` section.
fn verdict(md: &mut String, p: &PlanDoc) {
    let _ = write!(
        md,
        "## Verdict\n\n{}\n\n",
        verdict_words(
            &p.verdict,
            p.request.max_steps,
            false,
            p.price_after,
            p.request.budget_micro_usd_hr
        )
    );
    if let Some(b) = &p.budget_check {
        md.push_str(&budget_line(b));
        md.push_str("\n\n");
    }
    if p.unpriced_nodes.is_empty() {
        md.push_str("Every node is priced.\n\n");
    } else {
        let _ = write!(
            md,
            "Not priced: {}. They are left out of every price and of the budget.\n\n",
            list(&p.unpriced_nodes)
        );
    }
}

/// The budget judged with per-use prices: what the plan shown costs against the budget, and whether
/// those prices turned a holding plan into an over-budget one.
fn budget_line(b: &BudgetCheck) -> String {
    let (cost, budget) = (b.cost_micro_usd_hr, b.budget_micro_usd_hr);
    let against = if cost > budget {
        format!(
            "over the budget of ${}/h by ${}/h",
            dollars(budget),
            dollars(cost - budget)
        )
    } else {
        format!("within the budget of ${}/h", dollars(budget))
    };
    let mut s = format!(
        "Budget check, with per-use prices counted at the target run's visits: the plan shown costs ${}/h, {against}.",
        dollars(cost)
    );
    if b.rejudged {
        s.push_str(" The walk prices without per-use prices and found the plan within the budget; with them it is over, so the verdict is over budget.");
    }
    s
}

/// The `## Checked` section: the re-check of the proposed draft and the measured knee.
fn checked(md: &mut String, p: &PlanDoc) {
    let v = &p.verify;
    let _ = write!(
        md,
        "## Checked\n\nThe proposed draft was built from its text and checked again at the target: p99 {} ms, errors {}, capacity est. {}. It {} the target with the margin.\n\n",
        n(v.p99_ms),
        row_pct(v.error_rate),
        capacity(v.capacity_req_s),
        if v.holds { "holds" } else { "does not hold" }
    );
    let _ = write!(
        md,
        "Its cost, with per-use prices counted at that run's visits: ${}/h ({} nodes priced, {} not). It can differ from the plan's price above, which has no per-use part.\n\n",
        dollars(v.cost.total_micro_usd_hr),
        v.cost.priced,
        v.cost.unpriced
    );
    if let Some(note) = v.cost_note {
        let _ = write!(md, "Note: {}.\n\n", text(note));
    }
    let at = |k: f64, req_s: f64| format!("{} req/s ({k:.2}x the profile's rates)", n(req_s));
    let knee = match (&p.measured_knee, &p.sweep_error) {
        (Some(Verdict::Knee { k, req_s }), _) => format!(
            "A load sweep of the proposed design finds its knee near {}, to the sweep's search resolution: the last point within the plain limit (no margin) before the first one over it.",
            at(*k, *req_s)
        ),
        (Some(Verdict::BrokenAtLowest { k, req_s }), _) => format!(
            "A load sweep of the proposed design finds it over the limit already at its lowest point, {}.",
            at(*k, *req_s)
        ),
        (Some(Verdict::HoldsTo { k, req_s }), _) => format!(
            "A load sweep of the proposed design finds no point over the limit up to {}, the highest it ran; its knee is above that.",
            at(*k, *req_s)
        ),
        (None, Some(e)) => format!(
            "The load sweep of the proposed design did not run: {}.",
            text(e.trim_end_matches('.'))
        ),
        (None, None) => "No load sweep of the proposed design was made.".into(),
    };
    let _ = write!(md, "{knee}\n\n");
}

/// The command that repeats the plan: `-f profile.yaml` for an inline profile, else `--profile=NAME`;
/// names use the `=` form, so one starting with `-` stays a value.
fn command(rec: &RunRecord, p: &PlanDoc, seed: u64) -> Result<String, ReportError> {
    let r = &p.request;
    let profile = if rec.inline_profile.is_some() {
        "-f profile.yaml".to_string()
    } else {
        format!("--profile={}", shell(&rec.profile))
    };
    let target = r
        .target
        .multiple
        .map(|m| format!("{m}x"))
        .or_else(|| r.target.req_s.map(|q| q.to_string()))
        .ok_or(ReportError::Internal("target"))?;
    let p99 = shell(&humantime::format_duration(r.limit.p99).to_string());
    let mut c = format!(
        "kaanal sim plan {profile} --target={target} --p99 {p99} --errors {}",
        limit_pct(r.limit.error_rate)
    );
    if let Some(b) = r.budget_micro_usd_hr {
        let _ = write!(c, " --budget={}", dollars(b));
    }
    for k in &r.keep {
        let _ = write!(c, " --keep={}", shell(k));
    }
    let _ = write!(c, " --max-steps {} --seed {seed}", r.max_steps);
    if let Some(sc) = &rec.scenario {
        let _ = write!(c, " --scenario={}", shell(sc));
    }
    Ok(c)
}

#[cfg(test)]
mod recipe_tests;
#[cfg(test)]
mod tests;

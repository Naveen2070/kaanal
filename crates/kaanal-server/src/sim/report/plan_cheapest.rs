//! The cheapest-plan part of the plan report (Step 9c-2): the label, the search block, the
//! comparison of the design in force with each provider what-if, the list of changes, the verdict
//! in words that know a limit cut the search, and one section per provider. [`super::plan`] calls
//! these only for a plan with a `search`; a plain plan's bytes do not depend on this file.
//!
//! Three honesty rules shape the words. (1) Nothing here says "cheapest" without the limit: the
//! label is [`CHEAPEST_NOTE`] and every answer carries it. (2) A search cut by the run cap or the
//! time limit never says "cannot hold": it says none was found within the limit. (3) The steps of
//! a cheapest plan are the walk's, then the search's moves merged per node and knob, so the design
//! between two lines is not one the search checked: the report shows each change's own price
//! effect and the start and the end, never a running price.

use std::fmt::Write as _;
use std::time::Duration;

use kaanal_core::cost::CostReport;
use kaanal_sim::plan::{CHEAPEST_NOTE, Effect, PlanVerdict, SearchInfo, Step, StopReason};
use serde::Serialize;

use super::plan::{budget_line, change, dollars, effect, price, verdict_words};
use super::text;
use crate::sim::plan_draft::PlanDoc;
use crate::sim::plan_search::{CheapestAsked, PROVIDER_NOTE, ProviderSection};

/// The label of a cheapest plan, under the plan's own label.
pub(super) fn label(md: &mut String, p: &PlanDoc) {
    if let Some(note) = p.cheapest_note {
        // `write!` into a String cannot fail: the results are dropped
        let _ = write!(md, "*{}*\n\n", text(note));
    }
}

/// The re-run flags of a cheapest plan, in the `=` forms the CLI takes; the providers are those
/// the request resolved to.
pub(super) fn flags(c: &CheapestAsked) -> String {
    let providers = if c.providers.is_empty() {
        "none".to_string()
    } else {
        c.providers.join(",")
    };
    format!(
        " --cheapest --time={}s --run-cap={} --providers={providers}",
        c.time_limit_s, c.run_cap
    )
}

/// A duration given in ms, in words (`1s 250ms`).
fn span(ms: u64) -> String {
    humantime::format_duration(Duration::from_millis(ms)).to_string()
}

/// Why a search stopped, in words that never claim a minimum.
fn stop_words(s: StopReason) -> &'static str {
    match s {
        StopReason::Exhausted => "it ran out of candidates to try",
        StopReason::RunCap => "it reached the run cap, so cheaper designs may be left untried",
        StopReason::TimeLimit => "its time limit passed, so cheaper designs may be left untried",
    }
}

/// What cut a search short; `None` when it ended by itself.
fn cut(s: StopReason) -> Option<&'static str> {
    match s {
        StopReason::Exhausted => None,
        StopReason::RunCap => Some("run cap"),
        StopReason::TimeLimit => Some("time limit"),
    }
}

/// Whether the design a verdict is about holds the target (an over-budget one does: it costs more
/// than asked).
fn holds(v: &PlanVerdict) -> bool {
    matches!(
        v,
        PlanVerdict::AlreadyHolds
            | PlanVerdict::Holds
            | PlanVerdict::OverBudget { holds: true, .. }
    )
}

/// A verdict in plain words, aware of how the search ended. A design that does not hold after a
/// limit cut is "none found within the limit", not "cannot hold" (the walk was cut, so its reason is
/// the limit's).
pub(super) fn words(
    design: &str,
    v: &PlanVerdict,
    i: &SearchInfo,
    max_steps: u32,
    shown: Option<u64>,
    budget: Option<u64>,
) -> String {
    match (v, cut(i.stop)) {
        (PlanVerdict::AlreadyHolds, None) => {
            format!(
                "{design} already holds the target, and no cheaper design that holds was found within the limit."
            )
        }
        (PlanVerdict::AlreadyHolds, Some(what)) => format!(
            "{design} already holds the target. No cheaper design that holds was found within the limit: the search stopped at the {what} before it ran out of candidates, so one may exist."
        ),
        (
            PlanVerdict::CannotHold { .. }
            | PlanVerdict::NoProgress { .. }
            | PlanVerdict::StepCap
            | PlanVerdict::OverBudget { holds: false, .. },
            Some(what),
        ) => format!(
            "No design that holds the target was found within the limit: the search stopped at the {what} before it could say more. That does not show that none exists."
        ),
        _ => verdict_words(v, max_steps, false, shown, budget),
    }
}

/// The verdict of one provider section in words (Markdown): about "the translated design".
pub(super) fn section_words(
    p: &PlanDoc,
    v: &PlanVerdict,
    shown: Option<u64>,
    i: &SearchInfo,
) -> String {
    words(
        "The translated design",
        v,
        i,
        p.request.max_steps,
        shown,
        p.request.budget_micro_usd_hr,
    )
}

/// The lines every search block has: why it stopped, runs, time, and the candidates refused for size.
fn search_lines(md: &mut String, i: &SearchInfo, share: &str) {
    let _ = writeln!(md, "- Stopped because {}.", stop_words(i.stop));
    let _ = writeln!(md, "- Runs: {} of {} allowed.", i.runs, i.run_cap);
    let _ = writeln!(
        md,
        "- Time: {} used of {}{share}.",
        span(i.elapsed_ms),
        span(i.limit_ms)
    );
    for n in &i.notes {
        let _ = writeln!(md, "- Refused for size: {}", text(n));
    }
}

/// The `## Search` section: how the main search ran, and what it found against the design in force.
pub(super) fn search(md: &mut String, p: &PlanDoc) {
    let Some(i) = &p.search else { return };
    md.push_str("## Search\n\n");
    let sections = p.providers.as_ref().is_some_and(|s| !s.is_empty());
    let share = match (&p.request.cheapest, sections) {
        (Some(c), true) => format!(
            ", this search's share of the {} s time limit, which is split between it and each provider section",
            c.time_limit_s
        ),
        _ => ", the time limit".into(),
    };
    search_lines(md, i, &share);
    md.push('\n');
    if let (true, Some(before), Some(after)) =
        (i.design_in_force_holds, p.price_before, p.price_after)
        && after < before
    {
        let _ = write!(
            md,
            "The design in force already holds the target and could cost less: {} now, {} with the changes below, {} less.\n\n",
            price(Some(before)),
            price(Some(after)),
            price(Some(before - after))
        );
    }
    md.push_str("The search tries smaller sizes of each priced node and keeps one only if the design still holds the target with the margin. It assumes a bigger size never holds less, which is not always so, so a cheaper design can exist that it does not reach.\n\n");
}

/// A provider as a person writes it.
fn provider_name(p: &str) -> String {
    match p {
        "aws" => "AWS".into(),
        "gcp" => "GCP".into(),
        "azure" => "Azure".into(),
        other => text(other),
    }
}

/// The price effect of one change, signed.
fn effect_of(s: &Step) -> String {
    match s.price_delta_micro_usd_hr {
        Some(0) => "no price change".into(),
        Some(d) => format!(
            "{}${}/h",
            if d < 0 { "-" } else { "+" },
            dollars(d.unsigned_abs())
        ),
        None => "not priced".into(),
    }
}

/// The changes as a table, with each change's own price effect and note.
fn changes_table(md: &mut String, steps: &[Step]) {
    if steps.is_empty() {
        md.push_str("No changes.\n\n");
        return;
    }
    md.push_str("| # | node | change | price effect | note |\n| ---: | --- | --- | --- | --- |\n");
    for (i, s) in steps.iter().enumerate() {
        let _ = writeln!(
            md,
            "| {} | {} | {} | {} | {} |",
            i + 1,
            text(&s.mv.node),
            change(&s.mv),
            effect_of(s),
            s.note.as_deref().map_or("-".into(), text)
        );
    }
    md.push('\n');
}

/// The start and the end of a plan, each with its price.
fn ends(
    md: &mut String,
    design: &str,
    start: &Effect,
    before: Option<u64>,
    end: &Effect,
    after: Option<u64>,
) {
    let _ = write!(
        md,
        "{design} before any change: {}; {}. After the changes: {}; {}.\n\n",
        effect(start),
        price(before),
        effect(end),
        price(after)
    );
}

/// The `## Plan` section of a cheapest plan: the changes, not a price sequence.
pub(super) fn changes(md: &mut String, p: &PlanDoc) {
    md.push_str("## Plan\n\nThese are the changes the search found, in the order it found them: first what the walk needed to make the design hold, then what the search took off. Each change is one size knob of one node. The price effect is what that change adds or takes off, an estimate; the moves on one knob of one node are merged into one line, so the figures of the design between two lines are not shown. The figures are what the model gives at the target load.\n\n");
    changes_table(md, &p.steps);
    ends(
        md,
        "The design in force",
        &p.start,
        p.price_before,
        &p.end,
        p.price_after,
    );
}

/// What one design says in the comparison. The price is the design's cost with per-use prices at
/// one run's visits, the figure its section (and the Checked block) states.
#[derive(Debug, Clone, Serialize)]
pub struct CompareRow {
    pub name: String,
    pub price_micro_usd_hr: u64,
    /// How many nodes are left out of the price, and which when the cost names them.
    pub unpriced: usize,
    pub unpriced_nodes: Vec<String>,
    pub holds: bool,
    /// How far over the budget a holding design is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub over_budget_micro_usd_hr: Option<u64>,
    pub outcome: String,
    /// Place among the holders by price; `None` for one that does not hold, or when the holders
    /// leave different nodes out and cannot be ranked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank: Option<u32>,
}

/// A provider with no section, and why.
#[derive(Debug, Clone, Serialize)]
pub struct NotCompared {
    /// As a person writes it (`GCP`).
    pub provider: String,
    pub reason: String,
}

/// The comparison of the design in force with each provider what-if, built once on the server so the
/// report and the CLI say the same thing.
#[derive(Debug, Clone, Serialize)]
pub struct Comparison {
    pub not_compared: Vec<NotCompared>,
    /// Holders first, cheapest first, then the rest.
    pub rows: Vec<CompareRow>,
    /// Whether the holders' prices can be ranked against each other.
    pub comparable: bool,
    /// The sentence under the table: the winner, or why there is none.
    pub conclusion: String,
}

/// The nodes a cost leaves out, as a comparable key.
fn unpriced(c: &CostReport) -> (usize, Vec<String>) {
    let mut names: Vec<String> = c
        .nodes
        .iter()
        .filter(|n| n.total_micro_usd_hr.is_none())
        .map(|n| n.name.clone())
        .collect();
    names.sort();
    (c.unpriced, names)
}

/// The unpriced nodes in words: `1 (cdn)`, `2`, `0`.
fn unpriced_words(count: usize, nodes: &[String]) -> String {
    if nodes.is_empty() {
        count.to_string()
    } else {
        format!(
            "{count} ({})",
            nodes.iter().map(|n| text(n)).collect::<Vec<_>>().join(", ")
        )
    }
}

fn over_by(v: &PlanVerdict) -> Option<u64> {
    match v {
        PlanVerdict::OverBudget {
            over_by_micro_usd_hr,
            holds: true,
        } => Some(*over_by_micro_usd_hr),
        _ => None,
    }
}

/// A verdict as a short table cell.
fn outcome(v: &PlanVerdict, i: &SearchInfo) -> String {
    match v {
        PlanVerdict::AlreadyHolds => "holds, no change needed".into(),
        PlanVerdict::Holds => "holds".into(),
        PlanVerdict::OverBudget {
            over_by_micro_usd_hr: over,
            holds: true,
        } => format!("holds, over budget by ${}/h", dollars(*over)),
        _ if cut(i.stop).is_some() => "no holding design found within the limit".into(),
        PlanVerdict::OverBudget { holds: false, .. } => {
            "no holding design within the budget".into()
        }
        PlanVerdict::CannotHold { node, .. } => {
            node.as_ref().map_or("cannot hold by sizing".into(), |n| {
                format!("cannot hold by sizing, at '{}'", text(n))
            })
        }
        PlanVerdict::NoProgress { .. } => "stopped: two steps gained nothing".into(),
        PlanVerdict::StepCap => "stopped at the step cap".into(),
    }
}

/// The comparison of the design in force with each provider answer, the holders ranked by price.
/// `None` when no provider section ran or the plan is a plain one.
pub(crate) fn compare(p: &PlanDoc) -> Option<Comparison> {
    let (i, sections) = (p.search.as_ref()?, p.providers.as_ref()?);
    if sections.is_empty() {
        return None;
    }
    let row = |name: String, cost: &CostReport, v: &PlanVerdict| {
        let (unpriced, unpriced_nodes) = unpriced(cost);
        CompareRow {
            name,
            price_micro_usd_hr: cost.total_micro_usd_hr,
            unpriced,
            unpriced_nodes,
            holds: holds(v),
            over_budget_micro_usd_hr: over_by(v),
            outcome: outcome(v, i),
            rank: None,
        }
    };
    let mut rows = vec![row(
        if p.steps.is_empty() {
            "the design in force".into()
        } else {
            "the design in force, with the changes above".into()
        },
        &p.verify.cost,
        &p.verdict,
    )];
    let mut not_compared = Vec::new();
    for s in sections {
        match s {
            ProviderSection::Answer {
                provider,
                plan,
                cost,
                ..
            } => rows.push(row(
                format!("the design translated to {}", provider_name(provider)),
                cost,
                &plan.verdict,
            )),
            ProviderSection::Skipped { provider, reason } => not_compared.push(NotCompared {
                provider: provider_name(provider),
                reason: reason.clone(),
            }),
        }
    }
    // holders first, cheapest first; the sort is stable
    rows.sort_by_key(|r| (!r.holds, r.price_micro_usd_hr));
    // prices that leave out different nodes cannot be ranked against each other
    let comparable = rows.iter().filter(|r| r.holds).all(|r| {
        (r.unpriced, &r.unpriced_nodes)
            == rows
                .iter()
                .find(|h| h.holds)
                .map_or((r.unpriced, &r.unpriced_nodes), |h| {
                    (h.unpriced, &h.unpriced_nodes)
                })
    });
    if comparable {
        for (n, r) in (1..).zip(rows.iter_mut().filter(|r| r.holds)) {
            r.rank = Some(n);
        }
    }
    let mut holding = rows.iter().filter(|r| r.holds);
    let conclusion = match holding.next() {
        None => "No design holds at any size tried, so there is no winner.".to_string(),
        Some(_) if !comparable => "The designs that hold the target leave different nodes out of their prices, so the prices cannot be ranked against each other and no winner is named.".to_string(),
        Some(_) if rows.len() == 1 => "No provider was compared, so no winner is named.".to_string(),
        Some(w) => {
            let tie = holding
                .next()
                .filter(|n| n.price_micro_usd_hr == w.price_micro_usd_hr)
                .map_or(String::new(), |n| format!(" (tied with {})", n.name));
            let over = w.over_budget_micro_usd_hr.map_or(String::new(), |o| {
                format!(" It is over the budget by ${}/h.", dollars(o))
            });
            format!(
                "Winner: {}, {}{tie}. It is the cheapest design that holds the target found within the limit, not proven cheapest.{over}",
                w.name,
                price(Some(w.price_micro_usd_hr))
            )
        }
    };
    Some(Comparison {
        not_compared,
        rows,
        comparable,
        conclusion,
    })
}

/// The `## Comparison` section, from [`compare`]; left out when no provider section ran.
pub(super) fn comparison(md: &mut String, p: &PlanDoc) {
    let Some(c) = compare(p) else { return };
    md.push_str("## Comparison\n\n");
    // first, so a provider with no section is not missed under the table (defect 6 of the hand run)
    if !c.not_compared.is_empty() {
        md.push_str("Not compared:\n\n");
        for n in &c.not_compared {
            let _ = writeln!(md, "- {}: {}", n.provider, text(&n.reason));
        }
        md.push('\n');
    }
    md.push_str("The designs that hold the target come first, cheapest first; the ones that do not come after them with how they ended. The price is the design's cost an hour with per-use prices counted at one run's visits at the target, the figure its own section states: an on-demand estimate that leaves out the nodes with no price (the column counts them). A design that does not hold runs saturated, so its per-use prices are understated. A provider row is the design translated to that provider's classes, searched on its own.\n\n| rank | design | price an hour | nodes not priced | holds the target | how it ended |\n| ---: | --- | --- | --- | --- | --- |\n");
    for r in &c.rows {
        let at = match (r.holds, r.rank) {
            (false, _) => "-".to_string(),
            (true, Some(n)) => n.to_string(),
            (true, None) => "not comparable".to_string(),
        };
        let _ = writeln!(
            md,
            "| {at} | {} | {} | {} | {} | {} |",
            r.name,
            price(Some(r.price_micro_usd_hr)),
            unpriced_words(r.unpriced, &r.unpriced_nodes),
            if r.holds { "yes" } else { "no" },
            r.outcome
        );
    }
    let _ = write!(md, "\n{}\n\n", c.conclusion);
}

/// One section per provider answer: what the translated design costs and how the search went.
pub(super) fn sections(md: &mut String, p: &PlanDoc) {
    for s in p.providers.iter().flatten() {
        let ProviderSection::Answer {
            provider,
            plan,
            cost,
            cost_note,
            budget_check,
            info,
            notes,
        } = s
        else {
            continue;
        };
        let name = provider_name(provider);
        let _ = write!(
            md,
            "## If it ran on {name}\n\n*{}*\n\nThis what-if {PROVIDER_NOTE}.\n\n",
            text(CHEAPEST_NOTE)
        );
        let _ = write!(
            md,
            "{}\n\n",
            section_words(p, &plan.verdict, plan.price_after, info)
        );
        if let Some(b) = budget_check {
            let _ = write!(md, "{}\n\n", budget_line(b));
        } else {
            let _ = write!(
                md,
                "With per-use prices counted at one run's visits at the target, the design costs ${}/h ({} nodes priced, {} not).\n\n",
                dollars(cost.total_micro_usd_hr),
                cost.priced,
                cost.unpriced
            );
        }
        if let Some(n) = cost_note {
            let _ = write!(md, "Note: {}.\n\n", text(n));
        }
        search_lines(md, info, ", this section's share of the time limit");
        md.push('\n');
        translation(md, plan_notes(notes));
        changes_table(md, &plan.steps);
        let design = "The translated design";
        ends(
            md,
            design,
            &plan.start,
            plan.price_before,
            &plan.end,
            plan.price_after,
        );
    }
}

/// The notes of a section without the caveat and the label, which the section says itself.
fn plan_notes(notes: &[String]) -> Vec<&String> {
    notes
        .iter()
        .filter(|n| n.as_str() != PROVIDER_NOTE && n.as_str() != CHEAPEST_NOTE)
        .collect()
}

/// The translation's notes: the move made per node, and the nodes kept as written.
fn translation(md: &mut String, notes: Vec<&String>) {
    if notes.is_empty() {
        return;
    }
    md.push_str("The translation:\n\n");
    for n in notes {
        let _ = writeln!(md, "- {}", text(n));
    }
    md.push('\n');
}

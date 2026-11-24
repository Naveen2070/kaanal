//! The sizing plan's record and its proposed draft (Step 9c-1, RULES 141).
//!
//! [`proposed_text`] writes the plan's steps into the **applied** text with the draft patch code
//! (`draft::patch`), so comments and layout survive and the text holds only what was checked;
//! [`verify`] builds that text from scratch (parse, load, build) and checks it at the target the way
//! the walk did (`kaanal_sim::plan::check`: same seed, same window, same margin rule), then prices it
//! with one run at the target so per-use prices are counted at its visits (RULES 126). The draft the
//! developer reads is the design that was proven: a mismatch is an internal failure, not a plan.

use std::sync::Arc;
use std::time::Duration;

use kaanal_core::EffectiveChaos;
use kaanal_core::cost::CostReport;
use kaanal_core::resize::MoveKind;
use kaanal_sim::plan::{Effect, FamilyAnswer, PlanVerdict, Step, check};
use kaanal_sim::sweep::{Limit, PointFail, Verdict};
use kaanal_sim::{Limits, SimConfig, Stream};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use serde_yaml_ng::Value as Yaml;

use super::plan_search::{CheapestAsked, ProviderSection};
use super::run::{Job, execute_run};
use crate::draft::apply::build_draft;
use crate::draft::patch::{self, Op};
use crate::generation::Generation;
use crate::state::AppState;

/// What every plan says it is (R6.5, RULES 121 and 126).
pub const LABEL: &str = "relative what-if, not a prediction; prices are estimates in dollars an hour (fields named micro_usd are millionths of a dollar), of the priced nodes only";

/// Reported requests the pricing run aims for at most: it only needs the visit rates, which a
/// shorter window measures as well (the sweep's point size).
const PRICE_REQUESTS: f64 = 20_000.0;

/// The body's `target`: exactly one of a multiple of the profile's rates or a total rate.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TargetAsked {
    /// A multiple of the profile's rates, 0.1 to 64.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multiple: Option<f64>,
    /// Total requests per second over the profile's streams, its mix kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub req_s: Option<f64>,
}

/// The request as checked, on the record.
#[derive(Debug, Clone, Serialize)]
pub struct PlanAsked {
    /// The target as posted.
    pub target: TargetAsked,
    /// The load factor on the profile's rates it comes to.
    pub factor: f64,
    /// The total rate offered at the target, req/s.
    pub req_s: f64,
    /// The pass line (held with the sweep's margin).
    pub limit: Limit,
    /// The ceiling, micro-USD an hour of the priced nodes.
    pub budget_micro_usd_hr: Option<u64>,
    /// Nodes the plan must not move.
    pub keep: Vec<String>,
    /// The most steps.
    pub max_steps: u32,
    /// The cheapest search's fields (Step 9c-2); `None` for a plain plan, and then not serialised.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cheapest: Option<CheapestAsked>,
}

/// One entry of the database-family section: the walk on one flavor of the other family, or why a
/// database or a flavor has none.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FamilySection {
    /// The walk on the alternative.
    Answer(FamilyAnswer),
    /// Nothing to walk: the database (no `flavor`) or the flavor is skipped, and why.
    Skipped {
        /// The database node.
        node: String,
        /// The flavor dropped; `None` when the whole database is skipped.
        flavor: Option<String>,
        /// Why, in plain words.
        reason: String,
    },
}

/// The re-check of the proposed draft.
#[derive(Debug, Clone, Serialize)]
pub struct Verify {
    /// The proposed design holds the target (with the margin), checked as the walk checks.
    pub holds: bool,
    /// Its p99 at the target, ms.
    pub p99_ms: f64,
    /// Its error rate at the target (0 to 1).
    pub error_rate: f64,
    /// Its capacity estimate (the lowest analytic `saturates_at_req_s`).
    pub capacity_req_s: Option<f64>,
    /// What it costs an hour, estimated, with per-use prices at the target run's visits.
    pub cost: CostReport,
    /// Set when the design does not hold: [`SATURATED_COST`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_note: Option<&'static str>,
}

/// What a non-holding design's cost says: its run at the target is saturated, so requests that fail
/// or wait never reach the nodes behind, and per-use prices counted at those visits come out low.
pub const SATURATED_COST: &str = "the design does not hold the target: its run is saturated, so per-use prices (counted at its visits) are understated";

/// The most steps a database-family alternative's walk takes (the main walk's `max_steps` when it is
/// lower): a what-if beside the plan should not hold the single slot for minutes.
pub const FAMILY_MAX_STEPS: u32 = 10;

/// The budget judged with per-use prices (spec: they count, at the visits of the target run).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BudgetCheck {
    /// The ceiling asked for.
    pub budget_micro_usd_hr: u64,
    /// What the plan shown costs with per-use prices at the target run's visits (`verify.cost`).
    pub cost_micro_usd_hr: u64,
    /// The walk, which prices without per-use prices, said `holds`; with them the plan is over the
    /// budget, so the verdict is `over_budget`.
    pub rejudged: bool,
}

/// The verdict judged against `budget` with `cost`, the checked cost of the plan shown (per-use
/// prices included). A holding plan over the budget becomes `over_budget` (`rejudged`); an over-budget
/// holding plan is over by `cost - budget`; any other verdict stays, and the check says what the shown
/// plan costs against the budget. `None` without a budget.
pub(crate) fn judge_budget(
    v: PlanVerdict,
    budget: Option<u64>,
    cost: u64,
) -> (PlanVerdict, Option<BudgetCheck>) {
    let Some(b) = budget else {
        return (v, None);
    };
    let over = PlanVerdict::OverBudget {
        over_by_micro_usd_hr: cost.saturating_sub(b),
        holds: true,
    };
    let (v, rejudged) = match v {
        PlanVerdict::Holds if cost > b => (over, true),
        PlanVerdict::OverBudget { holds: true, .. } if cost > b => (over, false),
        v => (v, false),
    };
    let check = BudgetCheck {
        budget_micro_usd_hr: b,
        cost_micro_usd_hr: cost,
        rejudged,
    };
    (v, Some(check))
}

/// The sentences of a plan in plain text, the ones the Markdown report prints, so a screen shows
/// them without wording a verdict itself (Step 9c-3).
#[derive(Debug, Clone, Default, Serialize)]
pub struct Words {
    /// The verdict of the plan (cheapest-aware).
    pub verdict: String,
    /// The budget check line, when a budget was asked for.
    pub budget: Option<String>,
    /// The verdict of each provider section in `providers` order; `""` for a skipped one. `[]` for a plain plan.
    pub sections: Vec<String>,
}

/// A finished plan, as the record carries it.
#[derive(Debug, Clone, Serialize)]
pub struct PlanDoc {
    /// [`LABEL`].
    pub label: &'static str,
    /// What was asked.
    pub request: PlanAsked,
    /// How the walk ended, judged again against the budget with per-use prices ([`judge_budget`]).
    pub verdict: PlanVerdict,
    /// The budget with per-use prices, when one was asked for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_check: Option<BudgetCheck>,
    /// The main path, in order.
    pub steps: Vec<Step>,
    /// The target's effect on the design in force.
    pub start: Effect,
    /// And on the plan's design.
    pub end: Effect,
    /// Price of the design in force (priced nodes, no visits); `None` when none is priced.
    pub price_before: Option<u64>,
    /// Price of the plan's design.
    pub price_after: Option<u64>,
    /// Nodes with no price, left out of the prices and the budget.
    pub unpriced_nodes: Vec<String>,
    /// The database-family what-ifs (not part of the path).
    pub families: Vec<FamilySection>,
    /// The re-check of the proposed draft.
    pub verify: Verify,
    /// What one sweep of the proposed design finds; `None` when it could not run (`sweep_error`).
    pub measured_knee: Option<Verdict>,
    /// Why the sweep of the proposed design did not run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sweep_error: Option<String>,
    /// A cheapest plan's [`kaanal_sim::plan::CHEAPEST_NOTE`] (Step 9c-2); `label` stays.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cheapest_note: Option<&'static str>,
    /// How the main search ran; `None` for a plain plan (the 9c-1 walk).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search: Option<kaanal_sim::plan::SearchInfo>,
    /// The provider what-ifs of a cheapest plan, in `PROVIDERS` order (`[]` when none ran);
    /// `None` for a plain plan, and then not serialised.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub providers: Option<Vec<ProviderSection>>,
    /// The design in force against each provider what-if, built once ([`crate::sim::report::compare`])
    /// so the Markdown report and the CLI say the same; `None` when no provider section ran.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comparison: Option<crate::sim::report::Comparison>,
    /// The report's sentences in plain text ([`Words`]); set when the doc is built.
    pub words: Words,
}

/// The applied text with `steps` written in, one patch operation per step: `set` when the key is in
/// the text, `insert` when not (`instance`, `cpu.cores`, `memory.limit` in bytes, `replicas`).
///
/// # Errors
/// A step that cannot be patched (its node is not in the text, a value the text cannot take).
pub(crate) fn proposed_text(applied: &str, steps: &[Step]) -> Result<String, String> {
    let mut text = applied.to_string();
    for s in steps {
        let node = s.mv.node.as_str();
        let (key, value): (&[&str], Value) = match &s.mv.kind {
            MoveKind::Class { to, .. } => (&["instance"], json!(to)),
            MoveKind::Cores { to, .. } => (&["cpu", "cores"], json!(to)),
            MoveKind::Memory { to, .. } => (&["memory", "limit"], json!(to)),
            MoveKind::Copies { to, .. } => (&["replicas"], json!(to)),
        };
        let mut path = vec!["nodes", node];
        path.extend_from_slice(key);
        let doc: Yaml = serde_yaml_ng::from_str(&text).map_err(|e| e.to_string())?;
        let exists = path.iter().try_fold(&doc, |v, k| v.get(*k)).is_some();
        let what = if exists { "set" } else { "insert" };
        let op: Op = serde_json::from_value(json!({ "op": what, "path": path, "value": value }))
            .map_err(|e| e.to_string())?;
        text = patch::apply(&text, &[op])
            .map_err(|e| format!("node '{node}' {}: {e}", path[2..].join(".")))?
            .text;
    }
    Ok(text)
}

/// Builds `text` as a new design (the files beside it read again), checks it at load factor `k`
/// under `chaos` (compiled for `old`, whose node order the text must keep), and prices it with one
/// run at the target. Returns the re-check and the built design, for the sweep.
///
/// # Errors
/// The text does not build, names its nodes in another order, or a run fails.
pub(crate) fn verify(
    st: &AppState,
    text: &str,
    old: &Generation,
    chaos: &EffectiveChaos,
    base: &SimConfig,
    k: f64,
    limit: &Limit,
) -> Result<(Verify, Arc<Generation>), String> {
    let built = build_draft(text, &st.dir, st.drawn_seed, st.auth_key.clone(), st.port)
        .map_err(|d| format!("the proposed draft does not build: {}", d.message))?;
    let g = built.generation;
    let names = |g: &Generation| {
        g.topo
            .nodes
            .iter()
            .map(|n| n.name.clone())
            .collect::<Vec<_>>()
    };
    if names(&g) != names(old) {
        return Err(
            "the proposed draft does not name the nodes of the design in force in the same order"
                .into(),
        );
    }
    let c = check(
        &g.topo,
        &g.endpoints,
        chaos,
        base,
        k,
        limit,
        &Limits::server(),
    )
    .map_err(|e| match e {
        PointFail::TooLarge(m) | PointFail::Other(m) => {
            format!("the check of the proposed draft: {m}")
        }
    })?;
    let job = Job {
        g: Arc::new(g),
        chaos: chaos.clone(),
        cfg: price_cfg(base, k),
    };
    let (_, _, cost) =
        execute_run(&job).map_err(|e| format!("the pricing run of the proposed draft: {e}"))?;
    let v = Verify {
        holds: c.holds,
        p99_ms: c.point.p99_ms,
        error_rate: c.point.error_rate,
        capacity_req_s: c.capacity_req_s,
        cost,
        cost_note: (!c.holds).then_some(SATURATED_COST),
    };
    Ok((v, job.g))
}

/// `Ok` when the re-check of the proposed draft is exactly the walk's `end` (p99, error rate,
/// capacity estimate, compared bit for bit). [`verify`] runs the walk's own `check` with the walk's
/// arguments (seed, load factor, limit, caps, chaos), and a run is deterministic, so any difference
/// means the built text is not the design the walk checked.
///
/// # Errors
/// The two figures side by side.
pub(crate) fn round_trip(end: &Effect, v: &Verify) -> Result<(), String> {
    let same = |a: f64, b: f64| a.to_bits() == b.to_bits();
    let cap = match (end.capacity_req_s, v.capacity_req_s) {
        (Some(a), Some(b)) => same(a, b),
        (a, b) => a.is_none() && b.is_none(),
    };
    if same(end.p99_ms, v.p99_ms) && same(end.error_rate, v.error_rate) && cap {
        return Ok(());
    }
    Err(format!(
        "the proposed draft is not the design the walk checked: its re-check gives p99 {} ms, errors {}, capacity {:?} req/s; the walk ended at p99 {} ms, errors {}, capacity {:?} req/s",
        v.p99_ms, v.error_rate, v.capacity_req_s, end.p99_ms, end.error_rate, end.capacity_req_s
    ))
}

/// Why a plan cannot start or go on: the applied text no longer builds to the design in force. Lasting
/// (a live edit whose structure waits for an Apply, a file the loader read changed on disk) or a
/// moment (an Apply installs its generation before it writes the text).
pub const DIFFERS: &str =
    "the design in force differs from the applied text; Apply it (or retry if an Apply just ran)";

/// `Ok` when `applied` builds to the design in force `g` (same fingerprint, so the same files and
/// values): the text a plan patches must be the one it walked.
///
/// # Errors
/// The text does not build, or builds to another design.
pub(crate) fn same_design(st: &AppState, applied: &str, g: &Generation) -> Result<(), String> {
    let built = build_draft(
        applied,
        &st.dir,
        st.drawn_seed,
        st.auth_key.clone(),
        st.port,
    )
    .map_err(|d| format!("{DIFFERS} (it no longer builds: {})", d.message))?;
    if built.generation.fingerprint == g.fingerprint {
        Ok(())
    } else {
        Err(DIFFERS.into())
    }
}

/// **Private.** `base` at `k` times every stream's rate, its window the profile's or the one that
/// gives about [`PRICE_REQUESTS`] requests, whichever is shorter (at least a second); warmup a tenth.
pub(crate) fn price_cfg(base: &SimConfig, k: f64) -> SimConfig {
    let streams: Vec<Stream> = base
        .streams
        .iter()
        .map(|s| Stream {
            rate_per_s: s.rate_per_s * k,
            ..s.clone()
        })
        .collect();
    let rate: f64 = streams.iter().map(|s| s.rate_per_s).sum();
    let profile = base.duration.saturating_sub(base.warmup);
    let window = Duration::try_from_secs_f64(PRICE_REQUESTS / rate)
        .unwrap_or(profile)
        .min(profile)
        .max(Duration::from_secs(1));
    let warmup = window / 9;
    SimConfig {
        streams,
        duration: warmup + window,
        warmup,
        seed: base.seed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kaanal_core::resize::Move;

    fn step(node: &str, kind: MoveKind) -> Step {
        let e = Effect {
            p99_ms: 1.0,
            error_rate: 0.0,
            capacity_req_s: None,
        };
        Step {
            mv: Move {
                node: node.into(),
                kind,
            },
            price_micro_usd_hr: None,
            price_delta_micro_usd_hr: None,
            before: e.clone(),
            after: e,
            menu: Vec::new(),
            note: None,
        }
    }

    const TEXT: &str = "version: 1
# kept
nodes:
  app: { type: service, cpu: { cores: 1 } }
  db:
    type: database # the store
    instance: aws/m5.large
";

    #[test]
    fn steps_are_set_or_inserted_and_the_comments_survive() {
        let steps = [
            step("db", MoveKind::Copies { from: 1, to: 2 }),
            step("db", MoveKind::Copies { from: 2, to: 3 }),
            step("app", MoveKind::Cores { from: 1.0, to: 2.0 }),
            step(
                "db",
                MoveKind::Class {
                    from: "aws/m5.large".into(),
                    to: "aws/m5.xlarge".into(),
                },
            ),
        ];
        let t = proposed_text(TEXT, &steps).unwrap();
        assert!(t.contains("# kept") && t.contains("# the store"), "{t}");
        let c = kaanal_core::Config::from_yaml(&t).unwrap();
        assert_eq!(c.nodes["db"].replicas(), 3, "{t}");
        assert_eq!(c.nodes["db"].instance_name(), Some("aws/m5.xlarge"), "{t}");
        assert!(t.contains("cpu: { cores: 2.0 }"), "{t}");
        assert_eq!(
            proposed_text(TEXT, &[]).unwrap(),
            TEXT,
            "no step, no change"
        );
    }

    fn verified(p99_ms: f64, error_rate: f64, capacity_req_s: Option<f64>) -> Verify {
        Verify {
            holds: true,
            p99_ms,
            error_rate,
            capacity_req_s,
            cost: kaanal_core::cost::estimate(
                &kaanal_core::Config::from_yaml(TEXT).unwrap(),
                &std::collections::BTreeMap::new(),
            ),
            cost_note: None,
        }
    }

    /// Spec anti-goal: the draft names the design that was checked. The re-check runs the walk's own
    /// check, so any difference from the walk's end, whatever the verdict, is a different design.
    #[test]
    fn the_re_check_must_be_the_walks_end_exactly() {
        let end = Effect {
            p99_ms: 12.5,
            error_rate: 0.001,
            capacity_req_s: Some(300.0),
        };
        assert_eq!(
            round_trip(&end, &verified(12.5, 0.001, Some(300.0))),
            Ok(())
        );
        for v in [
            verified(12.500_000_1, 0.001, Some(300.0)),
            verified(12.5, 0.0, Some(300.0)),
            verified(12.5, 0.001, None),
        ] {
            let e = round_trip(&end, &v).unwrap_err();
            assert!(e.contains("not the design the walk checked"), "{e}");
        }
    }

    /// Spec: the budget includes per-use prices at the target run's visits. The walk prices without
    /// them, so the server judges the verdict again from the checked cost.
    #[test]
    fn the_budget_is_judged_again_with_per_use_prices() {
        let ob = |over, holds| PlanVerdict::OverBudget {
            over_by_micro_usd_hr: over,
            holds,
        };
        let check = |cost, rejudged| {
            Some(BudgetCheck {
                budget_micro_usd_hr: 2_000_000,
                cost_micro_usd_hr: cost,
                rejudged,
            })
        };
        // holds within the walk's prices, over with the per-use part: over budget, said so
        assert_eq!(
            judge_budget(PlanVerdict::Holds, Some(2_000_000), 2_300_000),
            (ob(300_000, true), check(2_300_000, true))
        );
        // still within: holds
        assert_eq!(
            judge_budget(PlanVerdict::Holds, Some(2_000_000), 1_900_000),
            (PlanVerdict::Holds, check(1_900_000, false))
        );
        // over budget and holding: by how much the checked cost is over
        assert_eq!(
            judge_budget(ob(100_000, true), Some(2_000_000), 2_400_000),
            (ob(400_000, true), check(2_400_000, false))
        );
        // a plan that does not hold keeps its verdict; the check says what the shown plan costs
        assert_eq!(
            judge_budget(ob(50_000, false), Some(2_000_000), 2_500_000),
            (ob(50_000, false), check(2_500_000, false))
        );
        assert_eq!(
            judge_budget(PlanVerdict::Holds, None, 9),
            (PlanVerdict::Holds, None)
        );
    }

    #[test]
    fn a_step_that_cannot_patch_fails_the_plan() {
        let e = proposed_text(TEXT, &[step("ghost", MoveKind::Copies { from: 1, to: 2 })]);
        assert!(e.unwrap_err().contains("ghost"), "names the node");
    }

    #[test]
    fn the_pricing_run_scales_every_stream_and_keeps_the_window_short() {
        let base = SimConfig {
            streams: vec![Stream {
                endpoint_key: "GET /a".into(),
                rate_per_s: 100.0,
                req_bytes: 1,
                resp_bytes: 2,
            }],
            duration: Duration::from_secs(66),
            warmup: Duration::from_secs(6),
            seed: 7,
        };
        let c = price_cfg(&base, 4.0);
        assert_eq!(c.streams[0].rate_per_s, 400.0);
        assert_eq!(
            c.duration.saturating_sub(c.warmup),
            Duration::from_secs(50),
            "20 000 / 400"
        );
        assert_eq!(c.seed, 7);
        assert_eq!(
            price_cfg(&base, 0.5)
                .duration
                .saturating_sub(price_cfg(&base, 0.5).warmup),
            Duration::from_secs(60),
            "the profile's window"
        );
    }
}

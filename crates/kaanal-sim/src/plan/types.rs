//! What the walk takes and gives (Step 9c-1). Prices are micro-USD an hour, estimates (RULES 121,
//! 126), of the priced nodes only.

use kaanal_core::Config;
use kaanal_core::resize::Move;
use serde::Serialize;

use super::Target;
use crate::sweep::Limit;

/// What the target load does to one design: one run's figures.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Effect {
    /// p99 latency at the target, ms.
    pub p99_ms: f64,
    /// Share of requests that failed at the target (0 to 1).
    pub error_rate: f64,
    /// The lowest analytic `saturates_at_req_s` of any node: an estimate, not a measurement.
    pub capacity_req_s: Option<f64>,
}

/// One move the walk tried at a step (an entry of the menu).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Candidate {
    /// The move.
    pub mv: Move,
    /// The design's price with the move, when the moved node is priced.
    pub price_micro_usd_hr: Option<u64>,
    /// What the target does with the move.
    pub effect: Effect,
    /// True when the design with the move holds the target.
    pub holds: bool,
}

/// One step of the plan.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Step {
    /// The move taken.
    pub mv: Move,
    /// The design's price after the step, when the moved node is priced.
    pub price_micro_usd_hr: Option<u64>,
    /// What the step adds to the price, when the moved node is priced.
    pub price_delta_micro_usd_hr: Option<i64>,
    /// The target's effect before the step.
    pub before: Effect,
    /// And after it.
    pub after: Effect,
    /// Every move tried for the node at this step, the taken one included.
    pub menu: Vec<Candidate>,
    /// What a reader should know: moves refused for size, a node that grows by copies only.
    pub note: Option<String>,
}

/// How the walk ended.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlanVerdict {
    /// The design in force holds the target: nothing to change.
    AlreadyHolds,
    /// The plan holds the target.
    Holds,
    /// The bottleneck cannot be sized (no move, or every move refused); the plan stops at it.
    CannotHold {
        /// The node; `None` when no node saturates (the reason names the crossed limit).
        node: Option<String>,
        /// Why, in plain words.
        reason: String,
    },
    /// A budget stopped the walk. Two cases, told apart by `holds`:
    /// - `holds: true`: the plan is the cheapest holding design the walk found and it costs more
    ///   than the budget, by `over_by_micro_usd_hr`.
    /// - `holds: false`: no holding design was found; the plan shown is where the walk stopped
    ///   (within the budget, not holding the target), and the next step it needed would cost the
    ///   budget plus `over_by_micro_usd_hr`.
    OverBudget {
        /// How far the holding design, or the next step needed, is over the budget.
        over_by_micro_usd_hr: u64,
        /// Whether the plan shown holds the target.
        holds: bool,
    },
    /// Two steps in a row gained nothing. The walk counts them across bottleneck changes, so the two
    /// may be on different nodes.
    NoProgress {
        /// The node of the second of the two steps.
        node: String,
    },
    /// The walk took `max_steps` steps without holding.
    StepCap,
}

/// The database-family what-if for one node (filled by the server, through [`super::walk`] on each
/// alternative; empty from the walk itself).
#[derive(Debug, Clone, Serialize)]
pub struct FamilyAnswer {
    /// The database node.
    pub node: String,
    /// The flavor tried.
    pub flavor: String,
    /// The walk on that alternative.
    pub plan: Box<Plan>,
    /// What else changed because of the flavor.
    pub notes: Vec<String>,
}

/// The walk's answer.
#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    /// How it ended.
    pub verdict: PlanVerdict,
    /// The steps, in order.
    pub steps: Vec<Step>,
    /// The target's effect on the design in force.
    pub start: Effect,
    /// And on the plan's design.
    pub end: Effect,
    /// Price of the design in force (priced nodes; `None` when none is priced).
    pub price_before: Option<u64>,
    /// Price of the plan's design.
    pub price_after: Option<u64>,
    /// Nodes with no price, left out of the prices and the budget.
    pub unpriced_nodes: Vec<String>,
    /// The plan's design (a clone of the input with the steps applied).
    #[serde(skip)]
    pub config: Config,
    /// The database-family answers (empty from [`super::walk`]).
    pub families: Vec<FamilyAnswer>,
}

/// What to plan for.
#[derive(Debug, Clone)]
pub struct Request {
    /// The load to hold.
    pub target: Target,
    /// The pass line (held with the sweep's margin).
    pub limit: Limit,
    /// The most the plan may cost, priced nodes only.
    pub budget_micro_usd_hr: Option<u64>,
    /// Nodes the plan must not move.
    pub keep: Vec<String>,
    /// The most steps (default 30).
    pub max_steps: u32,
}

impl Default for Request {
    fn default() -> Self {
        Request {
            target: Target::Multiple(1.0),
            limit: Limit::default(),
            budget_micro_usd_hr: None,
            keep: Vec::new(),
            max_steps: 30,
        }
    }
}

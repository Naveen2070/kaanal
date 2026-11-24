//! Sizing planner (Step 9c-1): the check a candidate plan must pass, and the walk; the cheapest
//! search (Step 9c-2).

mod check;
mod common;
mod search;
mod search_types;
mod types;
mod walk;

pub use check::{Check, Target, check};
pub use search::search;
pub use search_types::{
    CHEAPEST_NOTE, SearchInfo, SearchOptions, Searched, StopReason, split_time,
};
pub use types::{Candidate, Effect, FamilyAnswer, Plan, PlanVerdict, Request, Step};
pub use walk::walk;

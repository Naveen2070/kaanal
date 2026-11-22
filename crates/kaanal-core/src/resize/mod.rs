//! Sizing candidates for the planner (Step 9c, RULES 141): the size moves a node offers and how one
//! is applied to a cloned [`crate::Config`]. Nothing here runs a simulation or reads a file; the
//! check and the walk live in `kaanal-sim`.

mod family;
mod moves;

pub use family::{Alternative, Alternatives, Family, alternatives, family_of};
pub use moves::{Move, MoveKind, apply, copies_only_reason, moves_for, moves_for_oom, why_fixed};

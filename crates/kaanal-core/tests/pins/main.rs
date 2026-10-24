//! Integration tests for `kaanal-core`: pins (R3.8: one module per topic).
#![expect(
    clippy::many_single_char_names,
    reason = "short fixture names (t, e, c, s, w) keep the scenario tests readable"
)]

mod common;
mod delay;
mod layers;

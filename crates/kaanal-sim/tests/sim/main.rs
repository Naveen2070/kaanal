//! Integration tests for `kaanal-sim`: the simulator (R3.8: one module per topic).

#[path = "../common/mod.rs"]
mod shared;

mod auth;
mod behaviour;
mod common;
mod config;
mod forks;
mod links;
mod replicas;
mod streams;
mod summary;
mod visits;
mod zero_load;

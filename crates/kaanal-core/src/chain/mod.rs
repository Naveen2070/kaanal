//! Endpoint chains: compiles `endpoints:`, node `chains:` and the topology into resolved
//! call trees (spec sections 5.1–5.2).
//!
//! [`compile`] runs once at load. It resolves every node name, edge, `via:` and `with:`,
//! gives routes without a custom chain the default chain, checks every limit and returns
//! [`Endpoints`]: one [`Endpoint`] per route, each a tree of [`Step`]s the executor walks
//! with no lookups. Default bodies are built once per node, callees first, and shared
//! through `Arc`. They fix no op: a database step runs the op of whoever called it (E10),
//! so one body serves reads and writes.
//!
//! When the topology has an auth node, `compile` also creates its two built-in routes,
//! `POST /login` and `GET /.well-known/jwks.json` (spec X7): ordinary compiled endpoints
//! owned by the auth node, tagged with a [`Responder`] the server uses to build the body.
//! Core never builds tokens.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use indexmap::IndexMap;
use serde_json::Value;

use crate::chaos::FailKind;
use crate::config::{
    CallStep, Config, EndpointConfig, EndpointsConfig, ErrorMapping, NodeKind, Override, StepConfig,
};
use crate::diagnostics::{Located, PathSeg};
use crate::dist::Dist;
use crate::exec::Op;
use crate::pin::{PinState, StateOverride, StateOverrides, check_entry};
use crate::topology::{Edge, NodeId, Topology};

mod compiler;
mod endpoint;
mod entries;
mod place;
mod steps;
mod types;

use compiler::{Compiler, limits, normalize};
pub use compiler::{compile, skeleton};
use place::{Entry, Job, Keys, Place, callees_first, error_key};
pub use types::{
    CacheStep, Call, CollectionOp, Endpoint, Endpoints, EndpointsInput, Responder, RouteSpec, Seq,
    Step, StepKey, collection_op, default_status, resolve_error,
};
use types::{METHODS, collection_name_ok};

/// Most steps in one chain list.
pub const MAX_STEPS: usize = 256;

/// Most branches in one `parallel` step.
pub const MAX_BRANCHES: usize = 32;

/// Deepest nesting of `parallel` and inline chains inside one endpoint.
pub const MAX_NESTING: usize = 16;

/// Largest `repeat` (and edge `calls`) a chain may resolve.
pub const MAX_REPEAT: u32 = 1000;

/// Most hops an endpoint may resolve to after expanding defaults, `via` and `with`.
pub const MAX_HOPS: u64 = 100_000;

/// Deepest resolved call tree (D9).
pub const MAX_DEPTH: u32 = 64;

/// The built-in login route of an auth node (spec X7).
pub const LOGIN_KEY: &str = "POST /login";

/// The built-in JWKS route of an auth node (spec X7).
pub const JWKS_KEY: &str = "GET /.well-known/jwks.json";

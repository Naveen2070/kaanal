//! `OpenAPI` 3.0 ingestion and deterministic mock-data generation.
//!
//! [`ApiModel`] parses a spec into endpoints with `$ref`-free response schemas;
//! [`generate`] turns such a schema into realistic JSON, seeded so the same input
//! always gives the same output. This crate never depends on `kaanal-core` (rule
//! R2.4) — it keeps its own private copy of the FNV-1a hash, pinned identical by
//! a test.

#![warn(missing_docs)]

mod api;
mod generate;
mod warn;
mod words;

pub use api::{ApiError, ApiModel, Endpoint};
pub use generate::{apply_params, generate};
pub use warn::unsupported_keywords;

//! `OpenAPI` 3.0 ingestion and deterministic mock-data generation.
//!
//! [`ApiModel`] parses a spec into endpoints with `$ref`-free response schemas;
//! [`generate`] turns such a schema into realistic JSON, seeded so the same input
//! always gives the same output. This crate never depends on `kaanal-core` (rule
//! R2.4) — it keeps its own private copy of the FNV-1a hash, pinned identical by
//! a test.

#![warn(missing_docs)]

mod api;
mod dataset;
mod dictionary;
mod empty;
mod generate;
mod pack;
mod page;
mod warn;
mod words;

pub use api::{ApiModel, Endpoint, QueryParam};
pub use dataset::{DEFAULT_LOCALE, DataSet, Options, valid_locale_tag};
pub use dictionary::{Dictionary, dictionary_schema, dictionary_warnings};
pub use empty::{empty, empty_of_value};
pub use generate::{apply_params, generate};
pub use pack::{
    Currency, DataError, Geo, Name, NameOrder, Pack, Phone, Place, builtin_tags, pack_schema,
};
pub use page::{
    Envelope, PageError, PageSpec, Style, Window, decode_cursor, encode_cursor, generate_page,
    item_seed,
};

//! Paging: which query parameters of an operation page its response (style), the window a
//! request addresses (`window.rs`), and the page body (`envelope.rs`).

use crate::api::Endpoint;
use serde_json::Value;

mod envelope;
mod links;
mod window;
pub use envelope::{Envelope, generate_page, item_seed};
pub(crate) use envelope::{array_props, is_list_name};
pub use window::{PageError, Window, decode_cursor, encode_cursor};

/// How an endpoint addresses a window of its collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// Page number, 1-based unless the spec declares it 0-based ([`PageSpec::page_base`]).
    Page,
    /// Zero-based item offset.
    Offset,
    /// Opaque cursor token.
    Cursor,
}

/// A detected paging contract for one endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageSpec {
    /// The style of the index parameter.
    pub style: Style,
    /// Name of the index parameter, as the spec spells it.
    pub index_param: String,
    /// Name of the page-size parameter, if declared.
    pub size_param: Option<String>,
    /// Page size when the caller sends none.
    pub size_default: u32,
    /// Smallest accepted page size.
    pub size_min: u32,
    /// Largest accepted page size.
    pub size_max: u32,
    /// Lowest total item count the generator may pick.
    pub total_min: u32,
    /// Highest total item count the generator may pick.
    pub total_max: u32,
    /// Where the records go in the response.
    pub envelope: Envelope,
    /// Number of the first page: 0 when the page parameter declares `minimum: 0` (or `default: 0`
    /// and no minimum above 0, as Spring does), else 1. Always 1 for the offset and cursor styles.
    pub page_base: u32,
    /// Most records one page may hold, so its body stays within the generator's per-body value
    /// cap (`MAX_VALUES`); a larger page size is shrunk to it, never below 1.
    pub record_cap: u32,
}

const INDEX: &[&str] = &["page", "pagenumber"];
const OFFSET: &[&str] = &["offset", "skip"];
const CURSOR: &[&str] = &[
    "cursor",
    "after",
    "pagetoken",
    "nextpagetoken",
    "startingafter",
];
const SIZE: &[&str] = &[
    "limit",
    "size",
    "pagesize",
    "perpage",
    "count",
    "maxresults",
];

const HARD_CAP: i64 = 10_000;

fn normalise(s: &str) -> String {
    s.chars()
        .filter(|c| *c != '_' && *c != '-')
        .flat_map(char::to_lowercase)
        .collect()
}

fn clamp(v: i64) -> u32 {
    u32::try_from(v.clamp(0, HARD_CAP)).unwrap_or(0)
}

impl PageSpec {
    /// Detect the paging style of a `GET` endpoint whose response is an array, or an object
    /// with an array property ([`Envelope::detect`]).
    #[must_use]
    pub fn detect(ep: &Endpoint) -> Option<PageSpec> {
        if !ep.method.eq_ignore_ascii_case("get") {
            return None;
        }
        let schema = ep.schema.as_ref()?;
        // Page/offset index and size params must be numeric; a cursor may be a string.
        let find = |names: &[&str], any_type: bool| {
            ep.query_params
                .iter()
                .find(|p| (any_type || p.numeric) && names.contains(&normalise(&p.name).as_str()))
        };
        // ponytail: priority cursor > offset > page when several are declared.
        let (style, idx) = find(CURSOR, true)
            .map(|p| (Style::Cursor, p))
            .or_else(|| find(OFFSET, false).map(|p| (Style::Offset, p)))
            .or_else(|| find(INDEX, false).map(|p| (Style::Page, p)))?;
        let size = find(SIZE, false);
        let size_max = size.and_then(|p| p.maximum).map_or(100, clamp);
        let size_min = size.and_then(|p| p.minimum).map_or(1, clamp);
        let size_default = size.and_then(|p| p.default).map_or(20, clamp);
        let envelope = Envelope::detect(schema)?;
        // Bounds come from the array that is paged, not from an object envelope.
        let array = envelope.array_of(schema);
        let items = |k: &str| array.get(k).and_then(Value::as_i64);
        let (lo, hi) = (items("minItems").map(clamp), items("maxItems").map(clamp));
        let (mut total_min, mut total_max) = (lo.unwrap_or(23), hi.unwrap_or(137));
        // minItems wins over a smaller maxItems, as in the generator.
        if lo.is_some() {
            total_max = total_max.max(total_min);
        } else {
            total_min = total_min.min(total_max);
        }
        let zero_based =
            idx.minimum == Some(0) || (idx.default == Some(0) && idx.minimum.unwrap_or(0) <= 0);
        let page_base = u32::from(!(style == Style::Page && zero_based));
        let record_cap = envelope.record_cap(schema);
        Some(PageSpec {
            style,
            index_param: idx.name.clone(),
            size_param: size.map(|p| p.name.clone()),
            size_default,
            size_min,
            size_max,
            total_min,
            total_max,
            envelope,
            page_base,
            record_cap,
        })
    }
}

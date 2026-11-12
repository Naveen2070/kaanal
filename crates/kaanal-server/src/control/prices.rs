//! `GET /__kaanal/prices` and `GET /__kaanal/prices/{provider}/{region}` (RULES 121): the regions
//! the cost estimate can price in, and one region's price list. One source, the embedded files the
//! core reads, so a region the picker offers is a region the table has. Estimates.

use axum::Json;
use axum::extract::Path;
use axum::http::StatusCode;
use serde_json::{Value, json};

use super::{Reject, reject};

/// `GET /__kaanal/prices` — `{ defaults: { aws: "us-east-1", ... }, regions: { aws: [...], ... } }`.
pub async fn get_price_regions() -> Json<Value> {
    let defaults: serde_json::Map<String, Value> = kaanal_core::prices::DEFAULT_REGIONS
        .iter()
        .map(|(p, r)| ((*p).to_string(), json!(r)))
        .collect();
    let regions: serde_json::Map<String, Value> = ["aws", "azure", "gcp"]
        .iter()
        .map(|p| ((*p).to_string(), json!(kaanal_core::prices::regions(p))))
        .collect();
    Json(json!({ "defaults": defaults, "regions": regions }))
}

/// `GET /__kaanal/prices/{provider}/{region}` — `{ provider, region, prices: [[name,
/// usd_micro_hr], ...] }` sorted by name: the on-demand Linux price an hour in micro-USD of every
/// type that has one there.
///
/// # Errors
/// 404 for a provider or region the table does not have.
pub async fn get_prices(
    Path((provider, region)): Path<(String, String)>,
) -> Result<Json<Value>, Reject> {
    let table = kaanal_core::prices::region_table(&provider, &region).ok_or_else(|| {
        reject(
            StatusCode::NOT_FOUND,
            &format!("no prices for region '{region}' of '{provider}'"),
        )
    })?;
    let prices: Vec<Value> = table.iter().map(|(n, p)| json!([n, p])).collect();
    Ok(Json(
        json!({ "provider": provider, "region": region, "prices": prices }),
    ))
}

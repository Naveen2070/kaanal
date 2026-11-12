//! `GET /__kaanal/instances`: the cloud instance table (step 8P) Studio's picker lists. One source:
//! the table the engine fills `instance:` from, so a name the picker offers is a name the loader
//! accepts.

use axum::Json;
use serde_json::{Value, json};

/// `GET /__kaanal/instances` — `{ types: [[name, vcpu, memory_mib, net_mbps, usd_micro_hr], ...] }`,
/// sorted by name; `net_mbps` 0 means the source gives no figure (the card is then not limited) and
/// `usd_micro_hr` is the on-demand Linux price an hour in micro-USD, 0 for none. Estimates.
pub async fn get_instances() -> Json<Value> {
    let types: Vec<Value> = kaanal_core::instances::all()
        .iter()
        .map(|i| {
            json!([
                i.name,
                i.vcpu,
                i.memory_mib,
                i.net_mbps,
                i.price_micro_usd_hr
            ])
        })
        .collect();
    Json(json!({ "types": types }))
}

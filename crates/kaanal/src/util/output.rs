//! The `--json` switch: one global bit, set once in `main`, read where a command prints.

use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};

/// **Private.** Whether commands print JSON instead of text.
static JSON: AtomicBool = AtomicBool::new(false);

/// **Private.** Turns JSON output on or off.
pub fn set_json(on: bool) {
    JSON.store(on, Ordering::Relaxed);
}

/// **Private.** Whether JSON output is on.
pub fn json_on() -> bool {
    JSON.load(Ordering::Relaxed)
}

/// **Private.** `KAANAL_JSON` counts as the flag when it is `1` or `true`; empty or anything
/// else is off.
pub fn env_on() -> bool {
    std::env::var("KAANAL_JSON").is_ok_and(|v| v == "1" || v == "true")
}

/// **Private.** Prints `v` as one compact line in JSON mode (nothing for `Null`), else runs `human`.
pub fn emit(v: &Value, human: impl FnOnce()) {
    if !json_on() {
        human();
    } else if !v.is_null() {
        crate::out_line(&v.to_string());
    }
}

/// **Private.** Time left on a lease: whole seconds rounded up, or `Nms` under a second.
pub fn remaining(ms: u64) -> String {
    if ms >= 1000 {
        format!("{}s", ms.div_ceil(1000))
    } else {
        format!("{ms}ms")
    }
}

/// **Private.** One row per topology node: `{node, type, flavor, state, chaos, expires_in_ms}`.
/// `active` is `GET /scenarios/active` (effective chaos), `live` is `GET /chaos` (lease timers);
/// `expires_in_ms` is the soonest lease on the node, else null.
pub fn node_rows(topo: &Value, active: &Value, live: &Value) -> Vec<Value> {
    let eff = &active["effective"]["nodes"];
    topo["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|n| {
            let name = n["name"].as_str().unwrap_or("");
            let c = &eff[name];
            // ponytail: state (`active`) and countdown (`live`) are two calls, not one snapshot; a lease expiring between them can show down with no countdown, cosmetic and clears itself
            // ponytail: soonest lease across units, so a short latency lease on a permanently-down node shows "reverts in"; show per-unit if that confuses
            let expires = live["nodes"][name]["expires_in_ms"]
                .as_object()
                .and_then(|m| m.values().filter_map(Value::as_u64).min());
            let mut row = serde_json::json!({
                "node": n["name"], "type": n["type"], "flavor": n["flavor"],
                "state": if c["down"] == true { "down" } else { "up" },
                "chaos": c, "expires_in_ms": expires,
            });
            // only a replicated node carries a count, so other rows are unchanged
            if n["replicas"].is_u64() {
                row["replicas"] = n["replicas"].clone();
            }
            row
        })
        .collect()
}

/// **Private.** The node name, with ` x3` after a node that has replicas.
fn node_cell(r: &Value) -> String {
    let name = r["node"].as_str().unwrap_or("");
    r["replicas"]
        .as_u64()
        .map_or_else(|| name.to_string(), |n| format!("{name} x{n}"))
}

/// **Private.** The node table shared by `status` and `node list`.
pub fn print_node_table(rows: &[Value]) {
    println!(
        "{:<12} {:<12} {:<10} {:<6} CHAOS",
        "NODE", "TYPE", "FLAVOR", "STATE"
    );
    for r in rows {
        let mut text = if r["chaos"].is_null() {
            "-".to_string()
        } else {
            crate::chaos_text(&r["chaos"])
        };
        if let Some(ms) = r["expires_in_ms"].as_u64() {
            text = format!("{text} (reverts in {})", remaining(ms));
        }
        let line = format!(
            "{:<12} {:<12} {:<10} {:<6} {}",
            node_cell(r),
            r["type"].as_str().unwrap_or(""),
            r["flavor"].as_str().unwrap_or("-"),
            r["state"].as_str().unwrap_or(""),
            text
        );
        println!("{}", crate::clean(&line));
    }
}

#[cfg(test)]
mod tests {
    use super::remaining;

    #[test]
    fn remaining_rounds_up_to_whole_seconds() {
        assert_eq!(remaining(999), "999ms");
        assert_eq!(remaining(1000), "1s");
        assert_eq!(remaining(1001), "2s");
        assert_eq!(remaining(59_001), "60s");
    }
}

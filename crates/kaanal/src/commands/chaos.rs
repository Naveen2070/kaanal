//! `kaanal chaos`, `node`, `cycle` and `pins`: patch bodies, paths and the text views of live chaos and pins.

use std::fmt::Write as _;

use serde_json::{Map, Value, json};

use crate::cli::{ChaosCmd, NodeCmd};
use crate::util::parse::{parse_ms, parse_rate};
use crate::{
    Duration, Fail, PIN_STATES, call, clean, encode_segment, err, output, ttl_body, warn_all,
};

/// **Private.** `db#2` for a replica (human output only: the API takes `node` and `replica` apart).
fn node_label(node: &str, replica: Option<u16>) -> String {
    replica.map_or_else(|| node.to_string(), |r| format!("{node}#{r}"))
}

/// **Private.** `PUT`s `patch` to one node, or to one replica of it (the keyed route).
fn put_node(
    node: &str,
    replica: Option<u16>,
    mut patch: Map<String, Value>,
) -> Result<Value, Fail> {
    match replica {
        None => call("PUT", &node_path(node), Some(Value::Object(patch))),
        Some(r) => {
            patch.insert("node".into(), json!(node));
            patch.insert("replica".into(), json!(r));
            call("PUT", "/__kaanal/chaos", Some(Value::Object(patch)))
        }
    }
}

/// **Private.** Whether the effective chaos of `v` still holds `node` (or its `replica`) down.
fn still_down(v: &Value, node: &str, replica: Option<u16>) -> bool {
    match replica {
        None => v["effective"]["nodes"][node]["down"] == true,
        Some(r) => v["effective"]["replicas"].as_array().is_some_and(|a| {
            a.iter()
                .any(|e| e["node"] == node && e["replica"] == r && e["down"] == true)
        }),
    }
}

/// **Private.** `kaanal chaos set|clear`.
pub(crate) fn chaos(cmd: ChaosCmd) -> Result<(), Fail> {
    match cmd {
        ChaosCmd::Set {
            node,
            route,
            replica,
            kind,
            latency,
            error_rate,
            error,
            down,
            for_,
        } => {
            let mut patch = chaos_patch(latency, error_rate, error, kind, down)?;
            let revert = for_set(&mut patch, for_)?;
            let label = match (&node, &route) {
                (Some(n), Some(e)) => format!("{n} @ {e}"),
                (Some(n), None) => node_label(n, replica),
                (None, e) => e.clone().unwrap_or_default(),
            };
            let v = if route.is_some() {
                patch.extend(node.map(|n| ("node".to_string(), json!(n))));
                patch.extend(route.map(|e| ("endpoint".to_string(), json!(e))));
                call("PUT", "/__kaanal/chaos", Some(Value::Object(patch)))?
            } else {
                // clap guarantees a node when there is no endpoint
                put_node(&node.unwrap_or_default(), replica, patch)?
            };
            output::emit(&v, || {
                println!("{}", clean(&format!("{label}: {v}{revert}")));
            });
            warn_all(&v);
            Ok(())
        }
        ChaosCmd::Clear {
            node,
            route,
            replica,
        } => {
            let path = chaos_clear_path(node, route, replica);
            let v = call("DELETE", &path, None)?;
            output::emit(&v, || println!("cleared"));
            Ok(())
        }
    }
}

/// **Private.** The body of `chaos set`, checked here so a typo sends nothing.
pub(crate) fn chaos_patch(
    latency: Option<String>,
    error_rate: Option<String>,
    error: Option<u16>,
    kind: Option<String>,
    down: bool,
) -> Result<Map<String, Value>, Fail> {
    let mut patch = Map::new();
    if let Some(l) = latency {
        patch.insert(
            "latency_ms".into(),
            json!(parse_ms(&l).map_err(Fail::usage)?),
        );
    }
    if let Some(r) = error_rate {
        patch.insert(
            "error_rate".into(),
            json!(parse_rate(&r).map_err(Fail::usage)?),
        );
    }
    if let Some(s) = error {
        patch.insert("error_status".into(), json!(s));
    }
    if let Some(k) = kind {
        patch.insert("error_kind".into(), json!(k));
    }
    if down {
        patch.insert("down".into(), json!(true));
    }
    if patch.is_empty() {
        return Err(Fail::usage(
            "nothing to set: use --latency, --error-rate, --error, --kind or --down",
        ));
    }
    Ok(patch)
}

/// **Private.** Adds a validated `--for` to the patch as `ttl`; returns ` (reverts in D)` for the
/// text output, or nothing.
pub(crate) fn for_set(patch: &mut Map<String, Value>, d: Option<String>) -> Result<String, Fail> {
    let Some(Value::Object(b)) = ttl_body("--for", d)? else {
        return Ok(String::new());
    };
    let text = format!(" (reverts in {})", b["ttl"].as_str().unwrap_or_default());
    patch.extend(b);
    Ok(text)
}

/// **Private.** `kaanal node down|up`: `PUT /chaos/NODE` with only the down flag.
pub(crate) fn node(cmd: NodeCmd) -> Result<(), Fail> {
    match cmd {
        NodeCmd::Down {
            node,
            replica,
            for_,
        } => {
            let mut patch = Map::from_iter([("down".to_string(), json!(true))]);
            let revert = for_set(&mut patch, for_)?;
            let v = put_node(&node, replica, patch)?;
            output::emit(&v, || {
                println!("{}: down{revert}", clean(&node_label(&node, replica)));
            });
        }
        NodeCmd::List => {
            let topo = call("GET", "/__kaanal/topology", None)?;
            let active = call("GET", "/__kaanal/scenarios/active", None)?;
            let live = call("GET", "/__kaanal/chaos", None)?;
            let rows = output::node_rows(&topo, &active, &live);
            output::emit(&Value::Array(rows.clone()), || {
                output::print_node_table(&rows);
            });
        }
        NodeCmd::Up { node, replica } => {
            let patch = Map::from_iter([("down".to_string(), json!(false))]);
            let v = put_node(&node, replica, patch)?;
            // The server leaves calm keys out of `effective`, so a missing entry means "not down";
            // a present entry always carries a bool `down` (resolved_json).
            if still_down(&v, &node, replica) {
                return Err(err(format!(
                    "{} is still down: held by an active scenario or override (see kaanal scenario active)",
                    node_label(&node, replica)
                )));
            }
            output::emit(&v, || {
                println!("{}: up", clean(&node_label(&node, replica)));
            });
        }
    }
    Ok(())
}

/// **Private.** `/__kaanal/chaos/NODE`.
pub(crate) fn node_path(node: &str) -> String {
    format!("/__kaanal/chaos/{}", encode_segment(node))
}

/// **Private.** The `DELETE` path of `chaos clear` for a node (or one replica of it), an endpoint
/// or both.
pub(crate) fn chaos_clear_path(
    node: Option<String>,
    endpoint: Option<String>,
    replica: Option<u16>,
) -> String {
    match (node, endpoint, replica) {
        (None, None, _) => "/__kaanal/chaos".to_string(),
        (Some(n), None, Some(r)) => {
            format!("/__kaanal/chaos?node={}&replica={r}", encode_segment(&n))
        }
        (Some(n), None, None) => format!("/__kaanal/chaos/{}", encode_segment(&n)),
        (n, Some(e), _) => {
            let node = n.map_or(String::new(), |n| format!("node={}&", encode_segment(&n)));
            format!("/__kaanal/chaos?{node}endpoint={}", encode_segment(&e))
        }
    }
}

/// **Private.** ` (status 404, delay 2s)` for the parts that are set, else nothing.
pub(crate) fn pin_extra(status: &Value, delay: Option<String>) -> String {
    let parts: Vec<String> = status
        .as_u64()
        .map(|s| format!("status {s}"))
        .into_iter()
        .chain(delay.map(|d| format!("delay {d}")))
        .collect();
    if parts.is_empty() {
        String::new()
    } else {
        format!(" ({})", parts.join(", "))
    }
}

/// **Private.** [`pin_extra`] for a server pin object, whose delay is `delay_ms`.
pub(crate) fn pin_extra_ms(p: &Value) -> String {
    let delay = p["delay_ms"]
        .as_u64()
        .map(|ms| humantime::format_duration(Duration::from_millis(ms)).to_string());
    pin_extra(&p["status"], delay)
}

/// **Private.** `live` or `scenario:NAME` for a pin's `source`.
pub(crate) fn source_text(s: &Value) -> String {
    s["scenario"]
        .as_str()
        .map_or_else(|| "live".to_string(), |n| format!("scenario:{n}"))
}

/// **Private.** `kaanal cycle`: one `POST /cycle`, or `--off` as a `DELETE`. State names and the
/// delay are checked here so a typo sends nothing; ranges are the server's.
pub(crate) fn cycle(
    endpoint: &str,
    state: Option<String>,
    status: Option<u16>,
    delay: Option<String>,
    off: bool,
) -> Result<(), Fail> {
    if off {
        let path = format!("/__kaanal/pins?endpoint={}", encode_segment(endpoint));
        // The server answers 204 whether or not a pin existed, so this prints either way.
        let v = call("DELETE", &path, None)?;
        output::emit(&v, || println!("{} -> unpinned", clean(endpoint)));
        return Ok(());
    }
    let mut body = json!({ "endpoint": endpoint });
    if let Some(s) = state {
        if !PIN_STATES.contains(&s.as_str()) {
            return Err(Fail::usage(format!(
                "bad --state '{}': use {}",
                clean(&s),
                PIN_STATES.join(", ")
            )));
        }
        body["state"] = json!(s);
    }
    if let Some(d) = delay {
        let d = d.trim();
        humantime::parse_duration(d)
            .map_err(|_| Fail::usage(format!("bad --delay '{}' (use e.g. 2s)", clean(d))))?;
        body["delay"] = json!(d);
    }
    if let Some(s) = status {
        body["status"] = json!(s);
    }
    let v = call("POST", "/__kaanal/cycle", Some(body))?;
    output::emit(&v, || {
        let line = format!(
            "{} -> {}{}",
            v["endpoint"].as_str().unwrap_or(""),
            v["state"].as_str().unwrap_or(""),
            pin_extra_ms(&v)
        );
        println!("{}", clean(&line));
    });
    Ok(())
}

/// **Private.** `kaanal pins`: the live pins, then the effective pin of each endpoint with
/// its source.
pub(crate) fn pins() -> Result<(), Fail> {
    let v = call("GET", "/__kaanal/pins", None)?;
    output::emit(&v, || pins_text(&v));
    Ok(())
}

/// **Private.** The text of `kaanal pins`.
pub(crate) fn pins_text(v: &Value) {
    let live = v["live"].as_array().map_or(&[][..], Vec::as_slice);
    let eff = v["effective"].as_object();
    if live.is_empty() && eff.is_none_or(serde_json::Map::is_empty) {
        println!("no pins");
        return;
    }
    if live.is_empty() {
        println!("no live pins");
    } else {
        println!("live pins:");
    }
    for p in live {
        let line = format!(
            "  {:<24} {}{}",
            p["endpoint"].as_str().unwrap_or(""),
            p["state"].as_str().unwrap_or(""),
            pin_extra_ms(p)
        );
        println!("{}", clean(&line));
    }
    println!("effective pins:");
    for (key, p) in eff.into_iter().flatten() {
        let line = format!(
            "  {key:<24} {:<8} {}{}",
            p["state"].as_str().unwrap_or(""),
            source_text(&p["source"]),
            pin_extra_ms(p)
        );
        println!("{}", clean(&line));
    }
}

/// **Private.** One effective chaos entry as text: latency, each failure band (kinds other
/// than `status` show by name, `status` shows its code), then DOWN.
pub(crate) fn chaos_text(c: &Value) -> String {
    let mut s = format!("+{}ms", c["latency_ms"]);
    // `upto` is cumulative; show each band's own share of the requests.
    let mut prev = 0.0;
    for f in c["faults"].as_array().into_iter().flatten() {
        let upto = f["upto"].as_f64().unwrap_or(0.0);
        let own = upto - prev;
        prev = upto;
        let what = match f["kind"].as_str() {
            Some(k) if k != "status" => k.to_string(),
            _ => f["status"].to_string(),
        };
        let _ = write!(s, ", {:.0}% errors ({what})", own * 100.0);
    }
    if c["down"] == true {
        s.push_str(", DOWN");
    }
    s
}

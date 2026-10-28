//! JSON views of chaos layers and the effective chaos, with the key and path helpers they use.

use super::*;

/// **Private.** Places each view under its key: `{ nodes, endpoints, node_at }`.
pub(super) fn keyed_json(st: &AppState, items: Vec<(ChaosKey, Value)>) -> Map<String, Value> {
    let (mut nodes, mut endpoints, mut node_at, mut replicas, mut edges) =
        (Map::new(), Map::new(), Vec::new(), Vec::new(), Vec::new());
    for (k, view) in items {
        match k {
            ChaosKey::Node(id) => {
                nodes.insert(st.topo.nodes[id].name.clone(), view);
            }
            ChaosKey::Endpoint(e) => {
                endpoints.insert(e, view);
            }
            ChaosKey::NodeAt(id, e) => {
                let Value::Object(mut o) = view else { continue };
                o.insert("node".into(), json!(st.topo.nodes[id].name));
                o.insert("endpoint".into(), json!(e));
                node_at.push(Value::Object(o));
            }
            ChaosKey::Replica(id, r) => {
                let Value::Object(mut o) = view else { continue };
                o.insert("node".into(), json!(st.topo.nodes[id].name));
                o.insert("replica".into(), json!(r));
                replicas.push(Value::Object(o));
            }
            ChaosKey::Edge(e) => {
                let Value::Object(mut o) = view else { continue };
                key_fields(st, &ChaosKey::Edge(e), &mut o);
                edges.push(Value::Object(o));
            }
        }
    }
    let mut out = Map::new();
    out.insert("nodes".into(), Value::Object(nodes));
    out.insert("endpoints".into(), Value::Object(endpoints));
    out.insert("node_at".into(), Value::Array(node_at));
    // only when there is one, so a topology without replica chaos reads as before
    if !replicas.is_empty() {
        out.insert("replicas".into(), Value::Array(replicas));
    }
    if !edges.is_empty() {
        out.insert("edges".into(), Value::Array(edges));
    }
    out
}

/// **Private.** One compiled key: `{ latency_ms, down, faults: [{upto, kind, status}] }`.
pub(super) fn resolved_json(r: &Resolved) -> Value {
    let faults: Vec<Value> = r
        .faults()
        .iter()
        .map(|f| json!({ "upto": f.upto, "kind": f.kind.as_str(), "status": f.status }))
        .collect();
    let mut v = json!({
        "latency_ms": u64::try_from(r.latency_add.as_millis()).unwrap_or(u64::MAX),
        "down": r.down,
        "faults": faults,
    });
    // link fields only for a link, so a node's view reads as before
    if !r.jitter.is_zero() {
        v["jitter_ms"] = json!(u64::try_from(r.jitter.as_millis()).unwrap_or(u64::MAX));
    }
    if let Some(c) = r.bandwidth_cap {
        v["bandwidth_cap"] = json!(c);
    }
    v
}

/// **Private.** A compiled key that does nothing (a live override calmed it): not shown, not saved.
pub(super) fn is_calm(r: &Resolved) -> bool {
    r.latency_add.is_zero()
        && r.jitter.is_zero()
        && r.bandwidth_cap.is_none()
        && !r.down
        && r.faults().is_empty()
}

/// **Private.** The compiled chaos as JSON; calm keys are left out.
pub(super) fn effective_json(st: &AppState, e: &EffectiveChaos) -> Value {
    let items = e
        .entries()
        .into_iter()
        .filter(|(_, r)| !is_calm(r))
        .map(|(k, r)| (k, resolved_json(r)))
        .collect();
    Value::Object(keyed_json(st, items))
}

/// **Private.** `{ active, effective }` of a published snapshot.
pub(super) fn active_json(st: &AppState, snap: &Effective) -> Value {
    let now = Instant::now();
    let expires: Map<String, Value> = snap
        .leases
        .iter()
        .map(|(n, d)| (n.clone(), json!(remaining_ms(*d, now))))
        .collect();
    json!({
        "active": snap.stack.active(),
        "expires_in_ms": expires,
        "effective": effective_json(st, &snap.chaos),
    })
}

/// **Private.** `inline`, `file` or `runtime`.
pub(super) fn source_name(s: Source) -> &'static str {
    match s {
        Source::Inline => "inline",
        Source::File => "file",
        Source::Runtime => "runtime",
    }
}

/// **Private.** Adds the key's `node` / `endpoint` fields to an entry.
pub(super) fn key_fields(st: &AppState, k: &ChaosKey, o: &mut Map<String, Value>) {
    match k {
        ChaosKey::Node(id) => {
            o.insert("node".into(), json!(st.topo.nodes[*id].name));
        }
        ChaosKey::Endpoint(e) => {
            o.insert("endpoint".into(), json!(e));
        }
        ChaosKey::NodeAt(id, e) => {
            o.insert("node".into(), json!(st.topo.nodes[*id].name));
            o.insert("endpoint".into(), json!(e));
        }
        ChaosKey::Replica(id, r) => {
            o.insert("node".into(), json!(st.topo.nodes[*id].name));
            o.insert("replica".into(), json!(r));
        }
        ChaosKey::Edge(e) => {
            let (from, to) = (st.topo.edges[*e].from, st.topo.edges[*e].to);
            o.insert(
                "edge".into(),
                json!({ "from": st.topo.nodes[from].name, "to": st.topo.nodes[to].name }),
            );
        }
    }
}

/// **Private.** For a message: `node 'db' at endpoint 'GET /x'`.
pub(super) fn key_label(st: &AppState, k: &ChaosKey) -> String {
    match k {
        ChaosKey::Node(id) => format!("node '{}'", st.topo.nodes[*id].name),
        ChaosKey::Endpoint(e) => format!("endpoint '{e}'"),
        ChaosKey::NodeAt(id, e) => format!("node '{}' at endpoint '{e}'", st.topo.nodes[*id].name),
        ChaosKey::Replica(id, r) => format!("node '{}' replica {r}", st.topo.nodes[*id].name),
        ChaosKey::Edge(e) => {
            let (from, to) = (st.topo.edges[*e].from, st.topo.edges[*e].to);
            format!(
                "edge '{}' -> '{}'",
                st.topo.nodes[from].name, st.topo.nodes[to].name
            )
        }
    }
}

/// **Private.** A sparse state as scenario entries: only the fields that are set (never the
/// dense defaults), so reloading a saved file adds no explicit value that would last-win.
/// A layer that sets nothing (a bare `mode: merge`) is skipped.
pub(super) fn layers_json(st: &AppState, layers: &ChaosState) -> Vec<Value> {
    let mut out = Vec::new();
    for (k, l) in layers.entries() {
        let bare = ChaosLayer {
            replace: None,
            ..l.clone()
        };
        if bare == ChaosLayer::default() {
            continue;
        }
        let mut o = Map::new();
        key_fields(st, &k, &mut o);
        if let Some(d) = l.latency_add {
            o.insert(
                "latency_ms".into(),
                json!(u64::try_from(d.as_millis()).unwrap_or(u64::MAX)),
            );
        }
        if let Some(r) = l.error_rate {
            o.insert("error_rate".into(), json!(r));
        }
        if let Some(kind) = l.error_kind {
            o.insert("error_kind".into(), json!(kind.as_str()));
        }
        if let Some(s) = l.error_status {
            o.insert("error_status".into(), json!(s));
        }
        if let Some(d) = l.jitter {
            o.insert(
                "jitter_ms".into(),
                json!(u64::try_from(d.as_millis()).unwrap_or(u64::MAX)),
            );
        }
        if let Some(c) = l.bandwidth_cap {
            o.insert("bandwidth_cap".into(), json!(c));
        }
        if let Some(d) = l.down {
            o.insert("down".into(), json!(d));
        }
        out.push(Value::Object(o));
    }
    out
}

/// **Private.** The compiled chaos as scenario entries (`save` with `from: effective`).
/// Limit: a key with two or more failure bands has no single-entry form, and a node@endpoint
/// pair is compiled together with its node's own chaos (writing both would count the node
/// twice); both are refused with a message naming the key. A pair that compiled to exactly
/// its node's own chaos adds nothing and is skipped rather than refused.
pub(super) fn effective_entries(st: &AppState, e: &EffectiveChaos) -> Result<Vec<Value>, Reject> {
    let all = e.entries();
    let nodes: HashMap<usize, &Resolved> = all
        .iter()
        .filter_map(|(k, r)| match k {
            ChaosKey::Node(id) if !is_calm(r) => Some((*id, *r)),
            _ => None,
        })
        .collect();
    let mut out = Vec::new();
    for (k, r) in all {
        if let ChaosKey::NodeAt(id, _) | ChaosKey::Replica(id, _) = &k
            && let Some(own) = nodes.get(id)
        {
            if r == *own {
                continue;
            }
            return Err(reject(
                StatusCode::BAD_REQUEST,
                &format!(
                    "{} is compiled together with the node's own chaos and cannot be written alone; save from live or from a scenario",
                    key_label(st, &k)
                ),
            ));
        }
        if r.faults().len() > 1 {
            return Err(reject(
                StatusCode::BAD_REQUEST,
                &format!(
                    "{} has {} failure bands, which one entry cannot hold; save from live or from a scenario",
                    key_label(st, &k),
                    r.faults().len()
                ),
            ));
        }
        let mut o = Map::new();
        key_fields(st, &k, &mut o);
        let keyed = o.len();
        if !r.latency_add.is_zero() {
            o.insert(
                "latency_ms".into(),
                json!(u64::try_from(r.latency_add.as_millis()).unwrap_or(u64::MAX)),
            );
        }
        if !r.jitter.is_zero() {
            o.insert(
                "jitter_ms".into(),
                json!(u64::try_from(r.jitter.as_millis()).unwrap_or(u64::MAX)),
            );
        }
        if let Some(c) = r.bandwidth_cap {
            o.insert("bandwidth_cap".into(), json!(c));
        }
        if r.down {
            o.insert("down".into(), json!(true));
        }
        if let Some(f) = r.faults().first() {
            o.insert("error_rate".into(), json!(f.upto));
            o.insert("error_kind".into(), json!(f.kind.as_str()));
            if !matches!(k, ChaosKey::Edge(_)) {
                o.insert("error_status".into(), json!(f.status));
            }
        }
        if o.len() > keyed {
            out.push(Value::Object(o));
        }
    }
    Ok(out)
}

/// **Private.** A scenario error as `chaos[2].node: message`.
pub(super) fn place(path: &[PathSeg], msg: &str) -> String {
    let mut s = String::new();
    for p in path {
        match p {
            PathSeg::Key(k) => {
                if !s.is_empty() {
                    s.push('.');
                }
                s.push_str(k);
            }
            PathSeg::Index(i) => {
                let _ = write!(s, "[{i}]");
            }
        }
    }
    if s.is_empty() {
        msg.to_string()
    } else {
        format!("{s}: {msg}")
    }
}

/// **Private.** 404 for a scenario name.
pub(super) fn unknown(name: &str) -> Reject {
    reject(StatusCode::NOT_FOUND, &format!("unknown scenario '{name}'"))
}

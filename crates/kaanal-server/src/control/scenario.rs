//! The scenario handlers: list, get, put, delete, activate (with a ttl lease), deactivate and the active stack.

use super::*;

/// `GET /__kaanal/scenarios` — every defined scenario with its source, entry count and whether
/// it is active.
pub async fn list_scenarios(State(st): State<Arc<AppState>>) -> Json<Value> {
    // Locked first, so the set and the active stack are from the same write.
    let c = lock(&st);
    let snap = st.snapshot();
    let now = Instant::now();
    let out: Vec<Value> = c
        .scenarios
        .names()
        .into_iter()
        .filter_map(|n| c.scenarios.get(n))
        .map(|(s, src)| {
            let mut o = json!({
                "name": s.name,
                "source": source_name(src),
                "description": s.description,
                "entries": s.layers.entry_count() + s.pins.len(),
                "active": snap.stack.active().contains(&s.name),
            });
            o["pins"] = json!(pins_json(&s.pins));
            if let Some(d) = snap.leases.get(&s.name) {
                o["expires_in_ms"] = json!(remaining_ms(*d, now));
            }
            o
        })
        .collect();
    Json(Value::Array(out))
}

/// `GET /__kaanal/scenarios/{name}` — one definition.
///
/// # Errors
/// 404 for an unknown name.
pub async fn get_scenario(
    State(st): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<Value>, Reject> {
    let (c, g) = locked(&st)?;
    let (s, src) = c.scenarios.get(&name).ok_or_else(|| unknown(&name))?;
    Ok(Json(json!({
        "name": s.name,
        "source": source_name(src),
        "description": s.description,
        "chaos": layers_json(&g, &s.layers),
        "pins": pins_json(&s.pins),
    })))
}

/// **Private.** A scenario's pins as the entries a `PUT` accepts.
pub(crate) fn pins_json(pins: &kaanal_core::PinLayer) -> Vec<Value> {
    pins.iter()
        .map(|(k, e)| {
            let mut o = json!({ "endpoint": k, "state": e.state.name() });
            if let Some(s) = e.status {
                o["status"] = json!(s);
            }
            if let Some(d) = e.delay {
                o["delay"] = json!(humantime::format_duration(d).to_string());
            }
            o
        })
        .collect()
}

/// `PUT /__kaanal/scenarios/{name}` — defines or replaces a runtime scenario. The body is a
/// scenario file (`description?`, `chaos: [...]`) checked exactly like one: names to ids,
/// endpoints through `endpoint_key`. Replacing an active one re-compiles. Answers
/// `{ name, warnings }`.
///
/// # Errors
/// 400 for a bad name or entry, 404 for an unknown node or endpoint, 409 when the name is
/// defined in the config or a file.
pub async fn put_scenario(
    State(st): State<Arc<AppState>>,
    Gen(g): Gen,
    Path(name): Path<String>,
    payload: Result<Json<ScenarioConfig>, JsonRejection>,
) -> Result<Json<Value>, Reject> {
    valid_name(&name).map_err(|m| reject(StatusCode::BAD_REQUEST, &m))?;
    let Json(cfg) = payload.map_err(|e| reject(e.status(), &e.body_text()))?;
    let (scenario, warnings) =
        resolve_scenario(&name, &cfg, &g.topo, &g.endpoints, &|ep| g.empty_ok_for(ep)).map_err(
            |e| {
                let code = if e.not_found {
                    StatusCode::NOT_FOUND
                } else {
                    StatusCode::BAD_REQUEST
                };
                reject(code, &place(&e.path, &e.message))
            },
        )?;
    let mut c = lock_gen(&st, &g)?;
    let was_active = c.stack.active().contains(&name);
    let snap = commit(&st, &mut c, |c| {
        if let Some((_, src)) = c.scenarios.get(&name)
            && src != Source::Runtime
        {
            return Err(reject(
                StatusCode::CONFLICT,
                &format!(
                    "scenario '{name}' comes from the config or a file; it cannot be redefined at runtime"
                ),
            ));
        }
        c.scenarios
            .replace(scenario)
            .map_err(|m| reject(StatusCode::BAD_REQUEST, &m))
    })?;
    if was_active {
        // The effective chaos changed under a running stack.
        emit(&st, "update", Some(&name), &snap);
    }
    Ok(Json(json!({ "name": name, "warnings": warnings })))
}

/// `DELETE /__kaanal/scenarios/{name}` — undefines a runtime scenario (S19) and drops it from
/// the stack when active.
///
/// # Errors
/// 404 for an unknown name, 409 for an inline or file-defined one.
pub async fn delete_scenario(
    State(st): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<StatusCode, Reject> {
    let mut c = lock(&st);
    let was_active = c.stack.active().contains(&name);
    let snap = commit(&st, &mut c, |c| {
        match c.scenarios.get(&name) {
            None => return Err(unknown(&name)),
            Some((_, Source::Runtime)) => {}
            Some(_) => {
                return Err(reject(
                    StatusCode::CONFLICT,
                    &format!(
                        "scenario '{name}' comes from the config or a file and cannot be undefined at runtime"
                    ),
                ));
            }
        }
        c.scenarios
            .remove(&name)
            .map_err(|m| reject(StatusCode::CONFLICT, &m))?;
        c.stack.prune(&c.scenarios);
        Ok(())
    })?;
    if was_active {
        emit(&st, "deactivate", Some(&name), &snap);
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /__kaanal/scenarios/{name}/activate` — pushes (or moves) the scenario to the top of
/// the stack. Answers `{ active, expires_in_ms, effective }`.
///
/// An empty body is a plain activation (it also removes any lease the scenario had). A body
/// `{ "ttl": "30s" }` (100 ms to 24 h, humantime) leases it: when the time is up the scenario is
/// deactivated and an `expire` event is sent. Activating again replaces the lease.
///
/// # Errors
/// 400 for a bad body or ttl, 404 for an unknown name.
pub async fn activate(
    State(st): State<Arc<AppState>>,
    Gen(g): Gen,
    Path(name): Path<String>,
    body: Bytes,
) -> Result<Json<Value>, Reject> {
    let ttl = if body.is_empty() {
        None
    } else {
        let b: ActivateBody = serde_json::from_slice(&body)
            .map_err(|e| reject(StatusCode::BAD_REQUEST, &format!("bad body: {e}")))?;
        b.ttl.as_deref().map(parse_ttl).transpose()?
    };
    let mut c = lock_gen(&st, &g)?;
    let snap = commit(&st, &mut c, |c| {
        c.stack
            .activate(&c.scenarios, &name)
            .map_err(|_| unknown(&name))?;
        match ttl {
            Some(t) => c.leases.insert(name.clone(), Instant::now() + t),
            None => c.leases.remove(&name),
        };
        Ok(())
    })?;
    emit(&st, "activate", Some(&name), &snap);
    drop(c);
    st.lease_wake.notify_one();
    Ok(Json(active_json(&g, &snap)))
}

/// The accepted body of `POST /__kaanal/scenarios/{name}/activate`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ActivateBody {
    pub(super) ttl: Option<String>,
}

/// **Private.** A `ttl` string such as `30s`, within 100 ms to 24 h.
pub(super) fn parse_ttl(s: &str) -> Result<Duration, Reject> {
    humantime::parse_duration(s)
        .ok()
        .filter(|d| (Duration::from_millis(100)..=Duration::from_hours(24)).contains(d))
        .ok_or_else(|| {
            reject(
                StatusCode::BAD_REQUEST,
                "ttl must be 100ms to 24h, e.g. \"30s\"",
            )
        })
}

/// `POST /__kaanal/scenarios/{name}/deactivate` — removes it from the stack (204; also when it
/// was not active).
pub async fn deactivate(State(st): State<Arc<AppState>>, Path(name): Path<String>) -> StatusCode {
    let mut c = lock(&st);
    if !c.stack.active().contains(&name) {
        return StatusCode::NO_CONTENT;
    }
    if let Ok(snap) = commit(&st, &mut c, |c| {
        c.stack.deactivate(&name);
        Ok(())
    }) {
        emit(&st, "deactivate", Some(&name), &snap);
    }
    StatusCode::NO_CONTENT
}

/// `PUT /__kaanal/scenarios/active` — body `["a","b"]` sets the whole stack (bottom to top),
/// all or nothing. Answers like `activate`.
///
/// # Errors
/// 404 for an unknown name; nothing changes.
pub async fn set_active(
    State(st): State<Arc<AppState>>,
    Gen(g): Gen,
    payload: Result<Json<Vec<String>>, JsonRejection>,
) -> Result<Json<Value>, Reject> {
    let Json(names) = payload.map_err(|e| reject(e.status(), &e.body_text()))?;
    let mut c = lock_gen(&st, &g)?;
    let snap = commit(&st, &mut c, |c| {
        c.leases.clear();
        c.stack
            .set_active(&c.scenarios, &names)
            .map_err(|m| reject(StatusCode::NOT_FOUND, &m))
    })?;
    emit(&st, "set", None, &snap);
    Ok(Json(active_json(&g, &snap)))
}

/// `GET /__kaanal/scenarios/active` — the stack and the compiled result.
pub async fn get_active(State(st): State<Arc<AppState>>) -> Result<Json<Value>, Reject> {
    // the lock pairs the snapshot with the generation in force
    let (_c, g) = locked(&st)?;
    Ok(Json(active_json(&g, &st.snapshot())))
}

/// `DELETE /__kaanal/scenarios` — deactivates every scenario; the live layer stays (S12).
///
/// # Errors
/// Never in practice (the combined chaos was already valid).
pub async fn clear_scenarios(State(st): State<Arc<AppState>>) -> Result<StatusCode, Reject> {
    let mut c = lock(&st);
    let snap = commit(&st, &mut c, |c| {
        c.stack.clear_scenarios();
        Ok(())
    })?;
    emit(&st, "clear", None, &snap);
    Ok(StatusCode::NO_CONTENT)
}

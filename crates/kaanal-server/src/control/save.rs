//! `POST /scenarios/save`: turn live, effective or a named scenario into a scenario file.

use super::*;

/// Where `save` takes its entries from: `"live"`, `"effective"` or `{ "scenario": "<name>" }`.
/// (Tagged, so a scenario may itself be called `live` or `effective`.)
///
/// Strict: serde's own enum would also take `{"live": null}`; this takes exactly the three forms.
#[derive(Deserialize)]
#[serde(try_from = "Value")]
pub enum SaveFrom {
    /// The live layer.
    Live,
    /// The compiled result of the stack and the live layer.
    Effective,
    /// A defined scenario.
    Scenario(String),
}

impl TryFrom<Value> for SaveFrom {
    type Error = String;

    fn try_from(v: Value) -> Result<Self, String> {
        const HINT: &str = "from must be \"live\", \"effective\" or {\"scenario\": \"<name>\"}";
        match &v {
            Value::String(s) if s == "live" => Ok(SaveFrom::Live),
            Value::String(s) if s == "effective" => Ok(SaveFrom::Effective),
            Value::Object(o) if o.len() == 1 => match o.get("scenario") {
                Some(Value::String(n)) => Ok(SaveFrom::Scenario(n.clone())),
                _ => Err(HINT.into()),
            },
            _ => Err(HINT.into()),
        }
    }
}

/// The accepted body of `POST /__kaanal/scenarios/save`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveBody {
    pub(super) name: String,
    pub(super) from: Option<SaveFrom>,
    pub(super) force: Option<bool>,
    pub(super) clear_live: Option<bool>,
}

/// **Private.** A scenario file's text. JSON is YAML: one flow mapping per entry, strings
/// escaped by `serde_json`.
pub(super) fn scenario_yaml(description: Option<&str>, chaos: &[Value], pins: &[Value]) -> String {
    let mut yaml = String::new();
    if let Some(d) = description {
        let _ = writeln!(yaml, "description: {}", Value::String(d.to_string()));
    }
    for (head, list) in [("chaos", chaos), ("pins", pins)] {
        if list.is_empty() {
            continue;
        }
        let _ = writeln!(yaml, "{head}:");
        for e in list {
            let _ = writeln!(yaml, "  - {e}");
        }
    }
    yaml
}

/// **Private.** What a save writes: the chaos entries, the pin entries and a description.
pub(super) struct SaveSource {
    pub(super) chaos: Vec<Value>,
    pub(super) pins: Vec<Value>,
    pub(super) description: Option<String>,
}

/// **Private.** The [`SaveSource`] of a save's `from`. Runs under the control lock, so
/// `effective` is the latest published state.
pub(super) fn save_source(
    st: &AppState,
    g: &Generation,
    c: &MutexGuard<'_, Control>,
    from: SaveFrom,
) -> Result<SaveSource, Reject> {
    let (chaos, pins, description) = match from {
        SaveFrom::Live => {
            if let Some((k, _)) = c
                .stack
                .live()
                .entries()
                .into_iter()
                .find(|(_, l)| l.replace == Some(true))
            {
                return Err(reject(
                    StatusCode::BAD_REQUEST,
                    &format!(
                        "the live chaos on {} is an override, which a scenario file cannot hold; save from effective",
                        key_label(g, &k)
                    ),
                ));
            }
            (
                layers_json(g, c.stack.live()),
                pins_json(c.stack.live_pins()),
                None,
            )
        }
        // Under the lock, so this is the latest published state.
        SaveFrom::Effective => {
            let snap = st.snapshot();
            let mut won = kaanal_core::PinLayer::default();
            for (k, r) in snap.pins.iter() {
                won.insert(k.clone(), r.entry);
            }
            (effective_entries(g, &snap.chaos)?, pins_json(&won), None)
        }
        SaveFrom::Scenario(other) => match c.scenarios.get(&other) {
            None => return Err(unknown(&other)),
            Some((s, _)) => (
                layers_json(g, &s.layers),
                pins_json(&s.pins),
                s.description.clone(),
            ),
        },
    };
    Ok(SaveSource {
        chaos,
        pins,
        description,
    })
}

/// `POST /__kaanal/scenarios/save` — writes `scenarios/NAME.yaml` (S17) from the live layer
/// (default), the compiled `effective` result, or another scenario, and registers it as a file
/// scenario. Sparse: only the fields that were set. Answers `{ name, path, warnings }`; from
/// live or effective, `warnings` counts the leased fields the file makes permanent.
///
/// Limits: a live override cannot be saved (files are merge-only; use `from: effective`), and
/// `from: effective` refuses a key with two or more failure bands or a node@endpoint pair that
/// shares a node with node-level chaos (see `effective_entries`). `from` is `"live"` (default),
/// `"effective"` or `{ "scenario": "<name>" }`; any other `from` is a 422 from the extractor.
/// Saving over an active scenario emits an `update`
/// event. Directory use is check-then-use: the parent is verified just before the write, so a
/// directory swapped in that instant is not caught (the local user owns the directory anyway).
///
/// # Errors
/// 400 for a bad name, an inline name or nothing to save; 404 for an unknown `from`; 409 when
/// the scenario or its file exists and `force` is not set.
pub async fn save_scenario(
    State(st): State<Arc<AppState>>,
    Gen(g): Gen,
    payload: Result<Json<SaveBody>, JsonRejection>,
) -> Result<Json<Value>, Reject> {
    let Json(b) = payload.map_err(|e| reject(e.status(), &e.body_text()))?;
    valid_name(&b.name).map_err(|m| reject(StatusCode::BAD_REQUEST, &m))?;
    let force = b.force.unwrap_or(false);
    let mut c = lock_gen(&st, &g)?;
    match c.scenarios.get(&b.name) {
        Some((_, Source::Inline)) => {
            return Err(reject(
                StatusCode::BAD_REQUEST,
                &format!(
                    "scenario '{}' is defined in kaanal.yaml; pick another name",
                    b.name
                ),
            ));
        }
        Some(_) if !force => {
            return Err(reject(
                StatusCode::CONFLICT,
                &format!(
                    "scenario '{}' already exists (pass force to replace it)",
                    b.name
                ),
            ));
        }
        None if c.scenarios.names().len() >= MAX_SCENARIOS => {
            return Err(reject(
                StatusCode::BAD_REQUEST,
                &format!("at most {MAX_SCENARIOS} scenarios"),
            ));
        }
        _ => {}
    }
    let from = b.from.unwrap_or(SaveFrom::Live);
    // A file has no ttl: the leased live fields it writes are permanent once loaded (L5).
    // `effective` includes the live layer, so it carries the same leased fields.
    let leased = if matches!(from, SaveFrom::Live | SaveFrom::Effective) {
        chaos_lease::leased_fields(&c.chaos_leases, c.stack.live())
    } else {
        0
    };
    let warnings: Vec<String> = match leased {
        0 => vec![],
        1 => vec!["1 field has a ttl; the file has none".into()],
        n => vec![format!("{n} fields have a ttl; the file has none")],
    };
    let SaveSource {
        chaos: entries,
        pins,
        description,
    } = save_source(&st, &g, &c, from)?;
    if entries.is_empty() && pins.is_empty() {
        return Err(reject(StatusCode::BAD_REQUEST, "nothing to save"));
    }
    let yaml = scenario_yaml(description.as_deref(), &entries, &pins);
    // The memory copy comes from the same parse a reload would do, so disk and memory agree.
    let cfg = parse_scenario(&yaml).map_err(|e| reject(StatusCode::BAD_REQUEST, &e.to_string()))?;
    let (scenario, _) = resolve_scenario(&b.name, &cfg, &g.topo, &g.endpoints, &|ep| {
        g.empty_ok_for(ep)
    })
    .map_err(|e| reject(StatusCode::BAD_REQUEST, &place(&e.path, &e.message)))?;
    // Blocking file IO under the control mutex: a save is rare and tiny.
    let path = scenarios::write(&g.scenarios_dir, &b.name, &yaml, force).map_err(|e| match e {
        WriteError::Exists => reject(
            StatusCode::CONFLICT,
            &format!(
                "scenarios/{}.yaml already exists (pass force to replace it)",
                b.name
            ),
        ),
        WriteError::Other(m) => reject(StatusCode::INTERNAL_SERVER_ERROR, &m),
    })?;
    let was_active = c.stack.active().contains(&b.name);
    let snap = commit(&st, &mut c, |c| {
        c.scenarios
            .set_file(scenario)
            .map_err(|m| reject(StatusCode::BAD_REQUEST, &m))?;
        if b.clear_live.unwrap_or(false) {
            c.stack.live_mut().clear();
            c.stack.live_pins_mut().clear();
        }
        Ok(())
    })?;
    if was_active {
        emit(&st, "update", Some(&b.name), &snap);
    }
    Ok(Json(
        json!({ "name": b.name, "path": path.display().to_string(), "warnings": warnings }),
    ))
}

/// **Private.** The data of a `scenario` SSE message: `{ action, name, active }`, plus
/// `chaos_key` only when set (a live chaos expiry).
pub(super) fn scenario_data(
    action: &str,
    name: Option<&str>,
    chaos_key: Option<&Value>,
    active: &[String],
) -> Value {
    let mut v = json!({ "action": action, "name": name, "active": active });
    if let Some(k) = chaos_key {
        v["chaos_key"] = k.clone();
    }
    v
}

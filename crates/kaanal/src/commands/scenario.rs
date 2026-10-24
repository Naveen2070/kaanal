//! `kaanal scenario` and `kaanal endpoints`: list, activate, save and print scenarios, and write missing endpoints.

use crate::{
    DEFAULT_ENDPOINTS_FILE, EndpointsConfig, Fail, Path, PathBuf, SKELETON_HEADER, ScenarioCmd,
    ScenarioConfig, Value, active_suffix, call, clean, clean_lines, config_err, encode_segment,
    err, json, load, located_err, output, parse_endpoints, parse_scenario, pin_extra, skeleton,
    ttl_body, valid_name, validate_scenario_shape, warn_all,
};

/// **Private.** `/__kaanal/scenarios/NAME` plus `tail`.
pub(crate) fn scenario_path(name: &str, tail: &str) -> String {
    format!("/__kaanal/scenarios/{}{tail}", encode_segment(name))
}

/// **Private.** The `active` names of a stack answer.
pub(crate) fn active_names(v: &Value) -> Vec<&str> {
    v["active"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}

/// **Private.** The PUT body for a parsed scenario file.
pub(crate) fn scenario_body(cfg: &ScenarioConfig) -> Value {
    let mut body = json!({ "chaos": cfg.chaos, "pins": cfg.pins });
    if let Some(d) = &cfg.description {
        body["description"] = json!(d);
    }
    body
}

/// **Private.** A scenario name checked here, so a reserved one (`active`, `save`, `reset`) is
/// refused with a message instead of hitting another route.
pub(crate) fn named(name: &str) -> Result<&str, Fail> {
    valid_name(name).map_err(Fail::usage)?;
    Ok(name)
}

/// **Private.** `--from` as the API's tagged form: `live`, `effective` or `scenario:NAME`.
pub(crate) fn save_from(from: &str) -> Result<Value, Fail> {
    match from {
        "live" | "effective" => Ok(json!(from)),
        f => match f.strip_prefix("scenario:") {
            Some(n) if !n.is_empty() => Ok(json!({ "scenario": n })),
            _ => Err(Fail::usage(format!(
                "bad --from '{}': use live, effective or scenario:NAME",
                clean(f)
            ))),
        },
    }
}

/// **Private.** The `scenario show` text of one definition.
pub(crate) fn print_scenario(v: &Value) {
    let head = format!(
        "{} ({}){}",
        v["name"].as_str().unwrap_or(""),
        v["source"].as_str().unwrap_or(""),
        v["description"]
            .as_str()
            .map_or(String::new(), |d| format!(": {d}"))
    );
    println!("{}", clean(&head));
    for e in v["chaos"].as_array().into_iter().flatten() {
        println!("  - {}", clean(&e.to_string()));
    }
    for p in v["pins"].as_array().into_iter().flatten() {
        let extra = pin_extra(&p["status"], p["delay"].as_str().map(str::to_string));
        let line = format!(
            "  pin {}: {}{extra}",
            p["endpoint"].as_str().unwrap_or(""),
            p["state"].as_str().unwrap_or("")
        );
        println!("{}", clean(&line));
    }
}

/// **Private.** `prefix` + `active scenarios: a, b` (or the stack answer under `--json`).
pub(crate) fn say_active(v: &Value, prefix: &str) {
    output::emit(v, || {
        println!("{prefix}active scenarios: {}", active_names(v).join(", "));
    });
}

/// **Private.** `kaanal scenario ...`: thin calls to the control API, except `load`, which
/// parses the file here so a mistake shows as `file:line:col`.
pub(crate) fn scenario(cmd: ScenarioCmd) -> Result<(), Fail> {
    match cmd {
        ScenarioCmd::List => {
            let v = call("GET", "/__kaanal/scenarios", None)?;
            output::emit(&v, || {
                println!(
                    "{:<24} {:<8} {:<7} DESCRIPTION",
                    "NAME", "SOURCE", "ENTRIES"
                );
                for s in v.as_array().into_iter().flatten() {
                    let line = format!(
                        "{:<24} {:<8} {:<7} {}{}",
                        s["name"].as_str().unwrap_or(""),
                        s["source"].as_str().unwrap_or(""),
                        s["entries"],
                        s["description"].as_str().unwrap_or(""),
                        active_suffix(s)
                    );
                    println!("{}", clean(&line));
                }
            });
        }
        ScenarioCmd::Show { name } => {
            let v = call("GET", &scenario_path(named(&name)?, ""), None)?;
            output::emit(&v, || print_scenario(&v));
        }
        ScenarioCmd::Activate { name, ttl } => {
            let body = ttl_body("ttl", ttl)?;
            let v = call("POST", &scenario_path(named(&name)?, "/activate"), body)?;
            say_active(&v, "");
        }
        ScenarioCmd::Deactivate { name } => {
            let name = named(&name)?;
            // ponytail: the answer is a bare 204, so "was it active" is read just before; a
            // concurrent change in between can make this line wrong, never the state.
            let was = call("GET", "/__kaanal/scenarios/active", None)?;
            let v = call("POST", &scenario_path(name, "/deactivate"), None)?;
            output::emit(&v, || {
                if active_names(&was).contains(&name) {
                    println!("deactivated {name}");
                } else {
                    println!("ok: {name} is not active");
                }
            });
        }
        ScenarioCmd::Set { names } => {
            names.iter().try_for_each(|n| named(n).map(drop))?;
            let v = call("PUT", "/__kaanal/scenarios/active", Some(json!(names)))?;
            say_active(&v, "");
        }
        ScenarioCmd::Clear => {
            let v = call("DELETE", "/__kaanal/scenarios", None)?;
            output::emit(&v, || println!("cleared"));
        }
        ScenarioCmd::Active => {
            let v = call("GET", "/__kaanal/scenarios/active", None)?;
            output::emit(&v, || {
                for n in active_names(&v) {
                    println!("{}", clean(n));
                }
            });
        }
        ScenarioCmd::Load { file } => {
            let name = file
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(|| Fail::usage(format!("{}: no usable file name", file.display())))?;
            valid_name(name).map_err(|m| Fail::usage(format!("{}: {m}", file.display())))?;
            let src = std::fs::read_to_string(&file)
                .map_err(|e| err(format!("{}: {e}", file.display())))?;
            let cfg = parse_scenario(&src).map_err(|e| config_err(&file, &e))?;
            // Everything that needs no topology is checked here, placed in the file; the server
            // checks names against the topology.
            validate_scenario_shape(&cfg)
                .map_err(|e| config_err(&file, &e.located(Vec::new()).in_source(&src)))?;
            let body = scenario_body(&cfg);
            let v = call("PUT", &scenario_path(name, ""), Some(body))?;
            warn_all(&v);
            let v = call("POST", &scenario_path(name, "/activate"), None)?;
            say_active(&v, &format!("loaded {name}; "));
        }
        ScenarioCmd::Save {
            name,
            from,
            force,
            clear_live,
        } => {
            named(&name)?;
            let from = save_from(&from)?;
            let body =
                json!({ "name": name, "from": from, "force": force, "clear_live": clear_live });
            let v = call("POST", "/__kaanal/scenarios/save", Some(body))?;
            output::emit(&v, || {
                println!("saved {}", clean(v["path"].as_str().unwrap_or("")));
            });
            warn_all(&v);
        }
    }
    Ok(())
}

/// **Private.** `kaanal endpoints [--write]` (E3, D22): skeleton entries for spec routes
/// missing from `endpoints:`. Add-only: it appends to an external file (refusing if the
/// result would not parse with every old key), creates `kaanal.endpoints.yaml` when there is
/// no `endpoints:` key, and only prints for an inline map (R1.17: kaanal.yaml is never rewritten).
pub(crate) fn endpoints(config: &Path, write: bool) -> Result<(), Fail> {
    let (cfg, dir, src) = load(config)?;
    let (topo, routes) =
        kaanal_server::spec_routes(&cfg, &dir).map_err(|e| located_err(config, &src, &e))?;
    let (file, old, keys): (Option<PathBuf>, String, Vec<String>) = match &cfg.endpoints {
        Some(EndpointsConfig::Inline(m)) => (None, String::new(), m.keys().cloned().collect()),
        Some(EndpointsConfig::File(p)) => {
            let f = dir.join(p);
            let old = match std::fs::read_to_string(&f) {
                Ok(t) => t,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
                Err(e) => return Err(err(format!("{}: {e}", f.display()))),
            };
            let keys = parse_endpoints(&old)
                .map_err(|e| config_err(&f, &e))?
                .into_keys()
                .collect();
            (Some(f), old, keys)
        }
        None => (None, String::new(), Vec::new()),
    };
    let lines = skeleton(&topo, &routes, &keys);
    if lines.is_empty() {
        println!("nothing to add: every spec route has an entry");
        return Ok(());
    }
    let block = |nl: &str| {
        std::iter::once(SKELETON_HEADER)
            .chain(lines.iter().map(String::as_str))
            .fold(String::new(), |mut s, l| {
                s.push_str(l);
                s.push_str(nl);
                s
            })
    };
    // Spec paths and node names are user text: never let them drive the terminal.
    // Under an inline map the lines are pasted below `endpoints:`, so they need its indent.
    let indent = if matches!(cfg.endpoints, Some(EndpointsConfig::Inline(_))) {
        "  "
    } else {
        ""
    };
    let show = || {
        for l in clean_lines(&block("\n")).lines() {
            println!("{indent}{l}");
        }
    };
    match (write, &cfg.endpoints, file) {
        (false, ..) => show(),
        (true, Some(EndpointsConfig::Inline(_)), _) => {
            show();
            eprintln!(
                "note: endpoints: is inline in {}; paste these lines under it (kaanal never rewrites it)",
                config.display()
            );
        }
        (true, _, Some(f)) => {
            let nl = if old.contains("\r\n") { "\r\n" } else { "\n" };
            let mut text = old.clone();
            if !text.is_empty() && !text.ends_with('\n') {
                text.push_str(nl);
            }
            text.push_str(&block(nl));
            // Safety net (D22): the appended file must parse and keep every old entry.
            let back = parse_endpoints(&text).map_err(|e| {
                err(format!(
                    "{}: refusing to append, the result would not parse: {e} (a file that is one flow-style `{{ ... }}` map cannot be appended to: rewrite it in block style, or paste the lines in yourself)",
                    f.display()
                ))
            })?;
            if let Some(k) = keys.iter().find(|k| !back.contains_key(*k)) {
                return Err(err(format!(
                    "{}: refusing to append, entry '{k}' would be lost",
                    f.display()
                )));
            }
            std::fs::write(&f, text).map_err(|e| err(format!("{}: {e}", f.display())))?;
            println!("appended {} entries to {}", lines.len(), f.display());
        }
        (true, _, None) => {
            let f = dir.join(DEFAULT_ENDPOINTS_FILE);
            if f.exists() {
                return Err(err(format!(
                    "{} exists but {} has no endpoints: key; add `endpoints: {DEFAULT_ENDPOINTS_FILE}` to it, or move the file away",
                    f.display(),
                    config.display()
                )));
            }
            std::fs::write(&f, block("\n")).map_err(|e| err(format!("{}: {e}", f.display())))?;
            println!(
                "wrote {} entries to {}; add this line to {}: endpoints: {DEFAULT_ENDPOINTS_FILE}",
                lines.len(),
                f.display(),
                config.display()
            );
        }
    }
    Ok(())
}

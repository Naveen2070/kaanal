//! `kaanal draft`: show, replace, validate and apply the running instance's draft (the text of
//! its `kaanal.yaml`, kept apart from the running system until Apply).

use crate::{
    ConfigError, DraftCmd, Duration, Fail, Path, PathBuf, Value, call, clean, config_err, err,
    json, output,
};

/// The longest drain the server accepts, in seconds.
const MAX_TIMEOUT_S: u64 = 60;

/// **Private.** `kaanal draft ...`.
pub(crate) fn draft(cmd: DraftCmd) -> Result<(), Fail> {
    match cmd {
        DraftCmd::Show => show(),
        DraftCmd::Set { file } => set(&file),
        DraftCmd::Validate { file } => validate(file.as_deref()),
        DraftCmd::Apply { timeout } => apply(&timeout),
        DraftCmd::Save {
            confirm_loss,
            fix_scenarios,
        } => save(confirm_loss, fix_scenarios),
    }
}

/// **Private.** `draft show`: the draft text, as it is stored.
fn show() -> Result<(), Fail> {
    let v = call("GET", "/__kaanal/draft", None)?;
    output::emit(&v, || {
        crate::out_bytes(v["text"].as_str().unwrap_or("").as_bytes());
    });
    Ok(())
}

/// **Private.** The text of `file`, or a usage-free error naming it.
fn read(file: &Path) -> Result<String, Fail> {
    std::fs::read_to_string(file).map_err(|e| err(format!("{}: {e}", file.display())))
}

/// **Private.** `draft set -f FILE`: replaces the draft with the file's text (nothing is applied).
fn set(file: &Path) -> Result<(), Fail> {
    let text = read(file)?;
    let v = call("PUT", "/__kaanal/draft", Some(json!({ "text": text })))?;
    output::emit(&v, || {
        println!(
            "draft replaced from {} ({} bytes); `kaanal draft apply` makes it the running system",
            file.display(),
            text.len()
        );
    });
    Ok(())
}

/// **Private.** The first diagnostic of a validate answer as `file:line:col: message`, `label`
/// naming the draft or the file that was checked.
fn first_problem(v: &Value, label: &Path) -> Fail {
    let d = &v["diagnostics"][0];
    let pos = |k: &str| d[k].as_u64().and_then(|n| usize::try_from(n).ok());
    let e = ConfigError {
        line: pos("line"),
        col: pos("col"),
        msg: clean(d["message"].as_str().unwrap_or("the draft does not build")),
    };
    // a problem in another file (a spec, the endpoints file) names that file
    let file = d["file"].as_str().map(|f| PathBuf::from(clean(f)));
    config_err(file.as_deref().unwrap_or(label), &e)
}

/// **Private.** Prints the scenario files an answer says were left out, one line each.
fn print_flagged(v: &Value) {
    for f in v["flagged"].as_array().into_iter().flatten() {
        println!(
            "flagged: {}: scenario '{}': {}",
            clean(f["file"].as_str().unwrap_or("")),
            clean(f["scenario"].as_str().unwrap_or("")),
            clean(f["message"].as_str().unwrap_or(""))
        );
    }
}

/// **Private.** Validates the stored draft, or `file`'s text without storing it. A problem is
/// printed as `file:line:col: message` (`draft` for the stored one) and exits 1.
fn check(file: Option<&Path>) -> Result<Value, Fail> {
    let body = file
        .map(|f| read(f).map(|t| json!({ "text": t })))
        .transpose()?;
    let v = call("POST", "/__kaanal/draft/validate", body)?;
    if v["ok"] == true {
        return Ok(v);
    }
    output::emit(&v, || {});
    Err(first_problem(&v, file.unwrap_or(Path::new("draft"))))
}

/// **Private.** `draft validate [--file F]`.
fn validate(file: Option<&Path>) -> Result<(), Fail> {
    let v = check(file)?;
    crate::warn_all(&v);
    output::emit(&v, || {
        println!("ok");
        print_flagged(&v);
    });
    Ok(())
}

/// **Private.** `draft apply [--timeout T]`: checks the draft first (so a problem is shown with
/// its position), then makes it the running system.
fn apply(timeout: &str) -> Result<(), Fail> {
    let wait = humantime::parse_duration(timeout).map_err(|_| {
        Fail::usage(format!(
            "bad --timeout '{}' (e.g. 10s, at most {MAX_TIMEOUT_S}s)",
            clean(timeout)
        ))
    })?;
    if wait > Duration::from_secs(MAX_TIMEOUT_S) {
        return Err(Fail::usage(format!(
            "--timeout is at most {MAX_TIMEOUT_S}s (the drain waits for requests in flight)"
        )));
    }
    let checked = check(None)?;
    crate::warn_all(&checked);
    let ms = u64::try_from(wait.as_millis()).unwrap_or(u64::MAX);
    // the drain can take the whole budget, so the call waits a little longer than it
    let v = crate::call_for(
        "POST",
        "/__kaanal/draft/apply",
        Some(json!({ "timeout_ms": ms })),
        wait + Duration::from_secs(15),
    )?;
    output::emit(&v, || print_applied(&v));
    Ok(())
}

/// **Private.** `draft save`: writes the draft to the config file. A refusal (the file changed on
/// disk, edits that lost something) is the server's message and exits 1.
fn save(confirm_loss: bool, fix: Option<crate::cli::FixScenarios>) -> Result<(), Fail> {
    let fix = match fix {
        None => "none",
        Some(crate::cli::FixScenarios::Remove) => "remove",
        Some(crate::cli::FixScenarios::Rewrite) => "rewrite",
    };
    let v = call(
        "POST",
        "/__kaanal/draft/save",
        Some(json!({ "confirm_loss": confirm_loss, "fix_scenarios": fix })),
    )?;
    output::emit(&v, || {
        let list = |k: &str| -> Vec<String> {
            v[k].as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(clean)
                .collect()
        };
        println!("saved: {}", list("written").join(", "));
        let removed = list("removed");
        if !removed.is_empty() {
            println!("removed: {}", removed.join(", "));
        }
    });
    Ok(())
}

/// **Private.** The text of an apply answer: the generation, what was kept and dropped, and the
/// scenario files that were left out.
fn print_applied(v: &Value) {
    println!("applied generation {}", v["generation"]);
    let kept = &v["kept"];
    let names = |x: &Value| -> Vec<String> {
        x.as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(clean)
            .collect()
    };
    let scenarios = names(&kept["active_scenarios"]);
    let runtime = names(&kept["runtime_scenarios"]);
    println!(
        "kept: {} live chaos, {} pins, scenarios: {}{}",
        kept["live_chaos"],
        kept["pins"],
        if scenarios.is_empty() {
            "none".to_string()
        } else {
            scenarios.join(", ")
        },
        if runtime.is_empty() {
            String::new()
        } else {
            format!(", runtime scenarios: {}", runtime.join(", "))
        }
    );
    let d = &v["dropped"];
    let mut dropped = names(&d["live_chaos"]);
    dropped.extend(
        names(&d["active_scenarios"])
            .into_iter()
            .map(|n| format!("scenario {n}")),
    );
    dropped.extend(names(&d["pins"]).into_iter().map(|k| format!("pin {k}")));
    dropped.extend(names(&d["scenario_chaos"]));
    if !dropped.is_empty() {
        println!("dropped: {}", dropped.join("; "));
    }
    print_flagged(v);
}

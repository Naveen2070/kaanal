//! `kaanal sim`, `kaanal sample` and `kaanal skeleton`: thin clients of the simulation routes
//! (`docs/web/simulation.md`). All the logic is the server's; this reads the answers aloud.

mod print;

use crate::cli::SimCmd;
use crate::{Duration, Fail, Path, Value, call, call_text, clean, err, json, output};
use print::{print_run, print_sweep};

/// **Private.** How long `sim run` waits for a report before it says to look again later.
const WAIT: Duration = Duration::from_secs(300);

/// **Private.** `kaanal sim ...`.
pub(crate) fn sim(cmd: SimCmd) -> Result<(), Fail> {
    match cmd {
        SimCmd::Run {
            profile,
            file,
            duration,
            warmup,
            seed,
            scenario,
            no_wait,
        } => run(
            profile.as_deref(),
            file.as_deref(),
            duration.as_deref(),
            warmup.as_deref(),
            seed,
            scenario,
            no_wait,
        ),
        SimCmd::Sweep {
            profile,
            file,
            p99,
            errors,
            max_factor,
            seed,
            scenario,
            no_wait,
            report,
            force,
        } => {
            // a report file that exists is refused now, not after the sweep has run
            if let Some(f) = report.as_deref().filter(|f| !force && f.exists()) {
                return Err(err(format!(
                    "{} exists; --force overwrites it",
                    clean(&f.display().to_string())
                )));
            }
            // the limits first: a bad one is a usage error before anything is read or sent
            let body = limits(p99.as_deref(), errors.as_deref(), max_factor)?;
            sweep(
                body,
                profile.as_deref(),
                file.as_deref(),
                seed,
                scenario,
                no_wait,
                report.as_deref().map(|f| (f, force)),
            )
        }
        SimCmd::List => list(),
        SimCmd::Show { id } => show(&id),
        SimCmd::Report { id, output, force } => report_cmd(&id, output.as_deref(), force),
    }
}

/// **Private.** A duration flag as seconds; `eg` names examples in the usage error.
fn secs(flag: &str, text: &str, eg: &str) -> Result<f64, Fail> {
    humantime::parse_duration(text)
        .map(|d| d.as_secs_f64())
        .map_err(|_| Fail::usage(format!("bad {flag} '{}' (e.g. {eg})", clean(text))))
}

/// **Private.** The `profile` of a body from `-p` or `-f`, and whether none was named.
fn profile_of(profile: Option<&str>, file: Option<&Path>) -> Result<(Value, bool), Fail> {
    let profile: Value = match (profile, file) {
        (Some(n), None) => json!(n),
        (None, Some(f)) => {
            let text = std::fs::read_to_string(f)
                .map_err(|e| err(format!("{}: {e}", clean(&f.display().to_string()))))?;
            serde_yaml_ng::from_str(&text).map_err(|e| {
                err(format!(
                    "{}: not a profile: {}",
                    clean(&f.display().to_string()),
                    clean(&e.to_string())
                ))
            })?
        }
        // none given: an empty name, so the server says what the project has (RULES 58)
        _ => json!(""),
    };
    let unnamed = profile == json!("");
    Ok((profile, unnamed))
}

/// **Private.** Posts `body` (with `seed` and `scenario` when given) to `path` and, unless told
/// not to, waits for the record and prints it with `print`.
fn start(
    path: &str,
    mut body: Value,
    unnamed: bool,
    seed: Option<u64>,
    scenario: Option<String>,
    no_wait: bool,
    print: fn(&Value),
) -> Result<String, Fail> {
    if let Some(s) = seed {
        body["seed"] = json!(s);
    }
    if let Some(s) = scenario {
        body["scenario"] = json!(s);
    }
    let started = call("POST", path, Some(body)).map_err(|f| match f {
        // still a usage error (64), as when the CLI refused it before asking: only the text is new
        Fail(1, m) if unnamed && m.contains("HTTP 404: ") => {
            Fail::usage(m.split_once("HTTP 404: ").map_or(m.as_str(), |(_, t)| t))
        }
        f => f,
    })?;
    let id = started["id"].as_str().unwrap_or_default().to_string();
    if no_wait {
        output::emit(&started, || println!("{}", clean(&id)));
        return Ok(id);
    }
    let v = wait(&id)?;
    // a failed run is reported by the error below, not twice
    output::emit(&v, || {
        if v["error"].is_null() {
            print(&v);
        }
    });
    match v["error"].as_str() {
        Some(e) => Err(err(format!(
            "{} {}: {}",
            clean(v["kind"].as_str().unwrap_or("run")),
            clean(&id),
            clean(e)
        ))),
        None => Ok(id),
    }
}

/// **Private.** `sim run`: starts a run and, unless told not to, waits for its report.
fn run(
    profile: Option<&str>,
    file: Option<&Path>,
    duration: Option<&str>,
    warmup: Option<&str>,
    seed: Option<u64>,
    scenario: Option<String>,
    no_wait: bool,
) -> Result<(), Fail> {
    let (profile, unnamed) = profile_of(profile, file)?;
    let mut body = json!({ "profile": profile });
    if let Some(d) = duration {
        body["duration_s"] = json!(secs("--duration", d, "60s, 2m")?);
    }
    if let Some(w) = warmup {
        body["warmup_s"] = json!(secs("--warmup", w, "5s, 1m")?);
    }
    start(
        "/__kaanal/sim/run",
        body,
        unnamed,
        seed,
        scenario,
        no_wait,
        print_run,
    )
    .map(|_| ())
}

/// **Private.** The limit of `sim sweep` as body fields: `--p99` a duration above zero, sent as
/// `p99_ms`; `--errors` a percentage, `1%` or `1`, 0 to 100, sent as `error_pct`. The server
/// checks `max_factor`.
fn limits(p99: Option<&str>, errors: Option<&str>, max_factor: Option<f64>) -> Result<Value, Fail> {
    let mut body = json!({});
    if let Some(p) = p99 {
        let s = secs("--p99", p, "500ms, 1s")?;
        if s <= 0.0 {
            return Err(Fail::usage(format!(
                "bad --p99 '{}' (above zero, e.g. 500ms, 1s)",
                clean(p)
            )));
        }
        body["p99_ms"] = json!(s * 1000.0);
    }
    if let Some(e) = errors {
        let t = e.trim();
        let pct = t
            .strip_suffix('%')
            .unwrap_or(t)
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|x| (0.0..=100.0).contains(x))
            .ok_or_else(|| {
                Fail::usage(format!(
                    "bad --errors '{}' (a percentage, 0 to 100, e.g. 1%)",
                    clean(e)
                ))
            })?;
        body["error_pct"] = json!(pct);
    }
    if let Some(m) = max_factor {
        // JSON has no NaN or infinity: `json!` would turn one into null and the server would use its default
        if !m.is_finite() {
            return Err(Fail::usage(format!(
                "bad --max-factor '{m}' (a number, 1 to 64)"
            )));
        }
        body["max_factor"] = json!(m);
    }
    Ok(body)
}

/// **Private.** `sim sweep`: starts a load sweep with the limit fields `body` and, unless told
/// not to, waits for its finding. A sweep has no log, so none is fetched.
fn sweep(
    mut body: Value,
    profile: Option<&str>,
    file: Option<&Path>,
    seed: Option<u64>,
    scenario: Option<String>,
    no_wait: bool,
    report: Option<(&Path, bool)>,
) -> Result<(), Fail> {
    let (profile, unnamed) = profile_of(profile, file)?;
    body["profile"] = profile;
    let id = start(
        "/__kaanal/sim/sweep",
        body,
        unnamed,
        seed,
        scenario,
        no_wait,
        print_sweep,
    )?;
    // a failed sweep returned its error above: only a finished one has a report
    if let Some((file, force)) = report {
        let text = report_text(&id, false)?;
        write_file(file, &text, force)?;
        eprintln!("wrote {}", clean(&file.display().to_string()));
    }
    Ok(())
}

/// **Private.** The report of sweep `id`, cleaned of control text, as Markdown or (`json`) the
/// data block alone; ends in one newline.
fn report_text(id: &str, json: bool) -> Result<String, Fail> {
    let q = if json { "?format=json" } else { "" };
    let text = call_text(&format!(
        "/__kaanal/sim/runs/{}/report{q}",
        crate::encode_segment(id)
    ))?;
    // an unreadable or empty body must not become a blank report file
    if text.trim().is_empty() {
        return Err(err("the server sent no report text"));
    }
    Ok(crate::clean_lines(&text) + "\n")
}

/// **Private.** `sim report ID [-o FILE] [--force]`: the server's report, printed or written.
/// `--json` is the data alone (`report_version`, the points, the re-run recipe).
fn report_cmd(id: &str, output_file: Option<&Path>, force: bool) -> Result<(), Fail> {
    let text = report_text(id, output::json_on())?;
    let Some(file) = output_file else {
        crate::out_bytes(text.as_bytes());
        return Ok(());
    };
    write_file(file, &text, force)?;
    let shown = clean(&file.display().to_string());
    output::emit(&json!({ "written": shown }), || println!("wrote {shown}"));
    Ok(())
}

/// **Private.** Polls run `id` until it is no longer running.
fn wait(id: &str) -> Result<Value, Fail> {
    let end = std::time::Instant::now() + WAIT;
    loop {
        let v = call(
            "GET",
            &format!("/__kaanal/sim/runs/{}", crate::encode_segment(id)),
            None,
        )?;
        if v["state"] != "running" {
            return Ok(v);
        }
        if std::time::Instant::now() > end {
            return Err(err(format!(
                "{} is still going; look again with `kaanal sim show {}`",
                clean(id),
                clean(id)
            )));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// **Private.** `sim list`.
fn list() -> Result<(), Fail> {
    let v = call("GET", "/__kaanal/sim/runs", None)?;
    output::emit(&v, || {
        let runs = v["runs"].as_array().map_or(&[][..], Vec::as_slice);
        if runs.is_empty() {
            println!("no runs");
        }
        for r in runs {
            println!(
                "{:<5} {:<6} {:<8} generation {} profile {}{}",
                clean(r["id"].as_str().unwrap_or("")),
                clean(r["kind"].as_str().unwrap_or("run")),
                clean(r["state"].as_str().unwrap_or("")),
                r["generation"],
                clean(r["profile"].as_str().unwrap_or("")),
                r["scenario"]
                    .as_str()
                    .map_or(String::new(), |s| format!(" scenario {}", clean(s)))
            );
        }
    });
    Ok(())
}

/// **Private.** `sim show ID`.
fn show(id: &str) -> Result<(), Fail> {
    let v = call(
        "GET",
        &format!("/__kaanal/sim/runs/{}", crate::encode_segment(id)),
        None,
    )?;
    output::emit(&v, || {
        if v["kind"] == "sweep" {
            print_sweep(&v);
        } else {
            print_run(&v);
        }
    });
    Ok(())
}

/// **Private.** `kaanal sample ROUTE`. Text mode prints the body to stdout, so it can be piped,
/// and the size and status to stderr; `--json` prints the whole answer.
pub(crate) fn sample(endpoint: &str, seed: Option<u64>) -> Result<(), Fail> {
    let mut body = json!({ "endpoint": endpoint });
    if let Some(s) = seed {
        body["seed"] = json!(s);
    }
    let v = call("POST", "/__kaanal/sample", Some(body))?;
    output::emit(&v, || {
        let pretty = serde_json::to_string_pretty(&v["body"]).unwrap_or_default();
        crate::out_line(&pretty);
        eprintln!("{} bytes, status {}", v["bytes"], v["status"]);
    });
    Ok(())
}

/// **Private.** `kaanal skeleton [-o FILE] [--force]`.
pub(crate) fn skeleton_cmd(output_file: Option<&Path>, force: bool) -> Result<(), Fail> {
    let v = call("GET", "/__kaanal/openapi/skeleton", None)?;
    // an answer without text (an unreadable body) must not become an empty file
    let text = v["text"]
        .as_str()
        .filter(|t| !t.is_empty())
        .ok_or_else(|| err("the server sent no skeleton text"))?;
    let Some(file) = output_file else {
        output::emit(&v, || crate::out_bytes(text.as_bytes()));
        return Ok(());
    };
    let shown = clean(&file.display().to_string());
    write_file(file, text, force)?;
    output::emit(
        &json!({ "written": shown, "endpoints": v["endpoints"] }),
        || {
            println!("wrote {shown} ({} endpoints)", v["endpoints"]);
        },
    );
    Ok(())
}

/// **Private.** The `N` of the next temp name [`replace_file`] tries; shared by every write of
/// this process, so two writes never try the same name.
static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// **Private.** Writes `text` to a new temp file beside `file` and renames it over `file`, so a failed
/// write leaves the old file whole. The temp is made with `create_new` (`O_CREAT | O_EXCL`: an
/// existing path, a symlink included, is never opened, so nothing planted there is followed or
/// clobbered) under `FILE.PID.N.kaanal-tmp`, a clash retried with the next `N`; on any failure it
/// is removed. The rename replaces a symlink at `file` itself, not what it points to.
fn replace_file(file: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::sync::atomic::Ordering;
    let mut clash = None;
    for _ in 0..16 {
        let mut tmp = file.as_os_str().to_owned();
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        tmp.push(format!(".{}.{n}.kaanal-tmp", std::process::id()));
        let tmp = std::path::PathBuf::from(tmp);
        let mut f = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
        {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                clash = Some(e);
                continue;
            }
            Err(e) => return Err(e),
        };
        let r = f.write_all(text.as_bytes()).and_then(|()| f.sync_all());
        drop(f);
        let r = r.and_then(|()| std::fs::rename(&tmp, file));
        if r.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        return r;
    }
    Err(clash.unwrap_or_else(|| std::io::Error::other("no free temp name")))
}

/// **Private.** Writes `text` to `file`: an existing file is refused unless `force`.
fn write_file(file: &Path, text: &str, force: bool) -> Result<(), Fail> {
    let shown = clean(&file.display().to_string());
    if force {
        return replace_file(file, text).map_err(|e| err(format!("{shown}: {e}")));
    }
    // create_new: an existing file is an error here, not a race with a check
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(file)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                err(format!("{shown} exists; --force overwrites it"))
            } else {
                err(format!("{shown}: {e}"))
            }
        })?;
    if let Err(e) = std::io::Write::write_all(&mut f, text.as_bytes()) {
        drop(f);
        // the file is ours (create_new): do not leave half of it behind
        let _ = std::fs::remove_file(file);
        return Err(err(format!("{shown}: {e}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh, empty directory for one test.
    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("kaanal-write-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Temp siblings left in `d`.
    fn temps(d: &Path) -> Vec<String> {
        std::fs::read_dir(d)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".kaanal-tmp"))
            .collect()
    }

    /// Held by every test that writes with `--force`, so none moves [`NEXT`] while another reads it.
    static FORCE: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// The temp names the next `k` attempts will try for `file`, read from [`NEXT`] (hold [`FORCE`]).
    fn next_names(file: &Path, k: u32) -> Vec<std::path::PathBuf> {
        let n = NEXT.load(std::sync::atomic::Ordering::Relaxed);
        (n..n + k)
            .map(|i| {
                let mut p = file.as_os_str().to_owned();
                p.push(format!(".{}.{i}.kaanal-tmp", std::process::id()));
                std::path::PathBuf::from(p)
            })
            .collect()
    }

    #[test]
    fn force_replaces_the_target_and_leaves_a_planted_temp_name_alone() {
        let _held = FORCE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let d = dir("planted");
        let file = d.join("r.md");
        std::fs::write(&file, "old").unwrap();
        // the old fixed temp name, and the exact names the next four attempts will use, planted
        let mut planted = vec![d.join("r.md.kaanal-tmp")];
        planted.extend(next_names(&file, 4));
        for p in &planted {
            std::fs::write(p, "planted").unwrap();
        }
        let before = NEXT.load(std::sync::atomic::Ordering::Relaxed);
        write_file(&file, "new", true).unwrap();
        assert_eq!(
            NEXT.load(std::sync::atomic::Ordering::Relaxed) - before,
            5,
            "four clashes retried, the fifth name taken"
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "new");
        for p in &planted {
            assert_eq!(
                std::fs::read_to_string(p).unwrap(),
                "planted",
                "{}",
                p.display()
            );
        }
        assert_eq!(temps(&d).len(), planted.len(), "no temp of ours is left");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(unix)]
    #[test]
    fn force_never_writes_through_a_symlink_at_the_temp_name() {
        let _held = FORCE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let d = dir("symlink");
        let file = d.join("r.md");
        let victim = d.join("victim");
        std::fs::write(&victim, "keep").unwrap();
        std::os::unix::fs::symlink(&victim, d.join("r.md.kaanal-tmp")).unwrap();
        for p in next_names(&file, 4) {
            std::os::unix::fs::symlink(&victim, p).unwrap();
        }
        write_file(&file, "new", true).unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "new");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_failed_forced_write_leaves_no_temp() {
        let _held = FORCE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let d = dir("failed");
        // a directory cannot be replaced by a file: the rename fails after the temp was written
        let target = d.join("taken");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("inside"), "x").unwrap();
        assert!(write_file(&target, "new", true).is_err());
        assert!(temps(&d).is_empty(), "{:?}", temps(&d));
        assert!(target.join("inside").exists(), "the target is as it was");
        let _ = std::fs::remove_dir_all(&d);
    }
}

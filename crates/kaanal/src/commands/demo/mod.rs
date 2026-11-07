//! `kaanal demo`: writes the embedded e-commerce project (`files`) to `.kaanal/demo/` and
//! runs it through the same path as `up`, in the foreground, on `127.0.0.1` only (M1-F spec C1,
//! C3, C4, C10, C12, C13). `.kaanal/` stays the state dir, so `kaanal chaos`, `logs`, `down` and
//! `scenario` in the same folder find the instance through `./.kaanal/run.json` as usual.

use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};

use serde_json::json;

mod files;

use files::FILES;

use crate::{
    Fail, Mode, STATE_DIR, Started, err, lifecycle, load, lock_instance, output, private_dir,
    serve_locked,
};

/// **Private.** The template's own locale line; `--locale` replaces it.
const TEMPLATE_LOCALE: &str = "locale: en-IN";

/// **Private.** `kaanal demo` arguments. No `--host`, `-d` or `--wait`: the demo is loopback and
/// foreground only.
#[derive(clap::Args)]
pub struct DemoArgs {
    /// Port to serve on (default 8080, or the next free one up to 8100; 0 picks any free port)
    #[arg(long)]
    port: Option<u16>,
    /// Locale tag for generated data, e.g. de-DE (default: en-IN)
    #[arg(long, value_name = "TAG")]
    locale: Option<String>,
    /// Write the demo project to DIR (default ./kaanal-demo) instead of running it
    #[arg(long, value_name = "DIR", num_args = 0..=1, conflicts_with_all = ["port", "open"])]
    #[allow(clippy::option_option)] // clap's shape for absent, `--eject`, `--eject DIR`
    eject: Option<Option<PathBuf>>,
    /// With --eject: write into a folder that is not empty
    #[arg(long, requires = "eject")]
    force: bool,
    /// Open the URL in your default browser
    #[arg(long)]
    open: bool,
}

/// **Private.** Fails if `path` exists and is a symlink (or, on Windows, a junction): a project
/// folder from elsewhere could carry one at `.kaanal/demo` or inside it, and the template would
/// be written through it. A missing path is fine.
fn refuse_link(path: &Path) -> Result<(), Fail> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => Err(err(format!(
            "{}: is a symbolic link; refusing to write the demo through it (remove it and retry)",
            path.display()
        ))),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(err(format!("{}: {e}", path.display()))),
    }
}

/// **Private.** Writes every template file under `dir` (creating folders), with `locale:` set to
/// `locale`. Files that are not part of the template are left alone. `locale` must already be a
/// known pack tag ([`check_locale`]), so it is safe to write into YAML. No folder on the way
/// down, and no file, may be a symlink ([`refuse_link`]).
// Check-then-write: a process racing the same folder could still swap a link in between. Closing
// that needs O_NOFOLLOW opens, which std does not offer portably, and the folder is the user's own.
pub fn write_template(dir: &Path, locale: &str) -> Result<(), Fail> {
    refuse_link(dir)?;
    for (rel, text) in FILES {
        let path = dir.join(rel);
        let mut at = dir.to_path_buf();
        for part in Path::new(rel).components() {
            at.push(part);
            refuse_link(&at)?;
        }
        let text = if *rel == "kaanal.yaml" {
            debug_assert!(text.contains(TEMPLATE_LOCALE), "the template sets en-IN");
            text.replacen(TEMPLATE_LOCALE, &format!("locale: {locale}"), 1)
        } else {
            (*text).to_string()
        };
        path.parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&path, text))
            .map_err(|e| err(format!("{}: {e}", path.display())))?;
    }
    Ok(())
}

/// **Private.** `locale` must be a pack the project in `dir` can use: a built-in one, or one in
/// its `locales/`. Anything else (`../x` included, and a case-only mismatch) is a usage error
/// listing the tags, before anything is written.
fn check_locale(dir: &Path, locale: &str) -> Result<(), Fail> {
    let data = kaanal_data::DataSet::load(dir, None).map_err(|e| err(e.to_string()))?;
    if data.tags().contains(&locale) {
        return Ok(());
    }
    Err(Fail::usage(format!(
        "unknown locale '{}'; available: {}",
        locale.escape_debug(),
        data.tags().join(", ")
    )))
}

/// **Private.** `kaanal demo`. Checks come first and change nothing: a running instance in this
/// folder (any config) or an unknown locale is a usage error (64). `--eject` is handled before
/// all of that: it needs no instance, no lock and no `.kaanal/`. Then it takes the instance
/// lock, rewrites the template, and serves it as `up` does.
pub fn demo(a: &DemoArgs) -> Result<(), Fail> {
    if let Some(dir) = &a.eject {
        let dir = dir.as_deref().unwrap_or(Path::new("kaanal-demo"));
        return eject(dir, a.locale.as_deref().unwrap_or("en-IN"), a.force);
    }
    let state = Path::new(STATE_DIR);
    if matches!(lifecycle::lock_probe(state)?, lifecycle::Liveness::Running) {
        let port = lifecycle::read_run(state)
            .map_or(String::new(), |r| format!(" on port {}", r.addr.port()));
        return Err(Fail::usage(format!(
            "kaanal is already running in this folder{port}; stop it with `kaanal down` first"
        )));
    }
    refuse_link(state)?;
    let dir = state.join("demo");
    let locale = a.locale.as_deref().unwrap_or("en-IN");
    check_locale(&dir, locale)?;
    private_dir(state)?;
    // Lost a race with another start since the probe: that one is "already running" (exit 1).
    let lock = lock_instance()?;
    write_template(&dir, locale)?;
    if Path::new("kaanal.yaml").exists() {
        eprintln!(
            "note: ignoring ./kaanal.yaml; the demo runs its own project in {}",
            dir.display()
        );
    }
    let config = dir.join("kaanal.yaml");
    let loaded = load(&config)?;
    let host = IpAddr::V4(Ipv4Addr::LOCALHOST);
    serve_locked(&config, loaded, a.port, host, Mode::default(), lock, |s| {
        announce(s, locale, a.open);
    })
}

/// **Private.** `kaanal demo --eject`: writes the template to `dir` and starts nothing (no server,
/// no lock, no `.kaanal/`). `dir` may be new or empty; otherwise `force` is needed, and then only
/// the template's own files are overwritten. The result runs under plain `kaanal up`.
fn eject(dir: &Path, locale: &str, force: bool) -> Result<(), Fail> {
    check_locale(dir, locale)?;
    refuse_link(dir)?;
    match std::fs::read_dir(dir) {
        Ok(mut it) => {
            if !force && it.next().is_some() {
                return Err(Fail::usage(format!(
                    "{}: is not empty; pass --force to overwrite the demo's files in it",
                    dir.display()
                )));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(err(format!("{}: {e}", dir.display()))),
    }
    write_template(dir, locale)?;
    let mut written: Vec<&str> = FILES.iter().map(|(rel, _)| *rel).collect();
    // `kaanal up` in the new folder keeps the token and signing key in `.kaanal/`, so a `git add .`
    // must not pick them up; the same append-only edit as `init`, and only a warning if it fails.
    let ignore = dir.join(".gitignore");
    match refuse_link(&ignore)
        .map_err(|f| f.1)
        .and_then(|()| crate::init::ignore_state_dir(&ignore))
    {
        Ok(true) => written.push(".gitignore"),
        Ok(false) => {}
        Err(why) => eprintln!("warning: could not update .gitignore: {why}; add .kaanal/ yourself"),
    }
    let root = std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf());
    let shown = dir.display().to_string();
    let shown = if shown.contains(char::is_whitespace) {
        format!("\"{shown}\"")
    } else {
        shown
    };
    let next = format!("cd {shown} && kaanal up");
    if output::json_on() {
        let v = json!({ "root": root.display().to_string(), "written": written, "next": next });
        crate::out_line(&v.to_string());
    } else {
        println!(
            "Wrote the demo project ({} files) to {}",
            written.len(),
            root.display()
        );
        println!("Next:  {next}");
    }
    Ok(())
}

/// **Private.** What the demo prints once it is up. With `--json`: one object, never the token.
fn announce(s: &Started, locale: &str, open: bool) {
    let port = s.port;
    let url = format!("http://localhost:{port}/#token={}", s.token);
    if output::json_on() {
        let v = json!({
            "url": format!("http://localhost:{port}/"), "host": "127.0.0.1", "port": port,
            "pid": std::process::id(), "locale": locale, "seed": s.seed, "auth": s.auth,
            "warnings": s.warnings,
        });
        crate::out_line(&v.to_string());
    } else {
        println!("Demo:  {url}");
        println!("Seed:  {}, locale: {locale}", s.seed);
        println!("Login: demo@example.com / demo");
        println!(
            "Try:   kaanal scenario activate slow-db --ttl 30s   (in another terminal, this folder)"
        );
        println!("Stop:  Ctrl-C, or kaanal down from this folder");
    }
    if open {
        open_browser(&url);
    }
}

/// **Private.** Hands `url` to the platform's opener as one argument (no shell parses it). The
/// opener's command line holds the token for a moment, as the terminal output already does.
fn open_browser(url: &str) {
    let mut cmd = if cfg!(windows) {
        let mut c = std::process::Command::new("rundll32.exe");
        c.arg("url.dll,FileProtocolHandler");
        c
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else {
        std::process::Command::new("xdg-open")
    };
    cmd.arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    match cmd.spawn() {
        // Reaped on a side thread, so it never lingers as a zombie.
        Ok(mut child) => drop(std::thread::spawn(move || child.wait())),
        Err(e) => eprintln!("warning: could not open a browser ({e}); open the URL above"),
    }
}

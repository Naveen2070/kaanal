//! `kaanal init SPEC`: turns an `OpenAPI` file into a starter project (RULES R1.20, M1-F spec
//! C6 to C8, C15). Plain stdin prompts with defaults (no prompt crate); the config is written to
//! a temp file, checked exactly as `kaanal validate` checks it, and only then renamed into place.

use std::io::{BufRead, Write};
use std::path::Path;

use super::{Fail, STATE_DIR, err, fetch, load, located_err, output};

/// Written at the top of the config so editors offer completion (R1.15, `kaanal schema`).
const SCHEMA_HEADER: &str =
    "# yaml-language-server: $schema=https://kaanal.naveenr.in/schema/v1.json";
const CONFIG: &str = "kaanal.yaml";
const TEMP: &str = "kaanal.yaml.tmp";
/// Node names `render_config` already uses; a service may not take one.
const RESERVED: [&str; 3] = ["gateway", "database", "cache"];

/// **Private.** `kaanal init` arguments. A flag answers its prompt, so nothing is asked for it.
#[derive(clap::Args)]
pub struct InitArgs {
    /// `OpenAPI` 3.0 file, or an `https` URL (plain `http` to localhost only) to download it from
    spec: Option<String>,
    /// Take every default; never prompt
    #[arg(short, long)]
    yes: bool,
    /// Overwrite an existing kaanal.yaml, and a same-named spec file when downloading from a URL
    #[arg(long)]
    force: bool,
    /// Service node name (default: app)
    #[arg(long)]
    service: Option<String>,
    /// Port the mock listens on (default: 8080)
    #[arg(long)]
    port: Option<u16>,
    /// Locale tag for generated data, e.g. de-DE (default: en-IN)
    #[arg(long)]
    locale: Option<String>,
}

/// **Private.** Everything `render_config` needs, already validated.
#[derive(Debug)]
pub struct Params {
    pub service: String,
    pub port: u16,
    pub locale: String,
    pub spec: String,
}

/// **Private.** `^[a-z][a-z0-9_-]{0,31}$`, hand-checked: the only text that goes into YAML
/// unquoted, so no metacharacter can get through.
fn valid_name(s: &str) -> bool {
    let b = s.as_bytes();
    (1..=32).contains(&b.len())
        && b[0].is_ascii_lowercase()
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_' || *c == b'-')
}

/// **Private.** The service answer: a valid name that is not one of the generated nodes.
fn parse_service(s: &str) -> Result<String, String> {
    if !valid_name(s) {
        return Err(format!(
            "service name '{}' must be 1 to 32 characters: a lowercase letter, then lowercase letters, digits, '-' or '_'",
            s.escape_debug()
        ));
    }
    if RESERVED.contains(&s) {
        return Err(format!(
            "service name '{s}' is already a node in the generated project; pick another"
        ));
    }
    Ok(s.to_string())
}

/// **Private.** The port answer: 1 to 65535.
fn parse_port(s: &str) -> Result<u16, String> {
    Some(s)
        .filter(|s| s.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|s| s.parse().ok())
        .filter(|p| *p != 0)
        .ok_or_else(|| format!("port '{}' is not 1 to 65535", s.escape_debug()))
}

/// **Private.** The locale answer: a locale tag shape (whether the pack exists is `check`'s job).
fn parse_locale(s: &str) -> Result<String, String> {
    if kaanal_core::config::valid_locale_tag(s) {
        Ok(s.to_string())
    } else {
        Err(format!(
            "locale '{}' is not a locale tag such as en-US",
            s.escape_debug()
        ))
    }
}

/// **Private.** `prompt [default]: ` on the prompt stream, then one line; an empty line or end of
/// input is the default.
fn ask(
    prompts: &mut impl Write,
    input: &mut impl BufRead,
    prompt: &str,
    default: &str,
) -> Result<String, Fail> {
    if default.is_empty() {
        write!(prompts, "{prompt}: ")
    } else {
        write!(prompts, "{prompt} [{default}]: ")
    }
    .and_then(|()| prompts.flush())
    .map_err(|e| err(e.to_string()))?;
    let mut line = String::new();
    input.read_line(&mut line).map_err(|e| err(e.to_string()))?;
    let line = line.trim();
    Ok(if line.is_empty() { default } else { line }.to_string())
}

/// **Private.** A flag's value, else (when `prompting`) the answer, else the default. A bad flag
/// is a usage error at once; a bad typed answer is explained and asked again, three tries in all,
/// then the usage error.
fn field<T>(
    flag: Option<String>,
    prompting: bool,
    prompts: &mut impl Write,
    input: &mut impl BufRead,
    (prompt, default): (&str, &str),
    parse: fn(&str) -> Result<T, String>,
) -> Result<T, Fail> {
    match flag {
        Some(v) => parse(&v).map_err(Fail::usage),
        None if prompting => {
            let mut last = String::new();
            for _ in 0..3 {
                match parse(&ask(prompts, input, prompt, default)?) {
                    Ok(v) => return Ok(v),
                    Err(why) => {
                        let _ = writeln!(prompts, "{why}");
                        last = why;
                    }
                }
            }
            Err(Fail::usage(last))
        }
        None => parse(default).map_err(Fail::usage),
    }
}

/// **Private.** Suffix of the backup a forced download moves the old spec to until `init` succeeds.
const BACKUP: &str = ".kaanal-old";

/// **Private.** A spec this run downloaded. `backup` is set when `--force` moved an existing file
/// of that name aside (to `name` + [`BACKUP`]); it is moved back by `undo` or deleted by `keep`,
/// so a crash leaves the old bytes on disk, never lost.
struct Saved {
    name: String,
    backup: Option<String>,
}

impl Saved {
    /// `init` failed: remove the download and put the old file back. A step that fails is a
    /// warning naming the file, so the user knows what to clean up.
    fn undo(self) {
        if let Err(e) = std::fs::remove_file(&self.name)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!(
                "warning: could not remove the downloaded {}: {e}",
                self.name
            );
        }
        if let Some(b) = &self.backup
            && let Err(e) = std::fs::rename(b, &self.name)
        {
            eprintln!("warning: could not restore {} from {b}: {e}", self.name);
        }
    }

    /// `init` succeeded: the backup is no longer needed.
    fn keep(&self) {
        if let Some(b) = &self.backup
            && let Err(e) = std::fs::remove_file(b)
        {
            eprintln!("warning: could not remove {b}: {e}; delete it yourself");
        }
    }
}

/// **Private.** A path is used as given. A URL (already checked by `gather`) is fetched and
/// saved beside the config under [`fetch::file_name`]; an existing file there needs `--force`
/// and a free backup name (both checked before fetching), and is moved to the backup only once
/// the download is good. Returns the spec path for the config and what was saved.
fn resolve_spec(arg: &str, force: bool) -> Result<(String, Option<Saved>), Fail> {
    if !fetch::is_url(arg) {
        return Ok((arg.to_string(), None));
    }
    let url = fetch::parse(arg)?;
    let name = fetch::file_name(&url);
    let backup = match std::fs::symlink_metadata(&name) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(err(format!("{name}: {e}"))),
        Ok(_) if !force => {
            return Err(Fail::usage(format!(
                "{name} already exists; pass --force to overwrite it"
            )));
        }
        Ok(m) if !m.is_file() => return Err(err(format!("{name}: not a regular file"))),
        Ok(_) => {
            let b = format!("{name}{BACKUP}");
            if std::fs::symlink_metadata(&b).is_ok() {
                return Err(Fail::usage(format!(
                    "{b} already exists (an interrupted init?); restore or delete it and retry"
                )));
            }
            Some(b)
        }
    };
    let body = fetch::fetch_spec(&url)?;
    // ponytail: exists-check then rename (rename replaces on every OS), a race only with
    // another process creating the backup name in the same instant.
    if let Some(b) = &backup {
        std::fs::rename(&name, b).map_err(|e| err(format!("{name}: {e}")))?;
    }
    let saved = Saved {
        name: name.clone(),
        backup,
    };
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&name)
        .and_then(|mut f| f.write_all(&body));
    if let Err(e) = written {
        // Our file is not removed if someone else created it meanwhile.
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            if let Some(b) = &saved.backup {
                eprintln!("warning: {name} reappeared; the old file is kept as {b}");
            }
            return Err(Fail::usage(format!("{name} already exists; retry")));
        }
        saved.undo();
        return Err(err(format!("{name}: {e}")));
    }
    Ok((name, Some(saved)))
}

/// **Private.** Flags first, then prompts (only when `interactive` and not `--yes`), then defaults;
/// every value is validated before it can reach the file.
fn gather(
    a: &InitArgs,
    input: &mut impl BufRead,
    prompts: &mut impl Write,
    interactive: bool,
) -> Result<Params, Fail> {
    let prompting = interactive && !a.yes;
    let spec = match &a.spec {
        Some(s) => s.clone(),
        None if prompting => ask(prompts, input, "OpenAPI spec path or URL", "")?,
        None => String::new(),
    };
    if spec.is_empty() {
        return Err(Fail::usage(
            "init needs an OpenAPI file: kaanal init openapi.yaml",
        ));
    }
    // A bad URL is a usage error before any question; it is fetched only once all are answered.
    if fetch::is_url(&spec) {
        fetch::parse(&spec)?;
    }
    let service = field(
        a.service.clone(),
        prompting,
        prompts,
        input,
        ("Service name", "app"),
        parse_service,
    )?;
    let port = field(
        a.port.map(|p| p.to_string()),
        prompting,
        prompts,
        input,
        ("Port", "8080"),
        parse_port,
    )?;
    let locale = field(
        a.locale.clone(),
        prompting,
        prompts,
        input,
        ("Locale", "en-IN"),
        parse_locale,
    )?;
    Ok(Params {
        service,
        port,
        locale,
        spec,
    })
}

/// **Private.** The generated `kaanal.yaml`: gateway to one service to a database and a cache, with
/// flavor presets for the timings and no `endpoints:` block (the default chain gives every route
/// cache-then-database timing; `kaanal endpoints --write` refines it later). The spec path goes in
/// as a JSON string, which is a valid YAML double-quoted scalar for any path.
pub fn render_config(p: &Params) -> String {
    let spec = serde_json::to_string(&p.spec).unwrap_or_default();
    let (svc, port, locale) = (&p.service, p.port, &p.locale);
    format!(
        "{SCHEMA_HEADER}\n\
         version: 1\n\
         port: {port}\n\
         locale: {locale}\n\
         nodes:\n\
         \x20 gateway: {{ type: gateway, flavor: nginx }}\n\
         \x20 {svc}: {{ type: service, spec: {spec} }}\n\
         \x20 database: {{ type: database, flavor: postgres }}\n\
         \x20 cache: {{ type: cache, flavor: redis }}\n\
         edges:\n\
         \x20 - {{ from: gateway, to: {svc} }}\n\
         \x20 - {{ from: {svc}, to: database }}\n\
         \x20 - {{ from: {svc}, to: cache }}\n"
    )
}

/// **Private.** A `.gitignore` line that already ignores the state directory.
fn ignores_state_dir(line: &str) -> bool {
    matches!(
        line.trim(),
        ".kaanal" | ".kaanal/" | "/.kaanal" | "/.kaanal/" | ".kaanal/*" | ".kaanal/**"
    )
}

/// **Private.** Adds `.kaanal/` to `.gitignore` (created when missing): the file is read only to
/// find the line, then appended to, so existing bytes are never rewritten, and in its own line
/// ending. `Ok(false)` when it was already listed; `Err` carries the reason it was left alone (a
/// file that cannot be read as UTF-8 is never touched).
pub fn ignore_state_dir(path: &Path) -> Result<bool, String> {
    let old = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e.to_string()),
    };
    let text = std::str::from_utf8(&old).map_err(|_| "it is not valid UTF-8".to_string())?;
    if text
        .strip_prefix('\u{feff}')
        .unwrap_or(text)
        .lines()
        .any(ignores_state_dir)
    {
        return Ok(false);
    }
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let lead = if text.is_empty() || text.ends_with('\n') {
        ""
    } else {
        nl
    };
    std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
        .and_then(|mut f| write!(f, "{lead}{STATE_DIR}/{nl}"))
        .map_err(|e| e.to_string())?;
    Ok(true)
}

/// **Private.** Writes the config to a new temp file, runs `load` and `check` on it, and renames
/// it into place; our temp file is gone on any failure. A temp file already there belongs to an
/// interrupted init: refused, never overwritten. Errors name `kaanal.yaml`, not the temp file.
fn write_checked(text: &str) -> Result<(), Fail> {
    let tmp = Path::new(TEMP);
    let mut file = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(tmp)
    {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(Fail::usage(format!(
                "{TEMP} already exists (an interrupted init?); delete it and retry"
            )));
        }
        Err(e) => return Err(err(format!("{TEMP}: {e}"))),
    };
    if let Err(e) = file.write_all(text.as_bytes()) {
        drop(file);
        let _ = std::fs::remove_file(tmp);
        return Err(err(format!("{TEMP}: {e}")));
    }
    drop(file);
    let done = (|| {
        let (cfg, dir, src) = load(tmp)?;
        kaanal_server::check(&cfg, &dir).map_err(|e| located_err(tmp, &src, &e))?;
        let (_, routes) =
            kaanal_server::spec_routes(&cfg, &dir).map_err(|e| located_err(tmp, &src, &e))?;
        if routes.is_empty() {
            return Err(err("the OpenAPI file has no paths to mock"));
        }
        std::fs::rename(tmp, CONFIG).map_err(|e| err(format!("{CONFIG}: {e}")))
    })();
    done.map_err(|f| {
        let _ = std::fs::remove_file(tmp);
        Fail(f.0, f.1.replace(TEMP, CONFIG))
    })
}

/// **Private.** `kaanal init`: gather the answers, then write `kaanal.yaml` and the `.gitignore`
/// line in the current folder. Questions go to `prompts` (stderr in the binary), the result to
/// `out`. Nothing is touched when anything fails before the final rename; a `.gitignore` that
/// cannot be updated afterwards is a warning, not a failure.
pub fn init(
    a: &InitArgs,
    input: &mut impl BufRead,
    prompts: &mut impl Write,
    out: &mut impl Write,
    interactive: bool,
) -> Result<(), Fail> {
    if Path::new(CONFIG).exists() && !a.force {
        return Err(Fail::usage(format!(
            "{CONFIG} already exists; pass --force to overwrite it"
        )));
    }
    let mut p = gather(a, input, prompts, interactive)?;
    let saved;
    (p.spec, saved) = resolve_spec(&p.spec, a.force)?;
    let spec = Path::new(&p.spec);
    let done = if spec.is_file() {
        write_checked(&render_config(&p))
    } else {
        Err(err(format!(
            "{}: not a readable file",
            spec.display().to_string().escape_debug()
        )))
    };
    if let Err(f) = done {
        if let Some(s) = saved {
            s.undo();
        }
        return Err(f);
    }
    let mut written = vec![CONFIG];
    // A spec that `--force` replaced is named, in text and in `--json` (`replaced`).
    let mut replaced = Vec::new();
    if let Some(s) = &saved {
        s.keep();
        written.push(&s.name);
        if s.backup.is_some() {
            replaced.push(s.name.as_str());
        }
    }
    match ignore_state_dir(Path::new(".gitignore")) {
        Ok(true) => written.push(".gitignore"),
        Ok(false) => {}
        Err(why) => {
            eprintln!("warning: could not update .gitignore: {why}; add .kaanal/ yourself");
        }
    }
    let mut result = serde_json::json!({ "written": written });
    if !replaced.is_empty() {
        result["replaced"] = serde_json::json!(replaced);
    }
    output::emit(&result, || {
        let _ = writeln!(out, "Wrote {}", written.join(", "));
        for r in &replaced {
            let _ = writeln!(out, "Replaced the existing {r} (--force)");
        }
        let _ = writeln!(out, "Next: kaanal up");
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kaanal_core::config::{Config, NodeConfig};
    use std::path::PathBuf;

    fn args(spec: Option<&str>) -> InitArgs {
        InitArgs {
            spec: spec.map(String::from),
            yes: false,
            force: false,
            service: None,
            port: None,
            locale: None,
        }
    }

    fn params() -> Params {
        Params {
            service: "app".into(),
            port: 9000,
            locale: "en-US".into(),
            spec: "openapi.yaml".into(),
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("kaanal-init-unit-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn render_config_is_a_valid_project() {
        let text = render_config(&params());
        assert!(text.starts_with(SCHEMA_HEADER), "{text}");
        assert!(
            !text.contains("endpoints:") && !text.contains("Project"),
            "{text}"
        );
        assert!(text.contains("locale: en-US") && text.contains("port: 9000"));
        let cfg = Config::from_yaml(&text).unwrap();
        let kinds: Vec<_> = cfg.nodes.keys().map(String::as_str).collect();
        assert_eq!(kinds, ["gateway", "app", "database", "cache"]);
        assert!(matches!(cfg.nodes["gateway"], NodeConfig::Gateway(_)));
        let NodeConfig::Service(s) = &cfg.nodes["app"] else {
            panic!("app is not a service")
        };
        assert_eq!(s.spec.as_deref(), Some("openapi.yaml"));
        assert_eq!(cfg.nodes["database"].flavor(), Some("postgres"));
        assert_eq!(cfg.nodes["cache"].flavor(), Some("redis"));
        let edges: Vec<_> = cfg.edges.iter().map(|e| (&*e.from, &*e.to)).collect();
        assert_eq!(
            edges,
            [("gateway", "app"), ("app", "database"), ("app", "cache")]
        );
    }

    #[test]
    fn a_spec_path_with_quotes_stays_one_string() {
        let mut p = params();
        p.spec = "a \"b\"\\c.yaml".into();
        let cfg = Config::from_yaml(&render_config(&p)).unwrap();
        let NodeConfig::Service(s) = &cfg.nodes["app"] else {
            panic!()
        };
        assert_eq!(s.spec.as_deref(), Some("a \"b\"\\c.yaml"));
    }

    #[test]
    fn prompts_use_defaults_on_empty_lines() {
        let mut out = Vec::new();
        let mut input = "openapi.yaml\n\n\n\n".as_bytes();
        let p = gather(&args(None), &mut input, &mut out, true).unwrap();
        assert_eq!(
            (p.service.as_str(), p.port, p.locale.as_str()),
            ("app", 8080, "en-IN")
        );
        let shown = String::from_utf8(out).unwrap();
        assert!(shown.contains("Service name [app]: ") && shown.contains("Port [8080]: "));
        assert!(!shown.contains("Project"));
    }

    #[test]
    fn prompts_take_typed_answers_and_end_of_input_is_the_default() {
        let mut out = Vec::new();
        let mut input = "x.yaml\nweb\n3000\n".as_bytes();
        let p = gather(&args(None), &mut input, &mut out, true).unwrap();
        assert_eq!((p.service.as_str(), p.port), ("web", 3000));
        assert_eq!(p.locale, "en-IN");
    }

    #[test]
    fn flags_answer_their_prompt_so_nothing_is_asked() {
        let mut a = args(Some("o.yaml"));
        (a.service, a.port, a.locale) = (Some("s".into()), Some(81), Some("de-DE".into()));
        let mut out = Vec::new();
        let p = gather(&a, &mut "".as_bytes(), &mut out, true).unwrap();
        assert_eq!(String::from_utf8_lossy(&out), "");
        assert_eq!(
            (p.service.as_str(), p.port, p.locale.as_str()),
            ("s", 81, "de-DE")
        );
        // One flag removes only its own question.
        let mut a = args(Some("o.yaml"));
        a.port = Some(81);
        let mut out = Vec::new();
        gather(&a, &mut "\n\n".as_bytes(), &mut out, true).unwrap();
        let shown = String::from_utf8(out).unwrap();
        assert!(shown.contains("Locale") && !shown.contains("Port"));
    }

    #[test]
    fn yes_and_non_interactive_take_defaults_without_reading_input() {
        for (yes, interactive) in [(true, true), (false, false)] {
            let mut a = args(Some("o.yaml"));
            a.yes = yes;
            let mut out = Vec::new();
            let p = gather(&a, &mut "boom\n".as_bytes(), &mut out, interactive).unwrap();
            assert_eq!(String::from_utf8_lossy(&out), "");
            assert_eq!((p.service.as_str(), p.port), ("app", 8080));
        }
        let e = gather(&args(None), &mut "".as_bytes(), &mut Vec::new(), false).unwrap_err();
        assert_eq!(e.0, 64);
    }

    #[test]
    fn bad_flags_are_hard_usage_errors() {
        for (service, port, locale) in [
            ("a: b", 1, "en-US"),
            ("Web", 1, "en-US"),
            ("gateway", 1, "en-US"),
            ("app", 0, "en-US"),
            ("app", 1, "not a tag"),
        ] {
            let mut a = args(Some("o.yaml"));
            a.service = Some(service.into());
            a.port = Some(port);
            a.locale = Some(locale.into());
            // Prompting is on, yet a flag is never re-asked.
            let mut out = Vec::new();
            let e = gather(&a, &mut "app\n1\nen-US\n".as_bytes(), &mut out, true).unwrap_err();
            assert_eq!(e.0, 64, "{service} {port} {locale}");
            assert_eq!(String::from_utf8_lossy(&out), "");
        }
    }

    #[test]
    fn a_bad_typed_answer_is_explained_and_asked_again() {
        let mut out = Vec::new();
        let mut input = "o.yaml\nBad Name\nweb\n0\n9090\n\n".as_bytes();
        let p = gather(&args(None), &mut input, &mut out, true).unwrap();
        assert_eq!((p.service.as_str(), p.port), ("web", 9090));
        let shown = String::from_utf8(out).unwrap();
        assert!(shown.contains("service name 'Bad Name'"), "{shown}");
        assert!(shown.contains("port '0' is not 1 to 65535"), "{shown}");
    }

    #[test]
    fn three_bad_answers_fail_with_the_usage_error() {
        let mut out = Vec::new();
        let mut input = "o.yaml\nA\nB\nC\nweb\n".as_bytes();
        let e = gather(&args(None), &mut input, &mut out, true).unwrap_err();
        assert_eq!(e.0, 64);
        assert!(e.1.contains("service name 'C'"), "{}", e.1);
    }

    #[test]
    fn end_of_input_midway_takes_the_defaults() {
        let mut out = Vec::new();
        let p = gather(&args(None), &mut "o.yaml\nweb\n".as_bytes(), &mut out, true).unwrap();
        assert_eq!(
            (p.service.as_str(), p.port, p.locale.as_str()),
            ("web", 8080, "en-IN")
        );
    }

    #[test]
    fn gitignore_is_appended_to_in_its_own_line_ending() {
        let dir = scratch("ignore");
        let f = dir.join(".gitignore");
        assert_eq!(ignore_state_dir(&f), Ok(true));
        assert_eq!(std::fs::read(&f).unwrap(), b".kaanal/\n");
        std::fs::write(&f, "a\r\nb").unwrap();
        assert_eq!(ignore_state_dir(&f), Ok(true));
        assert_eq!(std::fs::read(&f).unwrap(), b"a\r\nb\r\n.kaanal/\r\n");
        assert_eq!(ignore_state_dir(&f), Ok(false));
        std::fs::write(&f, "a").unwrap();
        assert_eq!(ignore_state_dir(&f), Ok(true));
        assert_eq!(std::fs::read(&f).unwrap(), b"a\n.kaanal/\n");
    }

    #[test]
    fn existing_ignore_lines_add_nothing() {
        let dir = scratch("variants");
        let f = dir.join(".gitignore");
        for line in [
            ".kaanal",
            ".kaanal/",
            "/.kaanal",
            "/.kaanal/",
            ".kaanal/*",
            ".kaanal/**",
            "  .kaanal/  ",
        ] {
            let text = format!("# c\n{line}\nx\n");
            std::fs::write(&f, &text).unwrap();
            assert_eq!(ignore_state_dir(&f), Ok(false), "{line}");
            assert_eq!(std::fs::read_to_string(&f).unwrap(), text);
        }
    }

    #[test]
    fn a_bom_on_the_first_gitignore_line_does_not_hide_the_entry() {
        let dir = scratch("bom");
        let f = dir.join(".gitignore");
        std::fs::write(&f, "\u{feff}.kaanal/\nx\n").unwrap();
        assert_eq!(ignore_state_dir(&f), Ok(false));
    }

    #[test]
    fn a_port_is_plain_digits() {
        for bad in ["+80", "-1", "0", "65536", "", " 80", "8 0", "0x50"] {
            assert!(parse_port(bad).is_err(), "{bad}");
        }
        assert_eq!(parse_port("0080"), Ok(80));
        assert_eq!(parse_port("65535"), Ok(65535));
    }

    #[test]
    fn an_unreadable_gitignore_is_left_alone() {
        let dir = scratch("unreadable");
        let f = dir.join(".gitignore");
        let utf16: Vec<u8> = [0xff, 0xfe, b'a', 0, b'\n', 0].to_vec();
        std::fs::write(&f, &utf16).unwrap();
        assert!(ignore_state_dir(&f).is_err());
        assert_eq!(std::fs::read(&f).unwrap(), utf16);
        let d = dir.join("sub");
        std::fs::create_dir(&d).unwrap();
        assert!(ignore_state_dir(&d).is_err());
    }
}

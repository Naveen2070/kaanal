//! Scenario files: `scenarios/NAME.yaml` next to `kaanal.yaml` (S16).
//!
//! The only place that touches the disk for scenarios; the rules about what a scenario may
//! hold live in `kaanal_core::scenario`.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use kaanal_core::diagnostics::Located;
use kaanal_core::{MAX_SCENARIOS, ScenarioConfig, parse_scenario, valid_name};

/// Largest scenario file (R1.30).
const MAX_FILE: u64 = 1 << 20;

/// A path for messages: without the `\\?\` prefix Windows `canonicalize` adds.
fn plain(p: &Path) -> PathBuf {
    match p.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(s) if !s.starts_with("UNC\\") => PathBuf::from(s),
        _ => p.to_path_buf(),
    }
}

/// Reads every scenario file in `dir`, sorted by name: `*.yaml` regular files only. Skips
/// subdirectories, dotfiles and editor temp files (`*~`, `.#*`) silently. Skips with a
/// warning (second return value) symlinks (not followed), names that are not UTF-8, names
/// that look like scenario files but do not end in exactly `.yaml` (`.yml`, `.YAML`), and
/// entries whose type cannot be read. An absent `dir` means no files. Returns
/// `((name, parsed, file), warnings)`.
///
/// # Errors
/// An unreadable directory or file, a file over 1 MiB, a stem that is not a valid scenario
/// name, more than [`MAX_SCENARIOS`] files, or a parse error (placed in the file).
#[allow(clippy::type_complexity)]
pub fn discover(
    dir: &Path,
) -> Result<(Vec<(String, ScenarioConfig, PathBuf)>, Vec<String>), Located> {
    let (files, warnings) = list(dir)?;
    let found = files
        .into_iter()
        .map(|(stem, path)| {
            valid_name(&stem).map_err(|m| Located::new(Vec::new(), m).in_file(path.clone()))?;
            let src = read(&path)?;
            let cfg = parse_scenario(&src).map_err(|e| Located::parsed(path.clone(), e))?;
            Ok((stem, cfg, path))
        })
        .collect::<Result<Vec<_>, Located>>()?;
    Ok((found, warnings))
}

/// The scenario files in `dir` by [`discover`]'s rules, sorted by name, without reading them:
/// `((stem, file), warnings)`. An absent `dir` means no files.
///
/// # Errors
/// An unreadable directory, or more than [`MAX_SCENARIOS`] files.
#[allow(clippy::type_complexity)]
#[allow(clippy::case_sensitive_file_extension_comparisons)] // lower is already lower-cased
pub(crate) fn list(dir: &Path) -> Result<(Vec<(String, PathBuf)>, Vec<String>), Located> {
    let dir = plain(dir);
    let rd = match std::fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((Vec::new(), Vec::new())),
        Err(e) => return Err(Located::new(Vec::new(), format!("{}: {e}", dir.display()))),
    };
    let mut warnings = Vec::new();
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    for entry in rd {
        let entry =
            entry.map_err(|e| Located::new(Vec::new(), format!("{}: {e}", dir.display())))?;
        let file_name = entry.file_name();
        let shown = format!("scenarios/{}", file_name.to_string_lossy());
        let Some(name) = file_name.to_str() else {
            warnings.push(format!("skipped {shown}: the file name is not UTF-8"));
            continue;
        };
        if name.starts_with('.') || name.ends_with('~') {
            continue;
        }
        let lower = name.to_ascii_lowercase();
        if !(lower.ends_with(".yaml") || lower.ends_with(".yml")) {
            continue;
        }
        let kind = match entry.file_type() {
            Ok(t) => t,
            Err(e) => {
                warnings.push(format!("skipped {shown}: {e}"));
                continue;
            }
        };
        if kind.is_symlink() {
            warnings.push(format!(
                "skipped symlink {shown} (symlinks are not followed)"
            ));
            continue;
        }
        if !kind.is_file() {
            continue;
        }
        let Some(stem) = name.strip_suffix(".yaml") else {
            warnings.push(format!("skipped {shown}: scenario files must end in .yaml"));
            continue;
        };
        files.push((stem.to_string(), entry.path()));
    }
    if files.len() > MAX_SCENARIOS {
        return Err(Located::new(
            Vec::new(),
            format!(
                "{}: {} scenario files; at most {MAX_SCENARIOS}",
                dir.display(),
                files.len()
            ),
        ));
    }
    files.sort();
    Ok((files, warnings))
}

/// **Private.** The file as text, refusing more than 1 MiB without reading it all.
fn read(path: &Path) -> Result<String, Located> {
    // `in_file` makes the CLI print `path: message`.
    let fail = |m: String| Located::new(Vec::new(), m).in_file(path.to_path_buf());
    let file = std::fs::File::open(path).map_err(|e| fail(e.to_string()))?;
    let mut buf = Vec::new();
    file.take(MAX_FILE + 1)
        .read_to_end(&mut buf)
        .map_err(|e| fail(e.to_string()))?;
    if buf.len() as u64 > MAX_FILE {
        return Err(fail("scenario file is over 1 MiB".into()));
    }
    String::from_utf8(buf).map_err(|_| fail("not valid UTF-8".into()))
}

/// **Private.** Makes the new directory entry durable (unix). Best effort: a failure here
/// loses nothing already written. Windows has no directory fsync, so it is a no-op there.
fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

/// Why [`write`] failed.
pub enum WriteError {
    /// The file exists and `force` was not set.
    Exists,
    /// Anything else: a bad name, a directory that is not the canonical one, an IO error.
    Other(String),
}

/// Writes `NAME.yaml` into `dir` (S17) and returns its path. Without `force` the file is made
/// with `create_new`, so two saves cannot both win; with `force` the text goes to `.NAME.tmp`
/// and is renamed over the target (which replaces a symlink instead of following it).
/// Edge cases: two processes saving the same name at once share the temp path (one may fail
/// with a 500; the target is never half-written). On Windows a target held open by another
/// program makes the rename fail (500). Either way no data is lost. The directory is fsynced on unix (a no-op on Windows).
/// `dir` is `AppState.scenarios_dir`; it is created if missing, and the resolved parent of the
/// target must equal it.
///
/// # Errors
/// [`WriteError::Exists`] or [`WriteError::Other`].
pub fn write(dir: &Path, name: &str, yaml: &str, force: bool) -> Result<PathBuf, WriteError> {
    use std::io::Write as _;
    let other = |e: &dyn std::fmt::Display| WriteError::Other(format!("{}: {e}", dir.display()));
    valid_name(name).map_err(WriteError::Other)?;
    std::fs::create_dir_all(dir).map_err(|e| other(&e))?;
    let target = dir.join(format!("{name}.yaml"));
    let parent = target
        .parent()
        .and_then(|p| p.canonicalize().ok())
        .ok_or_else(|| other(&"cannot resolve the scenarios directory"))?;
    if plain(&parent) != plain(dir) {
        return Err(other(&"the scenarios directory moved since startup"));
    }
    if force {
        let tmp = dir.join(format!(".{name}.tmp"));
        // A planted file or symlink at the temp path is removed (the link itself, never its
        // target), then the temp is made with `create_new`, so the write cannot follow it.
        let done = match std::fs::remove_file(&tmp) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
        .and_then(|()| {
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp)?;
            f.write_all(yaml.as_bytes())?;
            f.sync_all()?;
            std::fs::rename(&tmp, &target)?;
            sync_dir(dir);
            Ok(())
        });
        if let Err(e) = done {
            let _ = std::fs::remove_file(&tmp);
            return Err(other(&e));
        }
    } else {
        let mut f = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
        {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(WriteError::Exists);
            }
            Err(e) => return Err(other(&e)),
        };
        if let Err(e) = f.write_all(yaml.as_bytes()).and_then(|()| f.sync_all()) {
            drop(f);
            let _ = std::fs::remove_file(&target);
            return Err(other(&e));
        }
        sync_dir(dir);
    }
    Ok(plain(&target))
}

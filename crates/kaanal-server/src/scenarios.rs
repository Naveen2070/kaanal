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
#[allow(clippy::case_sensitive_file_extension_comparisons)] // lower is already lower-cased
pub fn discover(
    dir: &Path,
) -> Result<(Vec<(String, ScenarioConfig, PathBuf)>, Vec<String>), Located> {
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

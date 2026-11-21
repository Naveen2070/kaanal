//! What one `generate` call draws from ([`Options`]: the locale pack and the field
//! dictionaries), and the project's data files they come from ([`DataSet`]).

use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::dictionary::Dictionary;
use crate::pack::{DataError, Pack, builtin_tags};

/// Generation options for one body.
#[derive(Debug, Clone)]
pub struct Options {
    /// The locale pack names, places, phones, currency and country come from.
    pub pack: Arc<Pack>,
    /// The built-in dictionary, consulted after `format` (D1.8).
    pub dictionary: Arc<Dictionary>,
    /// The project dictionary (`data.yaml`), consulted before `format` (D1.8).
    pub dictionary_project: Option<Arc<Dictionary>>,
}

impl Default for Options {
    /// The built-in `en-IN` pack and the built-in dictionary, no project dictionary.
    fn default() -> Self {
        Options {
            pack: Pack::builtin(DEFAULT_LOCALE).expect("en-IN is built in"),
            dictionary: Dictionary::builtin(),
            dictionary_project: None,
        }
    }
}

/// The locale used when neither the node nor the project sets one.
pub const DEFAULT_LOCALE: &str = "en-IN";

/// **Private.** Largest data file read (pack or dictionary), as for scenario files (R1.30).
const MAX_FILE: u64 = 1 << 20;

/// **Private.** Most pack files in `locales/`.
const MAX_PACKS: usize = 64;

/// A project's data: the built-in packs with `locales/*.yaml` over them, and the built-in
/// dictionary with the project's `data.yaml`. Loaded once at startup; [`DataSet::options_for`]
/// hands out the [`Options`] of one locale.
#[derive(Debug, Clone)]
pub struct DataSet {
    packs: BTreeMap<String, Arc<Pack>>,
    project: Option<Arc<Dictionary>>,
    warnings: Vec<String>,
}

impl DataSet {
    /// Loads the data next to a config in `base_dir`: every built-in pack, then each
    /// `locales/<tag>.yaml` (replacing a built-in of the same tag), then the dictionary `data`
    /// (relative to `base_dir`) or else `data.yaml` when it exists. A missing `locales/` or
    /// `data.yaml` is fine, and so is an empty `data.yaml`.
    ///
    /// Pack files are found by listing `locales/` (the scenario-file rules: `.yaml` regular
    /// files, symlinks skipped, at most 64), never by building a path from a tag. A file's name
    /// must be a locale tag equal to its `tag:`, and no two tags may differ only in case.
    ///
    /// # Errors
    /// The first problem, in file-name order, with [`DataError::file`] set: an unreadable file
    /// or one over 1 MiB, too many pack files, a file name that is not a tag, a `tag:` that
    /// differs from it or from another tag only in case, or a parse error. A configured `data`
    /// that is not a file or cannot be read has no `file` and `at` = `["data"]`: it belongs to
    /// the config.
    pub fn load(base_dir: &Path, data: Option<&str>) -> Result<DataSet, DataError> {
        Self::load_with_sources(base_dir, data).map(|(set, _)| set)
    }

    /// [`DataSet::load`], also handing back each file it read with its text, in reading order
    /// (the same files as [`DataSet::files`]), so a caller that digests them (the design
    /// fingerprint) neither reads them again nor races an edit made in between.
    ///
    /// # Errors
    /// As [`DataSet::load`].
    pub fn load_with_sources(
        base_dir: &Path,
        data: Option<&str>,
    ) -> Result<(DataSet, Vec<(PathBuf, String)>), DataError> {
        let mut sources = Vec::new();
        let mut packs: BTreeMap<String, Arc<Pack>> = builtin_tags()
            .iter()
            .map(|t| {
                (
                    (*t).to_string(),
                    Pack::builtin(t).expect("listed as built in"),
                )
            })
            .collect();
        let mut warnings = Vec::new();
        for file in pack_files(&base_dir.join("locales"), &mut warnings)? {
            let in_file = |e: DataError| DataError {
                file: Some(file.clone()),
                ..e
            };
            let tag = file
                .file_stem()
                .and_then(|s| s.to_str())
                .filter(|s| valid_locale_tag(s))
                .ok_or_else(|| {
                    in_file(DataError::unplaced(
                        &[],
                        "the file name must be a locale tag such as de-DE.yaml".into(),
                    ))
                })?
                .to_string();
            let src = read(&file).map_err(in_file)?;
            let pack = Pack::parse(&src).map_err(in_file)?;
            if pack.tag != tag {
                return Err(in_file(DataError::at_value(
                    &["tag"],
                    format!("tag: `{}` must match the file name `{tag}`", pack.tag),
                )));
            }
            // Tags are looked up exactly, so two that differ only in case would be a trap.
            if let Some(other) = packs
                .keys()
                .find(|k| k.eq_ignore_ascii_case(&tag) && **k != tag)
            {
                return Err(in_file(DataError::at_value(
                    &["tag"],
                    format!(
                        "tag: `{tag}` differs from the pack `{other}` only in case; name the file and tag `{other}` to replace it"
                    ),
                )));
            }
            packs.insert(tag, Arc::new(pack));
            sources.push((file, src));
        }
        let path = dictionary_file(base_dir, data);
        if data.is_some()
            && let Some(path) = &path
            && !path.is_file()
        {
            return Err(DataError::at_value(
                &["data"],
                format!("data: {} is not a file", path.display()),
            ));
        }
        let project = match path {
            None => None,
            Some(path) => {
                let src = read(&path).map_err(|e| match data {
                    // the config named it: place the error at `data:`
                    Some(_) => DataError {
                        file: None,
                        at: vec!["data".into()],
                        message: format!("data: {}: {}", path.display(), e.message),
                        ..e
                    },
                    None => DataError {
                        file: Some(path.clone()),
                        ..e
                    },
                })?;
                let d = Dictionary::parse(&src).map_err(|e| DataError {
                    file: Some(path.clone()),
                    ..e
                })?;
                warnings.extend(
                    d.warnings()
                        .iter()
                        .map(|w| format!("{}: {w}", path.display())),
                );
                sources.push((path, src));
                Some(Arc::new(d))
            }
        };
        Ok((
            DataSet {
                packs,
                project,
                warnings,
            },
            sources,
        ))
    }

    /// The options of locale `tag`: its pack, the built-in dictionary and the project's.
    ///
    /// # Errors
    /// An unknown tag (anything that is not a loaded pack, `../x` included), listing the
    /// available tags; it has no place, the caller knows which `locale:` it came from.
    pub fn options_for(&self, tag: &str) -> Result<Options, DataError> {
        let pack = self.packs.get(tag).ok_or_else(|| {
            let hint = self
                .packs
                .keys()
                .find(|k| k.eq_ignore_ascii_case(tag))
                .map_or_default(|k| format!(" (did you mean `{k}`?)"));
            DataError::unplaced(
                &[],
                format!(
                    "unknown locale `{tag}`{hint}; available: {} (add locales/{tag}.yaml to define it)",
                    self.tags().join(", ")
                ),
            )
        })?;
        Ok(Options {
            pack: Arc::clone(pack),
            dictionary: Dictionary::builtin(),
            dictionary_project: self.project.clone(),
        })
    }

    /// The available locale tags, sorted.
    pub fn tags(&self) -> Vec<&str> {
        self.packs.keys().map(String::as_str).collect()
    }

    /// Load-time warnings: skipped files, and project dictionary keys that normalise alike.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// The files [`DataSet::load`] reads for the same arguments, without reading them: the pack
    /// files of `locales/` by its listing rules, then the dictionary (`data` when given, else
    /// `data.yaml` when it exists). A given `data` is listed even when it is not a file (`load`
    /// refuses it). For a fingerprint of the project's inputs.
    ///
    /// # Errors
    /// The listing errors `load` reports: an unreadable `locales/` or too many pack files.
    pub fn files(base_dir: &Path, data: Option<&str>) -> Result<Vec<PathBuf>, DataError> {
        let mut files = pack_files(&base_dir.join("locales"), &mut Vec::new())?;
        files.extend(dictionary_file(base_dir, data));
        Ok(files)
    }
}

/// **Private.** The dictionary file: `data` under `base_dir` when given (checked by the caller),
/// else `data.yaml` when it is a file.
fn dictionary_file(base_dir: &Path, data: Option<&str>) -> Option<PathBuf> {
    match data {
        Some(p) => Some(base_dir.join(p)),
        None => Some(base_dir.join("data.yaml")).filter(|p| p.is_file()),
    }
}

/// **Private.** The pack files in `dir`, sorted, by the scenario-file rules (S16): `*.yaml`
/// regular files only. Skips subdirectories, dotfiles and editor temp files (`*~`) silently,
/// and with a warning: symlinks (not followed), names that are not UTF-8, names that look like
/// pack files but do not end in exactly `.yaml` (`.yml`, `.YAML`), and entries whose type
/// cannot be read. An absent `dir` has none; more than [`MAX_PACKS`] files is an error.
#[allow(clippy::case_sensitive_file_extension_comparisons)] // lower is already lower-cased
fn pack_files(dir: &Path, warnings: &mut Vec<String>) -> Result<Vec<PathBuf>, DataError> {
    let fail = |m: String| DataError {
        file: Some(dir.to_path_buf()),
        ..DataError::unplaced(&[], m)
    };
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(fail(e.to_string())),
    };
    let mut files = Vec::new();
    for entry in rd {
        let entry = entry.map_err(|e| fail(e.to_string()))?;
        let path = entry.path();
        let shown = path.display();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
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
        if !name.ends_with(".yaml") {
            warnings.push(format!("skipped {shown}: pack files must end in .yaml"));
            continue;
        }
        files.push(path);
    }
    if files.len() > MAX_PACKS {
        return Err(fail(format!(
            "{} pack files; at most {MAX_PACKS}",
            files.len()
        )));
    }
    files.sort();
    Ok(files)
}

/// **Private.** A data file as text, refusing more than 1 MiB without reading it all.
fn read(path: &Path) -> Result<String, DataError> {
    let fail = |m: String| DataError {
        file: Some(path.to_path_buf()),
        ..DataError::unplaced(&[], m)
    };
    let file = std::fs::File::open(path).map_err(|e| fail(e.to_string()))?;
    let mut buf = Vec::new();
    file.take(MAX_FILE + 1)
        .read_to_end(&mut buf)
        .map_err(|e| fail(e.to_string()))?;
    if buf.len() as u64 > MAX_FILE {
        return Err(fail("file is over 1 MiB".into()));
    }
    String::from_utf8(buf).map_err(|_| fail("not valid UTF-8".into()))
}

/// The same rule as core's `valid_locale_tag` (this crate cannot depend on core, R2.4); a
/// kaanal-server test pins the two to agree. Here it only vets file names in `locales/`.
#[doc(hidden)]
pub fn valid_locale_tag(s: &str) -> bool {
    let mut parts = s.split('-');
    let first = parts.next().unwrap_or_default();
    (2..=3).contains(&first.len())
        && first.bytes().all(|b| b.is_ascii_alphabetic())
        && parts.all(|p| (2..=8).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_alphanumeric()))
}

//! The layout sidecar: where Studio draws each node, kept beside the config in a file of a fixed
//! name ([`FILE`]) so `kaanal.yaml` stays free of positions. It is read when the draft is made and
//! written by Save after the config, through the same temp-file-and-rename ([`super::save::commit`]).
//!
//! The path is never built from input: the folder is the config file's own and the name is
//! [`FILE`]. A sidecar that is a link or not a regular file is neither followed on load nor
//! replaced on Save. Last writer wins: positions are low-stakes, so there is no conflict check.

use super::notes::Notes;
use super::save::{Change, commit};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::Path;

/// The sidecar's name, in the config file's folder.
pub const FILE: &str = "kaanal.layout.json";
/// The largest sidecar read (and written): 256 KiB.
pub const MAX_FILE_BYTES: u64 = 256 * 1024;
/// The most positions a layout holds.
pub const MAX_ENTRIES: usize = 2000;
/// The largest distance from 0 of a coordinate.
pub const MAX_COORD: f64 = 1e6;

/// Node name to `[x, y]`.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Layout(pub BTreeMap<String, [f64; 2]>);

/// **Private.** The file's shape: `{"version":1,"nodes":{...},"notes":[...]}`; `notes` is left out
/// when there are none, so a project without notes has the file it always had.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OnDisk {
    version: u32,
    nodes: Layout,
    #[serde(default, skip_serializing_if = "Notes::is_empty")]
    notes: Notes,
}

impl Layout {
    /// Whether this layout may be kept: at most [`MAX_ENTRIES`] positions, no empty name, and every
    /// coordinate finite and within ±[`MAX_COORD`]. The file size is [`check_size`]'s.
    ///
    /// # Errors
    /// What is wrong, in words.
    pub fn check(&self) -> Result<(), String> {
        if self.0.len() > MAX_ENTRIES {
            return Err(format!(
                "layout has {} positions (limit {MAX_ENTRIES})",
                self.0.len()
            ));
        }
        for (name, p) in &self.0 {
            if name.is_empty() {
                return Err("layout: a node name is empty".into());
            }
            if p.iter().any(|c| !c.is_finite() || c.abs() > MAX_COORD) {
                return Err(format!(
                    "layout: the position of '{name}' must be finite and within ±{MAX_COORD}"
                ));
            }
        }
        Ok(())
    }

    /// Moves the position of `from` to `to` (what `rename_node` does to the text).
    pub fn rename(&mut self, from: &str, to: &str) {
        if let Some(p) = self.0.remove(from) {
            self.0.insert(to.to_string(), p);
        }
    }

    /// Only the positions of the nodes `text` defines; all of them when `text` is not a config
    /// (nothing is dropped on a guess).
    pub fn of_nodes_in(&self, text: &str) -> Layout {
        match kaanal_core::Config::from_yaml(text) {
            Ok(c) => Layout(
                self.0
                    .iter()
                    .filter(|(k, _)| c.nodes.contains_key(*k))
                    .map(|(k, v)| (k.clone(), *v))
                    .collect(),
            ),
            Err(_) => self.clone(),
        }
    }
}

/// Whether the positions and the notes together fit the file limit.
///
/// # Errors
/// What is wrong, in words.
pub fn check_both(layout: &Layout, notes: &Notes) -> Result<(), String> {
    layout.check()?;
    notes.check()?;
    check_size(layout, notes)
}

/// Whether the file the positions and notes would make is at most [`MAX_FILE_BYTES`] (the one
/// serialisation of a check that is otherwise made of cheap tests).
///
/// # Errors
/// What is wrong, in words.
pub fn check_size(layout: &Layout, notes: &Notes) -> Result<(), String> {
    if file_text(layout, notes).len() as u64 > MAX_FILE_BYTES {
        return Err(format!(
            "positions and notes: larger than {} KiB as a file",
            MAX_FILE_BYTES / 1024
        ));
    }
    Ok(())
}

/// **Private.** The file's text.
fn file_text(layout: &Layout, notes: &Notes) -> String {
    let disk = OnDisk {
        version: 1,
        nodes: layout.clone(),
        notes: notes.clone(),
    };
    serde_json::to_string_pretty(&disk).unwrap_or_default() + "\n"
}

/// The positions and notes in `dir`'s sidecar, and a note when there is a file but it was not used
/// (never an error: a missing, unreadable, corrupt, oversized, linked or invalid file means no
/// positions and no notes).
pub fn load(dir: &Path) -> (Layout, Notes, Option<String>) {
    match read(&dir.join(FILE)) {
        Ok(l) => {
            let (layout, notes) = l.unwrap_or_default();
            (layout, notes, None)
        }
        Err(why) => (
            Layout::default(),
            Notes::default(),
            Some(format!(
                "{FILE} was not used ({why}); positions and notes start empty"
            )),
        ),
    }
}

/// **Private.** The sidecar at `path`; `None` when there is none. The name is looked at without
/// following a link, then opened and judged by its own handle, so a link swapped in between is
/// caught: on Windows the open does not follow a reparse point, on Unix the handle must be the
/// very file that was looked at.
fn read(path: &Path) -> Result<Option<(Layout, Notes)>, String> {
    let seen = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    if !seen.file_type().is_file() {
        return Err("it is a link or not a regular file, and is not followed".into());
    }
    let file = open_no_follow(path).map_err(|e| e.to_string())?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() || !same_file(&seen, &meta) {
        return Err("it changed while it was read".into());
    }
    let too_big = || format!("larger than {} KiB", MAX_FILE_BYTES / 1024);
    if meta.len() > MAX_FILE_BYTES {
        return Err(too_big());
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(too_big());
    }
    // serde_json stops at 128 levels of nesting, so a deep file is an error, not a stack overflow
    let disk: OnDisk = serde_json::from_slice(&bytes).map_err(|e| {
        // serde's own text can quote the offending value; a note names only the kind and place
        let what = match e.classify() {
            serde_json::error::Category::Io => "could not be read",
            serde_json::error::Category::Syntax => "is not valid JSON",
            serde_json::error::Category::Data => "does not have the layout shape",
            serde_json::error::Category::Eof => "ends too early",
        };
        format!("it {what} (line {}, column {})", e.line(), e.column())
    })?;
    if disk.version != 1 {
        return Err(format!("version {} (only 1 is known)", disk.version));
    }
    check_both(&disk.nodes, &disk.notes)?;
    Ok(Some((disk.nodes, disk.notes)))
}

#[cfg(windows)]
fn open_no_follow(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt as _;
    // FILE_FLAG_OPEN_REPARSE_POINT: a link is opened as itself, and then is not a regular file
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(0x0020_0000)
        .open(path)
}

#[cfg(not(windows))]
fn open_no_follow(path: &Path) -> std::io::Result<std::fs::File> {
    // std has no portable `O_NOFOLLOW` (its value differs by OS and CPU); `same_file` catches a swap
    // instead.
    std::fs::File::open(path)
}

#[cfg(unix)]
fn same_file(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    (a.dev(), a.ino()) == (b.dev(), b.ino())
}

#[cfg(not(unix))]
fn same_file(_: &std::fs::Metadata, _: &std::fs::Metadata) -> bool {
    true
}

/// Writes `layout` (only the nodes `text` defines) to the sidecar beside `config`, after Save
/// wrote the config. `dir_before` is the config folder's real path taken before the config was
/// written: the sidecar goes there only if the folder still resolves to it. Answers whether it was
/// written and, when not, why. Nothing to write (no positions, no notes) writes nothing and
/// removes nothing, unless `had_notes`: the file held notes that are now all deleted, so it is
/// rewritten without them. `unusable`: why the file was not used when it was read (invalid, locked,
/// too big), so what is on disk was never loaded: it is not replaced, whatever the draft holds.
pub fn save(
    config: &Path,
    dir_before: Option<&Path>,
    layout: &Layout,
    notes: &Notes,
    text: &str,
    had_notes: bool,
    unusable: Option<&str>,
) -> (bool, Option<String>) {
    if let Some(why) = unusable {
        return (
            false,
            Some(format!(
                "{FILE} was not replaced ({why}), so what it holds is kept; fix or delete it, then Reload"
            )),
        );
    }
    let kept = layout.of_nodes_in(text);
    if kept.0.is_empty() && notes.is_empty() && !had_notes {
        return (
            false,
            Some(format!("nothing to save; {FILE} left as it was")),
        );
    }
    let not = |why: String| (false, Some(format!("{FILE} was not written: {why}")));
    let dir = config.parent().and_then(|d| d.canonicalize().ok());
    let Some(dir) = dir.filter(|d| Some(d.as_path()) == dir_before) else {
        return not("the config's folder changed during Save".into());
    };
    let path = dir.join(FILE);
    match std::fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        // `commit` reads what it replaces (its undo copy): never a file bigger than we would load
        Ok(m) if m.file_type().is_file() && m.len() > MAX_FILE_BYTES => {
            return not(format!(
                "the existing file is larger than {} KiB, so it is not replaced",
                MAX_FILE_BYTES / 1024
            ));
        }
        Ok(m) if m.file_type().is_file() => {}
        Ok(_) => return not("it is a link or not a regular file, so it is not replaced".into()),
        Err(e) => return not(e.to_string()),
    }
    // the rename replaces the name itself: even a link planted after the check is not followed
    match commit(
        &[Change::Write(path, file_text(&kept, notes))],
        |from, to| std::fs::rename(from, to),
    ) {
        Ok(()) => (true, None),
        Err(e) => not(e.to_string()),
    }
}

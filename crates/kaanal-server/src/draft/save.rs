//! Save: writes the draft text to the config file. Until Save the disk is untouched; Save refuses
//! when the file changed since we read it (an editor or `git checkout` got there first) and when
//! a patch could not keep something and the caller has not said that is fine. Scenario files the
//! draft no longer resolves against can be removed in the same Save; every file changes or none.

use super::Draft;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::PoisonError;

/// Why Save wrote nothing.
#[derive(Debug)]
pub enum SaveError {
    /// Patches dropped these (comments, anchors) and `confirm_loss` was not set.
    Lost(Vec<String>),
    /// The file on disk is not what we read: it was changed, deleted or created outside Studio.
    Conflict,
    /// The server was not started from a config file.
    NoFile,
    /// The caller's plan for the scenario files could not be made (the draft does not build).
    Plan(String),
    /// A write failed. Every file is put back; if even that failed the message says which.
    Io(String),
}

/// One change to the disk.
#[derive(Debug, Clone)]
pub enum Change {
    /// Replace (or create) a file with this text.
    Write(PathBuf, String),
    /// Remove a file (already gone is fine).
    Delete(PathBuf),
}

/// What Save does besides writing the config, chosen for the text being saved.
#[derive(Debug, Default)]
pub struct Plan {
    /// Scenario files to delete.
    pub remove: Vec<PathBuf>,
    /// Scenario files to replace, with their new text.
    pub rewrite: Vec<(PathBuf, String)>,
}

/// What Save did, as paths relative to the project.
#[derive(Debug, Default)]
pub struct Saved {
    /// Files written.
    pub written: Vec<String>,
    /// Files removed.
    pub removed: Vec<String>,
    /// Whether the layout sidecar was written (after the config; see [`super::layout::save`]).
    pub layout_written: bool,
    /// Why it was not, when it was not. The config write stands either way.
    pub layout_note: Option<String>,
}

/// Writes the draft to its file, after the deletions and rewrites `plan` chooses, and returns what
/// was done. `plan` gets the text being saved and the renames made since it was set, and names
/// the scenario files to delete or rewrite (so the choice is made for exactly the text that is
/// written).
///
/// The draft lock is held only to read and to record the outcome, never across disk work (the
/// control routes take it on the async threads); `once` makes Saves one at a time. A `PUT` that
/// lands during a Save is not lost: the saved text is what was checked, and the draft keeps the
/// newer one (still different from the file, so the next Save writes it).
///
/// # Errors
/// See [`SaveError`]. Nothing is changed for any of them; `Io` puts every file back first.
pub fn save(
    draft: &std::sync::Mutex<Draft>,
    once: &std::sync::Mutex<()>,
    confirm_loss: bool,
    plan: impl FnOnce(&str, &[(String, String)]) -> Result<Plan, String>,
) -> Result<Saved, SaveError> {
    let _one = once.lock().unwrap_or_else(PoisonError::into_inner);
    let (file, text, base, lost, renames, layout) = {
        let d = draft.lock().unwrap_or_else(PoisonError::into_inner);
        let file = d.file.clone().ok_or(SaveError::NoFile)?;
        (
            file,
            d.text.clone(),
            d.base.clone(),
            d.lost.clone(),
            d.renames.clone(),
            d.layout.clone(),
        )
    };
    if !lost.is_empty() && !confirm_loss {
        return Err(SaveError::Lost(lost));
    }
    // a config that is a link (a dotfile manager's) is written through, not replaced by a file
    let target = file.canonicalize().unwrap_or_else(|_| file.clone());
    let on_disk = match std::fs::read_to_string(&target) {
        Ok(t) => Some(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(SaveError::Io(e.to_string())),
    };
    if on_disk != base {
        return Err(SaveError::Conflict);
    }
    let Plan { remove, rewrite } = plan(&text, &renames).map_err(SaveError::Plan)?;
    // the config goes last: if anything before it fails nothing refers to a missing file
    let mut changes: Vec<Change> = remove.iter().cloned().map(Change::Delete).collect();
    changes.extend(rewrite.iter().cloned().map(|(p, t)| Change::Write(p, t)));
    changes.push(Change::Write(target, text.clone()));
    // the folder the sidecar goes in, as it resolves before anything is written
    let dir_before = file.parent().and_then(|d| d.canonicalize().ok());
    commit(&changes, |from, to| std::fs::rename(from, to))
        .map_err(|e| SaveError::Io(e.to_string()))?;
    // after the config: a sidecar that cannot be written is a note, the config stands
    let (layout_written, layout_note) =
        super::layout::save(&file, dir_before.as_deref(), &layout, &text);
    {
        let mut d = draft.lock().unwrap_or_else(PoisonError::into_inner);
        if layout_written {
            d.layout_note = None;
            d.saved_layout = layout;
        }
        if d.text == text {
            d.lost.clear();
            if !rewrite.is_empty() {
                d.renames.clear();
            }
        }
        d.base = Some(text);
    }
    let name = |p: &Path| {
        p.file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
    };
    let shown = |p: &PathBuf| match p.parent().and_then(Path::file_name) {
        Some(dir) => format!("{}/{}", dir.to_string_lossy(), name(p)),
        None => name(p),
    };
    let mut written = vec![name(&file)];
    written.extend(rewrite.iter().map(|(p, _)| shown(p)));
    Ok(Saved {
        written,
        removed: remove.iter().map(shown).collect(),
        layout_written,
        layout_note,
    })
}

/// **Private.** Makes temp names differ within one process.
static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// **Private.** The temp file for `path`: same folder (so the rename stays on one filesystem),
/// written and flushed to disk, with the target's permissions.
fn stage(path: &Path, text: &str) -> std::io::Result<PathBuf> {
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    // pid, clock and a counter: not a name another user can guess, and `create_new` (O_EXCL)
    // never follows or reuses something already at that path, a planted link included
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = path.with_file_name(format!(
        ".{name}.kaanal-{}-{nanos:x}-{n}.tmp",
        std::process::id()
    ));
    let made = (|| {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        if let Ok(m) = std::fs::metadata(path) {
            f.set_permissions(m.permissions())?;
        }
        Ok(())
    })();
    if made.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    made.map(|()| tmp)
}

/// Makes every change or none. All the new files are written to temp files first (a failure
/// there changes nothing); then the changes are made in order, each remembering what it replaced;
/// if one fails the ones already made are put back and the temp files go. A reader of a single
/// file sees the old one or the new one, never half. `rename` is a parameter so a test can fail it.
///
/// # Errors
/// The first failure, after everything is restored; a file that could not be put back is named in
/// the error. The temp files are removed either way.
pub fn commit(
    changes: &[Change],
    rename: impl Fn(&Path, &Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let mut staged: Vec<Option<PathBuf>> = Vec::new();
    for c in changes {
        match c {
            Change::Write(p, t) => match stage(p, t) {
                Ok(tmp) => staged.push(Some(tmp)),
                Err(e) => {
                    staged
                        .iter()
                        .flatten()
                        .for_each(|t| drop(std::fs::remove_file(t)));
                    return Err(e);
                }
            },
            Change::Delete(_) => staged.push(None),
        }
    }
    // (path, what it held before: None when it was absent)
    let mut undo: Vec<(PathBuf, Option<Vec<u8>>)> = Vec::new();
    let mut failed = None;
    for (c, tmp) in changes.iter().zip(&staged) {
        let (path, step) = match (c, tmp) {
            (Change::Write(p, _), Some(t)) => (p, Some(t)),
            (Change::Delete(p), _) => (p, None),
            (Change::Write(..), None) => unreachable!("every write is staged"),
        };
        // what a failure must put back: unreadable (not just absent) stops here, before the change
        let before = match std::fs::read(path) {
            Ok(b) => Some(b),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => {
                failed = Some(e);
                break;
            }
        };
        let r = match step {
            Some(t) => rename(t, path),
            None => match std::fs::remove_file(path) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                other => other,
            },
        };
        match r {
            Ok(()) => undo.push((path.clone(), before)),
            Err(e) => {
                failed = Some(e);
                break;
            }
        }
    }
    if let Some(e) = failed {
        let mut stuck = Vec::new();
        for (path, before) in undo.into_iter().rev() {
            let put_back = match before {
                Some(bytes) => std::fs::write(&path, bytes),
                None => std::fs::remove_file(&path),
            };
            if let Err(x) = put_back {
                stuck.push(format!(
                    "{} ({x})",
                    path.file_name()
                        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
                ));
            }
        }
        staged
            .iter()
            .flatten()
            .for_each(|t| drop(std::fs::remove_file(t)));
        if !stuck.is_empty() {
            return Err(std::io::Error::other(format!(
                "{e}; and these could not be put back: {}",
                stuck.join(", ")
            )));
        }
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kaanal-save-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn names(d: &Path) -> Vec<String> {
        let mut v: Vec<_> = std::fs::read_dir(d)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn a_failure_between_write_and_rename_leaves_the_old_file_and_no_temp() {
        let d = dir("fail");
        let f = d.join("kaanal.yaml");
        std::fs::write(&f, "old\n").unwrap();
        let err = commit(&[Change::Write(f.clone(), "new\n".into())], |_, _| {
            Err(std::io::Error::other("boom"))
        });
        assert!(err.is_err());
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "old\n");
        assert_eq!(names(&d), ["kaanal.yaml"]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_good_write_replaces_the_file_whole_and_leaves_no_temp() {
        let d = dir("ok");
        let f = d.join("kaanal.yaml");
        std::fs::write(&f, "old\n").unwrap();
        commit(
            &[Change::Write(f.clone(), "new\r\ntext\n".into())],
            |a, b| std::fs::rename(a, b),
        )
        .unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), b"new\r\ntext\n");
        assert_eq!(names(&d), ["kaanal.yaml"]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn every_change_is_made_or_none() {
        let d = dir("multi");
        let (a, b, gone) = (d.join("a.yaml"), d.join("b.yaml"), d.join("gone.yaml"));
        std::fs::write(&a, "a-old").unwrap();
        std::fs::write(&gone, "gone-old").unwrap();
        // the second rename fails: the delete that came first is undone, and a is untouched
        let calls = std::cell::Cell::new(0);
        let r = commit(
            &[
                Change::Delete(gone.clone()),
                Change::Write(a.clone(), "a-new".into()),
                Change::Write(b.clone(), "b-new".into()),
            ],
            |from, to| {
                calls.set(calls.get() + 1);
                if calls.get() == 2 {
                    return Err(std::io::Error::other("boom"));
                }
                std::fs::rename(from, to)
            },
        );
        assert!(r.is_err());
        assert_eq!(
            std::fs::read_to_string(&a).unwrap(),
            "a-new".replace("new", "old")
        );
        assert_eq!(std::fs::read_to_string(&gone).unwrap(), "gone-old");
        assert!(!b.exists());
        assert_eq!(names(&d), ["a.yaml", "gone.yaml"], "no temp file is left");
        // and when nothing fails, all of them happen
        commit(
            &[
                Change::Delete(gone.clone()),
                Change::Write(a.clone(), "a-new".into()),
                Change::Write(b.clone(), "b-new".into()),
            ],
            |from, to| std::fs::rename(from, to),
        )
        .unwrap();
        assert_eq!(names(&d), ["a.yaml", "b.yaml"]);
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "b-new");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_file_that_cannot_be_read_stops_the_commit_and_restores_what_it_did() {
        let d = dir("unreadable");
        let (gone, folder) = (d.join("gone.yaml"), d.join("folder"));
        std::fs::write(&gone, "gone-old").unwrap();
        std::fs::create_dir(&folder).unwrap();
        // reading a folder fails with something other than NotFound
        let r = commit(
            &[
                Change::Delete(gone.clone()),
                Change::Write(folder.clone(), "x".into()),
            ],
            |from, to| std::fs::rename(from, to),
        );
        assert!(r.is_err());
        assert_eq!(std::fs::read_to_string(&gone).unwrap(), "gone-old");
        assert!(folder.is_dir(), "the folder was not replaced");
        assert_eq!(names(&d), ["folder", "gone.yaml"], "no temp file is left");
        let _ = std::fs::remove_dir_all(&d);
    }

    fn draft_for(file: PathBuf, text: &str) -> std::sync::Mutex<Draft> {
        std::sync::Mutex::new(Draft::from_source(Some(super::super::DraftSource {
            file,
            text: text.into(),
        })))
    }

    #[test]
    fn a_put_during_a_save_is_kept_and_the_saved_text_is_what_was_checked() {
        let d = dir("race");
        let f = d.join("kaanal.yaml");
        std::fs::write(&f, "v1\n").unwrap();
        let draft = draft_for(f.clone(), "v1\n");
        draft.lock().unwrap().text = "v2\n".into();
        draft.lock().unwrap().lost = vec!["a comment".into()];
        let once = std::sync::Mutex::new(());
        // the plan runs after the snapshot: a PUT lands right then
        let r = save(&draft, &once, true, |text, _| {
            assert_eq!(text, "v2\n");
            draft.lock().unwrap().text = "v3\n".into();
            Ok(Plan::default())
        });
        assert!(r.is_ok());
        assert_eq!(
            std::fs::read_to_string(&f).unwrap(),
            "v2\n",
            "what was checked is what is written"
        );
        {
            let g = draft.lock().unwrap();
            assert_eq!((g.text.as_str(), g.base.as_deref()), ("v3\n", Some("v2\n")));
            assert_eq!(g.lost, ["a comment"], "the newer text still owes its loss");
        }
        // the next Save writes the newer text without a conflict
        assert!(save(&draft, &once, true, |_, _| Ok(Plan::default())).is_ok());
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "v3\n");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_plan_that_fails_writes_nothing() {
        let d = dir("plan");
        let f = d.join("kaanal.yaml");
        std::fs::write(&f, "v1\n").unwrap();
        let draft = draft_for(f.clone(), "v1\n");
        draft.lock().unwrap().text = "v2\n".into();
        let once = std::sync::Mutex::new(());
        let r = save(&draft, &once, false, |_, _| Err("no build".into()));
        assert!(matches!(r, Err(SaveError::Plan(m)) if m == "no build"));
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "v1\n");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(unix)]
    #[test]
    fn a_config_that_is_a_link_is_written_through() {
        let d = dir("link");
        let real = d.join("real.yaml");
        let link = d.join("kaanal.yaml");
        std::fs::write(&real, "v1\n").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let draft = draft_for(link.clone(), "v1\n");
        draft.lock().unwrap().text = "v2\n".into();
        let once = std::sync::Mutex::new(());
        assert!(save(&draft, &once, false, |_, _| Ok(Plan::default())).is_ok());
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "v2\n");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn temp_names_are_unique_and_never_reuse_what_is_there() {
        let d = dir("tmpname");
        let f = d.join("kaanal.yaml");
        let (a, b) = (stage(&f, "a").unwrap(), stage(&f, "b").unwrap());
        assert_ne!(a, b);
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "a");
        // create_new refuses a path that already exists (a planted file or link)
        let planted = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&a);
        assert!(planted.is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_failed_stage_changes_nothing() {
        let d = dir("stage");
        let a = d.join("a.yaml");
        std::fs::write(&a, "a-old").unwrap();
        // a write into a folder that is not there fails while staging
        let bad = d.join("nope").join("b.yaml");
        let r = commit(
            &[
                Change::Write(a.clone(), "a-new".into()),
                Change::Write(bad, "b".into()),
            ],
            |from, to| std::fs::rename(from, to),
        );
        assert!(r.is_err());
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "a-old");
        assert_eq!(names(&d), ["a.yaml"]);
        let _ = std::fs::remove_dir_all(&d);
    }
}

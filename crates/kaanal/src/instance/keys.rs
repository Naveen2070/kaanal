//! The per-project signing key on disk (spec X4, X6, X14-X16). IO only: generating, parsing
//! and signing live in `kaanal_server::AuthKey`.
//!
//! The key lives at `<KAANAL_CONFIG_DIR or the per-user local config dir>/kaanal/keys/<id>.pem`,
//! where `<id>` is 16 hex digits of the SHA-256 of the canonical directory of `kaanal.yaml`
//! (moving a project silently starts a new key). The directory is made private with the
//! `.kaanal` code (R5.8). A new key goes to a unique temp file that is hard-linked into place,
//! so the final name never holds half a key, and two `up`s racing on one project adopt one key.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use kaanal_server::AuthKey;
use sha2::{Digest, Sha256};

use crate::{Fail, err, private_dir};

/// The project's signing key: read from its file, or generated and stored when there is
/// none (`Generating signing key...` on stdout). A corrupt, unreadable or (Unix) group- or
/// other-readable file stops startup and is never replaced (X4, X16).
pub(crate) fn signing_key(config: &Path) -> Result<AuthKey, Fail> {
    let dir = keys_dir(
        std::env::var_os("KAANAL_CONFIG_DIR"),
        dirs::config_local_dir(),
    )?;
    private_dir(&dir)?;
    let file = dir.join(format!("{}.pem", key_id(config)?));
    // another `up` may be writing the key in place (no hard links here): never read half of it
    wait_unlocked(&file, LOCK_WAIT);
    match std::fs::read_to_string(&file) {
        Ok(pem) => parse(&file, &pem),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Off stdout under --json, where `demo` prints exactly one object.
            if crate::output::json_on() {
                eprintln!("Generating signing key...");
            } else {
                println!("Generating signing key...");
            }
            let key = AuthKey::generate().map_err(err)?;
            let pem = key.to_pem().map_err(err)?;
            let stored = store(&dir, &file, &pem)?;
            if stored == pem {
                Ok(key)
            } else {
                // Lost the race: a simultaneous `up` of this project stored its key first.
                println!("Another kaanal start created the signing key first; using it.");
                parse(&file, &stored)
            }
        }
        Err(e) => Err(err(format!(
            "{}: cannot read the signing key: {e}",
            file.display()
        ))),
    }
}

/// **Private.** `<base>/kaanal/keys`, where `base` is `over` (`KAANAL_CONFIG_DIR`; empty
/// counts as unset) or `local` (`dirs::config_local_dir()`: `LocalAppData` on Windows, so the
/// key never roams with the profile).
fn keys_dir(over: Option<OsString>, local: Option<PathBuf>) -> Result<PathBuf, Fail> {
    over.filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or(local)
        .map(|base| base.join("kaanal").join("keys"))
        .ok_or_else(|| {
            err("no per-user config directory for the signing key; set KAANAL_CONFIG_DIR to a private directory")
        })
}

/// **Private.** The key file's name (X14): the first 16 hex digits of the SHA-256 of the
/// canonical directory of `config` (UTF-8, lossy), without Windows' `\\?\` prefix.
fn key_id(config: &Path) -> Result<String, Fail> {
    let dir = config
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let canon = std::fs::canonicalize(dir).map_err(|e| err(format!("{}: {e}", dir.display())))?;
    let text = canon.to_string_lossy();
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    Ok(Sha256::digest(text.as_bytes())
        .iter()
        .fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })[..16]
        .to_string())
}

/// **Private.** Puts `pem` at `file` without ever exposing half a key (X15): a unique temp
/// file in `dir` (`create_new`; mode 0600 on Unix), written and synced, then hard-linked to
/// `file`. If another `up` linked first, its key is adopted. Returns the PEM `file` holds.
fn store(dir: &Path, file: &Path, pem: &str) -> Result<String, Fail> {
    let stem = file
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    // pid + clock names the temp file; a crash may leave one behind, which is ignored (spec
    // section 10 Q4).
    let tmp = dir.join(format!("{stem}.{}-{nanos}.tmp", std::process::id()));
    let linked = write_new(&tmp, pem).and_then(|()| match std::fs::hard_link(&tmp, file) {
        // No hard links here (FAT, some network shares): create the final name directly.
        Err(e) if e.kind() != std::io::ErrorKind::AlreadyExists => {
            create_locked(file, pem, LOCK_WAIT)
        }
        r => r,
    });
    let _ = std::fs::remove_file(&tmp);
    match linked {
        Ok(()) => Ok(pem.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => std::fs::read_to_string(file)
            .map_err(|e| {
                err(format!(
                    "{}: cannot read the signing key: {e}",
                    file.display()
                ))
            }),
        Err(e) => Err(err(format!(
            "{}: cannot store the signing key: {e}",
            file.display()
        ))),
    }
}

/// How long a key file's lock is waited for. A lock older than this was left by a process that died
/// holding it, and is removed.
const LOCK_WAIT: Duration = Duration::from_secs(5);

/// **Private.** `<file>.lock`: present while one `up` writes the key into place.
fn lock_path(file: &Path) -> PathBuf {
    let mut name = file.as_os_str().to_owned();
    name.push(".lock");
    PathBuf::from(name)
}

/// **Private.** Waits until no other process holds `file`'s lock, for at most `wait`; a lock that old
/// is treated as left behind and removed.
fn wait_unlocked(file: &Path, wait: Duration) {
    let lock = lock_path(file);
    let start = Instant::now();
    while lock.exists() {
        let age = std::fs::metadata(&lock)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok());
        if age.is_some_and(|a| a >= wait) || start.elapsed() >= wait {
            let _ = std::fs::remove_file(&lock);
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// **Private.** The fallback when `hard_link` is unsupported (FAT, some shares): [`create_direct`]
/// under an exclusive lock file, so a second `up` waits for the first to finish and then adopts its
/// key (`AlreadyExists`) instead of reading half of it.
fn create_locked(file: &Path, pem: &str, wait: Duration) -> std::io::Result<()> {
    let lock = lock_path(file);
    for _ in 0..3 {
        let mut opts = std::fs::OpenOptions::new();
        match opts.write(true).create_new(true).open(&lock) {
            Ok(held) => {
                drop(held);
                let made = create_direct(file, pem);
                let _ = std::fs::remove_file(&lock);
                return made;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                wait_unlocked(file, wait);
                if file.exists() {
                    return Err(std::io::ErrorKind::AlreadyExists.into());
                }
                // the holder died before it wrote anything: take the lock ourselves
            }
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("the key file stayed locked"))
}

/// **Private.** Creates `file` (which must not exist; an existing one is `AlreadyExists`, so the
/// caller adopts it) and writes the key. A failed write removes the partial file again. Callers
/// hold the file's lock ([`create_locked`]), so no one else reads it while it is written.
fn create_direct(file: &Path, pem: &str) -> std::io::Result<()> {
    write_new(file, pem).inspect_err(|e| {
        if e.kind() != std::io::ErrorKind::AlreadyExists {
            let _ = std::fs::remove_file(file);
        }
    })
}

/// **Private.** Creates `path`, which must not exist (mode 0600 on Unix, fail closed), with
/// `text`, synced to disk.
fn write_new(path: &Path, text: &str) -> std::io::Result<()> {
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    f.write_all(text.as_bytes())?;
    f.sync_all()
}

/// **Private.** The key in `file`, whose text is `pem`. On Unix a file that group or others
/// can read is refused (X16). Text that is not a key is an error naming the file; the file is
/// never replaced (X4).
fn parse(file: &Path, pem: &str) -> Result<AuthKey, Fail> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(file)
            .map_err(|e| err(format!("{}: {e}", file.display())))?
            .permissions()
            .mode();
        if mode & 0o077 != 0 {
            return Err(err(format!(
                "{}: the signing key can be read by other users (mode {:o}); run: chmod 600 {}",
                file.display(),
                mode & 0o777,
                file.display()
            )));
        }
    }
    AuthKey::from_pem(pem).map_err(|e| {
        err(format!(
            "{}: {e}; delete the file to generate a new key",
            file.display()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kaanal-keys-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_fresh_locked_create_writes_the_key_and_leaves_no_lock() {
        let d = scratch("lock-fresh");
        let file = d.join("k.pem");
        create_locked(&file, "KEY", Duration::from_secs(2)).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "KEY");
        assert!(!lock_path(&file).exists());
    }

    #[test]
    fn a_second_writer_waits_for_the_lock_then_adopts_the_first_key() {
        let d = scratch("lock-wait");
        let file = d.join("k.pem");
        let lock = lock_path(&file);
        std::fs::write(&lock, "").unwrap();
        let (f2, l2) = (file.clone(), lock.clone());
        let holder = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            write_new(&f2, "FIRST").unwrap();
            std::fs::remove_file(&l2).unwrap();
        });
        let r = create_locked(&file, "SECOND", Duration::from_secs(5));
        holder.join().unwrap();
        assert_eq!(r.unwrap_err().kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "FIRST");
    }

    #[test]
    fn a_lock_left_by_a_dead_process_is_removed_and_the_key_is_written() {
        let d = scratch("lock-stale");
        let file = d.join("k.pem");
        let lock = lock_path(&file);
        let held = std::fs::File::create(&lock).unwrap();
        held.set_modified(SystemTime::now() - Duration::from_secs(60))
            .unwrap();
        drop(held);
        create_locked(&file, "KEY", Duration::from_millis(100)).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "KEY");
        assert!(!lock_path(&file).exists());
    }

    #[test]
    fn waiting_for_no_lock_returns_at_once() {
        let d = scratch("lock-none");
        let t = Instant::now();
        wait_unlocked(&d.join("k.pem"), Duration::from_secs(5));
        assert!(t.elapsed() < Duration::from_millis(500));
    }

    #[test]
    fn keys_dir_prefers_the_override_and_needs_one_of_them() {
        let local = Some(PathBuf::from("L"));
        let keys = |base: &str| Some(Path::new(base).join("kaanal").join("keys"));
        assert_eq!(keys_dir(Some("O".into()), local.clone()).ok(), keys("O"));
        assert_eq!(
            keys_dir(Some(OsString::new()), local.clone()).ok(),
            keys("L"),
            "an empty KAANAL_CONFIG_DIR counts as unset"
        );
        assert_eq!(keys_dir(None, local).ok(), keys("L"));
        let Err(Fail(code, msg)) = keys_dir(None, None) else {
            panic!("no directory must be an error")
        };
        assert_eq!(code, 1);
        assert!(msg.contains("set KAANAL_CONFIG_DIR"), "{msg}");
    }

    #[test]
    fn key_id_hashes_the_canonical_config_directory() {
        let d = scratch("id");
        std::fs::create_dir_all(d.join("sub")).unwrap();
        let a = key_id(&d.join("kaanal.yaml")).ok().unwrap();
        let b = key_id(&d.join("sub").join("..").join("other.yaml"))
            .ok()
            .unwrap();
        assert_eq!(a, b, "same directory, same key");
        let canon = std::fs::canonicalize(&d).unwrap();
        let text = canon.to_string_lossy();
        let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
        assert_eq!(
            a,
            Sha256::digest(text.as_bytes())
                .iter()
                .fold(String::new(), |mut s, b| {
                    let _ = write!(s, "{b:02x}");
                    s
                })[..16]
        );
        assert_ne!(
            key_id(&d.join("sub").join("kaanal.yaml")).ok().unwrap(),
            a,
            "another directory, another key"
        );
    }

    #[test]
    fn store_adopts_the_key_that_linked_first() {
        let d = scratch("store");
        let file = d.join("abc.pem");
        assert_eq!(store(&d, &file, "A").ok().as_deref(), Some("A"));
        assert_eq!(
            store(&d, &file, "B").ok().as_deref(),
            Some("A"),
            "the loser of a race adopts the winner's key"
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "A");
        let left: Vec<OsString> = std::fs::read_dir(&d)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(
            left,
            [OsString::from("abc.pem")],
            "no temp file left behind"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn the_no_hard_link_fallback_creates_once_and_never_overwrites() {
        let d = scratch("direct");
        let file = d.join("abc.pem");
        create_direct(&file, "A").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "A");
        let e = create_direct(&file, "B").unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "A",
            "the winner's key is kept and not removed by the failed attempt"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn a_corrupt_key_file_is_an_error_naming_it() {
        let d = scratch("corrupt");
        let file = d.join("abc.pem");
        std::fs::write(&file, "not a key").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let Err(Fail(_, msg)) = parse(&file, "not a key") else {
            panic!("a corrupt key must be an error")
        };
        assert!(
            msg.starts_with(&file.display().to_string())
                && msg.contains("delete the file to generate a new key"),
            "{msg}"
        );
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "not a key",
            "never replaced"
        );
    }
}

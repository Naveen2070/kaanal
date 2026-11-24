//! Unit tests for the instance lifecycle.

use super::*;

#[cfg(windows)]
#[test]
fn rotate_log_truncates_when_the_old_log_cannot_be_renamed() {
    use std::os::windows::fs::OpenOptionsExt;
    let d = dir("rot-locked");
    // read+write sharing but no FILE_SHARE_DELETE: renaming the file fails
    let mut held = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(3)
        .open(d.join("kaanal.log"))
        .unwrap();
    std::io::Write::write_all(&mut held, b"old").unwrap();
    let log = rotate_log(&d).unwrap();
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "");
    assert!(!d.join("kaanal.log.1").exists());
}

fn fake_run(pid: u32) -> Run {
    Run {
        addr: "127.0.0.1:1".parse().unwrap(),
        token: "t".into(),
        pid: Some(pid),
        startup: Startup::default(),
    }
}

#[test]
fn settle_unready_keeps_a_healthy_child_and_otherwise_always_ends_it() {
    use std::cell::Cell;
    let (shut, killed) = (Cell::new(false), Cell::new(false));
    let waited = Cell::new(false);
    let go = |healthy: bool, own: bool| {
        shut.set(false);
        killed.set(false);
        waited.set(false);
        settle_unready(
            || healthy.then(|| fake_run(1)),
            || own.then(|| fake_run(1)),
            |_| shut.set(true),
            || waited.set(true),
            || killed.set(true),
        )
    };
    assert!(matches!(go(true, true), Settled::Healthy(_)));
    assert!(!shut.get() && !killed.get());
    // own run.json: asked to shut down, waited for, and killed anyway (a slow exit must not hang us)
    assert!(matches!(go(false, true), Settled::Stopped));
    assert!(shut.get() && waited.get() && killed.get());
    // no (or foreign) run.json: no shutdown, killed
    go(false, false);
    assert!(!shut.get() && !waited.get() && killed.get());
}

#[test]
fn existing_instance_waits_on_a_busy_one_and_decides_on_free_or_healthy() {
    let mut seq = vec![Look::Busy, Look::Busy, Look::Healthy(fake_run(9))];
    let r = existing_instance(Duration::from_secs(5), Duration::from_millis(1), || {
        Ok(seq.remove(0))
    });
    assert!(matches!(r, Ok(Existing::Running(r)) if r.pid == Some(9)));
    let mut seq = vec![Look::Busy, Look::Free];
    let r = existing_instance(Duration::from_secs(5), Duration::from_millis(1), || {
        Ok(seq.remove(0))
    });
    assert!(matches!(r, Ok(Existing::Free)));
    let r = existing_instance(Duration::from_millis(30), Duration::from_millis(5), || {
        Ok(Look::Busy)
    });
    let Err(Fail(1, m)) = r else {
        panic!("a busy instance at the deadline is an error");
    };
    assert!(m.contains("not answering"), "{m}");
}

#[test]
fn server_waits_are_clamped() {
    assert_eq!(server_wait(Some(u64::MAX), 5000), Duration::from_secs(310));
    assert_eq!(server_wait(Some(1500), 5000), Duration::from_millis(1500));
    assert_eq!(server_wait(None, 5000), Duration::from_millis(5000));
    // adding it to a clock never overflows
    assert!(
        Instant::now()
            .checked_add(server_wait(Some(u64::MAX), 0))
            .is_some()
    );
}

#[test]
fn after_child_exit_reports_a_busy_winner_instead_of_a_plain_exit() {
    let poll = Duration::from_millis(1);
    let r = after_child_exit(Duration::ZERO, poll, || Ok(Look::Busy));
    let Err(Fail(1, m)) = r else {
        panic!("busy at the deadline is an error");
    };
    assert!(m.contains("not answering"), "{m}");
    assert!(
        after_child_exit(Duration::ZERO, poll, || Ok(Look::Free))
            .unwrap()
            .is_none()
    );
    let r = after_child_exit(Duration::ZERO, poll, || Ok(Look::Healthy(fake_run(3))));
    assert_eq!(r.unwrap().unwrap().pid, Some(3));
}

#[test]
fn rotate_log_replaces_the_old_backup() {
    let d = dir("rot");
    std::fs::write(d.join("kaanal.log"), "second").unwrap();
    std::fs::write(d.join("kaanal.log.1"), "first").unwrap();
    let log = rotate_log(&d).unwrap();
    assert_eq!(log, d.join("kaanal.log"));
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "");
    assert_eq!(
        std::fs::read_to_string(d.join("kaanal.log.1")).unwrap(),
        "second"
    );
    // no previous log: just a fresh one
    let d = dir("rot-none");
    assert!(rotate_log(&d).unwrap().exists());
    assert!(!d.join("kaanal.log.1").exists());
}

#[test]
fn tail_log_returns_the_last_n_cleaned_lines() {
    let d = dir("tail");
    let f = d.join("l");
    std::fs::write(&f, b"a\nb\x1b[31m\nc\xff\nd\n").unwrap();
    assert_eq!(tail_log(&f, 3), "b?[31m\nc\u{fffd}\nd");
    assert_eq!(tail_log(&f, 99).lines().count(), 4);
    assert_eq!(tail_log(&d.join("missing"), 5), "");
}

#[test]
fn wait_ready_times_out_with_a_fake_poll() {
    let mut n = 0;
    let r = poll_until(Duration::from_millis(40), Duration::from_millis(5), || {
        n += 1;
        Step::Pending
    });
    assert_eq!(r.unwrap_err(), ReadyError::TimedOut);
    assert!(n >= 2);
    let r = poll_until(Duration::from_secs(60), Duration::from_secs(60), || {
        Step::Exited
    });
    assert_eq!(r.unwrap_err(), ReadyError::ChildExited);
    let mut left = 2;
    let r = poll_until(Duration::from_secs(60), Duration::from_millis(1), || {
        left -= 1;
        if left == 0 {
            Step::Ready(Run {
                addr: "127.0.0.1:1".parse().unwrap(),
                token: "t".into(),
                pid: None,
                startup: Startup::default(),
            })
        } else {
            Step::Pending
        }
    });
    assert_eq!(r.unwrap().token, "t");
}
use std::fs::File;
use std::sync::atomic::{AtomicU32, Ordering};

/// A scratch dir removed on drop.
struct Tmp(std::path::PathBuf);

impl std::ops::Deref for Tmp {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn dir(name: &str) -> Tmp {
    static N: AtomicU32 = AtomicU32::new(0);
    let d = std::env::temp_dir().join(format!(
        "kaanal-life-{name}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    Tmp(d)
}

fn hold(d: &Path) -> File {
    let f = File::create(d.join("lock")).unwrap();
    f.try_lock().unwrap();
    f
}

fn write_run(d: &Path, token: &str) {
    std::fs::write(
        d.join("run.json"),
        format!(r#"{{"host":"127.0.0.1","port":4000,"token":"{token}","pid":77}}"#),
    )
    .unwrap();
}

const SHORT: Duration = Duration::from_millis(300);

fn code(r: Result<Run, Fail>) -> (u8, String) {
    match r {
        Err(Fail(c, m)) => (c, m),
        Ok(_) => panic!("expected a failure"),
    }
}

#[test]
fn lock_probe_is_free_without_a_lock_file() {
    assert_eq!(lock_probe(&dir("nolock")).unwrap(), Liveness::Free);
}

#[test]
fn lock_probe_is_running_while_another_handle_holds_it() {
    let d = dir("held");
    let _h = hold(&d);
    assert_eq!(lock_probe(&d).unwrap(), Liveness::Running);
}

#[test]
fn lock_probe_is_free_after_the_holder_drops() {
    let d = dir("dropped");
    drop(hold(&d));
    assert_eq!(lock_probe(&d).unwrap(), Liveness::Free);
}

#[test]
fn lock_probe_reports_an_error_instead_of_free() {
    // `lock` is a directory: opening it for locking fails, and that must not read as "stale".
    let d = dir("lockdir");
    std::fs::create_dir(d.join("lock")).unwrap();
    let Err(Fail(c, _)) = lock_probe(&d) else {
        panic!("an unusable lock must be an error");
    };
    assert_eq!(c, 1);
}

#[test]
fn read_run_wait_sees_a_file_that_appears_late() {
    let d = dir("late");
    let d2 = d.to_path_buf();
    let t = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(250));
        write_run(&d2, "tok");
    });
    let run = read_run_wait(&d, Duration::from_secs(10)).expect("file appears");
    assert_eq!(run.token, "tok");
    t.join().unwrap();
}

#[test]
fn read_run_wait_gives_up_after_the_budget() {
    assert!(read_run_wait(&dir("never"), SHORT).is_none());
}

#[test]
fn remove_stale_keeps_a_file_with_a_different_token() {
    let d = dir("tok");
    write_run(&d, "new");
    remove_stale(&d, "old");
    assert!(d.join("run.json").exists());
    remove_stale(&d, "new");
    assert!(!d.join("run.json").exists());
}

#[test]
fn require_instance_covers_every_row() {
    // free + file: stale, exit 2, file removed
    let d = dir("row-stale");
    write_run(&d, "t");
    let (c, m) = code(require_within(&d, SHORT));
    assert_eq!(c, 2);
    assert!(m.contains("removed stale state"), "{m}");
    assert!(!d.join("run.json").exists());
    // free + none: not running
    let (c, m) = code(require_within(&dir("row-none"), SHORT));
    assert_eq!(c, 2);
    assert!(m.contains("no .kaanal/run.json"), "{m}");
    // held + file: ok, and the file is never touched
    let d = dir("row-run");
    let _h = hold(&d);
    write_run(&d, "t");
    let run = require_within(&d, SHORT).unwrap();
    assert_eq!((run.token.as_str(), run.pid), ("t", Some(77)));
    assert!(d.join("run.json").exists());
    // held + none: starting, exit 1 after the budget
    let d = dir("row-start");
    let _h = hold(&d);
    let (c, m) = code(require_within(&d, SHORT));
    assert_eq!(c, 1);
    assert!(m.contains("starting or its state file is missing"), "{m}");
}

#[test]
fn unreadable_run_json_with_a_free_lock_is_removed_and_reported() {
    let d = dir("corrupt");
    std::fs::write(d.join("run.json"), "{ not json").unwrap();
    let (c, m) = code(require_within(&d, SHORT));
    assert_eq!(c, 2);
    assert!(m.contains("removed unreadable state"), "{m}");
    assert!(!d.join("run.json").exists());
}

#[test]
fn run_json_without_a_lock_file_is_stale_and_guarded() {
    let d = dir("nolock-file");
    write_run(&d, "t");
    let (c, m) = code(require_within(&d, SHORT));
    assert_eq!(c, 2);
    assert!(m.contains("removed stale state"), "{m}");
    assert!(!d.join("run.json").exists());
    assert!(d.join("lock").exists(), "the guard needed a lock file");
}

#[test]
fn a_free_probe_holds_the_shared_lock_until_dropped() {
    let d = dir("guard");
    std::fs::write(d.join("lock"), "").unwrap();
    let Probe::Free(Some(g)) = probe(&d).unwrap() else {
        panic!("free with a guard");
    };
    let f = std::fs::OpenOptions::new()
        .write(true)
        .open(d.join("lock"))
        .unwrap();
    assert!(matches!(
        f.try_lock(),
        Err(std::fs::TryLockError::WouldBlock)
    ));
    drop(g);
    assert!(f.try_lock().is_ok());
}

#[test]
fn transport_failure_removes_stale_state_when_the_lock_is_free() {
    let addr: SocketAddr = "127.0.0.1:4000".parse().unwrap();
    let d = dir("tf-stale");
    write_run(&d, "t");
    let Fail(c, _) = transport_failure(&d, addr, "refused");
    assert_eq!(c, 2);
    assert!(!d.join("run.json").exists());
}

#[test]
fn transport_failure_uses_the_lock() {
    let addr: SocketAddr = "127.0.0.1:4000".parse().unwrap();
    let d = dir("tf-free");
    let Fail(c, m) = transport_failure(&d, addr, "refused");
    assert_eq!(c, 2);
    assert!(m.contains("not running on 127.0.0.1:4000: refused"), "{m}");
    let d = dir("tf-held");
    let _h = hold(&d);
    write_run(&d, "t");
    let Fail(c, m) = transport_failure(&d, addr, "timed out");
    assert_eq!(c, 1);
    assert!(
        m.contains("running (pid 77)") && m.contains("timed out"),
        "{m}"
    );
}

#[test]
fn parse_tasklist_csv_reads_the_image_name() {
    let ok = "\"kaanal.exe\",\"1234\",\"Console\",\"1\",\"12,345 K\"\r\n";
    assert_eq!(parse_tasklist_csv(ok).as_deref(), Some("kaanal.exe"));
    let other = "\r\n\"ping.exe\",\"88\",\"Services\",\"0\",\"4,000 K\"\r\n";
    assert_eq!(parse_tasklist_csv(other).as_deref(), Some("ping.exe"));
    let none = "INFO: No tasks are running which match the specified criteria.\r\n";
    assert_eq!(parse_tasklist_csv(none), None);
    assert_eq!(parse_tasklist_csv(""), None);
}

#[test]
fn parse_ps_comm_trims_and_takes_the_basename() {
    assert_eq!(parse_ps_comm("kaanal\n").as_deref(), Some("kaanal"));
    assert_eq!(parse_ps_comm("  sleep \n").as_deref(), Some("sleep"));
    assert_eq!(
        parse_ps_comm("/usr/local/bin/kaanal\n").as_deref(),
        Some("kaanal")
    );
    assert_eq!(parse_ps_comm("\n"), None);
}

#[test]
fn verify_pid_accepts_this_executable_and_the_standard_name_only() {
    // The unit-test binary is `kaanal-<hash>`: it is kaanal only because it is current_exe.
    assert_eq!(verify_pid(std::process::id()), PidCheck::IsKaanal);
    assert!(is_kaanal_image("KAANAL.EXE", "kaanal.exe", "x", None));
    assert!(is_kaanal_image("kaanal", "kaanal", "", None));
    assert!(is_kaanal_image(
        "My-Kaanal.exe",
        "kaanal.exe",
        "my-kaanal",
        None
    ));
    assert!(!is_kaanal_image(
        "kaanal-1a2b.exe",
        "kaanal.exe",
        "kaanal",
        None
    ));
    assert!(!is_kaanal_image("ping.exe", "kaanal.exe", "", None));
}

#[test]
fn a_truncated_comm_matches_a_longer_own_stem_by_prefix() {
    let own = "kaanal-0123456789abcdef";
    // Linux comm: first 15 characters
    assert!(is_kaanal_image("kaanal-01234567", "kaanal", own, Some(15)));
    assert!(is_kaanal_image("KAANAL-01234567", "kaanal", own, Some(15)));
    // exact matching elsewhere
    assert!(!is_kaanal_image("kaanal-01234567", "kaanal", own, None));
    // another binary with a different prefix, or a shorter one, still does not match
    assert!(!is_kaanal_image("kaanal-99999999", "kaanal", own, Some(15)));
    assert!(!is_kaanal_image("kaanal-0123456", "kaanal", own, Some(15)));
}

#[test]
fn image_names_with_multibyte_text_do_not_panic() {
    assert!(!is_kaanal_image("日本語", "kaanal.exe", "", None));
    assert!(!is_kaanal_image("a日本語.exe", "kaanal.exe", "", Some(15)));
    assert!(is_kaanal_image("日本語.EXE", "kaanal", "日本語", None));
}

#[test]
fn a_failing_lock_probe_is_an_error_not_a_stop() {
    let d = dir("fs-probe-err");
    std::fs::create_dir(d.join("lock")).unwrap(); // unusable lock
    write_run(&d, "t");
    let r = force_stop(
        &d,
        "t",
        77,
        0,
        |_| panic!("no verify"),
        |_| panic!("no kill"),
    );
    let Err(Fail(c, _)) = r else {
        panic!("a probe error must not read as stopped");
    };
    assert_eq!(c, 1);
}

#[test]
fn force_stop_refuses_to_stop_itself() {
    let d = dir("fs-self");
    let _h = hold(&d);
    write_run(&d, "t");
    let r = force_stop(
        &d,
        "t",
        77,
        77,
        |_| panic!("no verify"),
        |_| panic!("no kill"),
    );
    let Err(Fail(1, m)) = r else {
        panic!("must refuse");
    };
    assert!(m.contains("refusing to stop myself"), "{m}");
}

#[test]
fn force_stop_reports_stopped_when_the_instance_vanishes_before_verifying() {
    let d = dir("fs-vanish");
    let h = std::cell::Cell::new(Some(hold(&d)));
    write_run(&d, "t");
    // verify cannot tell, but by then the lock is gone: a stop, not a failure
    let r = force_stop(
        &d,
        "t",
        77,
        0,
        |_| {
            h.take();
            PidCheck::Unknown("tasklist failed".into())
        },
        |_| panic!("must not kill"),
    );
    assert_eq!(r.unwrap(), Forced::Stopped);
}

#[test]
fn force_stop_refuses_unknown_and_other_while_the_lock_is_held() {
    let d = dir("fs-refuse");
    let _h = hold(&d);
    write_run(&d, "t");
    for check in [
        PidCheck::Unknown("x".into()),
        PidCheck::Other("ping.exe".into()),
    ] {
        let c = std::cell::Cell::new(Some(check));
        let r = force_stop(&d, "t", 77, 0, |_| c.take().unwrap(), |_| panic!("no kill"));
        assert!(r.is_err());
    }
}

#[test]
fn force_stop_does_not_kill_when_the_token_changes_before_the_kill() {
    let d = dir("fs-changed");
    let _h = hold(&d);
    write_run(&d, "old");
    let killed = std::cell::Cell::new(false);
    let r = force_stop(
        &d,
        "old",
        77,
        0,
        |_| {
            write_run(&d, "new"); // a new instance took over after the first check
            PidCheck::IsKaanal
        },
        |_| {
            killed.set(true);
            Ok(())
        },
    );
    let Err(Fail(c, m)) = r else {
        panic!("must refuse");
    };
    assert_eq!(c, 1);
    assert!(m.contains("changed while stopping"), "{m}");
    assert!(!killed.get());
}

#[test]
fn force_stop_kills_a_verified_unchanged_instance() {
    let d = dir("fs-kill");
    let h = std::cell::Cell::new(Some(hold(&d)));
    write_run(&d, "t");
    let r = force_stop(
        &d,
        "t",
        77,
        0,
        |_| PidCheck::IsKaanal,
        |pid| {
            assert_eq!(pid, 77);
            h.take(); // the process dies, the OS drops its lock
            Ok(())
        },
    );
    assert_eq!(r.unwrap(), Forced::Killed);
    assert!(!d.join("run.json").exists());
}

#[test]
fn run_or_gone_turns_a_vanished_instance_into_gone() {
    let d = dir("rog");
    let h = hold(&d);
    let t = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(250));
        drop(h);
    });
    let r = run_or_gone(&d, Duration::from_secs(5)).unwrap();
    assert!(matches!(r, Found::Gone(_)));
    t.join().unwrap();
    let d = dir("rog-held");
    let _h = hold(&d);
    assert!(run_or_gone(&d, SHORT).is_err());
}

#[test]
fn wait_lock_free_times_out_while_held_and_sees_a_release() {
    let d = dir("wait");
    let h = hold(&d);
    let t = Instant::now();
    assert!(!wait_lock_free(&d, Duration::from_millis(300)));
    assert!(t.elapsed() >= Duration::from_millis(300));
    let t = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        drop(h);
    });
    assert!(wait_lock_free(&d, Duration::from_secs(5)));
    t.join().unwrap();
}

#[test]
fn clear_stale_reports_each_row_and_leaves_a_held_lock_alone() {
    let d = dir("cs-none");
    assert_eq!(clear_stale(&d).unwrap().unwrap(), "kaanal is not running");
    write_run(&d, "t");
    let m = clear_stale(&d).unwrap().unwrap();
    assert!(m.contains("removed stale state from pid 77"), "{m}");
    assert!(!d.join("run.json").exists());
    let d = dir("cs-held");
    let _h = hold(&d);
    write_run(&d, "t");
    assert!(clear_stale(&d).unwrap().is_none());
    assert!(d.join("run.json").exists());
}

#[test]
fn run_json_without_startup_still_parses() {
    let d = dir("nostartup");
    write_run(&d, "t");
    let run = read_run(&d).unwrap();
    assert_eq!(run.startup, Startup::default());
    std::fs::write(
        d.join("run.json"),
        r#"{"port":4000,"token":"t","startup":{"auth":"a","seed":5,"warnings":["w"],"blank":true}}"#,
    )
    .unwrap();
    let run = read_run(&d).unwrap();
    assert!(run.startup.blank);
    assert_eq!(run.addr.ip(), IpAddr::V4(Ipv4Addr::LOCALHOST));
    assert_eq!(run.startup.seed, Some(5));
    assert_eq!(run.startup.warnings, ["w"]);
}

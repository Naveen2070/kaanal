//! Unit tests for the command helpers split out of `main.rs`.

#[test]
fn down_waits_for_the_longer_of_its_budget_and_an_earlier_drains_time_left() {
    use super::{ShutdownReply, drain_wait};
    use std::time::Duration;
    let s = Duration::from_secs;
    let reply = |left| ShutdownReply { status: 202, left };
    assert_eq!(drain_wait(s(5), None), s(7));
    assert_eq!(drain_wait(s(5), Some(&reply(None))), s(7));
    assert_eq!(drain_wait(s(5), Some(&reply(Some(s(60))))), s(62));
    assert_eq!(drain_wait(s(30), Some(&reply(Some(s(2))))), s(32));
}

#[test]
fn exposure_text_is_the_exact_single_spaced_message() {
    assert_eq!(
        crate::commands::serve::exposure_text("0.0.0.0".parse().unwrap()),
        "warning: binding 0.0.0.0 makes the mock and control ports reachable from your network; control calls still need the token and Host localhost/127.0.0.1/[::1]"
    );
}

use super::{clean, encode_segment, ttl_body};
use crate::instance::state_dir::lock_at;
use crate::util::parse::{parse_ms, parse_rate};
use std::time::Duration;

/// A scratch dir removed on drop, so a failing assert leaves nothing behind.
struct Tmp(std::path::PathBuf);

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn lock_dir(name: &str) -> Tmp {
    let d = std::env::temp_dir().join(format!("kaanal-lockat-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    Tmp(d)
}

#[test]
fn lock_at_waits_out_a_brief_probe() {
    let d = lock_dir("probe");
    let path = d.0.join("lock");
    let probe = std::fs::File::create(&path).unwrap();
    probe.try_lock_shared().unwrap();
    let t = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(80));
        probe.unlock().unwrap();
    });
    assert!(lock_at(&path, 50, Duration::from_millis(20)).is_ok());
    t.join().unwrap();
}

#[test]
fn lock_at_reports_a_held_lock_as_already_running() {
    let d = lock_dir("held");
    let path = d.0.join("lock");
    let holder = std::fs::File::create(&path).unwrap();
    holder.try_lock().unwrap();
    let start = std::time::Instant::now();
    let e = lock_at(&path, 5, Duration::from_millis(20)).unwrap_err();
    assert!(e.1.contains("already running"), "{}", e.1);
    assert!(
        start.elapsed() >= Duration::from_millis(80),
        "it must retry first"
    );
    drop(holder);
}

#[test]
fn ttl_body_sends_the_trimmed_value() {
    let b = ttl_body("ttl", Some(" 30s ".into()))
        .ok()
        .flatten()
        .unwrap();
    assert_eq!(b["ttl"], "30s");
    assert!(ttl_body("ttl", Some("soon".into())).is_err());
    assert!(matches!(ttl_body("ttl", None), Ok(None)));
}

#[test]
fn latency_parsing() {
    assert_eq!(parse_ms("300ms"), Ok(300));
    assert_eq!(parse_ms("2s"), Ok(2000));
    assert_eq!(parse_ms("1m 30s"), Ok(90_000));
    assert!(parse_ms("15").is_err(), "a unit is required");
    assert_eq!(parse_ms(" 40 ms "), Ok(40));
    assert!(parse_ms("-5ms").is_err());
    assert!(parse_ms("fast").is_err());
    assert!(parse_ms("").is_err());
    assert_eq!(parse_ms("1.5s"), Ok(1500));
    assert!(parse_ms("99999999999999999999s").is_err());
}

#[test]
fn rate_parsing() {
    assert_eq!(parse_rate("10%"), Ok(0.1));
    assert_eq!(parse_rate("100%"), Ok(1.0));
    assert_eq!(parse_rate("0.25"), Ok(0.25));
    assert_eq!(parse_rate("0"), Ok(0.0));
    assert!(parse_rate("150%").is_err());
    assert!(parse_rate("-1%").is_err());
    assert!(parse_rate("1.5").is_err());
    assert!(parse_rate("NaN").is_err());
    assert!(parse_rate("lots").is_err());
    assert!(parse_rate("").is_err());
}

#[test]
fn clean_replaces_control_chars() {
    assert_eq!(clean("unknown node 'db'"), "unknown node 'db'");
    assert_eq!(clean("a\u{1b}[31mb\r\n\t\u{7f}\u{9b}"), "a?[31mb?????");
    assert_eq!(clean("caf\u{e9}"), "caf\u{e9}");
    // text reordering and invisible characters are neutralised too
    assert_eq!(
        clean("a\u{202e}b\u{2066}c\u{200f}d\u{61c}e\u{2028}f\u{feff}"),
        "a?b?c?d?e?f?"
    );
    // joiners (emoji, scripts) and ordinary non-ASCII text stay
    assert_eq!(
        clean("\u{1f468}\u{200d}\u{1f469} \u{65e5}\u{672c}"),
        "\u{1f468}\u{200d}\u{1f469} \u{65e5}\u{672c}"
    );
}

#[test]
fn path_segments_are_percent_encoded() {
    assert_eq!(encode_segment("db-1_a.b~"), "db-1_a.b~");
    assert_eq!(encode_segment("a/b c"), "a%2Fb%20c");
    assert_eq!(encode_segment("\u{1b}"), "%1B");
    assert_eq!(encode_segment("\u{e9}"), "%C3%A9");
}

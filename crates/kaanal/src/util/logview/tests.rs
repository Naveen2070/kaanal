//! Unit tests for the log and waterfall rendering and the `--follow` helpers.

use super::*;
use serde_json::json;

fn entry() -> Value {
    json!({
        "id": "r04f2-0042", "client_id": null, "ts_ms": 45_296_789u64,
        "method": "GET", "path": "/orders/7", "endpoint": "GET /orders/{id}",
        "status": 200, "ms": 112, "trace_ms": 40.0, "scope": null, "pin": null,
        "flags": [], "chain_error": null, "failed_node": null, "kind": null,
        "rejected": null, "hops": [], "truncated": false
    })
}

fn hop(node: &str, kind: &str, start: f64, dur: f64) -> Value {
    json!({"node": node, "kind": kind, "start_ms": start, "dur_ms": dur,
           "wait_ms": 0.0, "lane": 0, "fork": null, "cut": false, "why": null})
}

fn with(mut e: Value, f: impl FnOnce(&mut Value)) -> Value {
    f(&mut e);
    e
}

#[test]
fn line_formats_flags_and_utc_time() {
    assert_eq!(
        line(&entry()),
        "12:34:56.789Z  r04f2-0042  GET /orders/7  200  112ms"
    );
    let e = with(entry(), |e| {
        e["scope"] = json!("orders-empty");
        e["pin"] = json!("empty");
        e["failed_node"] = json!("db");
        e["kind"] = json!("reset");
        e["client_id"] = json!("abc-1");
        e["rejected"] = json!("unmatched");
    });
    assert_eq!(
        line(&e),
        "12:34:56.789Z  r04f2-0042  GET /orders/7  200  112ms  scope=orders-empty  pinned=empty  failed=db  kind=reset  client=abc-1  unmatched"
    );
}

#[test]
fn waterfall_success() {
    let e = with(entry(), |e| {
        e["hops"] = json!([hop("nginx", "proc", 0.0, 10.0), hop("db", "op", 10.0, 30.0)]);
        e["hops"][1]["op"] = json!("read");
    });
    let b = |from: usize, to: usize| {
        (0..BAR)
            .map(|i| if (from..to).contains(&i) { '#' } else { ' ' })
            .collect::<String>()
    };
    assert_eq!(
        waterfall(&e),
        format!(
            "r04f2-0042  GET /orders/7  200  served 112ms  trace 40.0ms\n  nginx.proc       0.0ms      10.0ms  |{}|\n  db.read         10.0ms      30.0ms  |{}|",
            b(0, 10),
            b(10, 40)
        )
    );
}

#[test]
fn waterfall_queue_wait() {
    let e = with(entry(), |e| {
        e["hops"] = json!([hop("db", "op", 20.0, 10.0)]);
        e["hops"][0]["wait_ms"] = json!(10.0);
    });
    let row = waterfall(&e).lines().nth(1).unwrap().to_string();
    assert!(
        row.ends_with(&format!(
            "|{}{}{}{}|",
            " ".repeat(10),
            ".".repeat(10),
            "#".repeat(10),
            " ".repeat(10)
        )),
        "{row}"
    );
}

#[test]
fn waterfall_chaos_latency_why() {
    let e = with(entry(), |e| {
        e["flags"] = json!(["DI"]);
        e["hops"] = json!([hop("db", "op", 0.0, 40.0)]);
        e["hops"][0]["why"] = json!("chaos +30ms");
    });
    let w = waterfall(&e);
    assert!(
        w.lines()
            .next()
            .unwrap()
            .ends_with("trace 40.0ms  flags=DI"),
        "{w}"
    );
    assert!(w.ends_with("|  chaos +30ms"), "{w}");
}

#[test]
fn waterfall_rate_limited_429() {
    let e = with(entry(), |e| {
        e["status"] = json!(429);
        e["flags"] = json!(["RL"]);
        e["failed_node"] = json!("gw");
        e["chain_error"] =
            json!({"node": "gw", "status": 429, "kind": "status", "cause": "rate-limit"});
        e["hops"] = json!([hop("gw", "proc", 0.0, 0.0)]);
    });
    let w = waterfall(&e);
    assert!(w.contains("|x         "), "{w}");
    assert!(w.ends_with("\n  failed at gw: 429 (rate-limit)"), "{w}");
}

#[test]
fn waterfall_hang_timeout_cut_hop() {
    let e = with(entry(), |e| {
        e["status"] = json!(504);
        e["failed_node"] = json!("db");
        e["chain_error"] =
            json!({"node": "db", "status": 504, "kind": "timeout", "cause": "deadline"});
        e["hops"] = json!([hop("app", "proc", 0.0, 5.0), hop("db", "op", 5.0, 35.0)]);
        e["hops"][1]["cut"] = json!(true);
        e["hops"][1]["why"] = json!("chaos hang");
    });
    let w = waterfall(&e);
    assert!(w.contains(&"x".repeat(35)), "{w}");
    assert!(w.contains("chaos hang"), "{w}");
    assert!(
        w.ends_with("\n  failed at db: 504 timeout (deadline)"),
        "{w}"
    );
}

#[test]
fn waterfall_pinned_over_failure_footer() {
    let e = with(entry(), |e| {
        e["pin"] = json!("empty");
        e["chain_error"] = json!({"node": "db", "status": 503, "kind": "status", "cause": "chaos"});
        e["hops"] = json!([hop("db", "op", 0.0, 10.0)]);
    });
    let w = waterfall(&e);
    assert!(w.lines().next().unwrap().contains("pinned=empty"), "{w}");
    assert!(w.ends_with("\n  pinned=empty hid: db.error=503"), "{w}");
    assert!(w.contains('x'), "failed hop still marked: {w}");
}

#[test]
fn waterfall_parallel_lanes() {
    let e = with(entry(), |e| {
        e["hops"] = json!([hop("a", "proc", 0.0, 20.0), hop("b", "proc", 20.0, 20.0)]);
        e["hops"][0]["lane"] = json!(1);
        e["hops"][1]["lane"] = json!(2);
        e["hops"][0]["start_ms"] = json!(10.0);
        e["hops"][1]["start_ms"] = json!(10.0);
    });
    let rows: Vec<_> = waterfall(&e).lines().skip(1).map(String::from).collect();
    assert!(
        rows[0].contains(&format!(
            "|{}{}{}|",
            " ".repeat(10),
            "#".repeat(20),
            " ".repeat(10)
        )),
        "{rows:?}"
    );
    assert!(
        rows[0].ends_with("lane 1") && rows[1].ends_with("lane 2"),
        "{rows:?}"
    );
    assert!(
        rows[1].contains(&format!(
            "|{}{}{}|",
            " ".repeat(10),
            "#".repeat(20),
            " ".repeat(10)
        )),
        "{rows:?}"
    );
}

#[test]
fn waterfall_tiny_hop_gets_one_column() {
    let e = with(entry(), |e| {
        e["trace_ms"] = json!(1000.0);
        e["hops"] = json!([hop("a", "proc", 500.0, 0.1)]);
    });
    let row = waterfall(&e).lines().nth(1).unwrap().to_string();
    assert_eq!(row.matches('#').count(), 1, "{row}");
    // a hop at the very end still fits the bar
    let e = with(entry(), |e| {
        e["hops"] = json!([hop("a", "proc", 40.0, 0.0)]);
    });
    assert_eq!(waterfall(&e).matches('#').count(), 1);
}

#[test]
fn waterfall_truncated_entry_and_hopless_entries() {
    let e = with(entry(), |e| {
        e["truncated"] = json!(true);
        e["hops"] = json!([hop("a", "proc", 0.0, 1.0)]);
    });
    assert!(waterfall(&e).ends_with("\n  ... more hops (truncated, 1 shown)"));
    let e = with(entry(), |e| {
        e["status"] = json!(499);
        e["rejected"] = json!("client-gone");
    });
    assert!(waterfall(&e).ends_with("\n  - (client-gone)"));
    assert!(waterfall(&entry()).ends_with("\n  - (no hops recorded)"));
}

#[test]
fn terminal_escapes_in_names_are_neutralised() {
    let x = "\u{1b}[31m";
    let e = with(entry(), |e| {
        for k in [
            "id",
            "method",
            "path",
            "scope",
            "client_id",
            "kind",
            "rejected",
        ] {
            e[k] = json!(format!("{k}{x}"));
        }
        e["pin"] = json!(format!("pin{x}"));
        e["failed_node"] = json!(format!("fn{x}"));
        e["flags"] = json!([format!("FI{x}")]);
        e["hops"] = json!([hop(&format!("n{x}"), &format!("k{x}"), 0.0, 1.0)]);
        e["hops"][0]["op"] = json!(format!("op{x}"));
        e["hops"][0]["why"] = json!(format!("why{x}"));
        e["chain_error"] = json!({"node": format!("n{x}"), "status": 500,
            "kind": format!("ck{x}"), "cause": format!("cc{x}")});
    });
    // with a pin and no failed_node the footer is the "pinned ... hid" one
    let pinned = with(e.clone(), |e| e["failed_node"] = Value::Null);
    for text in [
        line(&e),
        waterfall(&e),
        waterfall(&pinned),
        lagged_notice(&format!("3{x}")),
    ] {
        assert!(
            !text.chars().any(|c| c.is_control() && c != '\n'),
            "{text:?}"
        );
    }
    let w = waterfall(&e);
    for part in ["why", "op", "ck", "cc", "FI", "client_id", "pin"] {
        assert!(w.contains(&format!("{part}?[31m")), "{part} in {w}");
    }
    assert!(waterfall(&pinned).contains("n?[31m.error=500"));
    let l = line(&e);
    for part in [
        "scope",
        "pin",
        "fn",
        "kind",
        "client_id",
        "rejected",
        "path",
    ] {
        assert!(l.contains(&format!("{part}?[31m")), "{part} in {l}");
    }
}

fn params(p: &[(&str, &str)]) -> Vec<(String, String)> {
    p.iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn client_side_filters_match_the_server_grammar() {
    let f = |p: &[(&str, &str)]| filter_of(&params(p), 10_000).unwrap();
    let e = with(entry(), |e| {
        e["status"] = json!(503);
        e["failed_node"] = json!("db");
        e["scope"] = json!("a,b");
        e["ts_ms"] = json!(9_000);
    });
    assert!(passes(&f(&[("status", "404,5xx")]), &e));
    assert!(!passes(&f(&[("status", "404")]), &e));
    assert!(passes(&f(&[("failed", "true")]), &e));
    assert!(!passes(&f(&[("failed", "true")]), &entry()));
    assert!(passes(&f(&[("scope", "a,b")]), &e));
    assert!(!passes(&f(&[("scope", "a")]), &e));
    assert!(passes(&f(&[("endpoint", "GET /orders/{id}")]), &e));
    assert!(!passes(&f(&[("endpoint", "GET /x")]), &e));
    // a value with no endpoint (an unmatched request) cannot pass an endpoint filter
    assert!(!passes(
        &f(&[("endpoint", "GET /orders/{id}")]),
        &json!({"status": 200})
    ));
    assert!(passes(&f(&[("since", "5s")]), &e));
    assert!(!passes(&f(&[("since", "500ms")]), &e));
    assert!(passes(&f(&[]), &e));
    assert!(filter_of(&params(&[("status", "9xx")]), 0).is_err());
}

#[test]
fn lagged_notice_text() {
    assert_eq!(
        lagged_notice("17"),
        "-- missed 17 requests (follow slower than the server) --"
    );
}

#[test]
fn seen_is_bounded_and_dedups() {
    let mut s = Seen::new();
    assert!(s.insert("a"));
    assert!(!s.insert("a"));
    for i in 0..SEEN_CAP {
        s.insert(&format!("x{i}"));
    }
    assert!(s.insert("a"), "the oldest id was forgotten");
    assert!(!s.insert(&format!("x{}", SEEN_CAP - 1)));
}

#[test]
fn sse_lines_assemble_into_messages() {
    let mut p = Sse::default();
    assert_eq!(p.push(": keep-alive"), None);
    assert_eq!(p.push(""), None);
    assert_eq!(p.push("event: request"), None);
    assert_eq!(p.push("data: {\"id\":\"r1\"}"), None);
    assert_eq!(
        p.push(""),
        Some(("request".into(), "{\"id\":\"r1\"}".into()))
    );
    assert_eq!(p.push("event:lagged"), None);
    assert_eq!(p.push("data:5"), None);
    assert_eq!(p.push(""), Some(("lagged".into(), "5".into())));
}

#[test]
fn sse_bytes_that_are_not_utf8_do_not_end_the_reader() {
    let line = lossy_line(b"data: caf\xe9\xff\r\n");
    assert_eq!(line, "data: caf\u{fffd}\u{fffd}");
    let mut p = Sse::default();
    p.push("event: request");
    p.push(&line);
    assert!(p.push("").is_some());
}

#[test]
fn a_full_queue_counts_drops_and_the_printer_takes_the_count() {
    let (tx, rx) = std::sync::mpsc::sync_channel(2);
    let dropped = AtomicU64::new(0);
    let msg = |n: u32| ("request".to_string(), n.to_string());
    for n in 0..5 {
        assert!(forward(&tx, &dropped, msg(n)));
    }
    // a message that is not a request, when it does not fit, is not a missed request
    assert!(forward(&tx, &dropped, ("scenario".into(), String::new())));
    assert_eq!(take_lagged(&dropped).as_deref(), Some("3"));
    assert_eq!(take_lagged(&dropped), None, "taking resets the count");
    assert_eq!(rx.recv().unwrap(), Some(msg(0)));
    assert_eq!(rx.recv().unwrap(), Some(msg(1)));
    drop(rx);
    assert!(!forward(&tx, &dropped, msg(10)));
}

#[test]
fn a_server_lagged_message_that_does_not_fit_adds_its_count() {
    let (tx, _rx) = std::sync::mpsc::sync_channel(1);
    let dropped = AtomicU64::new(0);
    assert!(forward(&tx, &dropped, ("request".into(), String::new())));
    assert!(forward(&tx, &dropped, ("request".into(), String::new())));
    assert!(forward(&tx, &dropped, ("lagged".into(), " 17\n".into())));
    assert!(forward(&tx, &dropped, ("lagged".into(), "junk".into())));
    assert_eq!(take_lagged(&dropped).as_deref(), Some("19"));
}

#[test]
fn drops_are_still_there_after_the_stream_ends() {
    // The reader sends the end marker after its last drop; the printer, on seeing it, takes the count.
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let dropped = AtomicU64::new(0);
    forward(&tx, &dropped, ("request".into(), "a".into()));
    forward(&tx, &dropped, ("request".into(), "b".into()));
    assert!(rx.recv().unwrap().is_some());
    tx.send(None).unwrap();
    assert_eq!(rx.recv().unwrap(), None);
    assert_eq!(take_lagged(&dropped).as_deref(), Some("1"));
}

/// Runs [`drain`] on a thread with a shared output; `act` runs on the test thread.
fn run_drain(
    idle: Duration,
    act: impl FnOnce(SyncSender<Option<(String, String)>>, &AtomicU64, &std::sync::Mutex<Vec<String>>),
) -> (End, Vec<String>) {
    use std::sync::{Arc, Mutex};
    let (tx, rx) = std::sync::mpsc::sync_channel(8);
    let dropped = Arc::new(AtomicU64::new(0));
    let out = Arc::new(Mutex::new(Vec::new()));
    let h = {
        let (dropped, out) = (Arc::clone(&dropped), Arc::clone(&out));
        std::thread::spawn(move || {
            drain(
                &rx,
                &dropped,
                idle,
                &mut |s| out.lock().unwrap().push(s.to_string()),
                &mut |e, d| out.lock().unwrap().push(format!("event {e} {d}")),
            )
        })
    };
    // let the printer reach its first wait
    std::thread::sleep(Duration::from_millis(100));
    act(tx, &dropped, &out);
    let end = h.join().unwrap();
    let lines = out.lock().unwrap().clone();
    (end, lines)
}

const NOTICE: &str = "-- missed 2 requests (follow slower than the server) --";

#[test]
fn drain_flushes_pending_drops_at_the_end_marker() {
    let (end, out) = run_drain(Duration::from_secs(30), |tx, dropped, _| {
        dropped.fetch_add(2, Ordering::Relaxed);
        tx.send(None).unwrap();
    });
    assert_eq!((end, out), (End::Marker, vec![NOTICE.to_string()]));
}

#[test]
fn drain_flushes_pending_drops_when_the_queue_is_cut() {
    let (end, out) = run_drain(Duration::from_secs(30), |tx, dropped, _| {
        dropped.fetch_add(2, Ordering::Relaxed);
        // the only sender goes away without an end marker: the reader thread died
        drop(tx);
    });
    assert_eq!(end, End::Disconnected);
    assert_eq!(out, vec![NOTICE.to_string()]);
}

#[test]
fn drain_flushes_pending_drops_while_idle() {
    let (end, out) = run_drain(Duration::from_millis(20), |tx, dropped, out| {
        dropped.fetch_add(2, Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(200));
        // still no message and no end marker: the notice must already be out
        assert_eq!(*out.lock().unwrap(), vec![NOTICE.to_string()]);
        tx.send(Some(("request".into(), "x".into()))).unwrap();
        tx.send(Some(("lagged".into(), "5".into()))).unwrap();
        tx.send(None).unwrap();
    });
    assert_eq!(end, End::Marker);
    assert_eq!(out[1], "event request x");
    assert_eq!(
        out[2],
        "-- missed 5 requests (follow slower than the server) --"
    );
}

#[test]
fn waterfall_marks_the_failing_nodes_deciding_hop_among_parallel_lanes() {
    let lanes = |cut_first: bool| {
        with(entry(), |e| {
            e["trace_ms"] = json!(40.0);
            e["chain_error"] =
                json!({"node": "db", "status": 503, "kind": "status", "cause": "chaos"});
            e["hops"] = json!([hop("db", "op", 0.0, 10.0), hop("db", "op", 0.0, 30.0)]);
            e["hops"][0]["lane"] = json!(1);
            e["hops"][1]["lane"] = json!(2);
            e["hops"][0]["cut"] = json!(cut_first);
        })
    };
    let count = |w: &str, row: usize, c: char| w.lines().nth(row).unwrap().matches(c).count();
    // nothing cut: the hop that ends the request (the longer lane) is the failed one
    let w = waterfall(&lanes(false));
    assert_eq!((count(&w, 1, '#'), count(&w, 1, 'x')), (10, 0), "{w}");
    assert_eq!((count(&w, 2, '#'), count(&w, 2, 'x')), (0, 30), "{w}");
    // a cut hop wins over a longer one, and only that lane is x
    let w = waterfall(&lanes(true));
    assert_eq!((count(&w, 1, '#'), count(&w, 1, 'x')), (0, 10), "{w}");
    assert_eq!((count(&w, 2, '#'), count(&w, 2, 'x')), (30, 0), "{w}");
}

#[test]
fn a_line_over_the_cap_is_skipped_to_its_end_and_never_buffered() {
    let mut r = std::io::Cursor::new(b"abc\n0123456789\nxy\nlast".to_vec());
    let mut buf = Vec::new();
    assert_eq!(read_capped(&mut r, &mut buf, 8).unwrap(), Line::Text);
    assert_eq!(buf, b"abc\n");
    assert_eq!(read_capped(&mut r, &mut buf, 8).unwrap(), Line::TooLong);
    assert_eq!(buf, b"");
    assert_eq!(read_capped(&mut r, &mut buf, 8).unwrap(), Line::Text);
    assert_eq!(buf, b"xy\n");
    assert_eq!(read_capped(&mut r, &mut buf, 8).unwrap(), Line::Text);
    assert_eq!(buf, b"last");
    assert_eq!(read_capped(&mut r, &mut buf, 8).unwrap(), Line::Eof);
    // a line of exactly the cap that ends in its newline is fine
    let mut r = std::io::Cursor::new(b"1234567\n".to_vec());
    assert_eq!(read_capped(&mut r, &mut buf, 8).unwrap(), Line::Text);
}

#[test]
fn an_sse_message_over_the_cap_comes_out_as_one_missed_request() {
    let mut p = Sse::default();
    p.push("event: request");
    let big = format!("data: {}", "a".repeat(MAX_MSG / 2));
    p.push(&big);
    p.push(&big); // together they pass the cap
    p.push(&big);
    assert_eq!(p.push(""), Some(("lagged".into(), "1".into())));
    // the next message is clean again
    p.push("event: request");
    p.push("data: {}");
    assert_eq!(p.push(""), Some(("request".into(), "{}".into())));
    // an over-long line flagged by the reader does the same
    p.push("event: request");
    p.overflow();
    assert_eq!(p.push(""), Some(("lagged".into(), "1".into())));
}

#[test]
fn waterfall_names_the_replica_of_a_replicated_hop() {
    let e = with(entry(), |e| {
        e["hops"] = json!([hop("app", "proc", 0.0, 5.0), hop("db", "op", 5.0, 4.0)]);
        e["hops"][1]["replica"] = json!(2);
    });
    let w = waterfall(&e);
    assert!(w.contains("db#2.op"), "{w}");
    assert!(w.contains("app.proc") && !w.contains("app#"), "{w}");
}

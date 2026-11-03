//! Unit tests for the trace parts and the log entries built from a request.
#![expect(
    clippy::many_single_char_names,
    reason = "short names for the two or three nodes of the fixture"
)]

use super::*;
use kaanal_core::{ChaosState, Config, Hop, NodeChaos, Op, compile_layers};

const YAML: &str = "version: 1\nnodes:\n  nginx: { type: gateway, proc: 1ms }\n  app: { type: service, proc: 1ms }\n  db: { type: database, ops: { read: 1ms, write: 1ms, scan: 1ms } }\n  c: { type: cache, hit_ratio: 1.0, hit: 1ms, miss: 2ms }\n  ct: { type: cache, hit_ratio: 1.0, hit: 1ms, miss: 2ms, timeout: 150ms }\nedges:\n  - { from: nginx, to: app }\n  - { from: app, to: db }\n  - { from: app, to: c }\n  - { from: app, to: ct }\n";

fn topo() -> Topology {
    Topology::build(&Config::from_yaml(YAML).unwrap()).unwrap()
}

fn node(t: &Topology, n: &str) -> usize {
    t.nodes.iter().position(|x| x.name == n).unwrap()
}

fn hop(node: usize, kind: HopKind, start_ms: u64, dur_ms: u64) -> Hop {
    Hop {
        node,
        replica: 0,
        kind,
        start: Duration::from_millis(start_ms),
        dur: Duration::from_millis(dur_ms),
    }
}

/// A plan of plain hops, and a `Finished` that ran every one of them.
fn straight(hops: Vec<Hop>) -> (Plan, Finished) {
    let items = hops.iter().cloned().map(PlanItem::Hop).collect();
    let ran = hops
        .into_iter()
        .enumerate()
        .map(|(idx, hop)| FinishedHop {
            idx,
            hop,
            wait: Duration::ZERO,
        })
        .collect();
    (Plan { items }, finished(ran, ChainResult::Ok, None))
}

fn finished(hops: Vec<FinishedHop>, result: ChainResult, cause: Option<FailCause>) -> Finished {
    Finished {
        tag: 0,
        order: 0,
        arrival: 0,
        done: 0,
        result,
        cause,
        hops,
        holds: Vec::new(),
    }
}

fn facts() -> Facts<'static> {
    Facts {
        id: "r0001-0001".into(),
        client_id: None,
        seq: 1,
        ts_ms: 1,
        ms: 1,
        method: "GET",
        path: "/x",
        endpoint: Some("GET /x"),
        scope: None,
    }
}

fn entry_of(t: &Topology, plan: &Plan, fin: &Finished, pin: Option<&'static str>) -> LogEntry {
    let chaos = EffectiveChaos::default();
    let run = Run {
        plan,
        fin,
        chaos: &chaos,
        topo: t,
        pin_delay: false,
    };
    let ans = Answer {
        status: 200,
        pin,
        failed: None,
    };
    build_entry(facts(), &ans, &run, String::new())
}

fn many(t: &Topology, n: usize) -> (Plan, Finished) {
    let a = node(t, "app");
    straight((0..n).map(|i| hop(a, HopKind::Proc, i as u64, 1)).collect())
}

#[test]
fn header_equals_cap_of_full_trace_for_every_existing_trace_case() {
    let t = topo();
    let (a, d) = (node(&t, "app"), node(&t, "db"));
    // Small: both texts are the same, the pin and error tails included.
    let (plan, mut fin) = straight(vec![
        hop(a, HopKind::Net, 0, 1),
        hop(a, HopKind::Proc, 1, 2),
        hop(d, HopKind::Op(Op::Read), 3, 4),
    ]);
    fin.result = ChainResult::Err {
        node: d,
        status: 503,
        kind: FailKind::Status,
        retry_after: None,
    };
    let p = trace_parts(&plan, &fin.hops, &fin.result, &t, Some("error"));
    assert_eq!(
        p.header,
        "net=1.0ms,app.proc=2.0ms,db.read=4.0ms,db.error=503,pinned=error"
    );
    assert_eq!(p.full, p.header);
    // Over the header cap, under the log cap: the header is cut, the log copy is whole.
    let (plan, fin) = many(&t, 400);
    let p = trace_parts(&plan, &fin.hops, &fin.result, &t, Some("slow"));
    assert!(p.header.len() <= TRACE_CAP, "{}", p.header.len());
    assert!(p.header.ends_with("hops,pinned=slow"), "{}", p.header);
    assert!(p.full.len() > TRACE_CAP && p.full.len() <= MAX_TRACE);
    assert!(p.full.ends_with("app.proc=1.0ms,pinned=slow"));
    let kept = p.header.split(",...+").next().unwrap();
    assert!(
        p.full.starts_with(kept),
        "the header is a prefix of the log copy"
    );
    // Over the log cap as well: it is cut by the same rule.
    let (plan, fin) = many(&t, 2000);
    let p = trace_parts(&plan, &fin.hops, &fin.result, &t, None);
    assert!(
        p.full.len() <= MAX_TRACE && p.full.ends_with(" hops"),
        "{}",
        p.full.len()
    );
    assert!(p.header.len() <= TRACE_CAP && p.header.len() < p.full.len());
    let ans = Answer {
        status: 200,
        pin: None,
        failed: None,
    };
    let chaos = EffectiveChaos::default();
    let run = Run {
        plan: &plan,
        fin: &fin,
        chaos: &chaos,
        topo: &t,
        pin_delay: false,
    };
    let e = build_entry(facts(), &ans, &run, p.full.clone());
    assert_eq!(e.trace, p.full);
}

/// Rebuilds the trace text from the structured hops (one fork at most).
fn rerender(hops: &[HopRec]) -> String {
    let mut out = String::new();
    let (mut in_fork, mut lane) = (false, 0);
    for (i, h) in hops.iter().enumerate() {
        let sep = |out: &mut String| {
            if !out.is_empty() && !out.ends_with(['[', '|']) {
                out.push(',');
            }
        };
        if h.fork.is_some() && !in_fork {
            sep(&mut out);
            out.push('[');
            (in_fork, lane) = (true, h.lane);
        } else if h.fork.is_some() && h.lane != lane {
            out.push('|');
            lane = h.lane;
        } else if h.fork.is_none() && in_fork {
            out.push(']');
            in_fork = false;
        }
        if h.wait_ms > 0.0 {
            sep(&mut out);
            let _ = write!(out, "{}.wait={:.1}ms", h.node, h.wait_ms);
        }
        sep(&mut out);
        let d = format!("{:.1}ms", h.dur_ms);
        let _ = match h.kind {
            "net" => write!(out, "net={d}"),
            k => write!(out, "{}.{}={d}", h.node, h.op.unwrap_or(k)),
        };
        if in_fork && i == hops.len() - 1 {
            out.push(']');
        }
    }
    out
}

#[test]
fn structured_hops_rerender_to_the_trace_text_while_not_truncated() {
    let t = topo();
    let (a, d) = (node(&t, "app"), node(&t, "db"));
    let items = vec![
        PlanItem::Hop(hop(a, HopKind::Net, 0, 1)),
        PlanItem::Fork {
            branches: 2,
            size: 5,
        },
        PlanItem::Branch { size: 1 },
        PlanItem::Hop(hop(a, HopKind::Proc, 1, 2)),
        PlanItem::Branch { size: 2 },
        PlanItem::Hop(hop(d, HopKind::CacheMiss, 1, 3)),
        PlanItem::Hop(hop(d, HopKind::Delay, 4, 5)),
        PlanItem::Hop(hop(d, HopKind::Op(Op::Write), 9, 1)),
    ];
    let ran = |idx: usize, wait: u64| {
        let PlanItem::Hop(h) = &items[idx] else {
            panic!()
        };
        FinishedHop {
            idx,
            hop: h.clone(),
            wait: Duration::from_millis(wait),
        }
    };
    let fin = finished(
        vec![ran(0, 0), ran(3, 0), ran(5, 2), ran(6, 0), ran(7, 0)],
        ChainResult::Ok,
        None,
    );
    let plan = Plan { items };
    let p = trace_parts(&plan, &fin.hops, &fin.result, &t, None);
    let e = entry_of(&t, &plan, &fin, None);
    assert!(!e.truncated);
    assert_eq!(rerender(&e.hops), p.full, "{:?}", e.hops);
    assert!(
        p.full
            .contains("[app.proc=2.0ms|db.wait=2.0ms,db.miss=3.0ms,db.delay=5.0ms]")
    );
}

#[test]
fn hops_over_512_set_truncated() {
    let t = topo();
    let (plan, fin) = many(&t, MAX_HOPS + 1);
    let e = entry_of(&t, &plan, &fin, None);
    assert_eq!((e.hops.len(), e.truncated), (MAX_HOPS, true));
    let (plan, fin) = many(&t, MAX_HOPS);
    let e = entry_of(&t, &plan, &fin, None);
    assert_eq!((e.hops.len(), e.truncated), (MAX_HOPS, false));
}

fn err(node: usize) -> ChainResult {
    ChainResult::Err {
        node,
        status: 503,
        kind: FailKind::Status,
        retry_after: None,
    }
}

#[test]
fn flags_per_cause() {
    let t = topo();
    let a = node(&t, "app");
    for (cause, flags) in [
        (Some(FailCause::Chaos), vec!["FI"]),
        (Some(FailCause::RateLimit), vec!["RL"]),
        (Some(FailCause::Overflow), vec!["UO"]),
        (Some(FailCause::Deadline), vec!["DT"]),
        (None, vec![]),
    ] {
        let (plan, mut fin) = many(&t, 1);
        if cause.is_some() {
            fin.result = err(a);
            fin.cause = cause;
        }
        assert_eq!(entry_of(&t, &plan, &fin, None).flags, flags, "{cause:?}");
    }
    // A pin adds PN; a pin delay adds PD and leads with the pin-delay hop.
    let (plan, fin) = straight(vec![hop(node(&t, "nginx"), HopKind::Delay, 0, 50)]);
    let chaos = EffectiveChaos::default();
    let run = Run {
        plan: &plan,
        fin: &fin,
        chaos: &chaos,
        topo: &t,
        pin_delay: true,
    };
    let ans = Answer {
        status: 200,
        pin: Some("slow"),
        failed: None,
    };
    let e = build_entry(facts(), &ans, &run, String::new());
    assert_eq!(e.flags, ["DI", "PD", "PN"]);
    assert_eq!(e.hops[0].why.as_deref(), Some("pin delay"));
    // A hang (a Delay hop followed by the node's timeout failure) is FI, never DI.
    let items = vec![
        PlanItem::Hop(hop(a, HopKind::Delay, 0, 50)),
        PlanItem::Fail {
            node: a,
            status: 504,
            kind: FailKind::Timeout,
            at: Duration::from_millis(50),
        },
    ];
    let ran = FinishedHop {
        idx: 0,
        hop: hop(a, HopKind::Delay, 0, 50),
        wait: Duration::ZERO,
    };
    let mut fin = finished(vec![ran], err(a), Some(FailCause::Chaos));
    fin.result = ChainResult::Err {
        node: a,
        status: 504,
        kind: FailKind::Timeout,
        retry_after: None,
    };
    let e = entry_of(&t, &Plan { items }, &fin, None);
    assert_eq!(e.flags, ["FI"]);
    assert_eq!(e.hops[0].why, None);
}

/// The chaos of one node: `latency_ms` added, and a hang when `hang`.
fn chaos_of(node: usize, latency_ms: u64, hang: bool) -> EffectiveChaos {
    let mut st = ChaosState::default();
    st.insert(
        node,
        NodeChaos {
            latency_add: Duration::from_millis(latency_ms),
            error_rate: if hang { 1.0 } else { 0.0 },
            error_kind: ErrorKind::Hang,
            ..NodeChaos::default()
        },
    );
    compile_layers(&[&st])
}

fn entry_with(t: &Topology, chaos: &EffectiveChaos, plan: &Plan, fin: &Finished) -> LogEntry {
    let run = Run {
        plan,
        fin,
        chaos,
        topo: t,
        pin_delay: false,
    };
    let ans = Answer {
        status: 200,
        pin: None,
        failed: None,
    };
    build_entry(facts(), &ans, &run, String::new())
}

#[test]
fn a_cache_hang_is_a_fault_not_injected_delay() {
    let t = topo();
    let c = node(&t, "c");
    // The core charges a hanging cache lookup its node `timeout` (else HANG_CAP) and ignores
    // its latency_add; it never emits a Fail, so the request succeeds but a fault was injected.
    let hung = |node: usize, dur: Duration| FinishedHop {
        idx: 0,
        hop: Hop {
            node,
            replica: 0,
            kind: HopKind::CacheMiss,
            start: Duration::ZERO,
            dur,
        },
        wait: Duration::ZERO,
    };
    let case = |node: usize, dur: Duration| {
        let h = hung(node, dur);
        let plan = Plan {
            items: vec![PlanItem::Hop(h.hop.clone())],
        };
        let fin = finished(vec![h], ChainResult::Ok, None);
        entry_with(&t, &chaos_of(node, 40, true), &plan, &fin)
    };
    let e = case(c, HANG_CAP);
    assert_eq!(e.flags, ["FI"]);
    assert_eq!(e.hops[0].why.as_deref(), Some("chaos hang"));
    // With a node timeout the hang is charged the timeout: same answer.
    let ct = node(&t, "ct");
    let e = case(ct, Duration::from_millis(150));
    assert_eq!(
        (e.flags.as_slice(), e.hops[0].why.as_deref()),
        (&["FI"][..], Some("chaos hang"))
    );
    // The same lookup with latency but no hang is attributed as delay.
    let (plan, fin) = straight(vec![hop(c, HopKind::CacheMiss, 0, 42)]);
    let e = entry_with(&t, &chaos_of(c, 40, false), &plan, &fin);
    assert_eq!(e.hops[0].why.as_deref(), Some("chaos +40ms"));
    assert_eq!(e.flags, ["DI"]);
    // A lookup cut to its timeout with no hang fault is not attributed.
    let (plan, fin) = straight(vec![hop(ct, HopKind::CacheMiss, 0, 150)]);
    let e = entry_with(&t, &chaos_of(ct, 40, false), &plan, &fin);
    assert_eq!((e.flags.len(), e.hops[0].why.as_deref()), (0, None));
}

#[test]
fn a_deadline_cut_hop_never_reports_more_than_it_ran() {
    let t = topo();
    let a = node(&t, "app");
    // Planned 301 ms (300 injected), cut to 150 ms by a deadline.
    let (plan, mut fin) = straight(vec![hop(a, HopKind::Proc, 0, 301)]);
    fin.hops[0].hop.dur = Duration::from_millis(150);
    let e = entry_with(&t, &chaos_of(a, 300, false), &plan, &fin);
    assert_eq!(e.hops[0].why.as_deref(), Some("chaos +150ms"));
    assert!(e.hops[0].cut && e.hops[0].dur_ms >= 150.0);
}

#[test]
fn chain_error_survives_error_map_and_pin() {
    let t = topo();
    let d = node(&t, "db");
    let (plan, mut fin) = many(&t, 1);
    fin.result = err(d);
    fin.cause = Some(FailCause::Chaos);
    // A pin hid the failure: 200, no failed node, but the chain's own error is kept.
    let e = entry_of(&t, &plan, &fin, Some("success"));
    assert_eq!(
        e.chain_error,
        Some(ChainErrorRec {
            node: "db".into(),
            status: 503,
            kind: "status",
            cause: "chaos"
        })
    );
    assert_eq!((e.status, e.failed_node.as_deref()), (200, None));
    // `error_map` made the client see 502 from "db": the chain error is still the 503.
    let chaos = EffectiveChaos::default();
    let run = Run {
        plan: &plan,
        fin: &fin,
        chaos: &chaos,
        topo: &t,
        pin_delay: false,
    };
    let failed = ("db".to_string(), FailKind::Status);
    let ans = Answer {
        status: 502,
        pin: None,
        failed: Some(&failed),
    };
    let e = build_entry(facts(), &ans, &run, String::new());
    assert_eq!(e.chain_error.as_ref().map(|c| c.status), Some(503));
    assert_eq!(
        (e.status, e.failed_node.as_deref(), e.kind),
        (502, Some("db"), None)
    );
    // Rate limit, overflow and deadline name themselves.
    for (cause, kind) in [
        (FailCause::RateLimit, "rate-limit"),
        (FailCause::Overflow, "overflow"),
        (FailCause::Deadline, "deadline"),
    ] {
        fin.cause = Some(cause);
        let e = entry_of(&t, &plan, &fin, None);
        assert_eq!(e.chain_error.unwrap().cause, kind);
    }
    // A reset keeps its kind (what the header prints), even under a pin.
    fin.result = ChainResult::Err {
        node: d,
        status: 502,
        kind: FailKind::Reset,
        retry_after: None,
    };
    let e = entry_of(&t, &plan, &fin, Some("success"));
    assert_eq!(e.chain_error.unwrap().kind, "reset");
}

#[test]
fn lanes_for_a_parallel_block() {
    let t = topo();
    let a = node(&t, "app");
    let h = |s| hop(a, HopKind::Proc, s, 1);
    let items = vec![
        PlanItem::Hop(h(0)),
        PlanItem::Fork {
            branches: 2,
            size: 4,
        },
        PlanItem::Branch { size: 1 },
        PlanItem::Hop(h(1)),
        PlanItem::Branch { size: 1 },
        PlanItem::Hop(h(1)),
        PlanItem::Hop(h(2)),
    ];
    let fin = finished(
        [0, 3, 5, 6]
            .into_iter()
            .map(|idx| {
                let PlanItem::Hop(x) = &items[idx] else {
                    panic!()
                };
                FinishedHop {
                    idx,
                    hop: x.clone(),
                    wait: Duration::ZERO,
                }
            })
            .collect(),
        ChainResult::Ok,
        None,
    );
    let e = entry_of(&t, &Plan { items }, &fin, None);
    let got: Vec<_> = e.hops.iter().map(|h| (h.lane, h.fork)).collect();
    assert_eq!(got, [(0, None), (1, Some(0)), (2, Some(0)), (0, None)]);
}

#[test]
fn a_hop_shorter_than_planned_is_cut() {
    let t = topo();
    let a = node(&t, "app");
    let (plan, mut fin) = straight(vec![hop(a, HopKind::Proc, 0, 10)]);
    fin.hops[0].hop.dur = Duration::from_millis(4);
    let e = entry_of(&t, &plan, &fin, None);
    assert!(e.hops[0].cut);
    assert_eq!(e.hops[0].dur_ms, 4.0);
}

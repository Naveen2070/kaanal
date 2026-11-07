//! The run log: a virtual-time sample of the requests of one run, one line per hop.
//!
//! [`sample`] is the one place the sampling rule lives; [`page`] filters and pages the stored
//! lines for `GET /sim/runs/{id}/log`. Both are pure, so they are tested here on a hand-built
//! report.

use kaanal_core::{ChainResult, Endpoints, HopKind, Topology};
use kaanal_sim::{Cause, RequestRecord, SimConfig, SimReport};
use serde::Serialize;

/// Failed requests kept, by arrival.
pub(crate) const MAX_FAILED: usize = 1000;
/// Slowest requests kept.
pub(crate) const SLOWEST: usize = 500;
/// Requests kept in all (failed, slowest and the even spread).
pub(crate) const MAX_REQUESTS: usize = 4000;
/// Lines kept per run: with eight runs kept, the memory bound of the log.
pub(crate) const MAX_LINES: usize = 20_000;

/// One line of the run log.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LogLine {
    /// Virtual time of the hop (arrival plus the hop's start), milliseconds.
    pub t_ms: f64,
    /// `info`, `warn` or `error`.
    pub level: &'static str,
    /// The stream's request counter.
    pub request: u64,
    /// The endpoint key of the request's stream.
    pub stream: String,
    /// The node the hop ran on.
    pub node: String,
    /// For a hop on a wire (`link request`, `link response`), the node that called over it; the
    /// wire is `from` to `node`. Absent on every other line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// The replica that ran it; `None` on a failure line (the engine does not record it).
    pub replica: Option<u16>,
    /// `served`, `queued`, `failed`, `limited`, `overflowed`, `timeout`, `short-circuited` or `retry` (a
    /// second or later attempt of a call starting; it takes no time of its own).
    pub event: &'static str,
    /// Queue wait before the hop, milliseconds; `None` on a failure line.
    pub wait_ms: Option<f64>,
    /// The hop's own duration, milliseconds; `None` on a failure line.
    pub service_ms: Option<f64>,
    /// The status of a failure.
    pub status: Option<u16>,
    /// What the hop was (`proc`, `net`, ...), or why a request failed.
    pub note: Option<String>,
    /// The retry (2 or more) a hop ran in; absent in a first attempt and on every other line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u8>,
}

/// The stored log of a run.
#[derive(Debug, Default)]
pub struct LogSample {
    /// The lines, in virtual-time order.
    pub lines: Vec<LogLine>,
    /// True when the line cap kept the rule from keeping every request it would have kept.
    pub truncated: bool,
}

fn ms(d: std::time::Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn hop_label(k: HopKind) -> &'static str {
    match k {
        HopKind::Net => "net",
        HopKind::Connect { resp: false } => "connect",
        HopKind::Connect { resp: true } => "connect response",
        HopKind::FirstByte => "first byte",
        HopKind::Retransmit { resp: false } => "retransmit",
        HopKind::Retransmit { resp: true } => "retransmit response",
        HopKind::Proc => "proc",
        HopKind::Op(_) => "op",
        HopKind::CacheHit => "cache hit",
        HopKind::CacheMiss => "cache miss",
        HopKind::Delay => "delay",
        HopKind::Link { resp: false, .. } => "link request",
        HopKind::Link { resp: true, .. } => "link response",
        HopKind::Nic { send: true, .. } => "node uplink",
        HopKind::Nic { send: false, .. } => "node downlink",
    }
}

fn failed(r: &RequestRecord) -> bool {
    matches!(r.result, ChainResult::Err { .. })
}

/// The retries a request made (none for most).
fn retries<'a>(rep: &'a SimReport, r: &RequestRecord) -> &'a [kaanal_sim::Attempt] {
    rep.attempts
        .get(&(r.stream, r.n))
        .map_or(&[], Vec::as_slice)
}

/// Lines a request yields: one per hop, and one for its failure.
fn cost(rep: &SimReport, r: &RequestRecord) -> usize {
    r.hops.len() + retries(rep, r).len() + usize::from(failed(r))
}

/// The lines of one request: a line per hop, then one for the failure, if any.
fn lines_of(
    rep: &SimReport,
    r: &RequestRecord,
    topo: &Topology,
    cfg: &SimConfig,
    out: &mut Vec<LogLine>,
) {
    let stream = cfg.streams[r.stream].endpoint_key.clone();
    let arrival = ms(r.arrival);
    let tries = rep.hop_attempts.get(&(r.stream, r.n));
    for (k, (h, wait)) in r.hops.iter().enumerate() {
        let (w, s) = (ms(*wait), ms(h.dur));
        let queued = w > s;
        out.push(LogLine {
            t_ms: arrival + ms(h.start),
            level: if queued { "warn" } else { "info" },
            request: r.n,
            stream: stream.clone(),
            node: topo.nodes[h.node].name.clone(),
            from: match h.kind {
                HopKind::Link { edge, .. } => {
                    Some(topo.nodes[topo.edges[usize::from(edge)].from].name.clone())
                }
                _ => None,
            },
            replica: Some(h.replica),
            event: if queued { "queued" } else { "served" },
            wait_ms: Some(w),
            service_ms: Some(s),
            status: None,
            note: Some(hop_label(h.kind).to_string()),
            attempt: tries.and_then(|t| t.get(k)).copied().filter(|&n| n > 1),
        });
    }
    for &(at, n, of, node) in retries(rep, r) {
        out.push(LogLine {
            t_ms: arrival + ms(at),
            level: "info",
            request: r.n,
            stream: stream.clone(),
            node: topo.nodes[node].name.clone(),
            from: None,
            replica: None,
            event: "retry",
            wait_ms: None,
            service_ms: None,
            status: None,
            note: Some(format!("attempt {n} of {of}")),
            attempt: None,
        });
    }
    if let ChainResult::Err {
        node, status, kind, ..
    } = &r.result
    {
        // the engine's own cause, never a guess from the status
        let (event, note) = match r.cause {
            Some(Cause::RateLimit) => ("limited", "rate limit"),
            Some(Cause::Overflow) => ("overflowed", "queue full"),
            Some(Cause::Deadline) => ("timeout", "deadline passed"),
            Some(Cause::Breaker) => ("short-circuited", "circuit breaker open"),
            Some(Cause::Chaos) | None => ("failed", kind.as_str()),
        };
        out.push(LogLine {
            t_ms: arrival + ms(r.latency),
            level: "error",
            request: r.n,
            stream,
            node: topo.nodes[*node].name.clone(),
            from: None,
            replica: None,
            event,
            wait_ms: None,
            service_ms: None,
            status: Some(*status),
            note: Some(note.to_string()),
            attempt: None,
        });
    }
}

/// Picks the requests to log, by line budget (whole requests only): every failure (the first
/// 1000), the 500 slowest of the rest, then an even spread of what is left, 4000 requests in
/// all. The spread is sized to the lines still free, and keeps its stride across the whole run.
/// Returns indexes into `report.requests` and whether the cap cost requests.
fn pick(report: &SimReport) -> (Vec<usize>, bool) {
    let reqs = &report.requests;
    let mut taken = vec![false; reqs.len()];
    let (mut keep, mut used) = (Vec::new(), 0usize);
    let fail_ids: Vec<usize> = (0..reqs.len())
        .filter(|i| failed(&reqs[*i]))
        .take(MAX_FAILED)
        .collect();
    for &i in &fail_ids {
        taken[i] = true;
    }
    let mut slow: Vec<usize> = (0..reqs.len()).filter(|i| !taken[*i]).collect();
    let by_latency = |a: &usize, b: &usize| reqs[*b].latency.cmp(&reqs[*a].latency).then(a.cmp(b));
    if SLOWEST < slow.len() {
        slow.select_nth_unstable_by(SLOWEST, by_latency);
        slow.truncate(SLOWEST);
    }
    slow.sort_unstable_by(by_latency);
    for &i in &slow {
        taken[i] = true;
    }
    // failures first (arrival order), then the slowest; the first that does not fit ends it
    for &i in fail_ids.iter().chain(&slow) {
        let c = cost(report, &reqs[i]);
        if used + c > MAX_LINES {
            return (keep, true);
        }
        used += c;
        keep.push(i);
    }
    let spread: Vec<usize> = (0..reqs.len()).filter(|i| !taken[*i]).collect();
    let want = MAX_REQUESTS.saturating_sub(keep.len()).min(spread.len());
    let free = MAX_LINES - used;
    let stride =
        |n: usize| -> Vec<usize> { (0..n).map(|k| spread[k * spread.len() / n]).collect() };
    let mut n = want;
    loop {
        let picks = stride(n);
        let sum: usize = picks.iter().map(|&i| cost(report, &reqs[i])).sum();
        if sum <= free {
            keep.extend(picks);
            return (keep, n < want);
        }
        // sum > free >= 0, so n > 0; fewer picks, same stride over the whole run
        n = (n * free / sum).min(n - 1);
    }
}

/// The log of `report`: the sample rule above, each kept request as a line per hop (and one for
/// its failure), all in virtual-time order, at most [`MAX_LINES`], never half a request.
#[must_use]
pub(crate) fn sample(
    report: &SimReport,
    topo: &Topology,
    _endpoints: &Endpoints,
    cfg: &SimConfig,
) -> LogSample {
    let (keep, truncated) = pick(report);
    let mut lines = Vec::new();
    for i in keep {
        lines_of(report, &report.requests[i], topo, cfg, &mut lines);
    }
    lines.sort_by(|a, b| a.t_ms.total_cmp(&b.t_ms));
    LogSample { lines, truncated }
}

/// What a log request asks for; the filters AND together.
#[derive(Debug, Default)]
pub(crate) struct Filter {
    /// Node name; an unknown one matches nothing.
    pub node: Option<String>,
    /// A wire as (calling node, called node): only the hops on that wire.
    pub wire: Option<(String, String)>,
    /// Replica number; matches only lines whose replica is known and equal.
    pub replica: Option<u16>,
    /// Endpoint key of the request's stream.
    pub endpoint: Option<String>,
    /// `info`, `warn` or `error`.
    pub level: Option<String>,
    /// The stream's request counter (per stream: pair it with `endpoint`).
    pub request: Option<u64>,
}

impl Filter {
    fn matches(&self, l: &LogLine) -> bool {
        self.node.as_ref().is_none_or(|n| *n == l.node)
            && self
                .wire
                .as_ref()
                .is_none_or(|(a, b)| l.from.as_deref() == Some(a.as_str()) && *b == l.node)
            && self.replica.is_none_or(|r| l.replica == Some(r))
            && self.endpoint.as_ref().is_none_or(|e| *e == l.stream)
            && self.level.as_ref().is_none_or(|v| v == l.level)
            && self.request.is_none_or(|n| n == l.request)
    }
}

/// One page: the matching lines after index `after` (the last stored index the caller has
/// seen), at most `limit`, and `next` (the stored index to pass as `after`), `None` at the end.
pub(crate) fn page<'a>(
    lines: &'a [LogLine],
    f: &Filter,
    after: Option<usize>,
    limit: usize,
) -> (Vec<&'a LogLine>, Option<usize>) {
    let start = after.map_or(0, |a| a.saturating_add(1)).min(lines.len());
    let mut hits = lines[start..]
        .iter()
        .enumerate()
        .filter(|(_, l)| f.matches(l));
    let got: Vec<(usize, &LogLine)> = hits.by_ref().take(limit).collect();
    let next = match (got.last(), hits.next()) {
        (Some((i, _)), Some(_)) => Some(start + i),
        _ => None,
    };
    (got.into_iter().map(|(_, l)| l).collect(), next)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use kaanal_core::{Config, FailKind};
    use kaanal_sim::Stream;
    use std::time::Duration;

    pub(super) fn lined(r: &SimReport, t: &Topology, e: &Endpoints, c: &SimConfig) -> Vec<LogLine> {
        sample(r, t, e, c).lines
    }

    pub(super) fn topo() -> Topology {
        let cfg = Config::from_yaml(
            "version: 1\nnodes:\n  nginx: { type: gateway, proc: 1ms }\n  app: { type: service, proc: 1ms }\nedges:\n  - { from: nginx, to: app, net: 1ms }\n",
        )
        .unwrap();
        Topology::build(&cfg).unwrap()
    }

    pub(super) fn rec(n: u64, lat_ms: u64, fail: bool, topo: &Topology) -> RequestRecord {
        let hop = kaanal_core::Hop {
            node: topo.id("app").unwrap(),
            replica: 0,
            kind: kaanal_core::HopKind::Proc,
            start: Duration::ZERO,
            dur: Duration::from_millis(lat_ms),
        };
        RequestRecord {
            stream: 0,
            n,
            arrival: Duration::from_millis(n),
            latency: Duration::from_millis(lat_ms),
            result: if fail {
                ChainResult::Err {
                    node: topo.id("app").unwrap(),
                    status: 500,
                    kind: FailKind::Status,
                    retry_after: None,
                }
            } else {
                ChainResult::Ok
            },
            cause: fail.then_some(kaanal_sim::Cause::Chaos),
            holds: Vec::new(),
            hops: vec![(hop, Duration::ZERO)],
        }
    }

    pub(super) fn cfg() -> SimConfig {
        SimConfig {
            streams: vec![Stream {
                endpoint_key: "GET /x".into(),
                rate_per_s: 1.0,
                req_bytes: 0,
                resp_bytes: 0,
            }],
            duration: Duration::from_secs(1),
            warmup: Duration::ZERO,
            seed: 1,
        }
    }

    pub(super) fn report(requests: Vec<RequestRecord>) -> SimReport {
        SimReport {
            requests,
            nodes: vec![],
            replica_utilization: vec![],
            link_utilization: vec![],
            nic_utilization: vec![],
            attempts: std::collections::BTreeMap::default(),
            hop_attempts: std::collections::BTreeMap::default(),
            wires: vec![],
            breaker_changes: vec![],
        }
    }

    #[test]
    fn a_retry_is_a_line_of_its_own_between_the_hops() {
        let t = topo();
        let app = t.id("app").unwrap();
        let mut rep = report(vec![rec(1, 10, false, &t)]);
        rep.attempts
            .insert((0, 1), vec![(Duration::from_millis(4), 2, 3, app)]);
        let lines = lined(&rep, &t, &Endpoints::default(), &cfg());
        let retry = lines
            .iter()
            .find(|l| l.event == "retry")
            .expect("a retry line");
        assert_eq!(retry.note.as_deref(), Some("attempt 2 of 3"));
        assert_eq!((retry.node.as_str(), retry.level), ("app", "info"));
        assert!(
            (retry.t_ms - 5.0).abs() < 1e-9,
            "arrival 1 ms plus the 4 ms offset: {}",
            retry.t_ms
        );
        assert!(retry.wait_ms.is_none() && retry.service_ms.is_none() && retry.status.is_none());
    }

    #[test]
    fn a_hop_line_says_which_retry_ran_it() {
        let t = topo();
        let app = t.id("app").unwrap();
        let mut r = rec(1, 10, false, &t);
        let first = r.hops[0].clone();
        r.hops.push(first);
        let mut rep = report(vec![r]);
        rep.attempts
            .insert((0, 1), vec![(Duration::from_millis(4), 2, 3, app)]);
        rep.hop_attempts.insert((0, 1), vec![1, 2]);
        let lines = lined(&rep, &t, &Endpoints::default(), &cfg());
        let tries: Vec<Option<u8>> = lines
            .iter()
            .filter(|l| l.event != "retry")
            .map(|l| l.attempt)
            .collect();
        assert_eq!(tries, vec![None, Some(2)]);
    }

    #[test]
    fn failures_and_the_slowest_are_kept_and_the_total_is_bounded() {
        let t = topo();
        let mut v: Vec<_> = (0..20_000).map(|n| rec(n, 1, n % 1000 == 7, &t)).collect();
        v[12_345].latency = Duration::from_millis(900);
        v[12_345].hops[0].0.dur = Duration::from_millis(900);
        let lines = lined(&report(v), &t, &Endpoints::default(), &cfg());
        let errors = lines.iter().filter(|l| l.level == "error").count();
        assert_eq!(errors, 20, "every failed request has its failure line");
        assert!(lines.iter().any(|l| l.request == 12_345), "the slowest");
        // 20 failed + 500 slowest + spread = 4000 requests; failed ones have 2 lines
        assert_eq!(lines.len(), 4000 + 20);
        assert!(lines.windows(2).all(|w| w[0].t_ms <= w[1].t_ms), "in order");
    }

    #[test]
    fn the_line_cap_cuts_and_a_small_run_is_kept_whole() {
        let t = topo();
        let all_fail: Vec<_> = (0..5000).map(|n| rec(n, 1, true, &t)).collect();
        let lines = lined(&report(all_fail), &t, &Endpoints::default(), &cfg());
        // 1000 failed (2 lines each) + 500 slowest (2) + spread up to 4000 requests (2)
        assert_eq!(lines.len(), 8000);
        let small = lined(
            &report((0..10).map(|n| rec(n, 1, false, &t)).collect()),
            &t,
            &Endpoints::default(),
            &cfg(),
        );
        assert_eq!(small.len(), 10);
    }

    #[test]
    fn a_wire_filter_keeps_only_the_hops_on_that_wire() {
        let cfg_yaml = Config::from_yaml(
            "version: 1\nnodes:\n  nginx: { type: gateway, proc: 1ms }\n  app: { type: service, proc: 1ms }\nedges:\n  - { from: nginx, to: app, bandwidth: 1000 }\n",
        )
        .unwrap();
        let t = Topology::build(&cfg_yaml).unwrap();
        let mut r = rec(1, 5, false, &t);
        r.hops.push((
            kaanal_core::Hop {
                node: t.id("app").unwrap(),
                replica: 0,
                kind: kaanal_core::HopKind::Link {
                    edge: 0,
                    resp: false,
                },
                start: Duration::ZERO,
                dur: Duration::from_millis(3),
            },
            Duration::ZERO,
        ));
        let lines = lined(&report(vec![r]), &t, &Endpoints::default(), &cfg());
        assert_eq!(lines.len(), 2);
        let wire = Filter {
            wire: Some(("nginx".into(), "app".into())),
            ..Filter::default()
        };
        let got = page(&lines, &wire, None, 10).0;
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].from.as_deref(), Some("nginx"));
        assert_eq!(got[0].note.as_deref(), Some("link request"));
        let backwards = Filter {
            wire: Some(("app".into(), "nginx".into())),
            ..Filter::default()
        };
        assert_eq!(page(&lines, &backwards, None, 10).0.len(), 0);
    }

    #[test]
    fn a_wait_longer_than_the_service_is_queued_and_paging_follows_next() {
        let t = topo();
        let mut r = rec(1, 5, false, &t);
        r.hops[0].1 = Duration::from_millis(20);
        let lines = lined(
            &report(vec![r, rec(2, 5, true, &t)]),
            &t,
            &Endpoints::default(),
            &cfg(),
        );
        assert_eq!(lines[0].event, "queued");
        assert_eq!(lines[0].level, "warn");
        let all = Filter::default();
        let (p, next) = page(&lines, &all, None, 2);
        assert_eq!((p.len(), next), (2, Some(1)));
        let (p, next) = page(&lines, &all, next, 2);
        assert_eq!((p.len(), next), (1, None));
        let errs = Filter {
            level: Some("error".into()),
            ..Filter::default()
        };
        assert_eq!(page(&lines, &errs, None, 10).0.len(), 1);
        let nobody = Filter {
            node: Some("nope".into()),
            ..Filter::default()
        };
        assert_eq!(page(&lines, &nobody, None, 10).0.len(), 0);
        let one = Filter {
            request: Some(lines[0].request),
            ..Filter::default()
        };
        assert!(
            page(&lines, &one, None, 10)
                .0
                .iter()
                .all(|l| l.request == lines[0].request)
        );
        let none = Filter {
            request: Some(u64::MAX),
            ..Filter::default()
        };
        assert_eq!(page(&lines, &none, None, 10).0.len(), 0);
    }
}

#[cfg(test)]
mod cap_tests {
    use super::tests::*;
    use super::*;
    use std::collections::BTreeMap;

    /// `n` requests of `hops` hops each, arriving 1 ms apart.
    fn fat(n: u64, hops: usize, fail_every: u64, t: &Topology) -> Vec<RequestRecord> {
        (0..n)
            .map(|i| {
                let mut r = rec(i, 1, fail_every > 0 && i % fail_every == 0, t);
                let h = r.hops[0].clone();
                r.hops = vec![h; hops];
                r
            })
            .collect()
    }

    fn per_request(lines: &[LogLine]) -> BTreeMap<u64, usize> {
        let mut m = BTreeMap::new();
        for l in lines {
            *m.entry(l.request).or_insert(0) += 1;
        }
        m
    }

    #[test]
    fn the_spread_covers_the_whole_run_under_the_line_cap_and_no_request_is_partial() {
        let t = topo();
        let rep = report(fat(10_000, 6, 0, &t));
        let lines = lined(&rep, &t, &Endpoints::default(), &cfg());
        assert!(lines.len() <= MAX_LINES, "{}", lines.len());
        let by = per_request(&lines);
        assert!(by.values().all(|&c| c == 6), "a request is never partial");
        assert!(by.keys().any(|&n| n < 1000), "first tenth");
        assert!(by.keys().any(|&n| n >= 9000), "last tenth");
        assert_eq!(lines, lined(&rep, &t, &Endpoints::default(), &cfg()));
    }

    #[test]
    fn failures_past_the_cap_are_cut_on_a_request_boundary_in_arrival_order() {
        let t = topo();
        let lines = lined(
            &report(fat(5000, 30, 1, &t)),
            &t,
            &Endpoints::default(),
            &cfg(),
        );
        assert!(lines.len() <= MAX_LINES);
        let by = per_request(&lines);
        assert!(
            by.values().all(|&c| c == 31),
            "30 hops and the failure line"
        );
        assert!(by.contains_key(&0), "the first failure is kept");
    }
}

#[cfg(test)]
mod cause_tests {
    use super::tests::*;
    use super::*;

    fn event(cause: Cause) -> LogLine {
        let t = topo();
        let mut r = rec(1, 1, true, &t);
        r.cause = Some(cause);
        let l = lined(&report(vec![r]), &t, &Endpoints::default(), &cfg());
        l.into_iter().find(|l| l.level == "error").unwrap()
    }

    #[test]
    fn the_event_comes_from_the_engines_cause_not_the_status() {
        // the same 500 status in all four: only the cause tells them apart
        let want = [
            (Cause::Chaos, "failed"),
            (Cause::Overflow, "overflowed"),
            (Cause::RateLimit, "limited"),
            (Cause::Deadline, "timeout"),
        ];
        for (c, e) in want {
            let l = event(c);
            assert_eq!((l.event, l.status), (e, Some(500)));
            assert_eq!((l.replica, l.wait_ms, l.service_ms), (None, None, None));
        }
    }
}

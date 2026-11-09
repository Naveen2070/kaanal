//! The request log: short ids, a bounded ring of finished requests, and the query filters.
//!
//! Pure data and logic (no `.await`, no HTTP types): the handlers build a [`LogEntry`] when a
//! request completes, push it into the [`LogRing`], and the control API queries it.
//!
//! Two orders exist on purpose. `seq` is arrival order (the number in the id, taken when the
//! request starts); `done` is completion order (assigned by the ring on push). A slow request
//! gets a low `seq` but a high `done`, so eviction answers use membership plus a high-water mark,
//! never `seq < oldest`.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};

use kaanal_core::scenario::endpoint_key;
use serde::Serialize;

/// Entries the ring keeps (the last N completions).
pub const LOG_CAP: usize = 1000;
/// `HopRec`s kept per entry; more sets `truncated`.
pub const MAX_HOPS: usize = 512;
/// Bytes of trace-header text kept per entry.
pub const MAX_TRACE: usize = 16 * 1024;

/// Hands out request ids: `r<boot>-<n>`, the boot nonce drawn once per start, `n` from 1.
pub struct IdGen {
    boot: u16,
    next: AtomicU64,
}

impl IdGen {
    /// A generator with a random boot nonce.
    pub fn new() -> Self {
        Self::with_boot(rand::random::<u16>())
    }

    /// A generator with a fixed boot nonce (tests).
    pub fn with_boot(boot: u16) -> Self {
        Self {
            boot,
            next: AtomicU64::new(1),
        }
    }

    /// The next `(seq, id text)`. Never resets, not even on `POST /reset`.
    pub fn next(&self) -> (u64, String) {
        let seq = self.next.fetch_add(1, Ordering::Relaxed);
        (seq, format_id(self.boot, seq))
    }

    /// The boot nonce.
    pub fn boot(&self) -> u16 {
        self.boot
    }
}

impl Default for IdGen {
    fn default() -> Self {
        Self::new()
    }
}

/// `r04f2-0042`: `r`, four lowercase hex digits, `-`, the number zero-padded to 4 digits.
pub fn format_id(boot: u16, seq: u64) -> String {
    format!("r{boot:04x}-{seq:04}")
}

/// Parses an id (`^r[0-9a-f]{4}-[0-9]{4,}$`) into `(boot, seq)`.
pub fn parse_id(text: &str) -> Option<(u16, u64)> {
    let (boot, seq) = text.strip_prefix('r')?.split_once('-')?;
    let hex = |b: u8| b.is_ascii_digit() || (b'a'..=b'f').contains(&b);
    if boot.len() != 4
        || !boot.bytes().all(hex)
        || seq.len() < 4
        || !seq.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    Some((u16::from_str_radix(boot, 16).ok()?, seq.parse().ok()?))
}

/// A client `X-Request-Id` worth keeping: `^[A-Za-z0-9._-]{1,64}$`.
pub fn client_id(raw: &str) -> Option<String> {
    let ok = (1..=64).contains(&raw.len())
        && raw
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    ok.then(|| raw.to_string())
}

/// One node's slice of the trace, for the waterfall. `lane` and `fork` name the innermost
/// `parallel` block and branch only; nested blocks flatten.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HopRec {
    /// Node name.
    pub node: String,
    /// Which copy of the node ran the hop; only for a node with more than one replica.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replica: Option<u16>,
    /// `net`, `proc`, `op`, `hit`, `miss` or `delay`.
    pub kind: &'static str,
    /// The database operation (`read`, `write`, `scan`) of an `op` hop.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub op: Option<&'static str>,
    /// Offset from the request start, simulated ms.
    pub start_ms: f64,
    /// Time the hop ran, simulated ms.
    pub dur_ms: f64,
    /// Time spent queued before it ran, simulated ms.
    pub wait_ms: f64,
    /// `0` for the main path; a fork's branches get `1..`.
    pub lane: u16,
    /// The fork's ordinal, for hops inside a parallel block.
    pub fork: Option<u16>,
    /// A failure cut this hop.
    pub cut: bool,
    /// Per-hop attribution, e.g. `chaos +40ms` or `pin delay`.
    pub why: Option<String>,
    /// The retry (2 or more) this hop belongs to, as the trace's `retry=N/M` marker says; absent in a
    /// first attempt and outside any retried call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u8>,
}

/// What the chain failed with before `error_map` and before a pin hid it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChainErrorRec {
    /// The failing node.
    pub node: String,
    /// The status it failed with.
    pub status: u16,
    /// How it failed, as the trace header prints it: `status`, `reset`, `malformed` or `timeout`.
    pub kind: &'static str,
    /// Why: `chaos`, `rate-limit`, `overflow` or `deadline`.
    pub cause: &'static str,
}

/// One finished request.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LogEntry {
    /// Our id text (`r04f2-0042`).
    pub id: String,
    /// The client's `X-Request-Id`, when it matched the grammar.
    pub client_id: Option<String>,
    /// Arrival order, the id's number.
    pub seq: u64,
    /// Completion order; set by [`LogRing::push`].
    pub done: u64,
    /// UTC ms since the epoch.
    pub ts_ms: u64,
    /// Uppercase HTTP method.
    pub method: String,
    /// Request path.
    pub path: String,
    /// The matched route key; `None` when unmatched.
    pub endpoint: Option<String>,
    /// What was sent.
    pub status: u16,
    /// Measured wall time, ms.
    pub ms: u64,
    /// Simulated total, ms.
    pub trace_ms: f64,
    /// Canonical scope key; `Some("")` for an empty header.
    pub scope: Option<String>,
    /// The UI state a pin forced.
    pub pin: Option<&'static str>,
    /// Response flags (`FI`, `DI`, `PD`, `RL`, `UO`, `DT`, `PN`).
    pub flags: Vec<&'static str>,
    /// What the chain failed with, even when a pin hid it.
    pub chain_error: Option<ChainErrorRec>,
    /// The node whose failure the client saw; `None` under a pin.
    pub failed_node: Option<String>,
    /// How it failed when not a plain status (`reset`, `malformed`, `timeout`).
    pub kind: Option<&'static str>,
    /// `unmatched`, `bad-scope`, `engine-stopped` or `client-gone` (the client left first).
    pub rejected: Option<&'static str>,
    /// A live payload edit (step 8A) answered it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub edited: bool,
    /// The trace-header text, at most [`MAX_TRACE`] bytes.
    pub trace: String,
    /// At most [`MAX_HOPS`] hops.
    pub hops: Vec<HopRec>,
    /// More hops existed than were kept.
    pub truncated: bool,
}

/// `status` filter item: `404` or `5xx`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusMatch {
    /// One status code.
    Exact(u16),
    /// A class: `5` matches 500-599.
    Class(u8),
}

/// Query filters; an unset field matches everything.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filter {
    /// Normalised route key.
    pub endpoint: Option<String>,
    /// Any of these (empty = all).
    pub status: Vec<StatusMatch>,
    /// Only requests with a `failed_node`.
    pub failed: bool,
    /// Exact canonical scope key (`Some("")` is the empty-header scope, not "any").
    pub scope: Option<String>,
    /// `ts_ms >= since_ms`.
    pub since_ms: Option<u64>,
    /// `done > after_done`.
    pub after_done: Option<u64>,
}

fn millis(d: std::time::Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

impl Filter {
    /// Builds a filter from query pairs. `limit` is accepted and ignored (the caller parses and
    /// bounds it). An unknown key, a repeated key, an empty value or a malformed value is an
    /// `Err` naming the parameter; nothing is partially applied. The one empty value allowed is
    /// `scope=`, the empty-header scope. `scope` is matched exactly as echoed by the API (the
    /// names joined by `,` in header order); `scope.rs` has no string canonicaliser to reuse.
    ///
    /// # Errors
    /// The message names the offending parameter.
    pub fn from_params(params: &[(String, String)], now_ms: u64) -> Result<Filter, String> {
        let mut f = Filter::default();
        let mut seen: Vec<&str> = Vec::new();
        for (k, v) in params {
            if seen.contains(&k.as_str()) {
                return Err(format!("parameter '{k}' is repeated"));
            }
            seen.push(k);
            if v.is_empty() && k != "scope" {
                return Err(format!("parameter '{k}' is empty"));
            }
            let bad = |why: &str| format!("parameter '{k}': {why}");
            match k.as_str() {
                "endpoint" => f.endpoint = Some(endpoint_key(v).map_err(|e| bad(&e))?),
                "status" => {
                    f.status = v
                        .split(',')
                        .map(|s| {
                            parse_status(s).ok_or_else(|| bad(&format!("invalid status '{s}'")))
                        })
                        .collect::<Result<_, _>>()?;
                }
                "failed" => {
                    f.failed = match v.as_str() {
                        "true" => true,
                        "false" => false,
                        _ => return Err(bad("expected true or false")),
                    }
                }
                "scope" => f.scope = Some(v.clone()),
                "since" => {
                    f.since_ms = Some(match humantime::parse_duration(v) {
                        Ok(d) => now_ms.saturating_sub(millis(d)),
                        Err(_) => humantime::parse_rfc3339(v)
                            .ok()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(millis)
                            .ok_or_else(|| {
                                bad("expected a duration like 5m or an RFC 3339 time")
                            })?,
                    });
                }
                "after" => {
                    f.after_done = Some(digits(v).ok_or_else(|| bad("expected an integer"))?);
                }
                "limit" => {}
                _ => return Err(format!("unknown parameter '{k}'")),
            }
        }
        Ok(f)
    }
}

/// Plain decimal digits only (`str::parse` would also take a leading `+`).
fn digits<T: std::str::FromStr>(s: &str) -> Option<T> {
    s.bytes()
        .all(|b| b.is_ascii_digit())
        .then(|| s.parse().ok())?
}

fn parse_status(s: &str) -> Option<StatusMatch> {
    match s.as_bytes() {
        [c @ b'1'..=b'5', b'x' | b'X', b'x' | b'X'] => Some(StatusMatch::Class(c - b'0')),
        _ => digits(s)
            .filter(|n| (100..=599).contains(n))
            .map(StatusMatch::Exact),
    }
}

/// Whether `e` passes every set filter.
pub fn matches(e: &LogEntry, f: &Filter) -> bool {
    f.endpoint
        .as_ref()
        .is_none_or(|ep| e.endpoint.as_ref() == Some(ep))
        && (f.status.is_empty()
            || f.status.iter().any(|m| match *m {
                StatusMatch::Exact(n) => e.status == n,
                StatusMatch::Class(c) => e.status / 100 == u16::from(c),
            }))
        && (!f.failed || e.failed_node.is_some())
        && f.scope.as_ref().is_none_or(|s| e.scope.as_ref() == Some(s))
        && f.since_ms.is_none_or(|t| e.ts_ms >= t)
        && f.after_done.is_none_or(|d| e.done > d)
}

/// A page of query results, oldest completion first.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Page {
    /// The last `limit` matches, ordered by `done`.
    pub entries: Vec<LogEntry>,
    /// Id of the lowest `seq` in the ring.
    pub oldest: Option<String>,
    /// Id of the highest `seq` in the ring.
    pub newest: Option<String>,
    /// Ring capacity.
    pub cap: usize,
    /// Entries dropped by the cap so far.
    pub evicted: u64,
}

/// The answer to [`LogRing::find`].
#[derive(Debug, Clone, PartialEq)]
pub enum Lookup {
    /// In the ring.
    Found(Box<LogEntry>),
    /// This boot's id, at or below the highest recorded, but gone (or still in flight).
    Evicted,
    /// Another boot's id, a number above the highest recorded, or no such client id.
    Unknown,
    /// Neither one of our ids nor a valid client id.
    Malformed,
}

/// The last [`LOG_CAP`] completed requests.
pub struct LogRing {
    boot: u16,
    entries: VecDeque<LogEntry>,
    done: u64,
    high_water: u64,
    evicted: u64,
}

impl LogRing {
    /// An empty ring for ids of this boot.
    pub fn new(boot: u16) -> Self {
        Self {
            boot,
            entries: VecDeque::with_capacity(LOG_CAP + 1),
            done: 0,
            high_water: 0,
            evicted: 0,
        }
    }

    /// Records a completion: assigns and returns `done`, evicting the oldest completion past the cap.
    pub fn push(&mut self, mut e: LogEntry) -> u64 {
        self.done += 1;
        e.done = self.done;
        self.high_water = self.high_water.max(e.seq);
        self.entries.push_back(e);
        if self.entries.len() > LOG_CAP {
            self.entries.pop_front();
            self.evicted += 1;
        }
        self.done
    }

    /// Empties the ring; `done` and the high-water mark keep counting so old ids stay `Evicted`.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// The last `limit` entries matching `f`, oldest completion first.
    pub fn query(&self, f: &Filter, limit: usize) -> Page {
        let mut entries: Vec<LogEntry> = self
            .entries
            .iter()
            .rev()
            .filter(|e| matches(e, f))
            .take(limit)
            .cloned()
            .collect();
        entries.reverse();
        Page {
            entries,
            oldest: self
                .entries
                .iter()
                .min_by_key(|e| e.seq)
                .map(|e| e.id.clone()),
            newest: self
                .entries
                .iter()
                .max_by_key(|e| e.seq)
                .map(|e| e.id.clone()),
            cap: LOG_CAP,
            evicted: self.evicted,
        }
    }

    /// Looks `key` up. Precedence: the literal `last` (highest `done`), then one of our ids, then
    /// a client id (newest match); so a client id shaped like an id resolves as an id. Ids are
    /// parsed numerically, so `r04f2-00042` and `r04f2-0042` name the same entry.
    pub fn find(&self, key: &str) -> Lookup {
        let found = |e: &LogEntry| Lookup::Found(Box::new(e.clone()));
        if key == "last" {
            return self.entries.back().map_or(Lookup::Unknown, found);
        }
        if let Some((boot, seq)) = parse_id(key) {
            if boot != self.boot || seq == 0 {
                return Lookup::Unknown;
            }
            return match self.entries.iter().find(|e| e.seq == seq) {
                Some(e) => found(e),
                None if seq <= self.high_water => Lookup::Evicted,
                None => Lookup::Unknown,
            };
        }
        if client_id(key).is_none() {
            return Lookup::Malformed;
        }
        self.entries
            .iter()
            .rev()
            .find(|e| e.client_id.as_deref() == Some(key))
            .map_or(Lookup::Unknown, found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOOT: u16 = 0x4f2;

    fn entry(seq: u64) -> LogEntry {
        LogEntry {
            id: format_id(BOOT, seq),
            client_id: None,
            seq,
            done: 0,
            ts_ms: 1_000,
            method: "GET".into(),
            path: "/orders/7".into(),
            endpoint: Some("GET /orders/{id}".into()),
            status: 200,
            ms: 1,
            trace_ms: 1.0,
            scope: None,
            pin: None,
            flags: vec![],
            chain_error: None,
            failed_node: None,
            kind: None,
            rejected: None,
            edited: false,
            trace: String::new(),
            hops: vec![],
            truncated: false,
        }
    }

    fn params(p: &[(&str, &str)]) -> Vec<(String, String)> {
        p.iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn filter(p: &[(&str, &str)]) -> Filter {
        Filter::from_params(&params(p), 10_000).unwrap()
    }

    #[test]
    fn id_grammar_round_trips() {
        assert_eq!(format_id(0x4f2, 42), "r04f2-0042");
        assert_eq!(parse_id("r04f2-0042"), Some((0x4f2, 42)));
        assert_eq!(parse_id("r04f2-12345"), Some((0x4f2, 12345)));
        assert_eq!(format_id(0x4f2, 12345), "r04f2-12345");
        assert_eq!(parse_id("r04f2-42"), None);
        assert_eq!(parse_id("r04F2-0042"), None);
        assert_eq!(parse_id("r4f2-0042"), None);
        assert_eq!(parse_id("x04f2-0042"), None);
        assert_eq!(parse_id("r04f2-004x"), None);
    }

    #[test]
    fn ids_increase_and_start_at_one() {
        let g = IdGen::with_boot(0xabcd);
        assert_eq!(g.boot(), 0xabcd);
        assert_eq!(g.next(), (1, "rabcd-0001".to_string()));
        assert_eq!(g.next().0, 2);
    }

    #[test]
    fn client_id_grammar() {
        assert_eq!(client_id("abc-1.2_x").as_deref(), Some("abc-1.2_x"));
        assert!(client_id("a b").is_none());
        assert!(client_id("a\nb").is_none());
        assert!(client_id(&"a".repeat(65)).is_none());
        assert!(client_id(&"a".repeat(64)).is_some());
        assert!(client_id("").is_none());
    }

    #[test]
    fn ring_keeps_the_last_1000_by_completion_and_counts_evictions() {
        let mut r = LogRing::new(BOOT);
        for s in 1..=1005 {
            r.push(entry(s));
        }
        let p = r.query(&Filter::default(), 5000);
        assert_eq!(p.entries.len(), LOG_CAP);
        assert_eq!(p.evicted, 5);
        assert_eq!(p.cap, LOG_CAP);
        assert_eq!(p.oldest.as_deref(), Some("r04f2-0006"));
        assert_eq!(p.newest.as_deref(), Some("r04f2-1005"));
        assert_eq!(r.find("r04f2-0005"), Lookup::Evicted);
    }

    #[test]
    fn out_of_order_pushes_keep_seq_and_done_separate() {
        let mut r = LogRing::new(BOOT);
        assert_eq!(r.push(entry(5)), 1);
        assert_eq!(r.push(entry(3)), 2);
        assert_eq!(r.push(entry(4)), 3);
        let p = r.query(&Filter::default(), 10);
        let pairs: Vec<_> = p.entries.iter().map(|e| (e.seq, e.done)).collect();
        assert_eq!(pairs, [(5, 1), (3, 2), (4, 3)]);
        assert_eq!(p.oldest.as_deref(), Some("r04f2-0003"));
        assert_eq!(p.newest.as_deref(), Some("r04f2-0005"));
        let after = r.query(&filter(&[("after", "1")]), 10);
        assert_eq!(
            after.entries.iter().map(|e| e.seq).collect::<Vec<_>>(),
            [3, 4]
        );
    }

    #[test]
    fn find_evicted_vs_unknown_with_out_of_order_completion() {
        let mut r = LogRing::new(BOOT);
        // seq 1 is slow and has not completed; 2..=1002 fill and overflow the ring.
        for s in 2..=1002 {
            r.push(entry(s));
        }
        // 2 was evicted and 1 never arrived: both are below the high-water mark, not unknown.
        assert_eq!(r.find("r04f2-0002"), Lookup::Evicted);
        assert_eq!(r.find("r04f2-0001"), Lookup::Evicted);
        assert!(matches!(r.find("r04f2-0003"), Lookup::Found(_)));
        assert_eq!(r.find("r0001-0003"), Lookup::Unknown);
        assert_eq!(r.find("r04f2-1003"), Lookup::Unknown);
        assert_eq!(r.find("not valid!"), Lookup::Malformed);
    }

    #[test]
    fn late_low_seq_after_eviction_is_evicted_not_unknown() {
        let mut r = LogRing::new(BOOT);
        for s in 2..=1001 {
            r.push(entry(s));
        }
        r.push(entry(1)); // slow request lands late and evicts seq 2 (earliest done)
        assert_eq!(r.find("r04f2-0002"), Lookup::Evicted);
        assert!(matches!(r.find("r04f2-0001"), Lookup::Found(_)));
    }

    #[test]
    fn eviction_drops_the_earliest_done_not_the_lowest_seq() {
        let mut r = LogRing::new(BOOT);
        for s in 2..=1000 {
            r.push(entry(s));
        }
        r.push(entry(1)); // done 1000, lowest seq
        for s in 1001..=1003 {
            r.push(entry(s));
        }
        assert!(matches!(r.find("r04f2-0001"), Lookup::Found(_)));
        for s in 2..=4 {
            assert_eq!(r.find(&format_id(BOOT, s)), Lookup::Evicted);
        }
        assert!(matches!(r.find("r04f2-0005"), Lookup::Found(_)));
    }

    #[test]
    fn find_precedence_and_leading_zero_alias() {
        let mut r = LogRing::new(BOOT);
        let mut a = entry(1);
        a.client_id = Some("r04f2-0002".into()); // shaped like an id
        r.push(a);
        r.push(entry(2));
        let Lookup::Found(e) = r.find("r04f2-0002") else {
            panic!()
        };
        assert_eq!(e.seq, 2); // resolved as an id, not as seq 1's client id
        let Lookup::Found(e) = r.find("r04f2-00002") else {
            panic!()
        };
        assert_eq!(e.seq, 2); // leading zeros alias the same entry
    }

    #[test]
    fn find_last_and_client_id_newest_wins() {
        let mut r = LogRing::new(BOOT);
        assert_eq!(r.find("last"), Lookup::Unknown);
        for s in [1, 2, 3] {
            let mut e = entry(s);
            e.client_id = (s != 3).then(|| "req-x".to_string());
            r.push(e);
        }
        let Lookup::Found(e) = r.find("last") else {
            panic!()
        };
        assert_eq!(e.seq, 3);
        let Lookup::Found(e) = r.find("req-x") else {
            panic!()
        };
        assert_eq!(e.seq, 2);
        assert_eq!(r.find("nope"), Lookup::Unknown);
    }

    #[test]
    fn clear_keeps_counters() {
        let mut r = LogRing::new(BOOT);
        r.push(entry(1));
        r.push(entry(2));
        r.clear();
        assert_eq!(r.find("r04f2-0001"), Lookup::Evicted);
        assert_eq!(r.query(&Filter::default(), 10).entries, []);
        assert_eq!(r.push(entry(3)), 3);
    }

    #[test]
    fn filter_status_exact_class_list() {
        let mk = |s| LogEntry {
            status: s,
            ..entry(1)
        };
        let f = filter(&[("status", "404")]);
        assert!(matches(&mk(404), &f) && !matches(&mk(500), &f));
        let f = filter(&[("status", "5xx")]);
        assert!(matches(&mk(503), &f) && !matches(&mk(404), &f));
        let f = filter(&[("status", "404,5xx")]);
        assert!(matches(&mk(404), &f) && matches(&mk(500), &f) && !matches(&mk(200), &f));
    }

    #[test]
    fn filter_failed_scope_since_after() {
        let mut e = entry(1);
        e.done = 7;
        e.ts_ms = 9_000;
        assert!(matches(&e, &filter(&[("failed", "false")])));
        assert!(!matches(&e, &filter(&[("failed", "true")])));
        e.failed_node = Some("db".into());
        assert!(matches(&e, &filter(&[("failed", "true")])));
        e.scope = Some("a,b".into());
        assert!(matches(&e, &filter(&[("scope", "a,b")])));
        assert!(!matches(&e, &filter(&[("scope", "a")])));
        // now = 10_000: "5s" is 5_000 <= 9_000 passes, "500ms" is 9_500 > 9_000 fails.
        assert!(matches(&e, &filter(&[("since", "5s")])));
        assert!(!matches(&e, &filter(&[("since", "500ms")])));
        assert_eq!(
            filter(&[("since", "1970-01-01T00:00:09Z")]).since_ms,
            Some(9_000)
        );
        assert!(matches(&e, &filter(&[("after", "6")])));
        assert!(!matches(&e, &filter(&[("after", "7")])));
        let ep = filter(&[("endpoint", "get /orders/{id}")]);
        assert_eq!(ep.endpoint.as_deref(), Some("GET /orders/{id}"));
        assert!(matches(&e, &ep));
    }

    #[test]
    fn filter_empty_scope_is_distinct_from_no_scope() {
        let empty = Filter {
            scope: Some(String::new()),
            ..Filter::default()
        };
        let mut e = entry(1);
        assert!(!matches(&e, &empty));
        e.scope = Some(String::new());
        assert!(matches(&e, &empty));
        assert!(matches(&e, &Filter::default()));
    }

    #[test]
    fn filter_rejects_unknown_empty_and_malformed_params_naming_the_param() {
        for (k, v) in [
            ("bogus", "1"),
            ("status", ""),
            ("status", "9xx"),
            ("status", "99"),
            ("failed", "yes"),
            ("since", "yesterday"),
            ("after", "-1"),
            ("endpoint", "orders"),
            ("endpoint", ""),
            ("after", "+5"),
            ("status", " 404"),
            ("status", "+404"),
        ] {
            let err = Filter::from_params(&params(&[(k, v)]), 0).unwrap_err();
            assert!(err.contains(k), "{k}={v}: {err}");
        }
    }

    #[test]
    fn limit_is_ignored_and_duplicates_rejected() {
        assert_eq!(filter(&[("limit", "5")]), Filter::default());
        for k in ["limit", "scope", "after"] {
            let err = Filter::from_params(&params(&[(k, "1"), (k, "2")]), 0).unwrap_err();
            assert!(err.contains(k), "{err}");
        }
    }

    #[test]
    fn empty_scope_param_filters_the_empty_header_scope() {
        let f = filter(&[("scope", "")]);
        assert_eq!(f.scope.as_deref(), Some(""));
        let mut r = LogRing::new(BOOT);
        for (s, scope) in [(1, None), (2, Some("")), (3, Some("a"))] {
            r.push(LogEntry {
                scope: scope.map(String::from),
                ..entry(s)
            });
        }
        let p = r.query(&f, 10);
        assert_eq!(p.entries.iter().map(|e| e.seq).collect::<Vec<_>>(), [2]);
    }

    #[test]
    fn query_returns_last_limit_oldest_first() {
        let mut r = LogRing::new(BOOT);
        for s in 1..=5 {
            r.push(entry(s));
        }
        let p = r.query(&Filter::default(), 2);
        assert_eq!(p.entries.iter().map(|e| e.seq).collect::<Vec<_>>(), [4, 5]);
    }
}

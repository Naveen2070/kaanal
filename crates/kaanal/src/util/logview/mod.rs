//! Pure rendering and filtering for `kaanal logs` and `kaanal trace`: log lines, the ASCII
//! waterfall, the follow de-dup set, the client-side filter and the SSE line parser. No network,
//! so all of it is unit tested with golden strings.

use std::collections::{HashSet, VecDeque};
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::time::Duration;

use kaanal_server::{Filter, StatusMatch};
use serde_json::Value;

use crate::clean;

/// **Private.** Columns of the waterfall bar.
const BAR: usize = 40;
/// How many of the most recent printed ids `logs -f` remembers (older ones may print again).
pub const SEEN_CAP: usize = 2000;

/// **Private.** `HH:MM:SS.mmmZ` (UTC) of a millisecond Unix timestamp.
fn clock(ts_ms: u64) -> String {
    let day = ts_ms % 86_400_000;
    format!(
        "{:02}:{:02}:{:02}.{:03}Z",
        day / 3_600_000,
        day / 60_000 % 60,
        day / 1000 % 60,
        day % 1000
    )
}

/// **Private.** A string field, cleaned for the terminal; empty when absent.
fn s(v: &Value) -> String {
    clean(v.as_str().unwrap_or(""))
}

/// **Private.** One log line for an entry or an SSE `request` event (they share these fields).
pub fn line(v: &Value) -> String {
    let mut out = format!(
        "{}  {}  {} {}  {}  {}ms",
        clock(v["ts_ms"].as_u64().unwrap_or(0)),
        s(&v["id"]),
        s(&v["method"]),
        s(&v["path"]),
        v["status"].as_u64().unwrap_or(0),
        v["ms"].as_u64().unwrap_or(0)
    );
    for (key, name) in [
        ("scope", "scope"),
        ("pin", "pinned"),
        ("failed_node", "failed"),
        ("kind", "kind"),
        ("client_id", "client"),
    ] {
        if v[key].is_string() {
            let _ = write!(out, "  {name}={}", s(&v[key]));
        }
    }
    if v["rejected"].is_string() {
        let _ = write!(out, "  {}", s(&v["rejected"]));
    }
    out
}

/// **Private.** Bar column of a time offset, clamped to the bar.
fn col(t: f64, total: f64) -> usize {
    if total <= 0.0 || t <= 0.0 {
        return 0;
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "a column index: the value is clamped to 0..=BAR, so the casts are exact"
    )]
    let c = (t / total * BAR as f64).floor() as usize;
    c.min(BAR)
}

/// **Private.** The 40-column bar of one hop: `.` queue wait, then `#` (or `x`) of at least one column.
fn bar(start: f64, dur: f64, wait: f64, total: f64, cut: bool) -> String {
    let first = col(start, total).min(BAR - 1);
    let last = col(start + dur, total).clamp(first + 1, BAR);
    let queued = col(start - wait, total).min(first);
    let mut b = String::with_capacity(BAR);
    for i in 0..BAR {
        b.push(match i {
            _ if i < queued => ' ',
            _ if i < first => '.',
            _ if i < last => {
                if cut {
                    'x'
                } else {
                    '#'
                }
            }
            _ => ' ',
        });
    }
    b
}

/// **Private.** The hop the chain failed at: of the failing node's hops (it can run in several
/// parallel lanes) the last one that was cut, else the one that ends latest (the one that ends
/// the request), the later listed on a tie.
fn failed_hop(hops: &[Value], node: &str) -> Option<usize> {
    let end = |i: usize| {
        hops[i]["start_ms"].as_f64().unwrap_or(0.0) + hops[i]["dur_ms"].as_f64().unwrap_or(0.0)
    };
    let same: Vec<usize> = (0..hops.len())
        .filter(|&i| hops[i]["node"].as_str() == Some(node))
        .collect();
    same.iter()
        .copied()
        .rfind(|&i| hops[i]["cut"] == true)
        .or_else(|| {
            same.iter()
                .copied()
                .reduce(|a, i| if end(i) >= end(a) { i } else { a })
        })
}

/// **Private.** The trace view of one entry: header, one row per hop, failure footer.
pub fn waterfall(e: &Value) -> String {
    let total = e["trace_ms"].as_f64().unwrap_or(0.0);
    let mut head = format!(
        "{}  {} {}  {}  served {}ms  trace {:.1}ms",
        s(&e["id"]),
        s(&e["method"]),
        s(&e["path"]),
        e["status"].as_u64().unwrap_or(0),
        e["ms"].as_u64().unwrap_or(0),
        total
    );
    for (key, name) in [
        ("scope", "scope"),
        ("pin", "pinned"),
        ("client_id", "client"),
    ] {
        if e[key].is_string() {
            let _ = write!(head, "  {name}={}", s(&e[key]));
        }
    }
    let flags: Vec<String> = e["flags"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(clean)
        .collect();
    if !flags.is_empty() {
        let _ = write!(head, "  flags={}", flags.join(","));
    }
    let mut out = head;
    let hops = e["hops"].as_array().map_or(&[][..], Vec::as_slice);
    if hops.is_empty() {
        let why = e["rejected"].as_str().map_or("no hops recorded", |r| r);
        let _ = write!(out, "\n  - ({})", clean(why));
    }
    let failed = e["chain_error"]["node"]
        .as_str()
        .and_then(|n| failed_hop(hops, n));
    let label = |h: &Value| {
        let what = h["op"].as_str().or(h["kind"].as_str()).unwrap_or("");
        clean(&format!("{}.{what}", h["node"].as_str().unwrap_or("")))
    };
    let width = hops
        .iter()
        .map(|h| label(h).chars().count())
        .max()
        .unwrap_or(0);
    for (i, h) in hops.iter().enumerate() {
        let (start, dur) = (
            h["start_ms"].as_f64().unwrap_or(0.0),
            h["dur_ms"].as_f64().unwrap_or(0.0),
        );
        let cut = h["cut"] == true || failed == Some(i);
        let mut note = h["why"].as_str().map(clean).unwrap_or_default();
        if h["lane"].as_u64().unwrap_or(0) > 0 {
            if !note.is_empty() {
                note.push_str(", ");
            }
            let _ = write!(note, "lane {}", h["lane"]);
        }
        let _ = write!(
            out,
            "\n  {:<width$}  {:>8.1}ms  {:>8.1}ms  |{}|  {note}",
            label(h),
            start,
            dur,
            bar(start, dur, h["wait_ms"].as_f64().unwrap_or(0.0), total, cut),
        );
        // trailing spaces from an empty note are noise
        out.truncate(out.trim_end().len());
    }
    if e["truncated"] == true {
        let _ = write!(out, "\n  ... more hops (truncated, {} shown)", hops.len());
    }
    if let Some(ce) = e["chain_error"].as_object() {
        let node = clean(ce["node"].as_str().unwrap_or(""));
        let status = ce["status"].as_u64().unwrap_or(0);
        let kind = ce["kind"].as_str().filter(|k| *k != "status");
        let cause = clean(ce["cause"].as_str().unwrap_or(""));
        match e["pin"].as_str() {
            // a pin replaced the failure the chain ended with
            Some(pin) if e["failed_node"].is_null() => {
                let _ = write!(out, "\n  pinned={} hid: {node}.error={status}", clean(pin));
            }
            _ => {
                let kind = kind.map_or(String::new(), |k| format!(" {}", clean(k)));
                let _ = write!(out, "\n  failed at {node}: {status}{kind} ({cause})");
            }
        }
    }
    out
}

/// Events the follow reader may queue for the printer before it starts counting drops.
pub const BACKLOG: usize = 4096;

/// **Private.** One SSE line from raw bytes: invalid UTF-8 becomes U+FFFD, the line break goes.
pub fn lossy_line(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw)
        .trim_end_matches(['\r', '\n'])
        .to_string()
}

/// **Private.** Queues a message for the printer without ever blocking the socket. When the
/// queue is full the message is counted into `dropped` instead (a request counts one, a server
/// `lagged` message counts what it reports); the printer reads the count with [`take_lagged`].
/// False once the printer is gone.
pub fn forward(
    tx: &SyncSender<Option<(String, String)>>,
    dropped: &AtomicU64,
    msg: (String, String),
) -> bool {
    match tx.try_send(Some(msg)) {
        Ok(()) => true,
        Err(TrySendError::Full(lost)) => {
            if let Some((event, data)) = lost {
                let n = match event.as_str() {
                    "request" => 1,
                    "lagged" => data.trim().parse().unwrap_or(1),
                    _ => 0,
                };
                dropped.fetch_add(n, Ordering::Relaxed);
            }
            true
        }
        Err(TrySendError::Disconnected(_)) => false,
    }
}

/// **Private.** The missed-request count so far, as text for [`lagged_notice`], and reset to zero.
pub fn take_lagged(dropped: &AtomicU64) -> Option<String> {
    match dropped.swap(0, Ordering::Relaxed) {
        0 => None,
        n => Some(n.to_string()),
    }
}

/// **Private.** How [`drain`] ended: the reader's end marker, or the queue was cut.
#[derive(Debug, PartialEq, Eq)]
pub enum End {
    Marker,
    Disconnected,
}

/// **Private.** The follow printer loop. Prints the missed-request notice (`out`) whenever the
/// reader has counted drops: before each wait, after an idle wait of `idle` (so a quiet
/// follower still reports), and when the stream ends, whichever way. `lagged` messages print
/// their own notice; every other message goes to `on_event`. Returns only when the stream ends.
pub fn drain(
    rx: &Receiver<Option<(String, String)>>,
    dropped: &AtomicU64,
    idle: Duration,
    out: &mut dyn FnMut(&str),
    on_event: &mut dyn FnMut(&str, &str),
) -> End {
    let flush = |out: &mut dyn FnMut(&str)| {
        if let Some(n) = take_lagged(dropped) {
            out(&lagged_notice(&n));
        }
    };
    loop {
        flush(out);
        match rx.recv_timeout(idle) {
            Ok(Some((event, data))) if event == "lagged" => out(&lagged_notice(&data)),
            Ok(Some((event, data))) => on_event(&event, &data),
            Err(RecvTimeoutError::Timeout) => {}
            Ok(None) => {
                flush(out);
                return End::Marker;
            }
            Err(RecvTimeoutError::Disconnected) => {
                flush(out);
                return End::Disconnected;
            }
        }
    }
}

/// **Private.** The `-f` notice for an SSE `lagged` event.
pub fn lagged_notice(count: &str) -> String {
    format!(
        "-- missed {} requests (follow slower than the server) --",
        clean(count.trim())
    )
}

/// **Private.** Ids already printed, bounded to the last [`SEEN_CAP`].
pub struct Seen {
    set: HashSet<String>,
    order: VecDeque<String>,
}

impl Seen {
    pub fn new() -> Self {
        Self {
            set: HashSet::new(),
            order: VecDeque::new(),
        }
    }

    /// Remembers `id`; false when it was already there.
    pub fn insert(&mut self, id: &str) -> bool {
        if !self.set.insert(id.to_string()) {
            return false;
        }
        self.order.push_back(id.to_string());
        if self.order.len() > SEEN_CAP
            && let Some(old) = self.order.pop_front()
        {
            self.set.remove(&old);
        }
        true
    }
}

/// **Private.** The server's filter grammar, built from the same query pairs the server gets, so
/// a bad value is refused the same way and live events are judged by the same rules.
pub fn filter_of(params: &[(String, String)], now_ms: u64) -> Result<Filter, String> {
    Filter::from_params(params, now_ms)
}

/// **Private.** Whether an entry or event passes the filter. An event carries `endpoint` too
/// (the route template), so live lines are filtered without fetching the entry.
pub fn passes(f: &Filter, v: &Value) -> bool {
    let status = v["status"].as_u64().unwrap_or(0);
    f.endpoint
        .as_deref()
        .is_none_or(|ep| v["endpoint"].as_str() == Some(ep))
        && (f.status.is_empty()
            || f.status.iter().any(|m| match *m {
                StatusMatch::Exact(n) => status == u64::from(n),
                StatusMatch::Class(c) => status / 100 == u64::from(c),
            }))
        && (!f.failed || v["failed_node"].is_string())
        && f.scope
            .as_deref()
            .is_none_or(|sc| v["scope"].as_str() == Some(sc))
        && f.since_ms
            .is_none_or(|t| v["ts_ms"].as_u64().unwrap_or(0) >= t)
}

/// Largest SSE line or message `logs -f` accepts; a bigger one is dropped and counted as missed.
pub const MAX_MSG: usize = 1024 * 1024;

/// **Private.** What [`read_capped`] found.
#[derive(Debug, PartialEq, Eq)]
pub enum Line {
    /// End of the stream.
    Eof,
    /// A line is in the buffer (with its line break, if it had one).
    Text,
    /// The line was longer than the cap; it was skipped to its end and the buffer is empty.
    TooLong,
}

/// **Private.** Reads one line of at most `cap` bytes into `buf`, never buffering more.
///
/// # Errors
/// The reader's own error.
pub fn read_capped(
    r: &mut impl std::io::BufRead,
    buf: &mut Vec<u8>,
    cap: usize,
) -> std::io::Result<Line> {
    use std::io::{BufRead as _, Read as _};
    buf.clear();
    let n = r.by_ref().take(cap as u64).read_until(b'\n', buf)?;
    if n == 0 {
        return Ok(Line::Eof);
    }
    if buf.last() == Some(&b'\n') || n < cap {
        return Ok(Line::Text);
    }
    buf.clear();
    loop {
        let chunk = r.fill_buf()?;
        if chunk.is_empty() {
            break;
        }
        if let Some(i) = chunk.iter().position(|&b| b == b'\n') {
            r.consume(i + 1);
            break;
        }
        let all = chunk.len();
        r.consume(all);
    }
    Ok(Line::TooLong)
}

/// **Private.** Assembles SSE lines into `(event, data)` messages.
#[derive(Default)]
pub struct Sse {
    event: String,
    data: String,
    too_big: bool,
}

impl Sse {
    /// Marks the message being read as too big: it will come out as one missed request.
    pub fn overflow(&mut self) {
        self.too_big = true;
        self.data.clear();
    }

    /// Feeds one line (no line break); a blank line ends a message and returns it. A message
    /// that outgrew [`MAX_MSG`] comes out as `("lagged", "1")` instead of its content.
    pub fn push(&mut self, line: &str) -> Option<(String, String)> {
        if line.is_empty() {
            let (ev, data) = (
                std::mem::take(&mut self.event),
                std::mem::take(&mut self.data),
            );
            if std::mem::take(&mut self.too_big) {
                return Some(("lagged".into(), "1".into()));
            }
            return (!ev.is_empty() || !data.is_empty()).then_some((ev, data));
        }
        if let Some(v) = line.strip_prefix("event:") {
            self.event = v.trim_start().to_string();
        } else if let Some(v) = line.strip_prefix("data:") {
            if self.data.len() + v.len() + 1 > MAX_MSG {
                self.overflow();
            }
            if !self.too_big {
                if !self.data.is_empty() {
                    self.data.push('\n');
                }
                self.data.push_str(v.strip_prefix(' ').unwrap_or(v));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests;

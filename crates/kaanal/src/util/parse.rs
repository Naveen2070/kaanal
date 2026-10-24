//! Small argument parsers and text cleaners shared by the commands (durations, rates, path segments, control-text cleaning).

use crate::{Duration, Fail, SEGMENT, Value, json, output, utf8_percent_encode};

/// **Private.** Server text may echo user input (e.g. a node name); never let it drive the terminal.
pub(crate) fn clean(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() || spoofs(c) { '?' } else { c })
        .collect()
}

/// **Private.** Invisible characters that reorder or hide text without being control characters:
/// bidi marks and overrides, line/paragraph separators, word joiner and invisible operators,
/// and the byte order mark. (Zero-width joiners stay: emoji and some scripts need them.)
pub(crate) fn spoofs(c: char) -> bool {
    matches!(
        c,
        '\u{61c}'
            | '\u{200e}'..='\u{200f}'
            | '\u{2028}'..='\u{2029}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{2069}'
            | '\u{feff}'
    )
}

/// **Private.** [`clean`] per line, keeping our own line breaks.
pub(crate) fn clean_lines(s: &str) -> String {
    s.lines().map(clean).collect::<Vec<_>>().join("\n")
}

/// **Private.** Percent-encodes one URL path segment.
pub(crate) fn encode_segment(s: &str) -> String {
    utf8_percent_encode(s, SEGMENT).to_string()
}

/// **Private.** The activate body for `--ttl` or `--for`, validated here so a typo sends nothing;
/// `what` names the flag in the error.
pub(crate) fn ttl_body(what: &str, ttl: Option<String>) -> Result<Option<Value>, Fail> {
    ttl.map(|t| {
        let t = t.trim();
        humantime::parse_duration(t)
            .ok()
            .filter(|d| (Duration::from_millis(100)..=Duration::from_hours(24)).contains(d))
            .ok_or_else(|| Fail::usage(format!("bad {what} '{t}' (100ms to 24h, e.g. 30s)")))?;
        Ok(json!({ "ttl": t }))
    })
    .transpose()
}

/// **Private.** ` (active)` or ` (active, expires in X)` for one list row, else nothing.
pub(crate) fn active_suffix(s: &Value) -> String {
    match (s["active"] == true, s["expires_in_ms"].as_u64()) {
        (true, Some(ms)) => format!(" (active, expires in {})", output::remaining(ms)),
        (true, None) => " (active)".to_string(),
        _ => String::new(),
    }
}

/// **Private.** `300ms` / `2s` / `1m 30s` → milliseconds. A unit is required.
pub(crate) fn parse_ms(s: &str) -> Result<u64, String> {
    humantime::parse_duration(s.trim())
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .ok_or_else(|| format!("bad latency '{s}' (use e.g. 300ms or 2s)"))
}

/// **Private.** `10%` or `0.1` → a 0.0–1.0 rate.
pub(crate) fn parse_rate(s: &str) -> Result<f64, String> {
    let t = s.trim();
    let v = match t.strip_suffix('%') {
        Some(p) => p.trim().parse::<f64>().map(|p| p / 100.0),
        None => t.parse::<f64>(),
    }
    .map_err(|_| format!("bad error rate '{s}' (use e.g. 10% or 0.1)"))?;
    if (0.0..=1.0).contains(&v) {
        Ok(v)
    } else {
        Err(format!("error rate '{s}' must be between 0% and 100%"))
    }
}

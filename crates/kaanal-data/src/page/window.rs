//! Window resolution: query parameters and a seed give the (total, offset, size) of a page.

use super::{PageSpec, Style};
use crate::generate::child_seed;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde_json::{Value, json};

/// Hard cap on the total item count of any paged collection.
const HARD_CAP: u32 = 10_000;

/// The slice of a collection one request addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    /// Number of items in the whole collection.
    pub total: u32,
    /// Zero-based index of the first item of the window.
    pub offset: u32,
    /// Page size, already clamped to the size maximum and to [`PageSpec::record_cap`]; the
    /// window holds `min(size, total - offset)` items.
    pub size: u32,
}

/// A rejected paging parameter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageError {
    /// A paging query parameter is malformed or out of range.
    Invalid {
        /// The parameter, as the spec spells it.
        param: String,
        /// What is wrong with it.
        detail: String,
    },
}

impl PageError {
    /// The `400` response body.
    #[must_use]
    pub fn body(&self) -> Value {
        let Self::Invalid { param, detail } = self;
        json!({"error": "invalid query parameter", "param": param, "detail": detail})
    }
}

fn invalid(param: &str, detail: impl Into<String>) -> PageError {
    PageError::Invalid {
        param: param.into(),
        detail: detail.into(),
    }
}

/// Only ASCII digits: no sign, no whitespace (`u32::from_str` alone accepts a leading `+`).
/// Digits past `u32::MAX` say so, rather than "not a non-negative integer".
fn parse_u32(param: &str, raw: &str) -> Result<u32, PageError> {
    if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid(
            param,
            format!("`{raw}` is not a non-negative integer"),
        ));
    }
    raw.parse().map_err(|_| {
        invalid(
            param,
            format!("`{raw}` exceeds the maximum supported value ({})", u32::MAX),
        )
    })
}

/// `base64url("k1:" + offset)`, no padding.
#[must_use]
pub fn encode_cursor(offset: u32) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for c in format!("k1:{offset}").as_bytes().chunks(3) {
        let n = c.iter().fold(0u32, |n, b| n << 8 | u32::from(*b)) << (8 * (3 - c.len()));
        for i in 0..=c.len() {
            out.push(char::from(A[(n >> (18 - 6 * i) & 63) as usize]));
        }
    }
    out
}

/// Inverse of [`encode_cursor`]; `None` for anything it did not produce. Canonical: a token
/// decodes only when re-encoding its offset gives the same string (no stray trailing bits, no
/// leading zeros, no sign).
#[must_use]
pub fn decode_cursor(s: &str) -> Option<u32> {
    let mut bytes = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for ch in s.bytes() {
        let v = match ch {
            b'A'..=b'Z' => ch - b'A',
            b'a'..=b'z' => ch - b'a' + 26,
            b'0'..=b'9' => ch - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        };
        acc = acc << 6 | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            bytes.push((acc >> bits & 0xff) as u8);
        }
    }
    let digits = std::str::from_utf8(&bytes).ok()?.strip_prefix("k1:")?;
    digits.parse().ok().filter(|v| encode_cursor(*v) == s)
}

impl PageSpec {
    /// Resolve the window a request addresses, or the `400` for a bad parameter.
    ///
    /// Size bounds are normalised first: `min >= 1`, `max >= min`, default within both.
    ///
    /// # Errors
    ///
    /// [`PageError::Invalid`] for a non-integer, out-of-range or forged value.
    pub fn window(
        &self,
        query: &[(String, String)],
        seed: u64,
        total_override: Option<u32>,
    ) -> Result<Window, PageError> {
        let get = |name: &str| query.iter().find(|(k, _)| k == name).map(|(_, v)| v);
        let min = self.size_min.max(1);
        let max = self.size_max.max(min);
        let mut size = self.size_default.clamp(min, max);
        if let Some(name) = &self.size_param
            && let Some(raw) = get(name)
        {
            let n = parse_u32(name, raw)?;
            if n < min {
                return Err(invalid(name, format!("must be at least {min}")));
            }
            size = n.min(max);
        }
        // Heavy records: shrink the page so its body stays within the value cap. Done before the
        // page number becomes an offset, so every page of a route has the same size.
        size = size.min(self.record_cap.max(1));
        let name = &self.index_param;
        let offset = match (get(name), self.style) {
            (None, _) => 0,
            (Some(raw), Style::Offset) => parse_u32(name, raw)?,
            (Some(raw), Style::Page) => {
                let page = parse_u32(name, raw)?;
                let base = self.page_base.min(1);
                if page < base {
                    return Err(invalid(name, format!("must be at least {base}")));
                }
                // ponytail: a product past u32 saturates; it is past any total anyway.
                (page - base).saturating_mul(size)
            }
            (Some(raw), Style::Cursor) => decode_cursor(raw)
                .ok_or_else(|| invalid(name, "not a cursor issued by this server"))?,
        };
        let total = total_override.map_or_else(
            || {
                let lo = self.total_min.min(self.total_max).min(HARD_CAP);
                let hi = self.total_max.min(HARD_CAP);
                ChaCha8Rng::seed_from_u64(child_seed(seed, "total", 0)).random_range(lo..=hi)
            },
            |t| t.min(HARD_CAP),
        );
        Ok(Window {
            total,
            offset,
            size,
        })
    }
}

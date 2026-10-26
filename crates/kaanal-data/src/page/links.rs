//! Response headers of a paged response: `x-total-count` and the RFC 8288 `link` header.

use super::{PageSpec, Style, Window, encode_cursor};
use std::fmt::Write;

/// RFC 3986 unreserved characters stay; every other byte becomes `%XX`.
fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

impl PageSpec {
    /// The relative URL of the page that starts at `offset`: `path` plus `query` with the index
    /// parameter replaced by a page number (from [`PageSpec::page_base`]), an offset or a cursor.
    /// The `link` header carries it, and so does a string `next`/`prev` field of the page and
    /// offset styles.
    pub(crate) fn page_url(
        &self,
        path: &str,
        query: &[(String, String)],
        offset: u32,
        size: u32,
    ) -> String {
        let idx = match self.style {
            Style::Page => (offset / size.max(1))
                .saturating_add(self.page_base)
                .to_string(),
            Style::Offset => offset.to_string(),
            Style::Cursor => encode_cursor(offset),
        };
        let mut q: Vec<String> = query
            .iter()
            .filter(|(k, _)| *k != self.index_param)
            .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
            .collect();
        q.push(format!("{}={idx}", encode(&self.index_param)));
        format!("{path}?{}", q.join("&"))
    }
}

impl Window {
    /// `x-total-count` always; `link` when it has at least one relation.
    ///
    /// Links are relative: `path` plus `query` with the index parameter replaced.
    #[must_use]
    pub fn headers(
        &self,
        spec: &PageSpec,
        path: &str,
        query: &[(String, String)],
    ) -> Vec<(&'static str, String)> {
        let mut out = vec![("x-total-count", self.total.to_string())];
        let size = self.size.max(1);
        let pages = self.total.div_ceil(size);
        let last_offset = pages.saturating_sub(1).saturating_mul(size);
        let next = self.offset.saturating_add(size);
        let has_next = next < self.total;
        let has_prev = self.offset > 0 && self.total > 0;
        let prev = self.offset.saturating_sub(size).min(last_offset);
        // (rel, offset) in header order; the value of a relation depends on the style.
        let mut rels: Vec<(&str, u32)> = Vec::new();
        if spec.style == Style::Cursor {
            if has_next {
                rels.push(("next", next));
            }
        } else {
            rels.push(("first", 0));
            if has_prev {
                rels.push(("prev", prev));
            }
            if has_next {
                rels.push(("next", next));
            }
            if self.total > 0 {
                rels.push(("last", last_offset));
            }
        }
        let links: Vec<String> = rels
            .into_iter()
            .map(|(rel, off)| format!("<{}>; rel=\"{rel}\"", spec.page_url(path, query, off, size)))
            .collect();
        if !links.is_empty() {
            out.push(("link", links.join(", ")));
        }
        out
    }
}

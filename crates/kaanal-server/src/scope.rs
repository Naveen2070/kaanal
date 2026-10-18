//! Per-request scenario scoping: the `X-Kaanal-Scenario` header.
//!
//! A request that names scenarios is answered with exactly that stack (in header order, no live
//! layer, no global activation). [`parse`] reads the header; [`AppState::scoped`] compiles each
//! distinct stack once and caches it until the next publish.

use std::sync::{Arc, PoisonError};

use axum::http::HeaderMap;
use kaanal_core::{ChaosStack, EffectiveChaos, EffectivePins, valid_name};

use crate::state::AppState;

/// The request header that scopes one request to a scenario stack.
pub const HEADER: &str = "x-kaanal-scenario";
/// The response header that echoes the (deduplicated) stack that answered. An empty request
/// header echoes an empty value.
pub const SCOPE_ECHO: &str = "x-kaanal-scope";
/// Most names one header may carry.
pub const MAX_NAMES: usize = 16;
/// Longest header value in bytes.
pub const MAX_HEADER_BYTES: usize = 256;
/// Most compiled stacks the cache keeps; the map is cleared when it is full.
pub const MAX_CACHED: usize = 256;

/// One compiled scoped stack. Immutable once cached.
pub struct Scoped {
    /// What the executor reads for a request carrying this scope.
    pub chaos: EffectiveChaos,
    /// The pins of the same stack (never the live pins).
    pub pins: EffectivePins,
}

/// The cache key (and echo value) of a stack: the names joined by `,`.
#[must_use]
pub fn key(names: &[String]) -> String {
    names.join(",")
}

/// Reads the scope header. `None` when absent (nothing is allocated); otherwise the names,
/// whitespace trimmed and deduplicated keeping the **last** position (`a,b,a` is `[b, a]`).
/// An empty value is `Some(vec![])`: an explicitly empty stack.
///
/// # Errors
/// The detail text of a bad scope: more than one header line, a value that is not ASCII/UTF-8,
/// over [`MAX_HEADER_BYTES`] bytes, over [`MAX_NAMES`] names, an empty segment or an invalid name.
pub fn parse(headers: &HeaderMap) -> Result<Option<Vec<String>>, String> {
    let mut lines = headers.get_all(HEADER).iter();
    let Some(first) = lines.next() else {
        return Ok(None);
    };
    if lines.next().is_some() {
        return Err(format!("send {HEADER} once, with comma-separated names"));
    }
    let value = first
        .to_str()
        .map_err(|_| format!("{HEADER} must be plain ASCII"))?;
    if value.len() > MAX_HEADER_BYTES {
        return Err(format!("{HEADER} is over {MAX_HEADER_BYTES} bytes"));
    }
    if value.trim().is_empty() {
        return Ok(Some(Vec::new()));
    }
    let mut names: Vec<String> = Vec::new();
    let mut count = 0;
    for seg in value.split(',') {
        let name = seg.trim();
        if name.is_empty() {
            return Err(format!("{HEADER} has an empty name"));
        }
        count += 1;
        if count > MAX_NAMES {
            return Err(format!("{HEADER} names more than {MAX_NAMES} scenarios"));
        }
        valid_name(name)?;
        names.retain(|n| n != name);
        names.push(name.to_string());
    }
    Ok(Some(names))
}

impl AppState {
    /// The compiled stack for `names` (`k` is their [`key`]), from the cache or built now.
    ///
    /// Locks, in this order only: [`AppState::control`] then `scoped` (write). A reader holds
    /// `scoped` (read) alone and never takes `control`. [`AppState::publish`] clears the cache
    /// while its caller holds `control`, so a stack compiled here can never outlive a change.
    ///
    /// # Errors
    /// A name that is not a defined scenario (`unknown scenario 'x'`).
    ///
    /// # Panics
    /// If the cache lock is poisoned.
    pub fn scoped(&self, k: &str, names: &[String]) -> Result<Arc<Scoped>, String> {
        if let Some(hit) = self.scoped.read().expect("scoped lock poisoned").get(k) {
            return Ok(Arc::clone(hit));
        }
        let c = self.control.lock().unwrap_or_else(PoisonError::into_inner);
        let mut stack = ChaosStack::default();
        stack.set_active(&c.scenarios, names)?;
        let built = Arc::new(Scoped {
            chaos: stack.compile(&c.scenarios),
            pins: stack.compile_pins(&c.scenarios),
        });
        let mut cache = self.scoped.write().expect("scoped lock poisoned");
        if cache.len() >= MAX_CACHED {
            cache.clear();
        }
        cache.insert(k.to_string(), Arc::clone(&built));
        Ok(built)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use kaanal_core::Config;
    use std::path::Path;

    fn headers(vals: &[&[u8]]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for v in vals {
            h.append(HEADER, HeaderValue::from_bytes(v).unwrap());
        }
        h
    }

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn parse_rows() {
        assert_eq!(parse(&HeaderMap::new()), Ok(None));
        assert_eq!(parse(&headers(&[b""])), Ok(Some(vec![])));
        assert_eq!(parse(&headers(&[b"  "])), Ok(Some(vec![])));
        assert_eq!(parse(&headers(&[b"a"])), Ok(Some(names(&["a"]))));
        assert_eq!(parse(&headers(&[b" a , b "])), Ok(Some(names(&["a", "b"]))));
        assert_eq!(parse(&headers(&[b"a,b,a"])), Ok(Some(names(&["b", "a"]))));
        let sixteen = (0..16)
            .map(|i| format!("s{i}"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(parse(&headers(&[sixteen.as_bytes()])).is_ok());
        let seventeen = format!("{sixteen},s16");
        assert!(parse(&headers(&[seventeen.as_bytes()])).is_err());
        // 17 repeats of one name still count as 17.
        assert!(parse(&headers(&[b"a,a,a,a,a,a,a,a,a,a,a,a,a,a,a,a,a"])).is_err());
        for bad in [
            &b"a,,b"[..],
            b"a,",
            b",a",
            b"Bad!",
            b"\xff",
            &[b'a'; 257][..],
        ] {
            assert!(parse(&headers(&[bad])).is_err(), "{bad:?}");
        }
        assert!(parse(&headers(&[b"a", b"b"])).is_err());
    }

    #[test]
    fn cache_clears_at_the_bound() {
        use std::fmt::Write as _;
        let mut yaml = String::from(
            "version: 1\nnodes:\n  gw: { type: gateway }\n  db: { type: database, flavor: postgres }\n  app: { type: service }\nedges:\n  - { from: gw, to: app }\n  - { from: app, to: db }\nscenarios:\n",
        );
        for i in 0..17 {
            let _ = writeln!(
                yaml,
                "  s{i:02}: {{ chaos: [ {{ node: db, latency_ms: 1 }} ] }}"
            );
        }
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let st = AppState::build(&Config::from_yaml(&yaml).unwrap(), &dir, String::new(), 0)
            .map_err(|e| e.to_string())
            .unwrap();
        let mut last = String::new();
        let mut n = 0;
        'outer: for i in 0..17 {
            for j in 0..17 {
                let pair = vec![format!("s{i:02}"), format!("s{j:02}")];
                last = key(&pair);
                st.scoped(&last, &pair).unwrap();
                n += 1;
                if n > MAX_CACHED {
                    break 'outer;
                }
            }
        }
        let cache = st.scoped.read().unwrap();
        assert!(cache.len() <= MAX_CACHED);
        assert!(cache.contains_key(&last));
    }
}

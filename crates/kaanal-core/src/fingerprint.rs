//! Design fingerprint building blocks: a comment- and order-insensitive digest of a YAML/JSON
//! text, and a short combination of labelled digests. Hash: SHA-256 (FIPS 180-4).

use sha2::{Digest, Sha256};

/// SHA-256 of `text` parsed as YAML (JSON is YAML) and written as compact JSON with sorted keys,
/// so comments, whitespace and key order do not matter. Text that does not parse, or has no JSON
/// form (e.g. non-string map keys), hashes its raw bytes, so a corrupt file still changes it.
/// Text whose aliases expand past the config bound (a "billion laughs" file, see
/// [`crate::config::preprocess`]) is never parsed: it hashes its raw bytes too, so this is safe on
/// untrusted spec, scenario and data files.
#[must_use]
pub fn canonical_digest(text: &str) -> [u8; 32] {
    // serde_json's map is a BTreeMap (no `preserve_order` in this workspace), so keys come out sorted.
    let canon = crate::config::preprocess(text)
        .ok()
        .and_then(|src| serde_yaml_ng::from_str::<serde_json::Value>(src).ok())
        .and_then(|v| serde_json::to_vec(&v).ok());
    Sha256::digest(canon.as_deref().unwrap_or(text.as_bytes())).into()
}

/// The first 12 lower-case hex digits of SHA-256 over `label NUL hex(digest) newline` for each
/// part, sorted by label (so input order does not matter).
#[must_use]
pub fn combine(parts: &[(String, [u8; 32])]) -> String {
    use std::fmt::Write;
    let mut sorted: Vec<&(String, [u8; 32])> = parts.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut h = Sha256::new();
    for (label, d) in sorted {
        h.update(label.as_bytes());
        h.update([0]);
        let mut hex = String::with_capacity(64);
        for b in d {
            let _ = write!(hex, "{b:02x}");
        }
        h.update(hex.as_bytes());
        h.update(b"\n");
    }
    let out = h.finalize();
    out.iter().take(6).fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

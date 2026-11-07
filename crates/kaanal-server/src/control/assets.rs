//! Studio's embedded asset table and the pure logic that serves it: exact-name lookup,
//! `Accept-Encoding` negotiation and per-encoding `ETag`s (R1.106).
//!
//! `build.rs` generates the table from the Vite manifest (`manifest.rs`); [`Bundle::embedded`]
//! is that table, or `None` when the binary was built without a bundle. The handlers read the
//! bundle from `AppState`, so a test can serve a fixture bundle without `cfg(kaanal_ui)`.

use axum::http::{Method, Uri};

/// The path every Studio asset is served under; the rest of the path is the asset's name.
pub(crate) const PREFIX: &str = "/__kaanal/studio/assets/";

/// One embedded file of the Studio bundle, served at `/__kaanal/studio/assets/{name}`.
#[derive(Debug)]
pub struct Asset {
    /// The file name under `assets/`, matched exactly (no decoding, no case folding).
    pub name: &'static str,
    /// The `Content-Type` it is served with.
    pub content_type: &'static str,
    /// The file as built.
    pub identity: &'static [u8],
    /// The precompressed `.br` sibling, when the build made one.
    pub br: Option<&'static [u8]>,
    /// The precompressed `.gz` sibling, when the build made one.
    pub gz: Option<&'static [u8]>,
    /// A hash of `identity`; the `ETag` is this plus a per-encoding suffix.
    pub hash: &'static str,
    /// True when the manifest shows Vite hashed the name: cached for a year as `immutable`.
    /// Otherwise (favicon, touch icon) `no-cache`.
    pub immutable: bool,
}

/// The body encoding [`negotiate`] picked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    /// The file as built, no `Content-Encoding`.
    Identity,
    /// The `.br` sibling, `Content-Encoding: br`.
    Br,
    /// The `.gz` sibling, `Content-Encoding: gzip`.
    Gzip,
}

impl Encoding {
    /// The `Content-Encoding` value, `None` for identity.
    pub fn header(self) -> Option<&'static str> {
        match self {
            Encoding::Identity => None,
            Encoding::Br => Some("br"),
            Encoding::Gzip => Some("gzip"),
        }
    }
}

/// A Studio bundle: the page served at `/__kaanal/studio` and its asset table.
#[derive(Clone, Copy, Debug)]
pub struct Bundle {
    /// The bundle's `index.html`.
    pub page: &'static [u8],
    /// Every file the Vite manifest references that passed the allowlist.
    pub assets: &'static [Asset],
}

impl Bundle {
    /// The bundle compiled into this binary (`cfg(kaanal_ui)`), or `None`: Studio is the status page.
    pub fn embedded() -> Option<Bundle> {
        EMBEDDED
    }
}

#[cfg(kaanal_ui)]
include!(concat!(env!("OUT_DIR"), "/studio_bundle.rs"));
/// **Private.** No bundle was built in.
#[cfg(not(kaanal_ui))]
const EMBEDDED: Option<Bundle> = None;

/// The asset named exactly `name`: no decoding, no case folding, no path normalising.
pub fn lookup(table: &'static [Asset], name: &str) -> Option<&'static Asset> {
    // A linear scan over tens of files.
    table.iter().find(|a| a.name == name)
}

/// **Private.** The asset a request names, only for a `GET`/`HEAD` with no query whose raw
/// (still percent-encoded) path is [`PREFIX`] plus a table name. The guard's token exemption
/// and the handler both use this, so they can never disagree.
pub(crate) fn hit(bundle: Option<Bundle>, method: &Method, uri: &Uri) -> Option<&'static Asset> {
    if !(method == Method::GET || method == Method::HEAD) || uri.query().is_some() {
        return None;
    }
    lookup(bundle?.assets, uri.path().strip_prefix(PREFIX)?)
}

/// The body to send: `br`, then `gzip`, then identity, by what `accept_encoding` allows (a
/// coding is allowed when listed, or covered by `*`, with a `q` above zero) and what was built.
/// Identity is the answer of last resort, even under `identity;q=0`.
pub fn negotiate(accept_encoding: Option<&str>, a: &Asset) -> (Encoding, &'static [u8]) {
    let ok = |coding| accepts(accept_encoding.unwrap_or(""), coding);
    match (a.br, a.gz) {
        (Some(b), _) if ok("br") => (Encoding::Br, b),
        (_, Some(g)) if ok("gzip") => (Encoding::Gzip, g),
        _ => (Encoding::Identity, a.identity),
    }
}

/// **Private.** True when `header` (an `Accept-Encoding` value) allows `coding`: its own entry
/// wins, else `*`; a `q` that is zero or does not parse refuses it; not listed refuses it.
fn accepts(header: &str, coding: &str) -> bool {
    let mut star = false;
    for item in header.split(',') {
        let mut parts = item.split(';').map(str::trim);
        let name = parts.next().unwrap_or("");
        let q = parts
            .find_map(|p| p.strip_prefix("q=").or_else(|| p.strip_prefix("Q=")))
            .is_none_or(|q| q.parse::<f32>().is_ok_and(|q| q > 0.0));
        if name.eq_ignore_ascii_case(coding) {
            return q;
        }
        if name == "*" {
            star = q;
        }
    }
    star
}

/// The strong `ETag` of `a` sent as `enc`: its hash plus a per-encoding suffix, so a cache never
/// answers a `br` request with the identity body's validator.
pub fn etag_for(a: &Asset, enc: Encoding) -> String {
    let suffix = match enc {
        Encoding::Identity => "",
        Encoding::Br => "-br",
        Encoding::Gzip => "-gz",
    };
    format!("\"{}{suffix}\"", a.hash)
}

/// **Private.** True when an `If-None-Match` value matches `etag` (`*`, or any listed tag,
/// compared weakly as RFC 9110 requires for this header).
pub(crate) fn etag_matches(if_none_match: &str, etag: &str) -> bool {
    if_none_match
        .split(',')
        .map(str::trim)
        .any(|t| t == "*" || t.strip_prefix("W/").unwrap_or(t) == etag)
}

/// **Private.** The `Cache-Control` of `a`, keyed by file.
pub(crate) fn cache_control(a: &Asset) -> &'static str {
    if a.immutable {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static TABLE: &[Asset] = &[
        Asset {
            name: "app.js",
            content_type: "text/javascript; charset=utf-8",
            identity: b"id",
            br: Some(b"br"),
            gz: Some(b"gz"),
            hash: "0123456789abcdef",
            immutable: true,
        },
        Asset {
            name: "plain.css",
            content_type: "text/css; charset=utf-8",
            identity: b"css",
            br: None,
            gz: None,
            hash: "fedcba9876543210",
            immutable: false,
        },
    ];

    #[test]
    fn lookup_is_exact() {
        assert_eq!(lookup(TABLE, "app.js").map(|a| a.name), Some("app.js"));
        for bad in [
            "../app.js",
            "%2e%2e/x",
            "%2e%2e/app.js",
            "APP.JS",
            "app.js/",
            "//app.js",
            "/app.js",
            "app.js?x=1",
            "app%2Ejs",
            "",
            "app.js.map",
        ] {
            assert!(lookup(TABLE, bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn hit_needs_get_or_head_no_query_and_the_raw_prefix() {
        let b = Some(Bundle {
            page: b"",
            assets: TABLE,
        });
        let uri = |s: &str| s.parse::<Uri>().unwrap();
        let ok = uri("/__kaanal/studio/assets/app.js");
        assert!(hit(b, &Method::GET, &ok).is_some());
        assert!(hit(b, &Method::HEAD, &ok).is_some());
        for m in [Method::POST, Method::OPTIONS, Method::PUT, Method::DELETE] {
            assert!(hit(b, &m, &ok).is_none(), "{m}");
        }
        assert!(hit(None, &Method::GET, &ok).is_none(), "no bundle, no hit");
        for bad in [
            "/__kaanal/studio/assets/app.js?x=1",
            "/__kaanal/studio/assets/app.js?",
            "/__kaanal/studio/assets/app%2Ejs",
            "/__kaanal/studio/assets//app.js",
            "/__kaanal/studio/assets/app.js/",
            "/__kaanal/studio/assets/x/../app.js",
            "/__kaanal/studio/assets%2Fapp.js",
            "/__kaanal/studio/app.js",
            "/__kaanal/Studio/assets/app.js",
        ] {
            assert!(hit(b, &Method::GET, &uri(bad)).is_none(), "{bad}");
        }
    }

    #[test]
    fn negotiate_prefers_br_then_gzip_then_identity() {
        let a = &TABLE[0];
        assert_eq!(negotiate(Some("gzip, br"), a), (Encoding::Br, &b"br"[..]));
        assert_eq!(negotiate(Some("gzip"), a), (Encoding::Gzip, &b"gz"[..]));
        assert_eq!(negotiate(Some("*"), a).0, Encoding::Br);
        assert_eq!(
            negotiate(Some("deflate"), a),
            (Encoding::Identity, &b"id"[..])
        );
        assert_eq!(negotiate(None, a).0, Encoding::Identity);
        assert_eq!(negotiate(Some(""), a).0, Encoding::Identity);
        assert_eq!(negotiate(Some("BR"), a).0, Encoding::Br, "case-insensitive");
        // nothing precompressed: identity whatever is asked
        assert_eq!(
            negotiate(Some("br, gzip"), &TABLE[1]),
            (Encoding::Identity, &b"css"[..])
        );
    }

    #[test]
    fn negotiate_honours_q_zero() {
        let a = &TABLE[0];
        assert_eq!(negotiate(Some("br;q=0, gzip"), a).0, Encoding::Gzip);
        assert_eq!(
            negotiate(Some("br; q=0.0, gzip;q=0.000"), a).0,
            Encoding::Identity
        );
        assert_eq!(negotiate(Some("identity;q=0, gzip"), a).0, Encoding::Gzip);
        assert_eq!(negotiate(Some("*;q=0"), a).0, Encoding::Identity);
        assert_eq!(negotiate(Some("br;q=0, *"), a).0, Encoding::Gzip);
        assert_eq!(negotiate(Some("br;q=0.5"), a).0, Encoding::Br);
        assert_eq!(negotiate(Some("br;q=junk, gzip"), a).0, Encoding::Gzip);
    }

    #[test]
    fn etags_differ_per_encoding() {
        let a = &TABLE[0];
        let tags = [Encoding::Identity, Encoding::Br, Encoding::Gzip].map(|e| etag_for(a, e));
        assert_eq!(tags[0], "\"0123456789abcdef\"");
        assert_eq!(tags[1], "\"0123456789abcdef-br\"");
        assert_eq!(tags[2], "\"0123456789abcdef-gz\"");
        assert_ne!(etag_for(&TABLE[1], Encoding::Identity), tags[0]);
    }

    #[test]
    fn if_none_match_compares_weakly_and_takes_lists() {
        let t = "\"abc-br\"";
        assert!(etag_matches("\"abc-br\"", t));
        assert!(etag_matches("W/\"abc-br\"", t));
        assert!(etag_matches("\"x\", \"abc-br\"", t));
        assert!(etag_matches("*", t));
        assert!(!etag_matches("\"abc\"", t));
        assert!(!etag_matches("\"abc-gz\"", t));
        assert!(!etag_matches("", t));
    }

    #[test]
    fn cache_control_is_keyed_by_file() {
        assert_eq!(
            cache_control(&TABLE[0]),
            "public, max-age=31536000, immutable"
        );
        assert_eq!(cache_control(&TABLE[1]), "no-cache");
    }
}

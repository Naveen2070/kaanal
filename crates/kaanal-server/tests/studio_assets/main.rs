//! Studio's page and asset table over HTTP (R1.106): `GET /__kaanal/studio` and
//! `GET|HEAD /__kaanal/studio/assets/{name}`, served from a fixture bundle injected with
//! `serve_with_studio`, so none of this needs a built UI.

mod page;
mod routes;

use kaanal_core::Config;
use kaanal_server::{Asset, BoundServer, Bundle, ServeOpts};
use std::path::{Path, PathBuf};

/// The hashed script, with both precompressed siblings.
pub(crate) const JS: &str = "index-B7PI925R.js";
/// The hashed style sheet, identity only.
pub(crate) const CSS: &str = "index-5UjPuW-k.css";
/// The favicon, not hashed.
pub(crate) const ICON: &str = "favicon.svg";

pub(crate) static ASSETS: &[Asset] = &[
    Asset {
        name: JS,
        content_type: "text/javascript; charset=utf-8",
        identity: b"console.log('identity')",
        br: Some(b"BR-BYTES"),
        gz: Some(b"GZ-BYTES"),
        hash: "0011223344556677",
        immutable: true,
    },
    Asset {
        name: CSS,
        content_type: "text/css; charset=utf-8",
        identity: b"body{margin:0}",
        br: None,
        gz: None,
        hash: "8899aabbccddeeff",
        immutable: true,
    },
    Asset {
        name: ICON,
        content_type: "image/svg+xml",
        identity: b"<svg/>",
        br: None,
        gz: None,
        hash: "0123456789abcdef",
        immutable: false,
    },
];

pub(crate) static PAGE: &[u8] =
    b"<!doctype html><script type=module src=\"/__kaanal/studio/assets/index-B7PI925R.js\"></script>";

pub(crate) struct Srv {
    pub base: String,
    pub token: String,
    pub client: reqwest::Client,
    _s: BoundServer,
}

fn opts() -> ServeOpts {
    ServeOpts {
        host: "127.0.0.1".parse().unwrap(),
        port: 0,
        token: kaanal_server::new_token(),
        auth_key: None,
        draft: None,
    }
}

fn cfg() -> Config {
    Config::from_yaml("version: 1\nnodes:\n  gw: { type: gateway }\n").unwrap()
}

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// A server serving `studio` (the fixture bundle, or `None` for no bundle).
pub(crate) async fn start_with(studio: Option<Bundle>) -> Srv {
    let o = opts();
    let token = o.token.clone();
    let s = kaanal_server::serve_with_studio(&cfg(), &dir(), o, studio)
        .await
        .unwrap();
    Srv {
        base: format!("http://127.0.0.1:{}", s.addr.port()),
        token,
        client: reqwest::Client::new(),
        _s: s,
    }
}

/// A server started with plain `serve`: the bundle compiled into this build, if any.
pub(crate) async fn start_embedded() -> Srv {
    let o = opts();
    let token = o.token.clone();
    let s = kaanal_server::serve(&cfg(), &dir(), o).await.unwrap();
    Srv {
        base: format!("http://127.0.0.1:{}", s.addr.port()),
        token,
        client: reqwest::Client::new(),
        _s: s,
    }
}

/// The fixture-bundle server.
pub(crate) async fn start() -> Srv {
    start_with(Some(Bundle {
        page: PAGE,
        assets: ASSETS,
    }))
    .await
}

impl Srv {
    /// `method path` with optional extra headers; no token.
    pub async fn req(
        &self,
        method: reqwest::Method,
        path: &str,
        headers: &[(&str, &str)],
    ) -> reqwest::Response {
        let mut r = self.client.request(method, format!("{}{path}", self.base));
        for (k, v) in headers {
            r = r.header(*k, *v);
        }
        r.send().await.unwrap()
    }

    /// `GET path` with no token.
    pub async fn get(&self, path: &str) -> reqwest::Response {
        self.req(reqwest::Method::GET, path, &[]).await
    }

    /// `method path` with the bearer token.
    pub async fn authed(&self, method: reqwest::Method, path: &str) -> reqwest::Response {
        let bearer = format!("Bearer {}", self.token);
        self.req(method, path, &[("Authorization", &bearer)]).await
    }

    /// The status of `method target` sent byte for byte over a raw socket (no client-side
    /// normalising of `..`, `%2e` or `//`), with the token when `token` is true.
    pub async fn raw_status(&self, method: &str, target: &str, token: bool) -> u16 {
        use std::io::{Read, Write};
        let host = self.base.trim_start_matches("http://").to_string();
        let auth = if token {
            format!("Authorization: Bearer {}\r\n", self.token)
        } else {
            String::new()
        };
        let req = format!(
            "{method} {target} HTTP/1.1\r\nHost: {host}\r\n{auth}Connection: close\r\nContent-Length: 0\r\n\r\n"
        );
        tokio::task::spawn_blocking(move || {
            let mut s = std::net::TcpStream::connect(&host).unwrap();
            s.set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            s.write_all(req.as_bytes()).unwrap();
            let mut buf = Vec::new();
            let _ = s.read_to_end(&mut buf);
            let text = String::from_utf8_lossy(&buf).into_owned();
            text.split(' ')
                .nth(1)
                .and_then(|c| c.parse().ok())
                .unwrap_or_else(|| panic!("no status line: {text:?}"))
        })
        .await
        .unwrap()
    }
}

/// The asset path of `name`.
pub(crate) fn asset(name: &str) -> String {
    format!("/__kaanal/studio/assets/{name}")
}

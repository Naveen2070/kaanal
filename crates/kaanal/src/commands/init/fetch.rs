//! `kaanal init URL`: fetches an `OpenAPI` spec under hard limits (M1-F spec C8, C9). One GET, no
//! cookies or auth, a total time budget and a body cap; redirects are followed here, not by
//! `ureq`, so every hop passes the same policy. Proxies come from the environment (`ureq`'s
//! default): unlike the control agent, nothing secret is sent.

use std::io::Read;
use std::time::{Duration, Instant};

use url::{Host, Url};

use crate::{Fail, err};

/// The body cap: reading stops here and the fetch fails (`Content-Length` is not trusted).
pub const MAX_BODY: u64 = 10 * 1024 * 1024;
/// The budget for the whole fetch, every redirect included.
pub const TIMEOUT: Duration = Duration::from_secs(10);
/// Redirects followed; the next one fails.
const MAX_REDIRECTS: usize = 3;
/// Saved under this name when the URL path does not name a spec file.
const DEFAULT_NAME: &str = "openapi.yaml";
/// A longer name from the URL falls back to [`DEFAULT_NAME`].
const MAX_NAME: usize = 100;

/// **Private.** An `init` argument that is meant as a URL (a Windows path never holds `://`).
pub fn is_url(arg: &str) -> bool {
    arg.contains("://")
}

/// **Private.** The URL for messages: no userinfo, query or fragment, control characters escaped.
pub fn shown(raw: &str) -> String {
    if let Ok(mut u) = Url::parse(raw) {
        let _ = u.set_username("");
        let _ = u.set_password(None);
        u.set_query(None);
        u.set_fragment(None);
        return u.as_str().escape_debug().to_string();
    }
    let s = raw.split(['?', '#']).next().unwrap_or_default();
    let s = match s.split_once("://") {
        Some((scheme, rest)) => {
            let end = rest.find('/').unwrap_or(rest.len());
            let host = rest[..end].rsplit('@').next().unwrap_or_default();
            format!("{scheme}://{host}{}", &rest[end..])
        }
        None => s.to_string(),
    };
    s.escape_debug().to_string()
}

/// **Private.** `localhost`, `127.0.0.0/8` or `::1`: the only hosts plain `http` may reach.
fn loopback(u: &Url) -> bool {
    match u.host() {
        Some(Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(a)) => a.is_loopback(),
        Some(Host::Ipv6(a)) => a.is_loopback(),
        None => false,
    }
}

/// **Private.** The rule for the first URL (`from` is `None`) and every redirect target: `http` or
/// `https` only, no userinfo, plain `http` only to a loopback host, never `https` down to `http`.
/// The reason names `to` as [`shown`] prints it.
pub fn check_hop(from: Option<&Url>, to: &Url) -> Result<(), String> {
    let at = shown(to.as_str());
    if !to.username().is_empty() || to.password().is_some() {
        return Err(format!(
            "{at}: a URL with a user name or password is not accepted"
        ));
    }
    match to.scheme() {
        "https" if !cfg!(feature = "https") => Err(format!(
            "{at}: https URLs need kaanal built with the 'https' feature; download the file and pass its path"
        )),
        "https" => Ok(()),
        "http" if from.is_some_and(|f| f.scheme() == "https") => Err(format!(
            "{at}: a redirect from https down to http is refused"
        )),
        "http" if loopback(to) => Ok(()),
        "http" => Err(format!(
            "{at}: plain http is allowed only to localhost, 127.0.0.0/8 or ::1; use https"
        )),
        other => Err(format!(
            "{at}: only http and https URLs are accepted, not {}",
            other.escape_debug()
        )),
    }
}

/// **Private.** The `init` argument as a URL that passes [`check_hop`]; anything else is a usage
/// error.
pub fn parse(arg: &str) -> Result<Url, Fail> {
    let u = Url::parse(arg)
        .map_err(|e| Fail::usage(format!("{}: not a valid URL ({e})", shown(arg))))?;
    check_hop(None, &u).map_err(Fail::usage)?;
    Ok(u)
}

/// **Private.** Where the download is saved: the URL's last path segment, decoded and reduced to
/// `[A-Za-z0-9._-]`, when it starts with a letter or digit, ends in `.yaml`, `.yml` or `.json`,
/// is at most 100 characters and is not a Windows device name; else `openapi.yaml`. Never
/// `kaanal.yaml`, the config itself.
pub fn file_name(u: &Url) -> String {
    let last = u
        .path_segments()
        .and_then(|mut s| s.next_back())
        .unwrap_or_default();
    let name: String = percent_encoding::percent_decode_str(last)
        .decode_utf8_lossy()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let lower = name.to_ascii_lowercase();
    let spec_like = [".yaml", ".yml", ".json"]
        .iter()
        .any(|ext| lower.len() > ext.len() && lower.ends_with(ext));
    // Windows device names (`nul.yaml` opens the device) are refused with any extension.
    let stem = lower.split('.').next().unwrap_or_default();
    let device = matches!(stem, "con" | "prn" | "aux" | "nul")
        || ((stem.starts_with("com") || stem.starts_with("lpt"))
            && stem.len() == 4
            && matches!(stem.as_bytes()[3], b'1'..=b'9'));
    if spec_like
        && !device
        && name.len() <= MAX_NAME
        && name.starts_with(|c: char| c.is_ascii_alphanumeric())
        && lower != "kaanal.yaml"
    {
        name
    } else {
        DEFAULT_NAME.to_string()
    }
}

/// **Private.** A `ureq` error as a short category: its own text can quote the URL (query
/// included) or server bytes, so it is never printed.
fn failure(e: &ureq::Error, timeout: Duration) -> String {
    use std::io::ErrorKind as K;
    match e {
        ureq::Error::Timeout(_) => format!("no complete answer within {timeout:?}"),
        ureq::Error::HostNotFound => "DNS lookup failed".into(),
        ureq::Error::ConnectionFailed
        | ureq::Error::ConnectProxyFailed(_)
        | ureq::Error::InvalidProxyUrl => "could not connect".into(),
        ureq::Error::Io(io) => match io.kind() {
            K::TimedOut => format!("no complete answer within {timeout:?}"),
            K::ConnectionRefused
            | K::ConnectionReset
            | K::ConnectionAborted
            | K::NotConnected
            | K::AddrNotAvailable
            | K::HostUnreachable
            | K::NetworkUnreachable => "could not connect".into(),
            _ => "read error".into(),
        },
        ureq::Error::Tls(_) => "TLS error".into(),
        #[cfg(feature = "https")]
        ureq::Error::Rustls(_) | ureq::Error::Pem(_) => "TLS error".into(),
        _ => "bad response".into(),
    }
}

/// **Private.** [`fetch_with`] at the real limits.
pub fn fetch_spec(u: &Url) -> Result<Vec<u8>, Fail> {
    fetch_with(u, MAX_BODY, TIMEOUT)
}

/// **Private.** GETs `start`, following at most three redirects by hand, and returns the body
/// once it parses as an `OpenAPI` 3.0 document. Errors are exit 1 and name the URL ([`shown`]),
/// never the body.
pub fn fetch_with(start: &Url, cap: u64, timeout: Duration) -> Result<Vec<u8>, Fail> {
    let deadline = Instant::now() + timeout;
    let mut u = start.clone();
    let mut redirects = 0;
    loop {
        let at = shown(u.as_str());
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(err(format!("{at}: no complete answer within {timeout:?}")));
        }
        let mut cfg = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .timeout_global(Some(left));
        // Plain http only ever reaches loopback (`check_hop`): never hand that to an environment
        // proxy, which would fetch it from the proxy's own localhost. https keeps the env proxy.
        if u.scheme() == "http" {
            cfg = cfg.proxy(None);
        }
        let agent: ureq::Agent = cfg.build().into();
        let mut res = agent
            .get(u.as_str())
            .call()
            .map_err(|e| err(format!("{at}: {}", failure(&e, timeout))))?;
        let status = res.status();
        if status.is_redirection() {
            redirects += 1;
            if redirects > MAX_REDIRECTS {
                return Err(err(format!("{at}: more than {MAX_REDIRECTS} redirects")));
            }
            let next = res
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .and_then(|l| u.join(l).ok())
                .ok_or_else(|| err(format!("{at}: HTTP {status} without a usable Location")))?;
            check_hop(Some(&u), &next).map_err(|why| err(format!("redirect refused: {why}")))?;
            u = next;
            continue;
        }
        if !status.is_success() {
            return Err(err(format!("{at}: HTTP {}", status.as_u16())));
        }
        let mut body = Vec::new();
        res.body_mut()
            .as_reader()
            .take(cap + 1)
            .read_to_end(&mut body)
            .map_err(|e| {
                // ureq wraps its own errors (the global timeout among them) in the io::Error.
                let why = match e.get_ref().and_then(|i| i.downcast_ref::<ureq::Error>()) {
                    Some(inner) => failure(inner, timeout),
                    None => failure(&ureq::Error::Io(e), timeout),
                };
                err(format!("{at}: {why}"))
            })?;
        if body.len() as u64 > cap {
            return Err(err(format!(
                "{at}: the response is larger than {} KiB",
                cap / 1024
            )));
        }
        let spec = std::str::from_utf8(&body).ok();
        if spec.is_none_or(|s| kaanal_data::ApiModel::from_openapi(s).is_err()) {
            return Err(err(format!(
                "{at}: the response is not an OpenAPI 3.0 document"
            )));
        }
        return Ok(body);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write as _;
    use std::io::Write;
    use std::net::TcpListener;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    /// A loopback server: for connection `i` it reads the request head, then runs `then(i, conn)`.
    fn serve(then: impl Fn(usize, std::net::TcpStream) + Send + 'static) -> Url {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let at = url(&format!("http://{}/s.yaml", l.local_addr().unwrap()));
        std::thread::spawn(move || {
            for (i, s) in l.incoming().enumerate() {
                let Ok(mut s) = s else { return };
                let (mut head, mut b) = (Vec::new(), [0u8]);
                while !head.ends_with(b"\r\n\r\n") && s.read(&mut b).is_ok_and(|n| n == 1) {
                    head.push(b[0]);
                }
                then(i, s);
            }
        });
        at
    }

    #[test]
    fn rejects_an_https_to_http_redirect() {
        let e = check_hop(
            Some(&url("https://example.com/a")),
            &url("http://127.0.0.1/a.yaml"),
        )
        .unwrap_err();
        assert!(e.contains("from https down to http"), "{e}");
        // The same target is fine from plain loopback http, and https to https is fine.
        assert!(check_hop(Some(&url("http://localhost/")), &url("http://127.0.0.1/a")).is_ok());
        if cfg!(feature = "https") {
            assert!(check_hop(Some(&url("https://a.test/")), &url("https://b.test/")).is_ok());
        }
    }

    #[test]
    fn plain_http_reaches_loopback_hosts_only() {
        for ok in [
            "http://localhost:1/a",
            "http://LOCALHOST/a",
            "http://127.0.0.1/a",
            "http://127.9.9.9/a",
            "http://[::1]:8/a",
        ] {
            assert!(check_hop(None, &url(ok)).is_ok(), "{ok}");
        }
        for bad in [
            "http://example.com/a",
            "http://localhost.example.com/a",
            "http://10.0.0.1/a",
            "http://[::ffff:127.0.0.1]/a",
            "http://0.0.0.0/a",
            "ftp://127.0.0.1/a",
            "file:///etc/passwd",
            "http://u:p@127.0.0.1/a",
        ] {
            assert!(check_hop(None, &url(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn shown_drops_userinfo_query_and_fragment_and_escapes() {
        assert_eq!(
            shown("https://u:p@h.test:8/a/b.yaml?k=s#f"),
            "https://h.test:8/a/b.yaml"
        );
        assert_eq!(shown("http://u:p@[bad/x\u{1b}?q"), "http://[bad/x\\u{1b}");
    }

    #[test]
    fn the_file_name_comes_from_the_last_segment() {
        for (u, name) in [
            ("http://h/v1/shop.yaml?x=1", "shop.yaml"),
            ("http://h/a/Spec.JSON", "Spec.JSON"),
            ("http://h/my%20api.yml", "my_api.yml"),
            ("http://h/..%2F..%2Fx.yaml", "openapi.yaml"),
            ("http://h/a%2F..%2Fx.yaml", "a_.._x.yaml"),
            ("http://h/openapi", "openapi.yaml"),
            ("http://h/", "openapi.yaml"),
            ("http://h/.yaml", "openapi.yaml"),
            ("http://h/kaanal.yaml", "openapi.yaml"),
            ("http://h/spec.yaml.exe", "openapi.yaml"),
        ] {
            assert_eq!(file_name(&url(u)), name, "{u}");
        }
    }

    #[test]
    fn device_names_and_long_names_fall_back() {
        for dev in [
            "con.yaml",
            "PRN.json",
            "Aux.yml",
            "nul.x.yaml",
            "com1.yaml",
            "COM9.json",
            "lpt1.yaml",
            "Lpt9.tar.yml",
        ] {
            assert_eq!(
                file_name(&url(&format!("http://h/{dev}"))),
                DEFAULT_NAME,
                "{dev}"
            );
        }
        for fine in [
            "com0.yaml",
            "com10.yaml",
            "console.yaml",
            "nuls.yaml",
            "lpt.yaml",
        ] {
            assert_eq!(file_name(&url(&format!("http://h/{fine}"))), fine);
        }
        let long = format!("{}.yaml", "a".repeat(95));
        assert_eq!(file_name(&url(&format!("http://h/{long}"))), long);
        let longer = format!("{}.yaml", "a".repeat(96));
        assert_eq!(file_name(&url(&format!("http://h/{longer}"))), DEFAULT_NAME);
    }

    #[test]
    fn times_out_a_slow_drip_body() {
        let at = serve(|_, mut s| {
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n");
            for _ in 0..100 {
                if s.write_all(b"a").and_then(|()| s.flush()).is_err() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        let t = Instant::now();
        let e = fetch_with(&at, MAX_BODY, Duration::from_millis(400)).unwrap_err();
        assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
        assert!(e.1.contains("no complete answer within 400ms"), "{}", e.1);
    }

    #[test]
    fn the_deadline_is_shared_across_hops() {
        // Each hop sleeps 350 ms; the total budget is 500 ms — the first hop completes and we
        // redirect to /h0.yaml, but the budget is exhausted before the second hop can return.
        // We only assert the timeout message; the URL in the error is whichever hop ureq was
        // talking to when the deadline fired, which is implementation-defined.
        let at = serve(|i, mut s| {
            std::thread::sleep(Duration::from_millis(350));
            let _ = s.write_all(
                format!("HTTP/1.1 302 Found\r\nLocation: /h{i}.yaml\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            );
        });
        let e = fetch_with(&at, MAX_BODY, Duration::from_millis(500)).unwrap_err();
        assert!(e.1.contains("no complete answer within 500ms"), "{}", e.1);
    }

    #[test]
    fn alias_expansion_in_a_spec_is_bounded() {
        // "Billion laughs": nine levels of ten aliases would be 10^9 nodes if expanded.
        let mut doc = String::from("l0: &l0 [x, x, x, x, x, x, x, x, x, x]\n");
        for i in 1..=9 {
            let refs = vec![format!("*l{}", i - 1); 10].join(", ");
            let _ = writeln!(doc, "l{i}: &l{i} [{refs}]");
        }
        doc.push_str("openapi: 3.0.3\ninfo: { title: x, version: \"1\" }\npaths: {}\n");
        assert!(doc.len() < 1024);
        let t = Instant::now();
        let r = kaanal_data::ApiModel::from_openapi(&doc);
        assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
        // serde_yaml_ng refuses it: "repetition limit exceeded".
        assert!(r.is_err_and(|e| e.contains("repetition limit")));
    }

    #[test]
    fn rejects_a_body_over_10_mb() {
        // The real cap is MAX_BODY; a small one under test. No Content-Length: the cap is
        // counted while reading.
        let cap = 64 * 1024;
        let at = serve(move |_, mut s| {
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n");
            let _ = s.write_all(&vec![b'a'; usize::try_from(cap).unwrap() + 1]);
        });
        let e = fetch_with(&at, cap, TIMEOUT).unwrap_err();
        assert_eq!(e.0, 1);
        assert!(e.1.contains("larger than 64 KiB"), "{}", e.1);
        assert_eq!(MAX_BODY, 10 * 1024 * 1024);
    }

    #[test]
    fn times_out_a_silent_server() {
        let at = serve(|_, s| {
            std::thread::sleep(Duration::from_secs(5));
            drop(s);
        });
        let t = Instant::now();
        let e = fetch_with(&at, MAX_BODY, Duration::from_millis(300)).unwrap_err();
        assert!(t.elapsed() < Duration::from_secs(4), "{:?}", t.elapsed());
        assert!(e.1.contains("no complete answer within 300ms"), "{}", e.1);
        assert_eq!(TIMEOUT, Duration::from_secs(10));
    }
}

//! Where a client command sends its request: the local instance found via `.kaanal/run.json`, or
//! a remote one named by `--endpoint` / `KAANAL_ENDPOINT` with `--token` / `KAANAL_TOKEN` (spec
//! E1-E6). Remote mode never reads `run.json`, the instance lock or the stale-state logic.
//!
//! The token is never printed, logged or put in an error message.

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::Path;
use std::sync::{Once, OnceLock};

use crate::{Fail, STATE_DIR, err, lifecycle};

/// **Private.** The instance a client command talks to.
pub struct Target {
    /// A name, an IPv4 address or a bracketed IPv6 address, ready for a URL.
    pub host: String,
    pub port: u16,
    pub token: String,
    /// From `--endpoint` / `KAANAL_ENDPOINT`, not from `.kaanal/run.json`.
    pub remote: bool,
}

/// **Private.** The `--endpoint` and `--token` flags, set once in `main`; env is read when needed.
static FLAGS: OnceLock<(Option<String>, Option<String>)> = OnceLock::new();

/// **Private.** The cleartext warning is printed once per process, however many calls a command makes.
static WARNED: Once = Once::new();

/// **Private.** Records the `--endpoint` and `--token` flags (flags beat env, E1).
pub fn set_overrides(endpoint: Option<String>, token: Option<String>) {
    let _ = FLAGS.set((endpoint, token));
}

/// **Private.** An environment variable; empty counts as unset (E1).
fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// **Private.** The endpoint and where it came from (`--endpoint` or `KAANAL_ENDPOINT`), if any.
fn endpoint() -> Option<(&'static str, String)> {
    let flag = FLAGS.get().and_then(|(e, _)| e.clone());
    flag.map(|e| ("--endpoint", e))
        .or_else(|| env("KAANAL_ENDPOINT").map(|e| ("KAANAL_ENDPOINT", e)))
}

/// **Private.** Where the endpoint comes from, for `up` to refuse it by name (E6).
pub fn endpoint_source() -> Option<&'static str> {
    endpoint().map(|(src, _)| src)
}

/// **Private.** A `--token` flag (even an empty one) with no endpoint is a usage error (E2), for
/// every command, so `down` and `up` cannot quietly act locally.
pub fn check_flags() -> Result<(), Fail> {
    if FLAGS.get().is_some_and(|(_, t)| t.is_some()) && endpoint().is_none() {
        return Err(Fail::usage(
            "--token needs --endpoint (or KAANAL_ENDPOINT); a local instance's token is in .kaanal/run.json",
        ));
    }
    Ok(())
}

/// **Private.** The target of a client call: remote when an endpoint is set (it then needs a
/// token from the flag or env), else the local instance (exit 2 if none). A `--token` flag with
/// no endpoint is a usage error; an exported `KAANAL_TOKEN` alone is ignored (E2).
pub fn resolve() -> Result<Target, Fail> {
    check_flags()?;
    let token_flag = FLAGS.get().and_then(|(_, t)| t.clone());
    let Some((src, raw)) = endpoint() else {
        let run = lifecycle::require_instance(Path::new(STATE_DIR))?;
        let host = match run.addr {
            SocketAddr::V4(a) => a.ip().to_string(),
            SocketAddr::V6(a) => format!("[{}]", a.ip()),
        };
        return Ok(Target {
            host,
            port: run.addr.port(),
            token: run.token,
            remote: false,
        });
    };
    let (host, port) = parse_endpoint(&raw).map_err(|m| Fail::usage(format!("bad {src}: {m}")))?;
    // Emptiness matters only here: an empty `--token ""` with no endpoint is still a usage error.
    let token = token_flag
        .filter(|t| !t.is_empty())
        .or_else(|| env("KAANAL_TOKEN"))
        .ok_or_else(|| Fail::usage("--endpoint needs a token (set KAANAL_TOKEN)"))?;
    if !is_loopback(&host) {
        WARNED.call_once(|| eprintln!("warning: sending the token over plain http to {host}"));
    }
    Ok(Target {
        host,
        port,
        token,
        remote: true,
    })
}

impl Target {
    /// **Private.** `http://HOST:PORT`, as messages show it (never with the token).
    pub fn base(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }

    /// **Private.** The URL of a control API path.
    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base())
    }

    /// **Private.** The `Host` header: always loopback with the port, which the server's guard
    /// accepts (R5.3, E3), even when we connect to another address.
    pub fn host_header(&self) -> String {
        format!("localhost:{}", self.port)
    }

    /// **Private.** The `Authorization` header value.
    pub fn auth(&self) -> String {
        format!("Bearer {}", self.token)
    }

    /// **Private.** A request that got no answer. Remote: exit 1 (exit 2 means "no local
    /// instance", which a remote cannot know, E5). Local: the lock decides (M16).
    pub fn unreachable(&self, why: &str) -> Fail {
        if self.remote {
            return err(format!("cannot reach {}: {why}", self.base()));
        }
        match format!("{}:{}", self.host, self.port).parse::<SocketAddr>() {
            Ok(addr) => lifecycle::transport_failure(Path::new(STATE_DIR), addr, why),
            Err(_) => err(format!("cannot reach {}: {why}", self.base())),
        }
    }

    /// **Private.** A remote's answer that needs its own words: 401 is `token rejected`, 403 `bad
    /// Host header` means a port mapping changed the port number (E3, E5). `None`: the usual error.
    pub fn refused(&self, status: u16, msg: &str) -> Option<Fail> {
        if !self.remote {
            return None;
        }
        match status {
            401 => Some(err(format!("{}: token rejected", self.base()))),
            403 if msg == "bad Host header" => Some(err(format!(
                "{}: HTTP 403: bad Host header (a port mapping must keep the port number the instance binds: map {p} to {p})",
                self.base(),
                p = self.port
            ))),
            _ => None,
        }
    }
}

/// **Private.** Parses `http://HOST:PORT` (one trailing `/` allowed) into a URL-ready host and
/// the port. HOST is a name, an IPv4 or a bracketed IPv6. The message never echoes the input: it
/// may hold a password (`http://user:pass@...`).
pub fn parse_endpoint(s: &str) -> Result<(String, u16), String> {
    const USE: &str = "use http://HOST:PORT";
    let lower = s.to_ascii_lowercase();
    if lower.starts_with("https://") {
        return Err("the control API is plain http (use http://HOST:PORT)".into());
    }
    if !lower.starts_with("http://") {
        return Err(format!("no http:// scheme ({USE})"));
    }
    let rest = &s["http://".len()..];
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    if rest.contains('@') {
        return Err(format!("no user or password in the URL ({USE})"));
    }
    if rest.contains(['/', '?', '#']) {
        return Err(format!("no path, query or fragment ({USE})"));
    }
    let (host, port) = if let Some(v6) = rest.strip_prefix('[') {
        let (ip, after) = v6
            .split_once(']')
            .ok_or_else(|| format!("unclosed [ ({USE})"))?;
        ip.parse::<Ipv6Addr>()
            .map_err(|_| format!("bad IPv6 address ({USE})"))?;
        let port = after
            .strip_prefix(':')
            .ok_or_else(|| format!("the port is required ({USE})"))?;
        (format!("[{ip}]"), port)
    } else {
        let (host, port) = rest
            .rsplit_once(':')
            .ok_or_else(|| format!("the port is required ({USE})"))?;
        let name_ok = host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
        if host.is_empty() || !name_ok {
            return Err(format!("bad host ({USE}; IPv6 goes in brackets)"));
        }
        (host.to_string(), port)
    };
    // Digits only: `u16::from_str` also takes a leading `+`.
    if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("the port must be 1-65535 ({USE})"));
    }
    match port.parse::<u16>() {
        Ok(p) if p != 0 => Ok((host, p)),
        _ => Err(format!("the port must be 1-65535 ({USE})")),
    }
}

/// **Private.** `localhost`, `127.0.0.0/8` or `[::1]`: the token does not leave the machine (E4).
pub fn is_loopback(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    if let Some(v6) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        return v6.parse::<Ipv6Addr>().is_ok_and(|ip| ip.is_loopback());
    }
    host.parse::<Ipv4Addr>().is_ok_and(|ip| ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_endpoint_accepts_the_forms_people_paste() {
        let ok = |s: &str| parse_endpoint(s).unwrap();
        assert_eq!(ok("http://localhost:8080"), ("localhost".into(), 8080));
        assert_eq!(ok("http://localhost:8080/"), ("localhost".into(), 8080));
        assert_eq!(ok("http://[::1]:9000"), ("[::1]".into(), 9000));
        assert_eq!(ok("http://10.0.0.5:80"), ("10.0.0.5".into(), 80));
        assert_eq!(ok("HTTP://my-box.lan:7"), ("my-box.lan".into(), 7));
    }

    #[test]
    fn parse_endpoint_rejects_everything_else() {
        for s in [
            "https://h:1",
            "http://h",
            "http://h:1/x",
            "http://h:1//",
            "http://h:1?a=b",
            "http://h:1#f",
            "http://u@h:1",
            "localhost:8080",
            "http://h:0",
            "http://h:70000",
            "http://h:+80",
            "http://h:",
            "http://:80",
            "http://::1:80",
            "http://[::1]",
            "http://[nope]:80",
            "http://h h:80",
            "",
        ] {
            assert!(parse_endpoint(s).is_err(), "{s}");
        }
        assert!(
            parse_endpoint("https://h:1")
                .unwrap_err()
                .contains("plain http")
        );
    }

    #[test]
    fn parse_endpoint_never_echoes_a_password() {
        let m = parse_endpoint("http://user:hunter2@h:1").unwrap_err();
        assert!(!m.contains("hunter2"), "{m}");
    }

    #[test]
    fn is_loopback_table() {
        for (h, want) in [
            ("localhost", true),
            ("LocalHost", true),
            ("127.0.0.1", true),
            ("127.255.0.9", true),
            ("[::1]", true),
            ("::1", false),
            ("[::2]", false),
            ("10.0.0.5", false),
            ("128.0.0.1", false),
            ("localhost.example.com", false),
            ("0.0.0.0", false),
        ] {
            assert_eq!(is_loopback(h), want, "{h}");
        }
    }

    #[test]
    fn remote_failures_never_carry_the_token() {
        let t = Target {
            host: "10.0.0.5".into(),
            port: 8080,
            token: "s3cr3t".into(),
            remote: true,
        };
        let Fail(c, m) = t.unreachable("connection refused");
        assert_eq!(c, 1);
        assert_eq!(m, "cannot reach http://10.0.0.5:8080: connection refused");
        let Fail(c, m) = t.refused(401, "missing or bad token").unwrap();
        assert_eq!((c, m.contains("token rejected")), (1, true));
        assert!(!m.contains("s3cr3t"), "{m}");
        let Fail(_, m) = t.refused(403, "bad Host header").unwrap();
        assert!(m.contains("port number") && !m.contains("s3cr3t"), "{m}");
        assert!(t.refused(404, "unknown node").is_none());
        let local = Target { remote: false, ..t };
        assert!(local.refused(401, "missing or bad token").is_none());
    }
}

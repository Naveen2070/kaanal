//! Field-level serde helpers: ratios, rates, statuses, URLs, durations and the unique-name rules.

use super::*;

/// **Private.** `hit_ratio` and `error_rate`: a probability in 0..=1.
pub(super) fn ratio<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    let v = f64::deserialize(d)?;
    if (0.0..=1.0).contains(&v) {
        Ok(Some(v))
    } else {
        Err(D::Error::custom(format!(
            "must be between 0 and 1 (got {v})"
        )))
    }
}

/// **Private.** Largest rps whose token interval is still at least 1 ns.
pub(super) const MAX_RPS: f64 = 1e9;

/// **Private.** Longest burst window (burst / rps) a limit may have.
pub(super) const MAX_BURST_WINDOW_S: f64 = 365.0 * 86_400.0;

/// **Private.** `rate_limit.rps`: finite, above 0 and at most 1e9.
pub(super) fn positive_rps<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    let v = f64::deserialize(d)?;
    if v > 0.0 && v.is_finite() && v <= MAX_RPS {
        Ok(v)
    } else if v > MAX_RPS && v.is_finite() {
        Err(D::Error::custom(format!(
            "rps must be at most 1000000000 (got {v})"
        )))
    } else {
        Err(D::Error::custom(format!("rps must be above 0 (got {v})")))
    }
}

/// **Private.** `pool.overflow_status`: a 5xx Kaanal can answer with.
pub(super) fn status_5xx<'de, D: Deserializer<'de>>(d: D) -> Result<u16, D::Error> {
    let s = u16::deserialize(d)?;
    if (500..=504).contains(&s) {
        Ok(s)
    } else {
        Err(D::Error::custom(format!(
            "overflow_status must be between 500 and 504 (got {s})"
        )))
    }
}

/// **Private.** `proxy`: `http://` or `https://` followed by something.
pub(super) fn http_url<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let s = String::deserialize(d)?;
    let rest = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"));
    if rest.is_some_and(|r| !r.is_empty()) {
        Ok(Some(s))
    } else {
        Err(D::Error::custom(format!(
            "proxy must be an http(s) URL like https://api.example.com (got '{s}')"
        )))
    }
}

/// **Private.** Longest `timeout:` (the chaos latency cap).
pub(super) const MAX_CALL_TIMEOUT: Duration = Duration::from_secs(300);

/// **Private.** Longest `detect:` (a health-check interval times its unhealthy threshold).
pub(super) const MAX_DETECT: Duration = Duration::from_secs(3600);

/// **Private.** `detect`: 0 (notice at once, the default) to 1 h, with a unit like `10s`.
pub(super) fn detect_window<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Duration>, D::Error> {
    let v: Duration = humantime_serde::deserialize(d)
        .map_err(|_| D::Error::custom("detect: use a duration with a unit, like `10s` or `0s`"))?;
    if v > MAX_DETECT {
        Err(D::Error::custom("detect must be at most 1h"))
    } else {
        Ok(Some(v))
    }
}

/// **Private.** `timeout`: above 0 and at most 5 min. A node-level field, so `diagnostics`
/// places the error at the value.
pub(super) fn call_timeout<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Duration>, D::Error> {
    let v: Duration = humantime_serde::deserialize(d)?;
    if v.is_zero() {
        Err(D::Error::custom("timeout must be above 0"))
    } else if v > MAX_CALL_TIMEOUT {
        Err(D::Error::custom("timeout must be at most 5 min"))
    } else {
        Ok(Some(v))
    }
}

/// **Private.** `token_ttl`: whole seconds from 1 s to 1 day. A node-level field, so
/// `diagnostics` places this error at the value (plan P1); per-user rules live in [`check_auth`].
pub(super) fn token_ttl<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Duration>, D::Error> {
    let v: Duration = humantime_serde::deserialize(d)?;
    if v < Duration::from_secs(1) || v.subsec_nanos() != 0 {
        return Err(D::Error::custom(
            "token_ttl must be whole seconds, at least 1s",
        ));
    }
    if v <= MAX_TOKEN_TTL {
        Ok(Some(v))
    } else {
        Err(D::Error::custom("token_ttl must be at most 1 day"))
    }
}

/// **Private.** `nodes`: a map in file order where each name is defined once (a plain
/// map would silently keep the last definition).
pub(super) fn unique_nodes<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<IndexMap<String, NodeConfig>, D::Error> {
    struct Nodes;
    impl<'de> serde::de::Visitor<'de> for Nodes {
        type Value = IndexMap<String, NodeConfig>;

        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("a map of node names to nodes")
        }

        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut m: A,
        ) -> Result<Self::Value, A::Error> {
            let mut out = IndexMap::new();
            while let Some((name, node)) = m.next_entry::<String, NodeConfig>()? {
                if out.contains_key(&name) {
                    return Err(A::Error::custom(format!("duplicate node '{name}'")));
                }
                out.insert(name, node);
            }
            Ok(out)
        }
    }
    d.deserialize_map(Nodes)
}

/// **Private.** Marks a `password` that was not text (`password: 4711`). serde would reject it
/// with a message that quotes the value, at the node's key, so the field takes any scalar and
/// [`check_auth`] reports it, placed at the value and without it. NUL cannot start a password
/// from a plain scalar; one written as `"\0not-text:…"` is rejected the same way.
pub(super) const NOT_TEXT: &str = "\u{0}not-text:";

/// **Private.** A password as text; a number, boolean or null becomes `NOT_TEXT` plus its kind
/// for [`check_auth`] to reject. Lists and maps fail with a fixed message (never the value).
pub(super) fn password_text<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    struct Text;
    impl serde::de::Visitor<'_> for Text {
        type Value = String;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a string")
        }
        fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<String, E> {
            Ok(v.to_string())
        }
        fn visit_string<E: serde::de::Error>(self, v: String) -> Result<String, E> {
            Ok(v)
        }
        fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<String, E> {
            Ok(format!("{NOT_TEXT}a boolean"))
        }
        fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<String, E> {
            Ok(format!("{NOT_TEXT}a number"))
        }
        fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<String, E> {
            Ok(format!("{NOT_TEXT}a number"))
        }
        fn visit_i128<E: serde::de::Error>(self, _: i128) -> Result<String, E> {
            Ok(format!("{NOT_TEXT}a number"))
        }
        fn visit_u128<E: serde::de::Error>(self, _: u128) -> Result<String, E> {
            Ok(format!("{NOT_TEXT}a number"))
        }
        fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<String, E> {
            Ok(format!("{NOT_TEXT}a number"))
        }
        fn visit_unit<E: serde::de::Error>(self) -> Result<String, E> {
            Ok(format!("{NOT_TEXT}null"))
        }
        fn visit_none<E: serde::de::Error>(self) -> Result<String, E> {
            Ok(format!("{NOT_TEXT}null"))
        }
    }
    d.deserialize_any(Text)
}

/// **Private.** `users`: no two accounts share a username.
pub(super) fn unique_users<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<UserConfig>, D::Error> {
    let users = Vec::<UserConfig>::deserialize(d)?;
    for (i, u) in users.iter().enumerate() {
        if users[..i].iter().any(|o| o.username == u.username) {
            return Err(D::Error::custom(format!(
                "duplicate username '{}'",
                u.username
            )));
        }
    }
    Ok(users)
}

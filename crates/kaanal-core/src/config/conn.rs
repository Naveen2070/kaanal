//! Connection set-up on an edge (RULES 68, step 6b): the TLS version a new connection negotiates
//! and how many calls a keep-alive connection serves before it closes.

use serde::{Deserialize, Deserializer, de};
use std::fmt;

/// The most calls one connection may serve (`keepalive`): a million, far past any real
/// `keepalive_requests`.
pub const MAX_KEEPALIVE: u32 = 1_000_000;

/// Calls a connection serves before it closes when `keepalive` is not written: 1000, nginx's
/// `keepalive_requests` default since 1.19.10 (Envoy and `HAProxy` have no limit by default).
pub const DEFAULT_KEEPALIVE: u32 = 1000;

/// The TLS a new connection on an edge negotiates, after the TCP handshake.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tls {
    /// Plain TCP: no TLS handshake (the default).
    #[default]
    Off,
    /// TLS 1.3 (RFC 8446): one extra round trip.
    V13,
    /// TLS 1.2 (RFC 5246): two extra round trips.
    V12,
}

impl<'de> Deserialize<'de> for Tls {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        /// **Private.** `false`, `true` (1.3), `1.3`, `1.2`, or the same as text.
        struct V;
        impl de::Visitor<'_> for V {
            type Value = Tls;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("`false`, `true` (TLS 1.3), `1.3` or `1.2`")
            }

            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Tls, E> {
                Ok(if v { Tls::V13 } else { Tls::Off })
            }

            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Tls, E> {
                // a YAML number: 1.3 and 1.2 are the only versions modelled
                if (v - 1.3).abs() < 1e-9 {
                    Ok(Tls::V13)
                } else if (v - 1.2).abs() < 1e-9 {
                    Ok(Tls::V12)
                } else {
                    Err(E::custom(format!(
                        "tls: {v} is not a version Kaanal models; use 1.3 or 1.2 (`true` is 1.3)"
                    )))
                }
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Tls, E> {
                match v.trim() {
                    "1.3" | "true" => Ok(Tls::V13),
                    "1.2" => Ok(Tls::V12),
                    "false" | "off" => Ok(Tls::Off),
                    other => Err(E::custom(format!(
                        "tls: `{other}` is not a version Kaanal models; use 1.3 or 1.2"
                    ))),
                }
            }
        }
        d.deserialize_any(V)
    }
}

/// **Private.** `keepalive:` read as a whole number from 1 to [`MAX_KEEPALIVE`].
pub(super) fn keepalive<'de, D: Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    let n = i64::deserialize(d).map_err(|_| {
        de::Error::custom(format!(
            "keepalive: use a whole number of calls per connection, 1 to {MAX_KEEPALIVE}"
        ))
    })?;
    u32::try_from(n)
        .ok()
        .filter(|n| (1..=MAX_KEEPALIVE).contains(n))
        .ok_or_else(|| {
            de::Error::custom(format!(
                "keepalive: {n} is out of range; use 1 (a new connection per call) to {MAX_KEEPALIVE}"
            ))
        })
}

/// **Private.** serde default of [`super::EdgeConfig::keepalive`].
pub(super) fn default_keepalive() -> u32 {
    DEFAULT_KEEPALIVE
}

/// What a call needs to know about the connection it rides: the TLS of a new one and how many calls
/// a connection serves. Copied from the edge into the topology and every call over it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Conn {
    /// TLS a new connection negotiates.
    pub tls: Tls,
    /// Calls one connection serves before it closes, at least 1.
    pub keepalive: u32,
}

impl Default for Conn {
    fn default() -> Self {
        Conn {
            tls: Tls::Off,
            keepalive: DEFAULT_KEEPALIVE,
        }
    }
}

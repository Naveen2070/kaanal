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

/// **Private.** `loss:` read as a probability, 0 (off) to 1.
pub(super) fn loss<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    let v = Option::<f64>::deserialize(d).map_err(|_| {
        de::Error::custom("loss: use a probability from 0 to 1, such as 0.01 for 1%")
    })?;
    match v {
        Some(p) if !(0.0..=1.0).contains(&p) => Err(de::Error::custom(format!(
            "loss: {p} is out of range; use a probability from 0 (none) to 1"
        ))),
        v => Ok(v),
    }
}

/// **Private.** `compress:` read as the share of its size a body keeps on the wire, above 0 and at
/// most 1 (1 = not compressed).
pub(super) fn compress<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    let v = Option::<f64>::deserialize(d).map_err(|_| {
        de::Error::custom(
            "compress: use the share of its size a body keeps, above 0 to 1, such as 0.3",
        )
    })?;
    match v {
        Some(r) if !(r > 0.0 && r <= 1.0) => Err(de::Error::custom(format!(
            "compress: {r} is out of range; use the share of its size a body keeps, above 0 to 1"
        ))),
        v => Ok(v),
    }
}

/// The permille of its size a compressed body keeps on the wire (1000 = uncompressed): `r` rounded
/// up to a thousandth, at least 1.
pub fn compress_permille(r: Option<f64>) -> u16 {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to 1..=1000"
    )]
    r.map_or(1000, |r| (r * 1000.0).ceil().clamp(1.0, 1000.0) as u16)
}

/// Which bodies of a call stream while the callee works (RULES 71, 76): the response (a download,
/// pipeline limit `first byte + size / slowest stage`), the request (an upload the callee consumes as
/// it arrives), both, or neither (buffered).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum StreamMode {
    /// Neither body streams.
    #[default]
    None,
    /// The response streams.
    Response,
    /// The request streams.
    Request,
    /// Both stream.
    Both,
}

impl StreamMode {
    /// Whether the response streams.
    pub fn response(self) -> bool {
        matches!(self, StreamMode::Response | StreamMode::Both)
    }

    /// Whether the request streams.
    pub fn request(self) -> bool {
        matches!(self, StreamMode::Request | StreamMode::Both)
    }
}

impl<'de> Deserialize<'de> for StreamMode {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        /// **Private.** `true` (both), `false` (neither), or `request`, `response`, `both`, `none`.
        struct V;
        impl de::Visitor<'_> for V {
            type Value = StreamMode;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("stream: `true`, `false`, `request`, `response`, `both` or `none`")
            }

            fn visit_bool<E: de::Error>(self, v: bool) -> Result<StreamMode, E> {
                Ok(if v {
                    StreamMode::Both
                } else {
                    StreamMode::None
                })
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<StreamMode, E> {
                match v.trim() {
                    "true" | "both" => Ok(StreamMode::Both),
                    "false" | "none" => Ok(StreamMode::None),
                    "request" => Ok(StreamMode::Request),
                    "response" => Ok(StreamMode::Response),
                    other => Err(E::custom(format!(
                        "stream: `{other}` is not a mode; use true, false, request, response, both or none"
                    ))),
                }
            }
        }
        d.deserialize_any(V)
    }
}

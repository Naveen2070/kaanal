//! UI-state pins: force an endpoint to answer as `loading`, `empty`, `error`, `slow` or
//! `success`, layered like chaos (scenarios, then the live layer on top).
//!
//! Pure data and rules. The chain still runs under a pin; the server decides the response from
//! the [`Outcome`] of [`resolve`].

use std::time::Duration;

use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::chain::Endpoint;

/// Longest pin or `states:` delay.
pub const MAX_PIN_DELAY: Duration = Duration::from_secs(300);
/// Default delay of a `loading` pin.
pub const LOADING_DELAY: Duration = Duration::from_secs(10);
/// Default delay of a `slow` pin.
pub const SLOW_DELAY: Duration = Duration::from_secs(2);

/// Tells whether an endpoint has an `empty` form (the server supplies it; core cannot read
/// schemas). `resolve_scenario` calls it only for `empty` pins.
pub type EmptyOk<'a> = &'a dyn Fn(&Endpoint) -> bool;

/// The UI state a pin forces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PinState {
    /// The response arrives late (default +10 s).
    Loading,
    /// A well-formed response with no data.
    Empty,
    /// An error status (default 500).
    Error,
    /// A response a little late (default +2 s).
    Slow,
    /// The normal response (masks a lower pin).
    Success,
}

/// Cycle order of [`PinState::next`].
const ORDER: [PinState; 5] = [
    PinState::Loading,
    PinState::Empty,
    PinState::Error,
    PinState::Slow,
    PinState::Success,
];

impl PinState {
    /// The lowercase name (`loading`, `empty`, `error`, `slow`, `success`).
    pub fn name(self) -> &'static str {
        match self {
            PinState::Loading => "loading",
            PinState::Empty => "empty",
            PinState::Error => "error",
            PinState::Slow => "slow",
            PinState::Success => "success",
        }
    }

    /// The state with this exact lowercase name.
    pub fn parse(s: &str) -> Option<PinState> {
        ORDER.into_iter().find(|p| p.name() == s)
    }

    /// The next state of the cycle `loading, empty, error, slow, success`, skipping states
    /// `allowed` rejects. `None` and `success` start over at `loading`. When nothing is
    /// allowed, `success`.
    pub fn next(current: Option<PinState>, allowed: impl Fn(PinState) -> bool) -> PinState {
        let from = current.map_or(0, |c| {
            ORDER.iter().position(|s| *s == c).map_or(0, |i| i + 1)
        });
        (0..ORDER.len())
            .map(|k| ORDER[(from + k) % ORDER.len()])
            .find(|s| allowed(*s))
            .unwrap_or(PinState::Success)
    }
}

/// A pin as written in a scenario file or sent to the API.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PinEntryConfig {
    /// `METHOD /path` of the endpoint to pin.
    pub endpoint: String,
    /// The state to force.
    pub state: PinState,
    /// Status of the response: 400-599 for `error`, 200-299 for `empty`. Absent = 500 for `error`, the endpoint's own success status for `empty`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    /// Extra delay before the answer (above 0, at most 5 min). Absent = the state's own delay (`loading` +10 s, `slow` +2 s, none otherwise).
    #[serde(
        default,
        with = "humantime_serde",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(schema_with = "crate::schema::duration_text")]
    pub delay: Option<Duration>,
}

/// A checked pin: what to force and how.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PinEntry {
    /// The state to force.
    pub state: PinState,
    /// Status override.
    pub status: Option<u16>,
    /// Delay override.
    pub delay: Option<Duration>,
}

/// One layer of pins (a scenario's, or the live one), keyed by normalized endpoint key.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PinLayer {
    map: IndexMap<String, PinEntry>,
}

impl PinLayer {
    /// The pin of an endpoint key.
    pub fn get(&self, key: &str) -> Option<&PinEntry> {
        self.map.get(key)
    }

    /// Sets (or replaces) the pin of an endpoint key; returns the old one.
    pub fn insert(&mut self, key: String, entry: PinEntry) -> Option<PinEntry> {
        self.map.insert(key, entry)
    }

    /// Removes the pin of an endpoint key, keeping the order of the rest.
    pub fn remove(&mut self, key: &str) -> Option<PinEntry> {
        self.map.shift_remove(key)
    }

    /// Removes every pin.
    pub fn clear(&mut self) {
        self.map.clear();
    }

    /// Pins in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &PinEntry)> {
        self.map.iter()
    }

    /// How many pins.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether there are no pins.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// The first rule a pin or `states:` entry breaks: the field and a message.
pub(crate) fn check_entry(
    state: PinState,
    status: Option<u16>,
    delay: Option<Duration>,
) -> Result<(), (&'static str, String)> {
    if state == PinState::Success && (status.is_some() || delay.is_some()) {
        let f = if status.is_some() { "status" } else { "delay" };
        return Err((f, "a success pin takes no status or delay".into()));
    }
    if let Some(s) = status {
        let ok = match state {
            PinState::Error => (400..=599).contains(&s),
            PinState::Empty => (200..=299).contains(&s),
            _ => {
                return Err(("status", format!("a {} pin takes no status", state.name())));
            }
        };
        if !ok {
            let range = if state == PinState::Error {
                "400-599"
            } else {
                "200-299"
            };
            return Err((
                "status",
                format!("status {s} must be {range} for {}", state.name()),
            ));
        }
    }
    if let Some(d) = delay
        && (d.is_zero() || d > MAX_PIN_DELAY)
    {
        return Err((
            "delay",
            "delay must be above 0 and at most 5 min".to_string(),
        ));
    }
    Ok(())
}

/// Checks a pin's `status` and `delay` against its state: `success` takes neither; `status`
/// is 400-599 on `error`, 200-299 on `empty`, never on `loading`/`slow`; `delay` is above 0
/// and at most 5 min.
///
/// # Errors
/// A message saying which rule failed.
pub fn validate_entry(
    state: PinState,
    status: Option<u16>,
    delay: Option<Duration>,
) -> Result<(), String> {
    check_entry(state, status, delay).map_err(|(_, m)| m)
}

/// A `states:` override as written on an `endpoints:` entry.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StateOverrideConfig {
    /// Status for this state (400-599 for `error`, 200-299 for `empty`). Absent = 500 for `error`, the endpoint's own success status for `empty`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    /// Body for this state, returned verbatim. Absent = the generated body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
    /// Delay for this state (above 0, at most 5 min). Absent = the state's own delay (`loading` +10 s, `slow` +2 s, none otherwise).
    #[serde(
        default,
        with = "humantime_serde",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(schema_with = "crate::schema::duration_text")]
    pub delay: Option<Duration>,
}

/// A checked `states:` override.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StateOverride {
    /// Status.
    pub status: Option<u16>,
    /// Body.
    pub body: Option<Value>,
    /// Delay.
    pub delay: Option<Duration>,
}

/// The `states:` overrides of one endpoint.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StateOverrides {
    /// For `loading`.
    pub loading: Option<StateOverride>,
    /// For `empty`.
    pub empty: Option<StateOverride>,
    /// For `error`.
    pub error: Option<StateOverride>,
    /// For `slow`.
    pub slow: Option<StateOverride>,
}

impl StateOverrides {
    /// The override of a state (`success` has none).
    pub fn get(&self, s: PinState) -> Option<&StateOverride> {
        match s {
            PinState::Loading => self.loading.as_ref(),
            PinState::Empty => self.empty.as_ref(),
            PinState::Error => self.error.as_ref(),
            PinState::Slow => self.slow.as_ref(),
            PinState::Success => None,
        }
    }

    /// Sets the override of a state (ignored for `success`).
    pub fn set(&mut self, s: PinState, o: StateOverride) {
        let slot = match s {
            PinState::Loading => &mut self.loading,
            PinState::Empty => &mut self.empty,
            PinState::Error => &mut self.error,
            PinState::Slow => &mut self.slow,
            PinState::Success => return,
        };
        *slot = Some(o);
    }
}

/// Which layer a pin came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinSource {
    /// The live layer.
    Live,
    /// The named scenario.
    Scenario(String),
}

/// A pin after folding, with its origin.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedPin {
    /// The winning entry.
    pub entry: PinEntry,
    /// The layer it came from.
    pub source: PinSource,
}

/// Every pin in force, folded from the layers (the higher layer wins whole per endpoint).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EffectivePins {
    map: IndexMap<String, ResolvedPin>,
}

impl EffectivePins {
    /// **Private.** Lays `layer` over what is there.
    pub(crate) fn overlay(&mut self, layer: &PinLayer, source: &PinSource) {
        for (k, e) in layer.iter() {
            self.map.insert(
                k.clone(),
                ResolvedPin {
                    entry: *e,
                    source: source.clone(),
                },
            );
        }
    }

    /// The pin in force for an endpoint key (a `success` pin is returned: it masks).
    pub fn get(&self, key: &str) -> Option<&ResolvedPin> {
        self.map.get(key)
    }

    /// Whether no endpoint is pinned.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// The pins in force, by endpoint key.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &ResolvedPin)> {
        self.map.iter()
    }
}

/// What a pin makes the response: the state, with status, body and delay settled.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    /// The state.
    pub state: PinState,
    /// Status to answer with, if the pin or `states:` set one (`error` defaults to 500).
    pub status: Option<u16>,
    /// Body from `states:`, if any.
    pub body: Option<Value>,
    /// Delay before the answer.
    pub delay: Duration,
}

/// Settles a pin: the entry wins over the endpoint's `states:`, which wins over the defaults
/// (`loading` +10 s, `slow` +2 s, `error` status 500).
pub fn resolve(pin: &PinEntry, ov: &StateOverrides) -> Outcome {
    let o = ov.get(pin.state);
    let default_delay = match pin.state {
        PinState::Loading => LOADING_DELAY,
        PinState::Slow => SLOW_DELAY,
        _ => Duration::ZERO,
    };
    let status = pin.status.or(o.and_then(|o| o.status));
    Outcome {
        state: pin.state,
        status: if pin.state == PinState::Error {
            status.or(Some(500))
        } else {
            status
        },
        body: o.and_then(|o| o.body.clone()),
        delay: pin
            .delay
            .or(o.and_then(|o| o.delay))
            .unwrap_or(default_delay),
    }
}

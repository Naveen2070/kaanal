//! The `endpoints:` map: per-endpoint config, error maps and the step (`call`, `cache`, `parallel`, `repeat`) deserializers.

use super::*;

/// `endpoints:`: a file path or an inline map (same schema either way).
#[derive(Debug, Clone, PartialEq)]
pub enum EndpointsConfig {
    /// Path to a YAML map of endpoints, relative to `kaanal.yaml`.
    File(String),
    /// The endpoints, keyed by `"METHOD /path"`, in file order.
    Inline(IndexMap<String, EndpointConfig>),
}

/// One `endpoints:` entry. Every field is optional; a spec route without an entry gets the
/// default chain.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EndpointConfig {
    /// Database operation for the chain (`read`, `write`, `scan`); default inferred from the
    /// method and path (`GET /x/{id}` read, `GET /x` scan, anything else write).
    pub op: Option<Op>,
    /// Steps the route's owner runs after its own `proc`; absent = the default chain.
    pub chain: Option<Vec<StepConfig>>,
    /// A fixed response served instead of generated data (`status` default 200, `body`).
    #[serde(rename = "override")]
    pub response: Option<Override>,
    /// Failure → response, looked up by failing node, then status (`504`), then class
    /// (`5xx`, `4xx`); a value is a status or `{ status, body }`.
    #[serde(default)]
    pub error_map: IndexMap<String, ErrorMapping>,
    /// Overrides for the UI states a pin can force, keyed `loading`, `empty`, `error` or
    /// `slow`: a `status`, a `body` and a `delay` each.
    #[serde(default)]
    pub states: IndexMap<String, StateOverrideConfig>,
    /// Name of an in-memory collection this route reads or writes (`GET /x` lists, `GET /x/{id}`
    /// gets, `POST /x` adds, `DELETE /x/{id}` removes); cannot be combined with `override`.
    pub collection: Option<String>,
    /// Records a paged list route reports in total, 0 to 10000 (default: seeded per route);
    /// cannot be combined with `override` or `collection`.
    #[serde(default, deserialize_with = "total")]
    #[schemars(extend("minimum" = 0, "maximum" = 10000))]
    pub total: Option<u32>,
}

/// **Private.** `total:` is an integer 0 to 10000; anything else gets one message.
fn total<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u32>, D::Error> {
    /// **Private.** Accepts only integers in range.
    struct V;
    const MSG: &str = "total must be 0 to 10000";
    impl Visitor<'_> for V {
        type Value = Option<u32>;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(MSG)
        }
        fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
            u32::try_from(v)
                .ok()
                .filter(|v| *v <= 10_000)
                .map(Some)
                .ok_or_else(|| E::custom(MSG))
        }
        fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
            u64::try_from(v)
                .map_err(|_| E::custom(MSG))
                .and_then(|v| self.visit_u64(v))
        }
        fn visit_f64<E: de::Error>(self, _: f64) -> Result<Self::Value, E> {
            Err(E::custom(MSG))
        }
        fn visit_str<E: de::Error>(self, _: &str) -> Result<Self::Value, E> {
            Err(E::custom(MSG))
        }
        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
    }
    d.deserialize_any(V)
}

/// An `error_map` value: the status to answer with and an optional body.
#[derive(Debug, Clone, PartialEq)]
pub struct ErrorMapping {
    /// HTTP status of the response.
    pub status: u16,
    /// Body; `None` keeps the default `{ "error": "chaos", "node": … }`.
    pub body: Option<serde_json::Value>,
}

/// One step of a chain (`chain:` lists and node `chains:`).
#[derive(Debug, Clone, PartialEq)]
pub enum StepConfig {
    /// `call: <node>` or a bare node name: charge the edge, then run the callee.
    Call(CallStep),
    /// `cache: <node>`: a lookup; a hit ends the enclosing sequence.
    Cache {
        /// The cache node.
        node: String,
    },
    /// `parallel: [...]`: branches start together; the block ends with the slowest.
    Parallel(Vec<Branch>),
}

/// A `call:` step as written.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CallStep {
    /// The callee.
    pub node: String,
    /// Op for this step (database or inline-ops monolith callees only).
    pub op: Option<Op>,
    /// Sequential invocations; default = the edge's `calls`.
    pub repeat: Option<u32>,
    /// Share of requests that take this call, above 0 and at most 1; absent = the edge's `prob`.
    /// One draw covers every `repeat`.
    pub prob: Option<f64>,
    /// An inline body for a service or monolith callee.
    pub chain: Option<Vec<StepConfig>>,
    /// A key of the callee's `chains:`.
    pub with: Option<String>,
    /// One of the callee's own endpoints, `"METHOD /path"`.
    pub via: Option<String>,
    /// Written as a bare node name (`- db`), so errors point at the list item itself.
    pub bare: bool,
}

/// One `parallel` branch: a single step or a list of steps.
#[derive(Debug, Clone, PartialEq)]
pub struct Branch {
    /// The branch's steps, in order.
    pub steps: Vec<StepConfig>,
    /// Written as a list (`[a, b]`), so its steps sit one level deeper in the file.
    pub list: bool,
}

/// **Private.** Field names a step may have, for messages.
pub(super) const STEP_FIELDS: &str = "call, op, repeat, prob, chain, with, via, cache, parallel";

impl<'de> Deserialize<'de> for StepConfig {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(StepVisitor)
    }
}

/// **Private.** Reads a step by its keys. No buffering, so serde keeps the YAML mark (D1).
pub(super) struct StepVisitor;

impl<'de> Visitor<'de> for StepVisitor {
    type Value = StepConfig;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a step: a node name, `call: <node>`, `cache: <node>` or `parallel: [...]`")
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<StepConfig, E> {
        Ok(StepConfig::Call(CallStep {
            node: v.to_string(),
            bare: true,
            ..CallStep::default()
        }))
    }

    // A node named like a YAML number or bool (`- 1`) is still a name (as in `nodes:`).
    fn visit_u64<E: de::Error>(self, v: u64) -> Result<StepConfig, E> {
        self.visit_str(&v.to_string())
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<StepConfig, E> {
        self.visit_str(&v.to_string())
    }

    fn visit_bool<E: de::Error>(self, v: bool) -> Result<StepConfig, E> {
        self.visit_str(&v.to_string())
    }

    fn visit_map<A: MapAccess<'de>>(self, mut m: A) -> Result<StepConfig, A::Error> {
        let mut call = CallStep::default();
        let (mut has_call, mut cache, mut parallel) = (false, None, None);
        let mut seen: Vec<String> = Vec::new();
        while let Some(key) = m.next_key::<String>()? {
            if seen.contains(&key) {
                return Err(de::Error::custom(format!("duplicate step field '{key}'")));
            }
            match key.as_str() {
                "call" => {
                    call.node = m.next_value()?;
                    has_call = true;
                }
                "op" => call.op = Some(m.next_value()?),
                "repeat" => call.repeat = Some(m.next_value()?),
                "prob" => call.prob = Some(m.next_value()?),
                "chain" => call.chain = Some(m.next_value()?),
                "with" => call.with = Some(m.next_value()?),
                "via" => call.via = Some(m.next_value()?),
                "cache" => cache = Some(m.next_value::<String>()?),
                "parallel" => parallel = Some(m.next_value::<Vec<Branch>>()?),
                other => {
                    return Err(de::Error::custom(format!(
                        "unknown step field '{other}' (expected: {STEP_FIELDS})"
                    )));
                }
            }
            seen.push(key);
        }
        let other = |kind: &str| seen.iter().find(|k| k.as_str() != kind).cloned();
        if let Some(node) = cache {
            if let Some(k) = other("cache") {
                return Err(de::Error::custom(format!(
                    "a cache step takes no other fields (got '{k}')"
                )));
            }
            return Ok(StepConfig::Cache { node });
        }
        if let Some(branches) = parallel {
            if let Some(k) = other("parallel") {
                return Err(de::Error::custom(format!(
                    "a parallel step takes no other fields (got '{k}')"
                )));
            }
            return Ok(StepConfig::Parallel(branches));
        }
        if has_call {
            return Ok(StepConfig::Call(call));
        }
        Err(de::Error::custom(format!(
            "a step needs one of call, cache, parallel (got: {})",
            seen.join(", ")
        )))
    }
}

impl<'de> Deserialize<'de> for Branch {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(BranchVisitor)
    }
}

/// **Private.** A branch is one step, or a list of steps.
pub(super) struct BranchVisitor;

impl BranchVisitor {
    /// **Private.** A branch of one step written without brackets.
    fn one(step: StepConfig) -> Branch {
        Branch {
            steps: vec![step],
            list: false,
        }
    }
}

impl<'de> Visitor<'de> for BranchVisitor {
    type Value = Branch;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a step or a list of steps")
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Branch, E> {
        StepVisitor.visit_str(v).map(Self::one)
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Branch, E> {
        StepVisitor.visit_u64(v).map(Self::one)
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Branch, E> {
        StepVisitor.visit_i64(v).map(Self::one)
    }

    fn visit_bool<E: de::Error>(self, v: bool) -> Result<Branch, E> {
        StepVisitor.visit_bool(v).map(Self::one)
    }

    fn visit_map<A: MapAccess<'de>>(self, m: A) -> Result<Branch, A::Error> {
        StepVisitor.visit_map(m).map(Self::one)
    }

    fn visit_seq<S: SeqAccess<'de>>(self, mut s: S) -> Result<Branch, S::Error> {
        let mut steps = Vec::new();
        while let Some(step) = s.next_element::<StepConfig>()? {
            steps.push(step);
        }
        Ok(Branch { steps, list: true })
    }
}

impl<'de> Deserialize<'de> for ErrorMapping {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        /// **Private.** The `{ status, body }` form.
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Full {
            status: u16,
            #[serde(default)]
            body: Option<serde_json::Value>,
        }
        /// **Private.** A status number or `{ status, body }`.
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = ErrorMapping;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a status (e.g. 502) or `{ status, body }`")
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<ErrorMapping, E> {
                let status = u16::try_from(v)
                    .map_err(|_| E::custom(format!("status {v} must be between 200 and 599")))?;
                Ok(ErrorMapping { status, body: None })
            }
            fn visit_map<A: MapAccess<'de>>(self, m: A) -> Result<ErrorMapping, A::Error> {
                let Full { status, body } = Full::deserialize(MapAccessDeserializer::new(m))?;
                Ok(ErrorMapping { status, body })
            }
        }
        d.deserialize_any(V)
    }
}

/// **Private.** An endpoints map in file order where each key appears once (D19).
pub(super) fn endpoint_entries<'de, A: MapAccess<'de>>(
    mut m: A,
) -> Result<IndexMap<String, EndpointConfig>, A::Error> {
    let mut out = IndexMap::new();
    while let Some(key) = m.next_key::<String>()? {
        if out.contains_key(&key) {
            return Err(de::Error::custom(format!("duplicate endpoint '{key}'")));
        }
        let entry = m.next_value()?;
        out.insert(key, entry);
    }
    Ok(out)
}

impl<'de> Deserialize<'de> for EndpointsConfig {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        /// **Private.** A path or a map.
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = EndpointsConfig;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a path to an endpoints file or a map of endpoints")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<EndpointsConfig, E> {
                Ok(EndpointsConfig::File(v.to_string()))
            }
            fn visit_map<A: MapAccess<'de>>(self, m: A) -> Result<EndpointsConfig, A::Error> {
                endpoint_entries(m).map(EndpointsConfig::Inline)
            }
        }
        d.deserialize_any(V)
    }
}

/// Parses an endpoints file: a map of entries keyed by `"METHOD /path"` (same schema as
/// an inline `endpoints:`). An empty or comment-only file has no entries. A UTF-8 BOM is
/// stripped (R6.4).
///
/// # Errors
/// A [`ConfigError`] at its line and column in this file for malformed YAML, a top level
/// that is not a map, an unknown field or a malformed step.
pub fn parse_endpoints(src: &str) -> Result<IndexMap<String, EndpointConfig>, ConfigError> {
    /// **Private.** The file's top level.
    struct File(IndexMap<String, EndpointConfig>);
    impl<'de> Deserialize<'de> for File {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            /// **Private.** The top-level map.
            struct V;
            impl<'de> Visitor<'de> for V {
                type Value = File;
                fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str("a map of endpoints keyed by \"METHOD /path\"")
                }
                fn visit_map<A: MapAccess<'de>>(self, m: A) -> Result<File, A::Error> {
                    endpoint_entries(m).map(File)
                }
            }
            d.deserialize_map(V)
        }
    }
    let src = src.strip_prefix('\u{feff}').unwrap_or(src);
    serde_yaml_ng::from_str::<Option<File>>(src)
        .map(|f| f.map(|f| f.0).unwrap_or_default())
        .map_err(|e| serde_error(&e))
}

/// **Private.** serde default for [`EdgeConfig::calls`].
pub(super) fn default_calls() -> NonZeroU32 {
    NonZeroU32::MIN
}

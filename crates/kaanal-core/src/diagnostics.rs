//! Positions for config errors (rule R1.15), computed only when something is wrong.
//!
//! `serde_yaml_ng` loses positions inside the `type`-tagged `nodes:` map: an error in any
//! node is reported at the start of `nodes:`. When parsing fails, [`explain`] re-reads the
//! source with `saphyr` (a tree with line/column spans), checks each node against its own
//! branch of the generated JSON Schema (unknown and missing fields), then lets serde check
//! each field alone (bad values), and reports the first problem at its real place.
//! [`Located`] gives errors found after parsing (graph, spec files, endpoints) the same
//! `line:col`, in `kaanal.yaml` or in the endpoints file. If `saphyr` cannot read the file or finds nothing, serde's own position is
//! kept: the two parsers may disagree on exotic YAML, and serde is the safety net.
//!
//! This module only improves messages: it runs after serde has already rejected the file
//! and never decides accept or reject (D11).

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Write as _};
use std::path::PathBuf;

use jsonschema::error::ValidationErrorKind;
use jsonschema::paths::LocationSegment;
use jsonschema::{ValidationError, Validator};
use saphyr::{
    AnnotatedMappingOwned, LoadableYamlNode, MarkedYamlOwned, ScalarOwned, YamlDataOwned,
};
use saphyr_parser::{Event, Parser, Span};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::config::{ConfigError, EdgeConfig, NodeConfig};

/// **Private.** Sources larger than this keep serde's own position (bounded work, D11).
const MAX_SOURCE: usize = 1 << 20;

/// **Private.** A source line longer than this keeps serde's position: block nesting on one
/// line (`- - - - ...`) would otherwise build a tree deep enough to overflow the stack.
const MAX_LINE: usize = 8 * 1024;

/// **Private.** Deepest block/flow nesting the guard allows before keeping serde's error:
/// `saphyr` builds (and drops) its tree recursively, so deep input could overflow the stack.
const MAX_NEST: usize = 128;

/// **Private.** Nesting `to_json` follows before giving up (`null`); real nodes are 3 deep.
const MAX_DEPTH: usize = 32;

/// **Private.** The seven `type:` values, for messages.
const KINDS: &str = "gateway, service, monolith, database, cache, external-api, auth";

/// One step of a path into `kaanal.yaml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathSeg {
    /// A mapping key (`nodes`, a node name, a field).
    Key(String),
    /// A list index (the third entry of `edges:` is `Index(2)`).
    Index(usize),
}

/// A path of mapping keys, e.g. `keys(&["nodes", "db", "spec"])`.
pub fn keys(parts: &[&str]) -> Vec<PathSeg> {
    parts
        .iter()
        .map(|p| PathSeg::Key((*p).to_string()))
        .collect()
}

/// An error at a place in `kaanal.yaml` or in the endpoints file, not yet turned into `line:col`.
#[derive(Debug, Clone, PartialEq)]
pub struct Located {
    /// Where in the file; empty when the error has no place (e.g. a port in use).
    pub path: Vec<PathSeg>,
    /// What is wrong, naming the node, edge, endpoint or field.
    pub msg: String,
    /// The file `path` points into; `None` means `kaanal.yaml`.
    pub file: Option<PathBuf>,
    /// A place already known (a parse error in `file`); used instead of `path`.
    pub pos: Option<(usize, usize)>,
    /// Point at the value of the last path entry instead of its key (a bad value).
    pub value: bool,
}

impl Located {
    /// An error at the key (or list item) `path` names.
    pub fn new(path: Vec<PathSeg>, msg: impl Into<String>) -> Located {
        Located {
            path,
            msg: msg.into(),
            file: None,
            pos: None,
            value: false,
        }
    }

    /// An error at the value of the last entry of `path` (an unknown node name, a bad number).
    pub fn at_value(path: Vec<PathSeg>, msg: impl Into<String>) -> Located {
        Located {
            value: true,
            ..Located::new(path, msg)
        }
    }

    /// The same error, placed in `file` instead of `kaanal.yaml`.
    #[must_use]
    pub fn in_file(self, file: PathBuf) -> Located {
        Located {
            file: Some(file),
            ..self
        }
    }

    /// A parse error of `file` that already knows its position.
    pub fn parsed(file: PathBuf, e: ConfigError) -> Located {
        Located {
            path: Vec::new(),
            msg: e.msg,
            file: Some(file),
            pos: e.line.zip(e.col),
            value: false,
        }
    }

    /// Places the error in `src` (the text of `file`, or of `kaanal.yaml` when `file` is
    /// `None`): a known `pos` as is; otherwise at the key of a mapping entry or the start of a
    /// list item, or at the value when `value` is set or the path ends in an edge's
    /// `from`/`to`. Without a match the error has no position and prints as `file: msg`.
    pub fn in_source(&self, src: &str) -> ConfigError {
        if let Some((line, col)) = self.pos {
            return ConfigError {
                line: Some(line),
                col: Some(col),
                msg: self.msg.clone(),
            };
        }
        let anchor = match self.path.as_slice() {
            _ if self.value => Anchor::Value,
            [.., PathSeg::Index(_), PathSeg::Key(k)] if k == "from" || k == "to" => Anchor::Value,
            _ => Anchor::Key,
        };
        let pos = if self.path.is_empty() {
            None
        } else {
            tree(src).and_then(|root| locate(&root, &self.path, anchor))
        };
        ConfigError {
            line: pos.map(|p| p.0),
            col: pos.map(|p| p.1),
            msg: self.msg.clone(),
        }
    }
}

impl fmt::Display for Located {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.msg)
    }
}

impl std::error::Error for Located {}

/// **Private.** Which end of an entry an error points at.
#[derive(Clone, Copy)]
enum Anchor {
    /// The key (`bogus:` in `bogus: 1`).
    Key,
    /// The value (`1` in `bogus: 1`).
    Value,
}

/// **Private.** A schema finding worth reporting.
enum Problem {
    /// A field the kind does not have.
    Unknown(String),
    /// A required field that is absent.
    Missing(String),
}

/// Turns a serde parse error into one placed at the real problem inside `nodes:` or
/// `edges:` (see the module docs). Returns `serde_err` unchanged when `saphyr` cannot read
/// the source, when the source is too big or too deeply nested, when serde's position lies
/// outside both sections, or when no problem is found.
pub(crate) fn explain(src: &str, serde_err: ConfigError) -> ConfigError {
    if src.len() > MAX_SOURCE || src.lines().any(|l| l.len() > MAX_LINE) || too_deep(src) {
        return serde_err;
    }
    section_error(src, serde_err.line).unwrap_or(serde_err)
}

/// **Private.** An upper bound on nesting per line: each block level needs at least one more
/// column of indentation, or a `- ` / `? ` on the same line; flow levels are `[` or `{`.
fn too_deep(src: &str) -> bool {
    src.lines().any(|l| {
        let indent = l.len() - l.trim_start_matches(' ').len();
        let marks =
            l.matches("- ").count() + l.matches("? ").count() + l.matches(['[', '{']).count();
        indent + marks > MAX_NEST
    })
}

/// **Private.** The first problem among the nodes and edges, by position, limited to the
/// section serde's line falls in.
fn section_error(src: &str, serde_line: Option<usize>) -> Option<ConfigError> {
    let root = tree(src)?;
    let section = |name: &str| {
        entry(&root, name).filter(|(k, v)| {
            serde_line.is_none_or(|l| (k.span.start.line()..=v.span.end.line()).contains(&l))
        })
    };
    let mut found = Vec::new();
    if let Some((_, nodes)) = section("nodes") {
        found.extend(node_error(nodes));
        found.extend(duplicate_error(src));
    }
    if let Some((_, edges)) = section("edges") {
        found.extend(edge_error(edges));
    }
    found.into_iter().min_by_key(|e| (e.line, e.col))
}

/// **Private.** The first problem among the nodes, in file order.
fn node_error(nodes: &MarkedYamlOwned) -> Option<ConfigError> {
    let YamlDataOwned::Mapping(map) = &nodes.data else {
        return None;
    };
    let schema = crate::schema::config_schema();
    // One compiled validator per kind, built on first use.
    let mut cache: BTreeMap<String, Option<(Value, Validator)>> = BTreeMap::new();
    map.iter()
        .find_map(|(k, node)| check_node(&schema, &mut cache, &key_text(k)?, k, node))
}

/// **Private.** The first edge field serde rejects on its own (a bad `net`, `calls: 0`),
/// placed at the field's value.
fn edge_error(edges: &MarkedYamlOwned) -> Option<ConfigError> {
    let YamlDataOwned::Sequence(items) = &edges.data else {
        return None;
    };
    items.iter().enumerate().find_map(|(i, item)| {
        let YamlDataOwned::Mapping(fields) = &item.data else {
            return None;
        };
        let get = |name: &str| {
            fields
                .iter()
                .find(|(k, _)| key_text(k).as_deref() == Some(name))
                .map(|(_, v)| to_json(v, 0, false))
        };
        let fixed = [("from", get("from")?), ("to", get("to")?)];
        fields.iter().find_map(|(k, v)| {
            let field = key_text(k)?;
            if field == "from" || field == "to" {
                return None;
            }
            probe::<EdgeConfig>(&fixed, &field, v)
                .map(|e| error_at(v, format!("edge #{i}: field '{field}': {e}")))
        })
    })
}

/// **Private.** One level of the YAML event walk in [`duplicate_error`].
enum Frame {
    /// A mapping: keys seen so far, the key being processed, and whether a key is next.
    Map {
        seen: BTreeSet<String>,
        key: Option<String>,
        want_key: bool,
    },
    /// A sequence and the index of the item being processed.
    Seq(usize),
}

/// **Private.** A value finished: the parent mapping wants a key next, the parent sequence
/// moves to its next item.
fn value_done(stack: &mut [Frame]) {
    match stack.last_mut() {
        Some(Frame::Map { want_key, .. }) => *want_key = true,
        Some(Frame::Seq(i)) => *i += 1,
        None => {}
    }
}

/// **Private.** The first key defined twice in one mapping under `nodes:`. `saphyr`'s tree
/// keeps only one of two equal keys, so this walks the parser's events (iteratively, with a
/// stack) to see both. A complex (non-scalar) key makes it give up.
fn duplicate_error(src: &str) -> Option<ConfigError> {
    let src = src.strip_prefix('\u{feff}').unwrap_or(src);
    let mut stack: Vec<Frame> = Vec::new();
    for event in Parser::new_from_str(src) {
        let (event, span) = event.ok()?;
        let is_key = matches!(stack.last(), Some(Frame::Map { want_key: true, .. }));
        match event {
            Event::Scalar(text, ..) if is_key => {
                let name = text.into_owned();
                let mut dup = false;
                if let Some(Frame::Map {
                    seen,
                    key,
                    want_key,
                }) = stack.last_mut()
                {
                    dup = !seen.insert(name.clone());
                    *key = Some(name.clone());
                    *want_key = false;
                }
                if dup {
                    return duplicate_at(&stack, &name, span);
                }
            }
            Event::Alias(_) if is_key => {
                if let Some(Frame::Map { key, want_key, .. }) = stack.last_mut() {
                    *key = Some(String::new());
                    *want_key = false;
                }
            }
            Event::MappingStart(..) | Event::SequenceStart(..) if is_key => return None,
            Event::MappingStart(..) => stack.push(Frame::Map {
                seen: BTreeSet::new(),
                key: None,
                want_key: true,
            }),
            Event::SequenceStart(..) => stack.push(Frame::Seq(0)),
            Event::MappingEnd | Event::SequenceEnd => {
                stack.pop();
                value_done(&mut stack);
            }
            Event::Scalar(..) | Event::Alias(_) => value_done(&mut stack),
            Event::DocumentEnd => break,
            _ => {}
        }
    }
    None
}

/// **Private.** The error for key `name` defined twice in the innermost mapping of `stack`,
/// or `None` when that mapping is not inside `nodes:`.
fn duplicate_at(stack: &[Frame], name: &str, span: Span) -> Option<ConfigError> {
    let (Frame::Map { key: Some(top), .. }, rest) = stack.split_first()? else {
        return None;
    };
    if top != "nodes" {
        return None;
    }
    let (_, parents) = rest.split_last()?;
    let (line, col) = (span.start.line(), span.start.col() + 1);
    let msg = match parents.split_first() {
        None => format!("duplicate node '{name}' (each node name may be defined once)"),
        Some((
            Frame::Map {
                key: Some(node), ..
            },
            inner,
        )) => {
            let mut path: Vec<PathSeg> = Vec::new();
            for f in inner {
                path.push(match f {
                    Frame::Map { key, .. } => PathSeg::Key(key.clone()?),
                    Frame::Seq(i) => PathSeg::Index(*i),
                });
            }
            path.push(PathSeg::Key(name.to_string()));
            format!("node '{node}': duplicate field '{}'", dotted(&path))
        }
        Some(_) => return None,
    };
    Some(ConfigError {
        line: Some(line),
        col: Some(col),
        msg,
    })
}

/// **Private.** The first problem in one node, or `None` if it looks valid: a missing or
/// unknown `type`, then unknown/missing fields from its schema branch, then the first
/// field serde rejects on its own (bad durations, ranges, flavors of the wrong family).
fn check_node(
    schema: &Value,
    cache: &mut BTreeMap<String, Option<(Value, Validator)>>,
    name: &str,
    key: &MarkedYamlOwned,
    node: &MarkedYamlOwned,
) -> Option<ConfigError> {
    let YamlDataOwned::Mapping(fields) = &node.data else {
        return Some(error_at(
            node,
            format!("node '{name}': expected a mapping like {{ type: service }}"),
        ));
    };
    let Some((_, tval)) = fields
        .iter()
        .find(|(k, _)| key_text(k).as_deref() == Some("type"))
    else {
        return Some(error_at(
            key,
            format!("node '{name}': missing field 'type' (one of: {KINDS})"),
        ));
    };
    let kind = key_text(tval).unwrap_or_default();
    let entry = cache.entry(kind.clone()).or_insert_with(|| {
        let branch = branch_for(schema, &kind)?;
        let validator = jsonschema::validator_for(&branch).ok()?;
        Some((branch, validator))
    });
    let Some((branch, validator)) = entry.as_ref() else {
        // Either not a kind at all, or a schema we could not compile (then serde's error stays).
        return branch_for(schema, &kind).is_none().then(|| {
            error_at(
                tval,
                format!("node '{name}': unknown type '{kind}' (one of: {KINDS})"),
            )
        });
    };
    schema_problem(schema, branch, validator, name, key, node)
        .or_else(|| serde_problem(&kind, name, fields))
}

/// **Private.** Unknown and missing fields in `node` per its schema branch; the first in
/// file order.
fn schema_problem(
    schema: &Value,
    branch: &Value,
    validator: &Validator,
    name: &str,
    key: &MarkedYamlOwned,
    node: &MarkedYamlOwned,
) -> Option<ConfigError> {
    let instance = to_json(node, 0, false);
    let mut problems = Vec::new();
    for err in validator.iter_errors(&instance) {
        field_problems(&err, &mut problems);
    }
    problems
        .into_iter()
        .filter_map(|(path, problem)| match problem {
            Problem::Unknown(field) => {
                let allowed = allowed_fields(schema, branch, &path);
                let mut at = path;
                at.push(PathSeg::Key(field));
                let pos = locate(node, &at, Anchor::Key)?;
                let expected = if allowed.is_empty() {
                    String::new()
                } else {
                    format!(" (expected: {})", allowed.join(", "))
                };
                Some((
                    pos,
                    format!("node '{name}': unknown field '{}'{expected}", dotted(&at)),
                ))
            }
            Problem::Missing(field) => {
                let pos = if path.is_empty() {
                    position(key)
                } else {
                    locate(node, &path, Anchor::Value)?
                };
                let mut at = path;
                at.push(PathSeg::Key(field));
                Some((
                    pos,
                    format!("node '{name}': missing field '{}'", dotted(&at)),
                ))
            }
        })
        .min_by_key(|(pos, _)| *pos)
        .map(|((line, col), msg)| ConfigError {
            line: Some(line),
            col: Some(col),
            msg,
        })
}

/// **Private.** Collects unknown/missing fields from `err`, looking inside `anyOf` (how
/// schemars writes an optional field) but not `oneOf` (durations, whose three branches
/// would all complain; the serde probe explains those better).
fn field_problems(err: &ValidationError<'_>, out: &mut Vec<(Vec<PathSeg>, Problem)>) {
    let path = || err.instance_path().iter().map(seg).collect::<Vec<_>>();
    match err.kind() {
        ValidationErrorKind::AdditionalProperties { unexpected } => {
            out.extend(
                unexpected
                    .iter()
                    .map(|f| (path(), Problem::Unknown(f.clone()))),
            );
        }
        ValidationErrorKind::Required { property } => {
            let field = property.as_str().unwrap_or_default().to_string();
            out.push((path(), Problem::Missing(field)));
        }
        ValidationErrorKind::AnyOf { context } => {
            for e in context.iter().flatten() {
                field_problems(e, out);
            }
        }
        _ => {}
    }
}

/// **Private.** The first field (file order) that serde rejects when the node is
/// deserialized with that field alone, placed at the field's value.
fn serde_problem(
    kind: &str,
    name: &str,
    fields: &AnnotatedMappingOwned<MarkedYamlOwned>,
) -> Option<ConfigError> {
    let fixed = [("type", Value::from(kind))];
    fields.iter().find_map(|(k, v)| {
        let field = key_text(k)?;
        if field == "type" {
            return None;
        }
        probe::<NodeConfig>(&fixed, &field, v)
            .map(|e| error_at(v, format!("node '{name}': field '{field}': {e}")))
    })
}

/// **Private.** serde's error for `T` built from `fixed` plus this one field, or `None` if
/// it is fine. `serde_yaml_ng` reads any plain scalar as a string (`password: 12345`) and
/// JSON does not, so a field that is fine with its scalars as text counts as fine.
fn probe<T: DeserializeOwned>(
    fixed: &[(&str, Value)],
    field: &str,
    v: &MarkedYamlOwned,
) -> Option<String> {
    let build = |text: bool| {
        let mut m = Map::new();
        for (k, val) in fixed {
            m.insert((*k).into(), val.clone());
        }
        m.insert(field.into(), to_json(v, 0, text));
        Value::Object(m)
    };
    let err = serde_json::from_value::<T>(build(false)).err()?;
    if serde_json::from_value::<T>(build(true)).is_ok() {
        return None;
    }
    Some(err.to_string())
}

/// **Private.** The schema branch for `type: kind`, made standalone by copying in the
/// shared `$defs` its `$ref`s point at.
fn branch_for(schema: &Value, kind: &str) -> Option<Value> {
    let mut branch = schema
        .pointer("/$defs/NodeConfig/oneOf")?
        .as_array()?
        .iter()
        .find(|b| b.pointer("/properties/type/const").and_then(Value::as_str) == Some(kind))?
        .clone();
    branch
        .as_object_mut()?
        .insert("$defs".into(), schema.get("$defs")?.clone());
    Some(branch)
}

/// **Private.** Field names the schema allows at `path` inside `branch` (following `$ref`
/// and the `anyOf` of optional fields), sorted; empty if it cannot tell. `type` is left
/// out at the top level (every node has it).
fn allowed_fields(schema: &Value, branch: &Value, path: &[PathSeg]) -> Vec<String> {
    let mut s = branch;
    for seg in path {
        let PathSeg::Key(k) = seg else {
            return Vec::new();
        };
        match resolve(schema, s).get("properties").and_then(|p| p.get(k)) {
            Some(next) => s = next,
            None => return Vec::new(),
        }
    }
    let mut names: Vec<String> = resolve(schema, s)
        .get("properties")
        .and_then(Value::as_object)
        .map(|p| {
            p.keys()
                .filter(|k| !path.is_empty() || k.as_str() != "type")
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// **Private.** Follows `$ref`s into `$defs` and the non-null branch of an `anyOf`.
fn resolve<'a>(schema: &'a Value, mut s: &'a Value) -> &'a Value {
    // schemars nests at most a couple of levels; the bound only guards a malformed schema.
    for _ in 0..4 {
        if let Some(name) = s
            .get("$ref")
            .and_then(Value::as_str)
            .and_then(|r| r.strip_prefix("#/$defs/"))
            && let Some(target) = schema["$defs"].get(name)
        {
            s = target;
        } else if let Some(obj) = s
            .get("anyOf")
            .and_then(Value::as_array)
            .and_then(|alts| alts.iter().find(|a| a["type"] != "null"))
        {
            s = obj;
        } else {
            break;
        }
    }
    s
}

/// **Private.** The first YAML document of `src` as a tree with spans (BOM stripped).
fn tree(src: &str) -> Option<MarkedYamlOwned> {
    let src = src.strip_prefix('\u{feff}').unwrap_or(src);
    MarkedYamlOwned::load_from_str(src).ok()?.into_iter().next()
}

/// **Private.** A scalar key as serde reads it: `true:` names the node "true".
fn key_text(k: &MarkedYamlOwned) -> Option<String> {
    match &k.data {
        YamlDataOwned::Value(ScalarOwned::String(s)) => Some(s.clone()),
        YamlDataOwned::Value(ScalarOwned::Integer(i)) => Some(i.to_string()),
        YamlDataOwned::Value(ScalarOwned::FloatingPoint(f)) => Some(f.0.to_string()),
        YamlDataOwned::Value(ScalarOwned::Boolean(b)) => Some(b.to_string()),
        YamlDataOwned::Value(ScalarOwned::Null) => Some("null".into()),
        _ => None,
    }
}

/// **Private.** The entry `key` of a mapping node, as (key node, value node).
fn entry<'a>(
    m: &'a MarkedYamlOwned,
    key: &str,
) -> Option<(&'a MarkedYamlOwned, &'a MarkedYamlOwned)> {
    let YamlDataOwned::Mapping(map) = &m.data else {
        return None;
    };
    map.iter()
        .find(|(k, _)| key_text(k).as_deref() == Some(key))
}

/// **Private.** 1-based `(line, col)` of `path` below `root`.
fn locate(root: &MarkedYamlOwned, path: &[PathSeg], anchor: Anchor) -> Option<(usize, usize)> {
    let mut node = root;
    let mut key = None;
    for seg in path {
        match seg {
            PathSeg::Key(k) => {
                let (kn, v) = entry(node, k)?;
                (key, node) = (Some(kn), v);
            }
            PathSeg::Index(i) => {
                let YamlDataOwned::Sequence(items) = &node.data else {
                    return None;
                };
                (key, node) = (None, items.get(*i)?);
            }
        }
    }
    let at = match (anchor, key) {
        (Anchor::Key, Some(k)) => k,
        _ => node,
    };
    Some(position(at))
}

/// **Private.** 1-based `(line, col)` where `n` starts. `saphyr` lines are 1-based and
/// columns 0-based (in chars).
fn position(n: &MarkedYamlOwned) -> (usize, usize) {
    (n.span.start.line(), n.span.start.col() + 1)
}

/// **Private.** `msg` at the start of `n`.
fn error_at(n: &MarkedYamlOwned, msg: String) -> ConfigError {
    let (line, col) = position(n);
    ConfigError {
        line: Some(line),
        col: Some(col),
        msg,
    }
}

/// **Private.** A `jsonschema` path segment as a [`PathSeg`].
fn seg(s: LocationSegment<'_>) -> PathSeg {
    match s {
        LocationSegment::Property(p) => PathSeg::Key(p.into_owned()),
        LocationSegment::Index(i) => PathSeg::Index(i),
    }
}

/// **Private.** `ops.read`, `users[1].username`.
fn dotted(path: &[PathSeg]) -> String {
    let mut s = String::new();
    for seg in path {
        match seg {
            PathSeg::Key(k) => {
                if !s.is_empty() {
                    s.push('.');
                }
                s.push_str(k);
            }
            PathSeg::Index(i) => {
                let _ = write!(s, "[{i}]");
            }
        }
    }
    s
}

/// **Private.** The tree as JSON for schema checks and serde probes. Keys use the text
/// serde sees; aliases are already resolved by `saphyr`; anything odd becomes `null`.
fn to_json(n: &MarkedYamlOwned, depth: usize, text: bool) -> Value {
    if depth > MAX_DEPTH {
        return Value::Null;
    }
    match &n.data {
        YamlDataOwned::Value(scalar) => match scalar {
            ScalarOwned::Null => Value::Null,
            ScalarOwned::Boolean(b) if text => Value::String(b.to_string()),
            ScalarOwned::Integer(i) if text => Value::String(i.to_string()),
            ScalarOwned::FloatingPoint(f) if text => Value::String(f.0.to_string()),
            ScalarOwned::Boolean(b) => Value::Bool(*b),
            ScalarOwned::Integer(i) => Value::from(*i),
            ScalarOwned::FloatingPoint(f) => {
                serde_json::Number::from_f64(f.0).map_or(Value::Null, Value::Number)
            }
            ScalarOwned::String(s) => Value::String(s.clone()),
        },
        YamlDataOwned::Sequence(items) => {
            items.iter().map(|i| to_json(i, depth + 1, text)).collect()
        }
        YamlDataOwned::Mapping(m) => Value::Object(
            m.iter()
                .filter_map(|(k, v)| Some((key_text(k)?, to_json(v, depth + 1, text))))
                .collect::<Map<String, Value>>(),
        ),
        YamlDataOwned::Tagged(_, inner) => to_json(inner, depth + 1, text),
        _ => Value::Null,
    }
}

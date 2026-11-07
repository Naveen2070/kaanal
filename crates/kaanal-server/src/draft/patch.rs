//! Patches to the draft text: the UI's `{ op, path, value }` edits (and the two Studio
//! operations `add_node` and `rename_node`), applied with [`kaanal_core::splice`] so the
//! comments, key order and flow style of the file survive.

use kaanal_core::NodeKind;
use kaanal_core::PathSeg;
use kaanal_core::presets::starter;
use kaanal_core::splice::{Edit, splice};
use serde::Deserialize;
use serde_json::Value;
use serde_yaml_ng::Value as Yaml;
use std::fmt::Write as _;

/// One patch operation, as the API takes it. Which fields it needs depends on `op`:
/// `set`/`insert`/`append`/`remove` take `path` (and `value`), `swap` takes `path` and `with` (two items of
/// one list), `add_node` takes `name` and `kind`,
/// `rename_node` takes `from` and `to`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Op {
    #[serde(rename = "op")]
    what: Kind,
    /// Mapping keys (strings) and list indexes (integers). For `insert` the last step is the new
    /// key and the rest is the mapping it goes into.
    #[serde(default)]
    path: Vec<Value>,
    /// `swap`: the path of the second item (the same list as `path`).
    #[serde(default, rename = "with")]
    with_path: Vec<Value>,
    /// The new value (`set` and `insert`); `null` is a value, a missing `value` is not.
    #[serde(default, deserialize_with = "present")]
    value: Option<Yaml>,
    /// `add_node`: the new node's name.
    name: Option<String>,
    /// `add_node`: its kind (`service`, `external-api`, ...).
    kind: Option<String>,
    /// `rename_node`: the node's name now.
    from: Option<String>,
    /// `rename_node`: the name it gets.
    to: Option<String>,
}

/// **Private.** A `value` that is in the body, even when it is `null`.
fn present<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Yaml>, D::Error> {
    Yaml::deserialize(d).map(Some)
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Set,
    Insert,
    Append,
    Remove,
    Swap,
    AddNode,
    RenameNode,
}

/// What a patch did.
#[derive(Debug)]
pub struct Applied {
    /// The new draft text.
    pub text: String,
    /// What could not be kept (comments, anchors).
    pub lost: Vec<String>,
    /// Where a `rename_node` changed the text, one entry per place (`nodes.db`, `edges[1].to`).
    pub touched: Vec<String>,
    /// Things a rename cannot do for the caller (a file that is not part of the draft).
    pub notes: Vec<String>,
    /// The renames made, `(from, to)`, in order.
    pub renamed: Vec<(String, String)>,
}

/// The most operations one patch may hold.
pub const MAX_OPS: usize = 1000;

/// Applies `ops` to `text` in order, each on the result of the one before.
///
/// # Errors
/// A message for a bad operation (a path that is not keys and indexes, a missing value, an
/// `insert` without a key, an unknown node kind, a rename that cannot be done everywhere) or an
/// edit that cannot be made (nothing at the path, a key that is already there, text that is not
/// YAML). The text is then unchanged.
pub fn apply(text: &str, ops: &[Op]) -> Result<Applied, String> {
    // every op re-parses the whole text, so the count bounds the work one request can ask for
    if ops.len() > MAX_OPS {
        return Err(format!("a patch may hold at most {MAX_OPS} operations"));
    }
    let mut out = Applied {
        text: text.to_string(),
        lost: Vec::new(),
        touched: Vec::new(),
        notes: Vec::new(),
        renamed: Vec::new(),
    };
    for o in ops {
        let (edits, touched) = match o.what {
            Kind::RenameNode => {
                let (from, to) = (need(o.from.as_ref(), "from")?, need(o.to.as_ref(), "to")?);
                let plan = super::rename::plan(&out.text, from, to)?;
                out.notes.extend(plan.notes);
                out.renamed.push((from.to_string(), to.to_string()));
                (plan.edits, plan.touched)
            }
            _ => (vec![edit(o)?], Vec::new()),
        };
        let done = splice(&out.text, &edits).map_err(|e| e.to_string())?;
        if matches!(o.what, Kind::RenameNode) && !done.lost.is_empty() {
            // half a rename is worse than none: a reference left behind breaks the system
            return Err(format!(
                "cannot rename everywhere, so nothing was changed: {}",
                done.lost.join("; ")
            ));
        }
        out.text = done.text;
        out.lost.extend(done.lost);
        out.touched.extend(touched);
    }
    Ok(out)
}

/// **Private.** A path of keys (text) and list indexes (integers) as edit segments.
fn segments(path: &[Value]) -> Result<Vec<PathSeg>, String> {
    path.iter()
        .map(|s| match s {
            Value::String(k) => Ok(PathSeg::Key(k.clone())),
            Value::Number(n) => n
                .as_u64()
                .and_then(|i| usize::try_from(i).ok())
                .map(PathSeg::Index)
                .ok_or_else(|| format!("path step {n} is not a list index")),
            _ => Err("a path is made of keys (text) and list indexes (numbers)".to_string()),
        })
        .collect()
}

/// **Private.** A field an operation needs.
fn need<'a>(v: Option<&'a String>, name: &str) -> Result<&'a str, String> {
    v.map(String::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("this operation needs '{name}'"))
}

/// **Private.** One `set`, `insert`, `remove` or `add_node` as a splice edit.
fn edit(o: &Op) -> Result<Edit, String> {
    if let Kind::AddNode = o.what {
        let (name, kind) = (
            need(o.name.as_ref(), "name")?,
            need(o.kind.as_ref(), "kind")?,
        );
        let kind = NodeKind::from_name(kind).ok_or_else(|| {
            format!("unknown node kind '{kind}' (gateway, service, monolith, database, cache, external-api, auth, load-balancer, storage)")
        })?;
        return Ok(Edit::Insert {
            parent: vec![PathSeg::Key("nodes".into())],
            key: name.to_string(),
            value: starter(kind).to_string(),
        });
    }
    let mut path = segments(&o.path)?;
    let value = || {
        o.value
            .as_ref()
            .map(render)
            .ok_or_else(|| format!("{:?} needs a value", o.what))
    };
    Ok(match o.what {
        Kind::Set => Edit::Set {
            value: value()?,
            path,
        },
        Kind::Append => Edit::Append {
            parent: path,
            value: value()?,
        },
        Kind::Remove => Edit::Remove { path },
        Kind::Swap => Edit::Swap {
            a: path,
            b: segments(&o.with_path)?,
        },
        Kind::Insert => {
            let Some(PathSeg::Key(key)) = path.pop() else {
                return Err("insert ends in the new key (text)".into());
            };
            Edit::Insert {
                parent: path,
                key,
                value: value()?,
            }
        }
        Kind::AddNode | Kind::RenameNode => unreachable!("handled before"),
    })
}

/// **Private.** Whether `s` reads back as the same string when written bare.
fn plain(s: &str) -> bool {
    // a duration or size (`2ms`, `1.5s`): digits, then letters only, so never a number
    let unit = s.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.');
    // `0x1f`, `0o7`, `0b1` are numbers
    let radix =
        s.len() > 1 && s.starts_with('0') && s[1..].starts_with(|c: char| c.is_ascii_alphabetic());
    if !radix
        && unit.len() < s.len()
        && s.starts_with(|c: char| c.is_ascii_digit())
        && !unit.is_empty()
        && unit.chars().all(|c| c.is_ascii_alphabetic())
        && s.matches('.').count() <= 1
    {
        return true;
    }
    let mut c = s.chars();
    c.next()
        .is_some_and(|f| f.is_ascii_alphabetic() || f == '_')
        && c.all(|x| x.is_ascii_alphanumeric() || matches!(x, '_' | '.' | '-'))
        && !matches!(
            s.to_ascii_lowercase().as_str(),
            "true" | "false" | "null" | "yes" | "no" | "on" | "off" | "y" | "n"
        )
}

/// A string as it is written in the file: bare when that reads back as the same text, JSON-quoted
/// (valid YAML) otherwise.
pub(super) fn scalar(s: &str) -> String {
    if plain(s) {
        s.to_string()
    } else {
        Value::from(s).to_string()
    }
}

/// **Private.** A value as YAML in the project's flow style: bare words where they are safe,
/// JSON quoting (valid YAML) otherwise, `{ a: 1, b: x }` and `[a, b]` for collections. The value
/// is read as YAML (not JSON) so an object keeps the order the client sent: `{ type: cache, ... }`.
fn render(v: &Yaml) -> String {
    match v {
        Yaml::String(s) => scalar(s),
        Yaml::Sequence(a) => format!("[{}]", a.iter().map(render).collect::<Vec<_>>().join(", ")),
        Yaml::Mapping(m) if m.is_empty() => "{}".into(),
        Yaml::Mapping(m) => {
            let mut out = String::from("{ ");
            for (i, (k, x)) in m.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                let key = k.as_str().map_or_else(|| render(k), scalar);
                let _ = write!(out, "{key}: {}", render(x));
            }
            out.push_str(" }");
            out
        }
        Yaml::Null => "null".into(),
        Yaml::Bool(b) => b.to_string(),
        Yaml::Number(n) => n.to_string(),
        Yaml::Tagged(t) => render(&t.value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A value as the API receives it: JSON, read as YAML.
    fn y(json: &str) -> Yaml {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn values_render_in_flow_style() {
        let r = |j: &str| render(&y(j));
        assert_eq!(r("8"), "8");
        assert_eq!(r("true"), "true");
        assert_eq!(r("null"), "null");
        assert_eq!(r("\"db\""), "db");
        assert_eq!(
            r(r#"{ "from": "a", "to": "b", "net": "2ms" }"#),
            "{ from: a, to: b, net: 2ms }",
            "the order the client sent"
        );
        assert_eq!(r(r#"["a", 1]"#), "[a, 1]");
        assert_eq!(r("{}"), "{}");
    }

    #[test]
    fn a_null_value_is_a_value_and_a_missing_one_is_not() {
        let op = |j: &str| serde_json::from_str::<Op>(j).unwrap();
        let set_null = apply(
            "a: 1\n",
            &[op(r#"{ "op": "set", "path": ["a"], "value": null }"#)],
        );
        assert_eq!(set_null.unwrap().text, "a: null\n");
        let missing = apply("a: 1\n", &[op(r#"{ "op": "set", "path": ["a"] }"#)]);
        assert!(missing.unwrap_err().contains("needs a value"));
    }

    #[test]
    fn strings_that_would_change_meaning_are_quoted() {
        let r = |s: &str| render(&Yaml::String(s.to_string()));
        assert_eq!(r("yes"), "\"yes\"");
        assert_eq!(r("8"), "\"8\"");
        assert_eq!(r("a: b"), "\"a: b\"");
        assert_eq!(r(""), "\"\"");
        assert_eq!(r("line\nbreak"), "\"line\\nbreak\"");
        assert_eq!(r("1e3"), "\"1e3\"");
        assert_eq!(r("0xab"), "\"0xab\"", "reads back as a number if bare");
        assert_eq!(r("0o7"), "\"0o7\"");
        assert_eq!(r("0.5s"), "0.5s");
        assert_eq!(r("2ms"), "2ms", "a duration is safe bare");
        assert_eq!(r("1.5s"), "1.5s");
    }
}

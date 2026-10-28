//! Patches to the draft text: the UI's `{ op, path, value }` edits, applied with
//! [`kaanal_core::splice`] so the comments, key order and flow style of the file survive.

use kaanal_core::PathSeg;
use kaanal_core::splice::{Edit, Spliced, splice};
use serde::Deserialize;
use serde_json::Value;
use serde_yaml_ng::Value as Yaml;
use std::fmt::Write as _;

/// One patch operation, as the API takes it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Op {
    #[serde(rename = "op")]
    kind: Kind,
    /// Mapping keys (strings) and list indexes (integers). For `insert` the last step is the new
    /// key and the rest is the mapping it goes into.
    path: Vec<Value>,
    /// The new value (`set` and `insert`); `null` is a value, a missing `value` is not.
    #[serde(default, deserialize_with = "present")]
    value: Option<Yaml>,
}

/// **Private.** A `value` that is in the body, even when it is `null`.
fn present<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Yaml>, D::Error> {
    Yaml::deserialize(d).map(Some)
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Kind {
    Set,
    Insert,
    Remove,
}

/// Applies `ops` to `text` in order.
///
/// # Errors
/// A message for a bad operation (a path that is not keys and indexes, a missing value, an
/// `insert` without a key) or an edit that cannot be made (nothing at the path, a key that is
/// already there, text that is not YAML). The text is then unchanged.
pub fn apply(text: &str, ops: &[Op]) -> Result<Spliced, String> {
    let edits = ops.iter().map(edit).collect::<Result<Vec<_>, _>>()?;
    splice(text, &edits).map_err(|e| e.to_string())
}

/// **Private.** One operation as a splice edit.
fn edit(o: &Op) -> Result<Edit, String> {
    let mut path = o
        .path
        .iter()
        .map(|s| match s {
            Value::String(k) => Ok(PathSeg::Key(k.clone())),
            Value::Number(n) => n
                .as_u64()
                .and_then(|i| usize::try_from(i).ok())
                .map(PathSeg::Index)
                .ok_or_else(|| format!("path step {n} is not a list index")),
            _ => Err("a path is made of keys (text) and list indexes (numbers)".to_string()),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let value = || {
        o.value
            .as_ref()
            .map(render)
            .ok_or_else(|| format!("{:?} needs a value", o.kind))
    };
    Ok(match o.kind {
        Kind::Set => Edit::Set {
            value: value()?,
            path,
        },
        Kind::Remove => Edit::Remove { path },
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

/// **Private.** A value as YAML in the project's flow style: bare words where they are safe,
/// JSON quoting (valid YAML) otherwise, `{ a: 1, b: x }` and `[a, b]` for collections. The value
/// is read as YAML (not JSON) so an object keeps the order the client sent: `{ type: cache, ... }`.
fn render(v: &Yaml) -> String {
    let scalar = |s: &str| {
        if plain(s) {
            s.to_string()
        } else {
            Value::from(s).to_string()
        }
    };
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

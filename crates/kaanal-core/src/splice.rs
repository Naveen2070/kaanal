//! Edits to a config text that change only the bytes they touch.
//!
//! The Studio draft is the text of a `kaanal.yaml`, and the UI changes it with path-plus-value
//! edits. Re-serializing a parsed document would drop the comments, the order of keys and the
//! flow style (`{ from: a, to: b }`) the project uses, so an edit is instead one replacement of
//! a byte range found from the parser's spans, and every other byte stays as it was (BOM and
//! line endings included). Edits apply one after the other, each on the result of the last.
//!
//! What cannot be edited safely is not edited and not guessed: a value with an anchor or that is
//! an alias (changing it would change its copies), or a shape this module does not handle, is
//! left alone and listed in [`Spliced::lost`]. Comments an edit orphans are listed there too.

use std::fmt::Write as _;

use saphyr_parser::{Event, Parser, ScalarStyle};

mod append;
use append::append;

use crate::diagnostics::PathSeg;

/// One edit of the text, by path into the document. A `value` is already-rendered YAML text, in
/// the project's flow style for anything nested (`{ from: a, to: b }`).
#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    /// Replaces the value at `path`.
    Set {
        /// Where.
        path: Vec<PathSeg>,
        /// The new value text.
        value: String,
    },
    /// Adds `key: value` to the mapping at `parent` (the root for an empty path).
    Insert {
        /// The mapping.
        parent: Vec<PathSeg>,
        /// The new key (plain text).
        key: String,
        /// The value text.
        value: String,
    },
    /// Adds `value` as the last item of the list at `parent`, in the list's own style.
    Append {
        /// The list.
        parent: Vec<PathSeg>,
        /// The item text.
        value: String,
    },
    /// Removes the mapping entry or list item at `path`.
    Remove {
        /// Where.
        path: Vec<PathSeg>,
    },
    /// Renames the mapping key at `path` (its last step) to `to`; the value is not touched.
    RenameKey {
        /// The entry whose key changes.
        path: Vec<PathSeg>,
        /// The new key (plain text).
        to: String,
    },
}

/// The text after the edits, and what could not be kept.
#[derive(Debug, Clone, PartialEq)]
pub struct Spliced {
    /// The edited text.
    pub text: String,
    /// One line for each edit that was skipped and each comment that an edit left behind.
    pub lost: Vec<String>,
}

/// Why an edit list could not be applied at all (the text is not changed).
#[derive(Debug, Clone, PartialEq)]
pub enum SpliceError {
    /// The text is not YAML this module reads (or has a complex key); carries the parser's message.
    Parse(String),
    /// No value at this path (dotted for display).
    NotFound(String),
    /// A key to insert is already in the mapping.
    Exists(String),
}

impl std::fmt::Display for SpliceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpliceError::Parse(m) => write!(f, "cannot read the text: {m}"),
            SpliceError::NotFound(p) => write!(f, "nothing at {p}"),
            SpliceError::Exists(p) => write!(f, "{p} is already there"),
        }
    }
}

impl std::error::Error for SpliceError {}

/// Applies `edits` to `src` in order.
///
/// # Errors
/// The text does not parse, a path is missing, or an inserted key exists; nothing is changed.
pub fn splice(src: &str, edits: &[Edit]) -> Result<Spliced, SpliceError> {
    let bom = if src.starts_with('\u{feff}') {
        "\u{feff}"
    } else {
        ""
    };
    let mut text = src[bom.len()..].to_string();
    let mut lost = Vec::new();
    for edit in edits {
        let root = parse(&text)?;
        let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
        match apply(&text, root.as_ref(), edit, nl)? {
            Outcome::Replace(range, with, why) => {
                lost.extend(why);
                if let Edit::Remove { path } = edit {
                    lost.extend(orphans(&text, root.as_ref(), path));
                }
                text.replace_range(range, &with);
            }
            Outcome::Skip(why) => lost.push(why),
        }
    }
    Ok(Spliced {
        text: format!("{bom}{text}"),
        lost,
    })
}

/// The scalar leaves of `src`, each with its path and its text exactly as written (quotes
/// included): what a "set every leaf to itself" round trip uses, and what a UI lists.
///
/// # Errors
/// The text does not parse.
pub fn scalar_paths(src: &str) -> Result<Vec<(Vec<PathSeg>, String)>, SpliceError> {
    let body = src.strip_prefix('\u{feff}').unwrap_or(src);
    let mut out = Vec::new();
    if let Some(root) = parse(body)? {
        leaves(&root, &mut Vec::new(), body, &mut out);
    }
    Ok(out)
}

// ---- the span tree -------------------------------------------------------------------------

/// **Private.** One value of the document with its byte range.
struct Node {
    kind: Kind,
    /// Where the value starts. For a block collection, where its first entry starts.
    start: usize,
    /// Where its content ends: the end of a scalar (its last character, never the line break
    /// after a block scalar), of a flow collection's closing bracket, or of the last thing inside
    /// a block collection.
    end: usize,
    anchored: bool,
    /// A scalar's value as the parser reads it (quotes and escapes resolved); empty otherwise.
    value: String,
    /// A `|` or `>` scalar. Its span is its text lines only (the header is before it), so an
    /// edit replaces everything after the key's colon, as for a block collection.
    block_scalar: bool,
}

enum Kind {
    Scalar,
    Alias,
    Map { flow: bool, entries: Vec<Entry> },
    Seq { flow: bool, items: Vec<Node> },
}

struct Entry {
    key: String,
    key_start: usize,
    key_end: usize,
    value: Node,
}

/// **Private.** A collection being built from events.
enum Open {
    Map {
        flow: bool,
        start: usize,
        anchored: bool,
        entries: Vec<Entry>,
        key: Option<(String, usize, usize)>,
    },
    Seq {
        flow: bool,
        start: usize,
        anchored: bool,
        items: Vec<Node>,
    },
}

/// **Private.** Hands a finished value to the collection on top of `stack` (or makes it the
/// root). In a mapping a value with no pending key *is* the key, which must be a scalar.
fn attach(
    stack: &mut [Open],
    root: &mut Option<Node>,
    mut node: Node,
    text: &str,
) -> Result<(), SpliceError> {
    match stack.last_mut() {
        None => *root = Some(node),
        Some(Open::Seq { items, .. }) => items.push(node),
        Some(Open::Map { entries, key, .. }) => {
            if let Some((k, key_start, key_end)) = key.take() {
                if matches!(node.kind, Kind::Scalar) && node.start == node.end {
                    // an empty value: the parser puts it at the next token, which may be on the
                    // next line; it belongs right after the colon (when that is the next thing)
                    if let Some(i) = text[key_end..].find(':')
                        && text[key_end..key_end + i].trim().is_empty()
                    {
                        node.start = key_end + i + 1;
                        node.end = node.start;
                    }
                }
                entries.push(Entry {
                    key: k,
                    key_start,
                    key_end,
                    value: node,
                });
            } else {
                if !matches!(node.kind, Kind::Scalar) {
                    return Err(SpliceError::Parse(
                        "a mapping key that is not a scalar".into(),
                    ));
                }
                if entries.iter().any(|e| e.key == node.value) {
                    return Err(SpliceError::Parse(format!(
                        "duplicate key '{}'",
                        node.value
                    )));
                }
                *key = Some((node.value, node.start, node.end));
            }
        }
    }
    Ok(())
}

/// **Private.** A scalar value's node. A block scalar's span runs into the next line (its line
/// break and that line's indent): it ends at its text.
fn scalar_node(
    text: &str,
    val: &str,
    style: ScalarStyle,
    anchored: bool,
    s: usize,
    e: usize,
) -> Node {
    let block_scalar = matches!(style, ScalarStyle::Literal | ScalarStyle::Folded);
    let mut e = e;
    while block_scalar && e > s && text[..e].ends_with(['\n', '\r', ' ', '\t']) {
        e -= 1;
    }
    Node {
        kind: Kind::Scalar,
        start: s,
        end: e,
        anchored,
        value: val.to_string(),
        block_scalar,
    }
}

/// The deepest collection nesting `parse` builds. The tree is walked and dropped recursively, and
/// the parser itself allows unlimited block nesting, so without a bound a draft of `- - - - ...`
/// would overflow a worker's stack. (`serde_yaml_ng` stops at 128 for Validate and Apply.)
const MAX_DEPTH: usize = 64;

/// **Private.** Refuses one more level of nesting past [`MAX_DEPTH`].
fn deep_enough(stack: &[Open]) -> Result<(), SpliceError> {
    if stack.len() >= MAX_DEPTH {
        return Err(SpliceError::Parse(format!(
            "nested deeper than {MAX_DEPTH} levels"
        )));
    }
    Ok(())
}

/// **Private.** The first document of `text` (no BOM) as a span tree; `None` when it is empty.
fn parse(text: &str) -> Result<Option<Node>, SpliceError> {
    // saphyr counts characters; the rest of this module works in bytes
    let mut at: Vec<usize> = text.char_indices().map(|(b, _)| b).collect();
    at.push(text.len());
    let byte = |c: usize| at[c.min(at.len() - 1)];
    let (mut stack, mut root): (Vec<Open>, Option<Node>) = (Vec::new(), None);
    for ev in Parser::new_from_str(text) {
        let (ev, span) = ev.map_err(|e| SpliceError::Parse(e.to_string()))?;
        let (s, e) = (byte(span.start.index()), byte(span.end.index()));
        match ev {
            Event::Scalar(val, style, anchor, _) => {
                let node = scalar_node(text, &val, style, anchor != 0, s, e);
                attach(&mut stack, &mut root, node, text)?;
            }
            Event::Alias(_) => {
                let node = Node {
                    kind: Kind::Alias,
                    start: s,
                    end: e,
                    anchored: false,
                    value: String::new(),
                    block_scalar: false,
                };
                attach(&mut stack, &mut root, node, text)?;
            }
            Event::MappingStart(anchor, _) => {
                deep_enough(&stack)?;
                stack.push(Open::Map {
                    flow: e > s,
                    start: s,
                    anchored: anchor != 0,
                    entries: Vec::new(),
                    key: None,
                });
            }
            Event::SequenceStart(anchor, _) => {
                deep_enough(&stack)?;
                stack.push(Open::Seq {
                    flow: e > s,
                    start: s,
                    anchored: anchor != 0,
                    items: Vec::new(),
                });
            }
            Event::MappingEnd => {
                let Some(Open::Map {
                    flow,
                    start,
                    anchored,
                    entries,
                    ..
                }) = stack.pop()
                else {
                    return Err(SpliceError::Parse("unbalanced mapping".into()));
                };
                let end = if flow {
                    flow_end(text, e)
                } else {
                    entries.last().map_or(start, |x| x.value.end)
                };
                let node = Node {
                    kind: Kind::Map { flow, entries },
                    start,
                    end,
                    anchored,
                    value: String::new(),
                    block_scalar: false,
                };
                attach(&mut stack, &mut root, node, text)?;
            }
            Event::SequenceEnd => {
                let Some(Open::Seq {
                    flow,
                    start,
                    anchored,
                    items,
                }) = stack.pop()
                else {
                    return Err(SpliceError::Parse("unbalanced sequence".into()));
                };
                let end = if flow {
                    flow_end(text, e)
                } else {
                    items.last().map_or(start, |x| x.end)
                };
                let node = Node {
                    kind: Kind::Seq { flow, items },
                    start,
                    end,
                    anchored,
                    value: String::new(),
                    block_scalar: false,
                };
                attach(&mut stack, &mut root, node, text)?;
            }
            Event::DocumentEnd => break,
            _ => {}
        }
    }
    // a document that is only comments (or nothing) parses to an empty scalar: no root
    Ok(root.filter(|n| !(matches!(n.kind, Kind::Scalar) && n.start == n.end)))
}

/// **Private.** Where a flow collection ends: just after its closing bracket. The parser's span
/// can run on over the spaces behind it, and an insert after the collection would land past them.
fn flow_end(text: &str, e: usize) -> usize {
    text[..e].trim_end().len()
}

/// **Private.** `path` of the node, dotted for messages.
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
    if s.is_empty() {
        "the document root".into()
    } else {
        s
    }
}

/// **Private.** Where a located node hangs: its container and its place in it.
enum Slot<'a> {
    Root,
    Entry(&'a Node, usize),
    Item(&'a Node, usize),
}

/// **Private.** The node at `path`, its slot, and whether it sits inside an anchored value (editing
/// it would change every alias of that value).
fn locate<'a>(
    root: Option<&'a Node>,
    path: &[PathSeg],
) -> Result<(&'a Node, Slot<'a>, bool), SpliceError> {
    let missing = || SpliceError::NotFound(dotted(path));
    let mut node = root.ok_or_else(missing)?;
    let mut slot = Slot::Root;
    let mut inside = false;
    for seg in path {
        let parent = node;
        inside |= parent.anchored;
        match (seg, &parent.kind) {
            (PathSeg::Key(k), Kind::Map { entries, .. }) => {
                let i = entries
                    .iter()
                    .position(|e| &e.key == k)
                    .ok_or_else(missing)?;
                (node, slot) = (&entries[i].value, Slot::Entry(parent, i));
            }
            (PathSeg::Index(i), Kind::Seq { items, .. }) => {
                (node, slot) = (items.get(*i).ok_or_else(missing)?, Slot::Item(parent, *i));
            }
            _ => return Err(missing()),
        }
    }
    Ok((node, slot, inside))
}

fn leaves(n: &Node, path: &mut Vec<PathSeg>, text: &str, out: &mut Vec<(Vec<PathSeg>, String)>) {
    match &n.kind {
        Kind::Scalar if n.start < n.end && !n.block_scalar => {
            out.push((path.clone(), text[n.start..n.end].to_string()));
        }
        Kind::Map { entries, .. } => {
            for e in entries {
                path.push(PathSeg::Key(e.key.clone()));
                leaves(&e.value, path, text, out);
                path.pop();
            }
        }
        Kind::Seq { items, .. } => {
            for (i, it) in items.iter().enumerate() {
                path.push(PathSeg::Index(i));
                leaves(it, path, text, out);
                path.pop();
            }
        }
        _ => {}
    }
}

// ---- the edits -----------------------------------------------------------------------------

/// **Private.** What one edit does to the text.
enum Outcome {
    /// Replace the range with the text; the list is what the replacement could not keep.
    Replace(std::ops::Range<usize>, String, Vec<String>),
    Skip(String),
}

fn line_start(t: &str, pos: usize) -> usize {
    t[..pos].rfind('\n').map_or(0, |i| i + 1)
}

/// The end of the line holding `pos`, before its line ending.
fn line_end(t: &str, pos: usize) -> usize {
    let nl = t[pos..].find('\n').map_or(t.len(), |i| pos + i);
    if t[..nl].ends_with('\r') { nl - 1 } else { nl }
}

/// The start of the line after the one holding `pos` (the text length on the last line).
fn next_line(t: &str, pos: usize) -> usize {
    t[pos..].find('\n').map_or(t.len(), |i| pos + i + 1)
}

/// Where a value that follows the key of `entry` begins to be written: just after its colon.
fn after_colon(t: &str, e: &Entry) -> usize {
    t[e.key_end..]
        .find(':')
        .map_or(e.key_end, |i| e.key_end + i + 1)
}

/// **Private.** Why an edit inside or on an anchored value, or on an alias, is left alone.
const SHARED: &str = "an anchored or aliased value, or inside one: edit it in the file";

/// **Private.** A new mapping key as it is written: bare when that reads back as the same text,
/// double-quoted (JSON escapes, valid YAML) otherwise, so a key can never carry syntax.
fn key_text(key: &str) -> String {
    let mut c = key.chars();
    let bare = c
        .next()
        .is_some_and(|f| f.is_ascii_alphabetic() || f == '_')
        && c.all(|x| x.is_ascii_alphanumeric() || matches!(x, '_' | '-'))
        && !matches!(
            key.to_ascii_lowercase().as_str(),
            "true" | "false" | "null" | "yes" | "no" | "on" | "off" | "y" | "n"
        );
    if bare {
        key.to_string()
    } else {
        serde_json::Value::from(key).to_string()
    }
}

/// **Private.** An edit that is left alone, with the line that says why.
#[expect(
    clippy::unnecessary_wraps,
    reason = "callers `return skip(..)` as their own result"
)]
fn skip(path: &[PathSeg], why: &str) -> Result<Outcome, SpliceError> {
    Ok(Outcome::Skip(format!("{}: {why}", dotted(path))))
}

fn apply(t: &str, root: Option<&Node>, edit: &Edit, nl: &str) -> Result<Outcome, SpliceError> {
    match edit {
        Edit::Set { path, value } => set(t, root, path, value),
        Edit::Insert { parent, key, value } => insert(t, root, parent, key, value, nl),
        Edit::Append { parent, value } => append(t, root, parent, value, nl),
        Edit::Remove { path } => remove(t, root, path),
        Edit::RenameKey { path, to } => rename_key(root, path, to),
    }
}

fn set(
    t: &str,
    root: Option<&Node>,
    path: &[PathSeg],
    value: &str,
) -> Result<Outcome, SpliceError> {
    let (n, slot, inside) = locate(root, path)?;
    if n.anchored || inside || matches!(n.kind, Kind::Alias) {
        return skip(path, SHARED);
    }
    let block = n.block_scalar
        || matches!(
            &n.kind,
            Kind::Map { flow: false, .. } | Kind::Seq { flow: false, .. }
        );
    let entry = match &slot {
        Slot::Entry(parent, i) => match &parent.kind {
            Kind::Map { entries, .. } => Some(&entries[*i]),
            _ => None,
        },
        _ => None,
    };
    if block || (matches!(n.kind, Kind::Scalar) && n.start == n.end) {
        // a block collection or an empty value: the new text goes right after the key's colon
        let Some(e) = entry else {
            return skip(
                path,
                if matches!(slot, Slot::Root) {
                    "the whole document: edit it in the file"
                } else {
                    "a block value inside a list: edit it in the file"
                },
            );
        };
        let from = after_colon(t, e);
        let to = if block { n.end } else { from };
        // a comment between `key:` and a block value goes with the value it was written for
        let lost = if t[from..n.start.max(from)].contains('#') {
            vec![format!(
                "{}: a comment before the old value is gone",
                dotted(path)
            )]
        } else {
            Vec::new()
        };
        return Ok(Outcome::Replace(from..to, format!(" {value}"), lost));
    }
    Ok(Outcome::Replace(
        n.start..n.end,
        value.to_string(),
        Vec::new(),
    ))
}

fn insert(
    t: &str,
    root: Option<&Node>,
    parent: &[PathSeg],
    key: &str,
    value: &str,
    nl: &str,
) -> Result<Outcome, SpliceError> {
    // an empty document gets its first key at the end
    if root.is_none() && parent.is_empty() {
        let lead = if t.is_empty() || t.ends_with('\n') {
            ""
        } else {
            nl
        };
        let at = t.len();
        return Ok(Outcome::Replace(
            at..at,
            format!("{lead}{}: {value}{nl}", key_text(key)),
            Vec::new(),
        ));
    }
    let (n, _, inside) = locate(root, parent)?;
    if n.anchored || inside {
        return skip(parent, SHARED);
    }
    let Kind::Map { flow, entries } = &n.kind else {
        return skip(
            parent,
            "not a mapping (an empty or list value): edit it in the file",
        );
    };
    if entries.iter().any(|e| e.key == key) {
        let mut p = parent.to_vec();
        p.push(PathSeg::Key(key.to_string()));
        return Err(SpliceError::Exists(dotted(&p)));
    }
    if *flow {
        return Ok(match entries.last() {
            Some(last) => {
                let at = last.value.end;
                Outcome::Replace(at..at, format!(", {}: {value}", key_text(key)), Vec::new())
            }
            None => Outcome::Replace(
                n.start..n.end,
                format!("{{ {}: {value} }}", key_text(key)),
                Vec::new(),
            ),
        });
    }
    let Some(last) = entries.last() else {
        return skip(parent, "an empty mapping: edit it in the file");
    };
    let first = &entries[0];
    let indent = " ".repeat(
        t[line_start(t, first.key_start)..first.key_start]
            .chars()
            .count(),
    );
    let at = line_end(t, last.value.end);
    Ok(Outcome::Replace(
        at..at,
        format!("{nl}{indent}{}: {value}", key_text(key)),
        Vec::new(),
    ))
}

fn rename_key(root: Option<&Node>, path: &[PathSeg], to: &str) -> Result<Outcome, SpliceError> {
    let (_, slot, inside) = locate(root, path)?;
    let Slot::Entry(parent, i) = slot else {
        return skip(path, "not a mapping entry: nothing to rename");
    };
    if inside {
        return skip(path, SHARED);
    }
    let Kind::Map { entries, .. } = &parent.kind else {
        return skip(path, "not a mapping entry: nothing to rename");
    };
    if entries.iter().any(|e| e.key == to) {
        let mut p = path[..path.len() - 1].to_vec();
        p.push(PathSeg::Key(to.to_string()));
        return Err(SpliceError::Exists(dotted(&p)));
    }
    let e = &entries[i];
    Ok(Outcome::Replace(
        e.key_start..e.key_end,
        key_text(to),
        Vec::new(),
    ))
}

fn remove(t: &str, root: Option<&Node>, path: &[PathSeg]) -> Result<Outcome, SpliceError> {
    let (n, slot, inside) = locate(root, path)?;
    if n.anchored || inside || matches!(n.kind, Kind::Alias) {
        return skip(path, SHARED);
    }
    let (parent, idx, starts, is_entry) = match slot {
        Slot::Root => return skip(path, "the document root cannot be removed"),
        Slot::Entry(p, i) => match &p.kind {
            Kind::Map { entries, .. } => (
                p,
                i,
                entries
                    .iter()
                    .map(|e| (e.key_start, e.value.end))
                    .collect::<Vec<_>>(),
                true,
            ),
            _ => return skip(path, "unsupported container"),
        },
        Slot::Item(p, i) => match &p.kind {
            Kind::Seq { items, .. } => (
                p,
                i,
                items.iter().map(|x| (x.start, x.end)).collect::<Vec<_>>(),
                false,
            ),
            _ => return skip(path, "unsupported container"),
        },
    };
    let flow = matches!(
        &parent.kind,
        Kind::Map { flow: true, .. } | Kind::Seq { flow: true, .. }
    );
    let (start, end) = starts[idx];
    if flow {
        let range = if starts.len() == 1 {
            // the only entry: leave the brackets, nothing between them
            parent.start + 1..parent.end - 1
        } else if idx + 1 < starts.len() {
            start..starts[idx + 1].0
        } else {
            starts[idx - 1].1..end
        };
        return Ok(Outcome::Replace(range, String::new(), Vec::new()));
    }
    // block: the whole lines of the entry. The first thing on its line must be the entry itself
    // (not a `- ` that belongs to a list item holding more).
    let ls = line_start(t, start);
    let before = t[ls..start].trim();
    if !(before.is_empty() || (!is_entry && before == "-")) {
        return skip(
            path,
            "shares its line with a list marker: edit it in the file",
        );
    }
    let lost = if starts.len() == 1 {
        vec![format!(
            "{}: was the only entry, so what held it is now empty (null)",
            dotted(path)
        )]
    } else {
        Vec::new()
    };
    Ok(Outcome::Replace(ls..next_line(t, end), String::new(), lost))
}

/// **Private.** The comment lines directly above the entry or item at `path` in `before` (the
/// text before the removal), which a removal leaves without the thing they described.
fn orphans(before: &str, root: Option<&Node>, path: &[PathSeg]) -> Vec<String> {
    let Ok((n, slot, _)) = locate(root, path) else {
        return Vec::new();
    };
    let start = match slot {
        Slot::Entry(p, i) => match &p.kind {
            Kind::Map { entries, .. } => entries[i].key_start,
            _ => return Vec::new(),
        },
        _ => n.start,
    };
    let mut found = Vec::new();
    let mut at = line_start(before, start);
    while at > 0 {
        let prev = line_start(before, at - 1);
        let line = before[prev..at].trim();
        if !line.starts_with('#') {
            break;
        }
        found.push(format!(
            "{}: the comment `{line}` is left without its entry",
            dotted(path)
        ));
        at = prev;
    }
    found.reverse();
    found
}

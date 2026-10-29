//! A bound on YAML alias expansion, checked before any parser expands anything.
//!
//! A "billion laughs" file (each anchor lists the previous one ten times) is a few KiB of text
//! that expands to billions of nodes; both the typed parse and the diagnostics pass would work on
//! all of them. The check reads the YAML as events (which do not expand aliases) and adds up what
//! each `*alias` would add. It is exact for anything the parser reads; text it cannot read is left
//! for the real parser to report. Every document of a multi-document text is counted (serde stops
//! at the second, but the diagnostics pass would still load it whole).

use std::collections::HashMap;

use saphyr_parser::{Event, Parser};

use super::ConfigError;

/// The most nodes aliases may add to a file in all. A real config uses a few small anchors (a
/// shared `ops:` block); a file past this is a bomb, whatever its size.
pub(super) const MAX_ALIAS_NODES: usize = 50_000;

/// **Private.** `Err` when the aliases in `src` would add more than [`MAX_ALIAS_NODES`] nodes.
pub(super) fn check(src: &str) -> Result<(), ConfigError> {
    // open collections: (anchor id, nodes inside so far); scalars and aliases add to the top one
    let mut open: Vec<(usize, usize)> = vec![(0, 0)];
    let mut sizes: HashMap<usize, usize> = HashMap::new();
    let mut added = 0usize;
    for ev in Parser::new_from_str(src) {
        let Ok((ev, span)) = ev else { return Ok(()) };
        match ev {
            Event::Scalar(_, _, anchor, _) => {
                if anchor != 0 {
                    sizes.insert(anchor, 1);
                }
                if let Some(t) = open.last_mut() {
                    t.1 = t.1.saturating_add(1);
                }
            }
            Event::Alias(id) => {
                let n = sizes.get(&id).copied().unwrap_or(1);
                added = added.saturating_add(n);
                if added > MAX_ALIAS_NODES {
                    return Err(ConfigError {
                        line: Some(span.start.line()),
                        col: Some(span.start.col() + 1),
                        msg: format!(
                            "too much alias expansion: the `*alias` references would add more than {MAX_ALIAS_NODES} nodes to the file"
                        ),
                    });
                }
                if let Some(t) = open.last_mut() {
                    t.1 = t.1.saturating_add(n);
                }
            }
            Event::MappingStart(anchor, _) | Event::SequenceStart(anchor, _) => {
                open.push((anchor, 1));
            }
            Event::MappingEnd | Event::SequenceEnd if open.len() > 1 => {
                let (anchor, n) = open.pop().unwrap_or((0, 0));
                if anchor != 0 {
                    sizes.insert(anchor, n);
                }
                if let Some(t) = open.last_mut() {
                    t.1 = t.1.saturating_add(n);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

//! Canvas notes: short captions Studio pins on the canvas, kept in the layout sidecar
//! ([`super::layout`]) beside the positions. They are not part of the design: not in the config,
//! not in undo, not renamed with a node, and they never change the draft's `rev`.

use super::layout::MAX_COORD;
use serde::{Deserialize, Serialize};

/// The most notes a project holds.
pub const MAX_NOTES: usize = 50;
/// The longest note text, in characters.
pub const MAX_NOTE_CHARS: usize = 500;
/// The longest note id, in bytes.
pub const MAX_ID_BYTES: usize = 64;
/// What every note id starts with (a component's id never holds a raw colon).
pub const ID_PREFIX: &str = "note:";

/// One note: its id, its text, and its top-left corner in canvas coordinates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Note {
    pub id: String,
    pub text: String,
    pub at: [f64; 2],
}

/// The notes of a project, in the order they were made.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Notes(pub Vec<Note>);

impl Notes {
    /// Whether there are none (the sidecar then leaves the key out).
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether these may be kept: at most [`MAX_NOTES`], each id starting [`ID_PREFIX`], at most
    /// [`MAX_ID_BYTES`] long and unique, each text at most [`MAX_NOTE_CHARS`] characters, each
    /// coordinate finite and within ±[`MAX_COORD`].
    ///
    /// # Errors
    /// What is wrong, in words (never the note's own text).
    pub fn check(&self) -> Result<(), String> {
        if self.0.len() > MAX_NOTES {
            return Err(format!("notes: {} notes (limit {MAX_NOTES})", self.0.len()));
        }
        let mut seen = std::collections::HashSet::new();
        for n in &self.0 {
            if !n.id.starts_with(ID_PREFIX) || n.id.len() > MAX_ID_BYTES {
                return Err(format!(
                    "notes: an id must start with '{ID_PREFIX}' and be at most {MAX_ID_BYTES} bytes"
                ));
            }
            if !seen.insert(n.id.as_str()) {
                return Err(format!("notes: the id '{}' is used twice", n.id));
            }
            if n.text.chars().count() > MAX_NOTE_CHARS {
                return Err(format!(
                    "notes: '{}' is over {MAX_NOTE_CHARS} characters",
                    n.id
                ));
            }
            if n.at.iter().any(|c| !c.is_finite() || c.abs() > MAX_COORD) {
                return Err(format!(
                    "notes: the position of '{}' must be finite and within ±{MAX_COORD}",
                    n.id
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: &str) -> Note {
        Note {
            id: id.into(),
            text: "hi".into(),
            at: [1.0, 2.0],
        }
    }

    fn refused(notes: Vec<Note>) -> String {
        Notes(notes).check().unwrap_err()
    }

    #[test]
    fn a_normal_list_is_kept() {
        assert!(Notes(vec![note("note:1"), note("note:2")]).check().is_ok());
        assert!(Notes::default().check().is_ok());
    }

    #[test]
    fn check_refuses_51_notes_and_keeps_50() {
        let many = |n: usize| Notes((0..n).map(|i| note(&format!("note:{i}"))).collect());
        assert!(many(50).check().is_ok());
        assert!(many(51).check().unwrap_err().contains("51 notes"));
    }

    #[test]
    fn check_refuses_a_duplicate_id() {
        assert!(refused(vec![note("note:1"), note("note:1")]).contains("twice"));
    }

    #[test]
    fn check_refuses_a_bad_prefix_and_a_long_id() {
        assert!(refused(vec![note("app")]).contains("must start with"));
        let long = format!("note:{}", "x".repeat(MAX_ID_BYTES));
        assert!(refused(vec![note(&long)]).contains("at most"));
    }

    #[test]
    fn check_refuses_a_nonfinite_or_huge_position() {
        for at in [
            [f64::NAN, 0.0],
            [0.0, f64::INFINITY],
            [1e7, 0.0],
            [0.0, -1e7],
        ] {
            let n = Note {
                at,
                ..note("note:1")
            };
            assert!(refused(vec![n]).contains("finite"), "{at:?}");
        }
        let edge = Note {
            at: [1e6, -1e6],
            ..note("note:1")
        };
        assert!(Notes(vec![edge]).check().is_ok());
    }

    #[test]
    fn the_text_limit_counts_characters_not_bytes() {
        let ok = Note {
            text: "é".repeat(MAX_NOTE_CHARS),
            ..note("note:1")
        };
        assert!(Notes(vec![ok]).check().is_ok());
        let over = Note {
            text: "a".repeat(MAX_NOTE_CHARS + 1),
            ..note("note:1")
        };
        assert!(refused(vec![over]).contains("characters"));
    }
}

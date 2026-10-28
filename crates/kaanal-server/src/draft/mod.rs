//! The Studio draft: the text of the `kaanal.yaml` being edited, apart from the system that
//! runs. Replacing the draft touches neither the running system nor the disk; Apply and Save
//! (later tasks) are what do. The draft is canonical YAML text, so comments and order survive.

pub(crate) mod apply;
pub(crate) mod gate;
pub(crate) mod remap;

/// The longest draft text accepted: 1 MiB. A `kaanal.yaml` is a few KiB; this bounds memory and
/// the work of validating a pasted file.
pub const MAX_DRAFT_BYTES: usize = 1024 * 1024;

/// The largest request body the draft routes read: a text of [`MAX_DRAFT_BYTES`] whose every
/// byte is a control character is six bytes of JSON (`\u000a`), so the body limit is looser than
/// the text limit; the text limit is what answers a long draft with its own message.
pub const MAX_DRAFT_BODY: usize = MAX_DRAFT_BYTES * 6 + 4096;

/// What a draft is started from: the config file and its text, as `kaanal up` read them.
pub struct DraftSource {
    /// The config file the text came from.
    pub file: std::path::PathBuf,
    /// Its text, byte for byte (comments, order and line endings included).
    pub text: String,
}

/// The draft in force. Behind [`crate::state::AppState::draft`]; last writer wins (single user).
#[derive(Debug, Default, Clone)]
pub struct Draft {
    /// The text being edited.
    pub text: String,
    /// The text of the generation in force (what was last applied): `dirty` is the difference.
    pub applied: String,
}

impl Draft {
    /// A draft that starts as `src` (clean), or empty with no source.
    pub fn from_source(src: Option<DraftSource>) -> Draft {
        let text = src.map(|s| s.text).unwrap_or_default();
        Draft {
            applied: text.clone(),
            text,
        }
    }

    /// Whether the draft differs from what is applied.
    pub fn dirty(&self) -> bool {
        self.text != self.applied
    }
}

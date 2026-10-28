//! The Studio draft: the text of the `kaanal.yaml` being edited, apart from the system that
//! runs. Replacing the draft touches neither the running system nor the disk; Apply and Save
//! (later tasks) are what do. The draft is canonical YAML text, so comments and order survive.

pub(crate) mod apply;
pub(crate) mod gate;
pub(crate) mod patch;
pub(crate) mod remap;
pub(crate) mod rename;
pub(crate) mod save;

/// The longest draft text accepted: 1 MiB. A `kaanal.yaml` is a few KiB; this bounds memory and
/// the work of validating a pasted file.
pub const MAX_DRAFT_BYTES: usize = 1024 * 1024;

/// The largest request body the draft routes read: a text of [`MAX_DRAFT_BYTES`] whose every
/// byte is a control character is six bytes of JSON (`\u000a`), so the body limit is looser than
/// the text limit; the text limit is what answers a long draft with its own message.
pub const MAX_DRAFT_BODY: usize = MAX_DRAFT_BYTES * 6 + 4096;

/// The text a blank start's draft begins with: no nodes, so Apply refuses it until one is added
/// (the topology's own rules about an entry node apply).
pub const BLANK_TEXT: &str = "version: 1\nnodes: {}\n";

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
    /// The config file Save writes (`None` when the server was not started from one).
    pub file: Option<std::path::PathBuf>,
    /// What the file held when it was last read or written by us: Save refuses if the disk differs
    /// (`None`: the file did not exist, so Save refuses if one has appeared).
    pub base: Option<String>,
    /// What patches could not keep (comments, anchors), until the text is replaced whole.
    pub lost: Vec<String>,
    /// The `rename_node` operations made since the text was set, in order: what Save's
    /// `fix_scenarios: rewrite` applies to scenario files that still use the old names.
    pub renames: Vec<(String, String)>,
}

impl Draft {
    /// A draft that starts as `src` (clean), or empty with no source.
    pub fn from_source(src: Option<DraftSource>) -> Draft {
        let Some(s) = src else {
            return Draft::default();
        };
        Draft {
            applied: s.text.clone(),
            base: Some(s.text.clone()),
            text: s.text,
            // absolute, so a later change of the working directory cannot move it
            file: Some(std::path::absolute(&s.file).unwrap_or(s.file)),
            lost: Vec::new(),
            renames: Vec::new(),
        }
    }

    /// The draft of a blank start: [`BLANK_TEXT`], nothing applied (so it is dirty), saved to
    /// `file`. `base` is what `file` holds now (`None` when it does not exist), so a Save
    /// refuses if it changed meanwhile, and replaces an existing file only if nobody touched it.
    pub fn blank(file: &std::path::Path) -> Draft {
        Draft {
            text: BLANK_TEXT.to_string(),
            applied: String::new(),
            file: Some(std::path::absolute(file).unwrap_or_else(|_| file.to_path_buf())),
            base: std::fs::read_to_string(file).ok(),
            lost: Vec::new(),
            renames: Vec::new(),
        }
    }

    /// Whether the draft differs from what is applied.
    pub fn dirty(&self) -> bool {
        self.text != self.applied
    }
}

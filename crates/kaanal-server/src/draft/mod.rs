//! The Studio draft: the text of the `kaanal.yaml` being edited, apart from the system that
//! runs. Replacing the draft touches neither the running system nor the disk; Apply and Save
//! (later tasks) are what do. The draft is canonical YAML text, so comments and order survive.

pub(crate) mod apply;
pub(crate) mod gate;
pub mod layout;
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
pub const BLANK_TEXT: &str = "version: 1\nnodes: {}\nendpoints:\n  \"GET /\":\n    override: { body: { ok: true } }\ntraffic:\n  default:\n    rps: 100\n    duration: 30s\n    journeys:\n      - { endpoint: \"GET /\" }\n";

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
    /// Counts every change of `text` (whole-text PUT and patch), so a simulation run can say which
    /// draft it described. Starts at 0.
    pub rev: u64,
    /// Where Studio draws each node; edits never change `rev` (a drag is not a change to the
    /// system). Read from the sidecar when the draft is made, written by Save.
    pub layout: layout::Layout,
    /// Why the sidecar was not used when it was read (`None`: it was, or there was none).
    pub layout_note: Option<String>,
    /// The layout as last read from or written to the sidecar: `layout_dirty` is the difference.
    pub saved_layout: layout::Layout,
    /// The text parsed as JSON for `GET /draft`, with the `rev` it was parsed at: polling reads
    /// it instead of parsing again. A change of `text` always bumps `rev`, which drops it.
    pub doc_cache: Option<(u64, std::sync::Arc<DocParse>)>,
}

/// A draft text as JSON, or why it is not a config (see `control::doc::draft_doc`).
pub type DocParse = (Option<serde_json::Value>, Option<apply::Diagnostic>);

impl Draft {
    /// A draft that starts as `src` (clean), or empty with no source.
    pub fn from_source(src: Option<DraftSource>) -> Draft {
        let Some(s) = src else {
            return Draft::default();
        };
        // absolute, so a later change of the working directory cannot move it
        let file = std::path::absolute(&s.file).unwrap_or(s.file);
        let (layout, layout_note) = file.parent().map_or_default(layout::load);
        Draft {
            applied: s.text.clone(),
            base: Some(s.text.clone()),
            text: s.text,
            file: Some(file),
            lost: Vec::new(),
            renames: Vec::new(),
            rev: 0,
            saved_layout: layout.clone(),
            layout,
            layout_note,
            doc_cache: None,
        }
    }

    /// The draft of a blank start: [`BLANK_TEXT`], nothing applied (so it is dirty), saved to
    /// `file`. `base` is what `file` holds now (`None` when it does not exist), so a Save
    /// refuses if it changed meanwhile, and replaces an existing file only if nobody touched it.
    pub fn blank(file: &std::path::Path) -> Draft {
        let abs = std::path::absolute(file).unwrap_or_else(|_| file.to_path_buf());
        let (layout, layout_note) = abs.parent().map_or_default(layout::load);
        Draft {
            text: BLANK_TEXT.to_string(),
            applied: String::new(),
            file: Some(abs),
            base: std::fs::read_to_string(file).ok(),
            lost: Vec::new(),
            renames: Vec::new(),
            rev: 0,
            saved_layout: layout.clone(),
            layout,
            layout_note,
            doc_cache: None,
        }
    }

    /// Whether the layout differs from what the sidecar holds (a Save would write it).
    pub fn layout_dirty(&self) -> bool {
        self.layout != self.saved_layout
    }

    /// Whether a Save would change nothing: the draft has a file, its text is what the file held
    /// when it was last read or written by us, and the layout is as saved. False on a blank start
    /// until the first Save (or while the file on disk differs from the blank text).
    pub fn saved(&self) -> bool {
        self.file.is_some()
            && self.base.as_deref() == Some(self.text.as_str())
            && !self.layout_dirty()
    }

    /// Whether the draft differs from what is applied.
    pub fn dirty(&self) -> bool {
        self.text != self.applied
    }
}

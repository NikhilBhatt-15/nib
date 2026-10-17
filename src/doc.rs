//! Documents: the text behind a tab. A document may be backed by a file, an
//! untitled note, or a read-only "virtual" text (an old version or the
//! clipboard shown in a comparison). Its Scintilla document is created on
//! first display so restoring a large session costs nothing up front.

use crate::fileio::{self, Stamp};
use crate::textenc::{self, Enc, Eol};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewState {
    pub anchor: usize,
    pub caret: usize,
    pub first_line: usize,
    pub x_offset: i64,
}

/// Where an unloaded document's text comes from when it is first shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// Already in its Scintilla document.
    Loaded,
    /// Read the file.
    File,
    /// The session's saved buffer (unsaved changes or an untitled note).
    Buffer,
    /// Text supplied directly (templates, AI results, virtual documents).
    Text(Vec<u8>),
    /// Nothing: a new empty note.
    Empty,
}

pub struct Doc {
    pub id: u64,
    pub path: Option<PathBuf>,
    /// "Untitled N" number for notes without a file.
    pub untitled: u32,
    /// Title for virtual documents.
    pub title: Option<String>,
    /// Virtual: read-only, never saved or kept in the session.
    pub virt: bool,
    pub enc: Enc,
    pub eol: Eol,
    pub lang: usize,
    /// Scintilla document pointer; 0 until loaded.
    pub sci: usize,
    pub source: Source,
    /// Differs from the file on disk (or untitled with content).
    pub dirty: bool,
    /// Edited since the session buffer was last written.
    pub snap_needed: bool,
    /// A session buffer file exists for this document.
    pub has_buffer: bool,
    pub stamp: Option<Stamp>,
    pub large: bool,
    pub readonly: bool,
    pub preview: bool,
    pub pinned: bool,
    pub view: [ViewState; 2],
    /// The file changed on disk while we had unsaved edits.
    /// Unsaved even though Scintilla is at its save point (recovered text with no file behind it).
    pub force_dirty: bool,
    pub disk_changed: bool,
    pub missing: bool,
    pub last_edit: u64,
    /// Text length when last shown, for the status bar of unloaded docs.
    pub len_hint: usize,
}

impl Doc {
    pub fn new(id: u64) -> Doc {
        Doc {
            id,
            path: None,
            untitled: 0,
            title: None,
            virt: false,
            enc: Enc::Utf8,
            eol: Eol::Crlf,
            lang: 0,
            sci: 0,
            source: Source::Empty,
            dirty: false,
            snap_needed: false,
            has_buffer: false,
            stamp: None,
            large: false,
            readonly: false,
            preview: false,
            pinned: false,
            view: [ViewState::default(); 2],
            force_dirty: false,
            disk_changed: false,
            missing: false,
            last_edit: 0,
            len_hint: 0,
        }
    }

    pub fn name(&self) -> String {
        if let Some(t) = &self.title {
            return t.clone();
        }
        match &self.path {
            Some(p) => p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string()),
            None => {
                if self.untitled <= 1 {
                    "Untitled".into()
                } else {
                    format!("Untitled {}", self.untitled)
                }
            }
        }
    }

    pub fn dir_hint(&self) -> String {
        self.path.as_ref().and_then(|p| p.parent()).map(|d| d.display().to_string()).unwrap_or_default()
    }

    pub fn is_markdown(&self) -> bool {
        crate::languages::LANGS[self.lang].id == "markdown"
    }
}

/// A file read from disk and converted for editing.
pub struct Loaded {
    pub text: Vec<u8>,
    pub enc: Enc,
    pub eol: Eol,
    pub stamp: Option<Stamp>,
    pub large: bool,
    pub binary: bool,
}

pub const MAX_OPEN_BYTES: u64 = 2 * 1024 * 1024 * 1024 - 1;

pub fn read_file(path: &Path, forced: Option<Enc>, default_eol: Eol, large_mb: u32) -> Result<Loaded, String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.is_dir() {
        return Err("That is a folder.".into());
    }
    if meta.len() > MAX_OPEN_BYTES {
        return Err(format!("The file is {}; Nib opens files up to 2 GB.", crate::util::human_bytes(meta.len())));
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let enc = forced.unwrap_or_else(|| textenc::detect(&bytes));
    let binary = textenc::looks_binary(&bytes, enc);
    let text = textenc::decode(&bytes, enc);
    let eol = textenc::detect_eol(&text, default_eol);
    Ok(Loaded { large: bytes.len() as u64 > large_mb as u64 * 1024 * 1024, text, enc, eol, stamp: fileio::stamp(path), binary })
}

/// Encodes and writes `text`. Returns the new stamp, and whether characters
/// had to be replaced because the encoding can't represent them.
pub fn write_file(path: &Path, text: &[u8], enc: Enc) -> Result<(Option<Stamp>, bool, Vec<u8>), String> {
    let (bytes, lossy) = textenc::encode(text, enc);
    fileio::save_atomic(path, &bytes).map_err(|e| e.to_string())?;
    Ok((fileio::stamp(path), lossy, bytes))
}

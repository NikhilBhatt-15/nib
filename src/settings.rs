//! User settings in settings.json. The file is hand-editable (Preferences ›
//! Open settings file) and re-applied when saved; unknown or invalid values
//! fall back to defaults field by field.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// A theme id from theme::THEMES; "system" follows Windows.
    pub theme: String,
    /// Editor font face; empty picks Cascadia Mono, then Consolas.
    pub font: String,
    pub font_size: f32,
    /// Zoom steps applied on top of font_size (Ctrl+= / Ctrl+-).
    pub zoom: i32,
    pub word_wrap: bool,
    pub line_numbers: bool,
    pub show_whitespace: bool,
    pub highlight_current_line: bool,
    pub tab_width: u32,
    pub insert_spaces: bool,
    pub auto_indent: bool,
    /// Draw a column guide at this column; 0 = off.
    pub ruler: u32,
    /// Saving to the file itself: "off", "delay" (after typing stops) or "focus" (when Nib loses focus).
    /// Unsaved work is always kept in the session either way.
    pub autosave: String,
    pub autosave_delay_ms: u32,
    /// Reopen the previous tabs, including unsaved ones, at start-up.
    pub restore_session: bool,
    /// Ask before closing a tab with unsaved changes (the app itself never asks: the session keeps them).
    pub confirm_close_dirty: bool,
    pub default_encoding: String,
    /// "crlf" or "lf" for new documents.
    pub default_eol: String,
    /// Files bigger than this open in large-file mode (no highlighting, no wrap).
    pub large_file_mb: u32,
    /// Folder for notes; empty means Documents\Nib Notes.
    pub notes_dir: String,
    /// Extra folders indexed by Quick Open and searched by "Search all notes".
    pub search_roots: Vec<String>,
    pub tray_icon: bool,
    /// Closing the window hides it to the tray so the scratchpad shortcut keeps working.
    pub close_to_tray: bool,
    pub scratchpad_hotkey: String,
    pub always_on_top: bool,
    pub history: HistorySettings,
    pub cleanup: Cleanup,
    /// Extra key bindings: chord -> command id ("" removes a default binding).
    pub keys: BTreeMap<String, String>,
    pub ai: AiSettings,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HistorySettings {
    /// Keep a copy of every saved version of a file.
    pub enabled: bool,
    pub max_versions: u32,
    /// Files larger than this are not versioned.
    pub max_file_mb: u32,
}

impl Default for HistorySettings {
    fn default() -> Self {
        Self { enabled: true, max_versions: 50, max_file_mb: 8 }
    }
}

/// What "Paste cleaned" and "Clean up clipboard" do.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Cleanup {
    pub trim_trailing_spaces: bool,
    pub straighten_quotes: bool,
    pub remove_invisible: bool,
    pub collapse_blank_lines: bool,
    pub dedent: bool,
    pub unwrap_lines: bool,
    pub strip_bullets: bool,
}

impl Default for Cleanup {
    fn default() -> Self {
        Self { trim_trailing_spaces: true, straighten_quotes: true, remove_invisible: true, collapse_blank_lines: true, dedent: true, unwrap_lines: false, strip_bullets: false }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiSettings {
    /// Off by default: nothing leaves the machine unless this is turned on.
    pub enabled: bool,
    /// "local" (an OpenAI-compatible server such as Ollama or LM Studio on this PC) or "anthropic" (Claude API).
    pub provider: String,
    /// Local server base URL, e.g. http://localhost:11434/v1
    pub local_endpoint: String,
    pub local_model: String,
    pub anthropic_model: String,
    /// "low" | "medium" | "high" — how much the Claude model reasons before answering.
    pub anthropic_effort: String,
    /// Claude API key, encrypted for the current Windows user with DPAPI (set it from the command palette).
    pub anthropic_key_protected: String,
    /// Text sent to the model is cut off at this many KB.
    pub max_input_kb: u32,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: "local".into(),
            local_endpoint: "http://localhost:11434/v1".into(),
            local_model: "llama3.2".into(),
            anthropic_model: "claude-opus-5-5".into(),
            anthropic_effort: "medium".into(),
            anthropic_key_protected: String::new(),
            max_input_kb: 200,
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            font: String::new(),
            font_size: 11.0,
            zoom: 0,
            word_wrap: false,
            line_numbers: true,
            show_whitespace: false,
            highlight_current_line: true,
            tab_width: 4,
            insert_spaces: true,
            auto_indent: true,
            ruler: 0,
            autosave: "off".into(),
            autosave_delay_ms: 1000,
            restore_session: true,
            confirm_close_dirty: true,
            default_encoding: "utf-8".into(),
            default_eol: "crlf".into(),
            large_file_mb: 50,
            notes_dir: String::new(),
            search_roots: Vec::new(),
            tray_icon: true,
            close_to_tray: false,
            scratchpad_hotkey: "Win+Alt+N".into(),
            always_on_top: false,
            history: HistorySettings::default(),
            cleanup: Cleanup::default(),
            keys: BTreeMap::new(),
            ai: AiSettings::default(),
        }
    }
}

pub const AUTOSAVE_MODES: &[(&str, &str)] = &[("off", "Off"), ("delay", "After a delay"), ("focus", "When focus changes")];

impl Settings {
    pub fn load(path: &Path) -> Self {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let mut s = Self::parse(&text).unwrap_or_default();
        s.sanitize();
        s
    }

    /// Parses settings JSON, tolerating // comments and trailing commas.
    pub fn parse(text: &str) -> Option<Self> {
        if text.trim().is_empty() {
            return None;
        }
        let cleaned = strip_json_comments(text);
        // A bad value in one field shouldn't reset everything: merge field by
        // field over the defaults.
        let value: serde_json::Value = serde_json::from_str(&cleaned).ok()?;
        let mut base = serde_json::to_value(Settings::default()).ok()?;
        merge_valid(&mut base, &value);
        serde_json::from_value(base).ok()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self).unwrap_or_default();
        crate::fileio::save_atomic(path, json.as_bytes())
    }

    pub fn sanitize(&mut self) {
        let d = Settings::default();
        if !crate::theme::THEMES.iter().any(|(id, _)| *id == self.theme) {
            self.theme = d.theme;
        }
        if !AUTOSAVE_MODES.iter().any(|(id, _)| *id == self.autosave) {
            self.autosave = d.autosave;
        }
        if crate::keys::Chord::parse(&self.scratchpad_hotkey).is_none() && !self.scratchpad_hotkey.is_empty() {
            self.scratchpad_hotkey = d.scratchpad_hotkey;
        }
        if crate::textenc::Enc::from_id(&self.default_encoding).is_none() {
            self.default_encoding = d.default_encoding;
        }
        if crate::textenc::Eol::from_label(&self.default_eol).is_none() {
            self.default_eol = d.default_eol;
        }
        if !matches!(self.ai.provider.as_str(), "local" | "anthropic") {
            self.ai.provider = d.ai.provider;
        }
        if !matches!(self.ai.anthropic_effort.as_str(), "low" | "medium" | "high" | "xhigh" | "max") {
            self.ai.anthropic_effort = d.ai.anthropic_effort;
        }
        self.font_size = if self.font_size.is_finite() { self.font_size.clamp(6.0, 48.0) } else { d.font_size };
        self.zoom = self.zoom.clamp(-10, 20);
        self.tab_width = self.tab_width.clamp(1, 16);
        self.ruler = self.ruler.min(400);
        self.autosave_delay_ms = self.autosave_delay_ms.clamp(200, 60_000);
        self.large_file_mb = self.large_file_mb.clamp(1, 4096);
        self.history.max_versions = self.history.max_versions.clamp(1, 1000);
        self.history.max_file_mb = self.history.max_file_mb.clamp(1, 512);
        self.ai.max_input_kb = self.ai.max_input_kb.clamp(1, 4096);
    }

    pub fn encoding(&self) -> crate::textenc::Enc {
        crate::textenc::Enc::from_id(&self.default_encoding).unwrap_or(crate::textenc::Enc::Utf8)
    }

    pub fn eol(&self) -> crate::textenc::Eol {
        crate::textenc::Eol::from_label(&self.default_eol).unwrap_or(crate::textenc::Eol::Crlf)
    }
}

/// Copies each field of `from` into `into` when its type matches the default's,
/// recursing into objects, so a single bad value keeps its default.
fn merge_valid(into: &mut serde_json::Value, from: &serde_json::Value) {
    use serde_json::Value;
    let (Value::Object(dst), Value::Object(src)) = (into, from) else { return };
    for (k, v) in src {
        match dst.get_mut(k) {
            Some(d @ Value::Object(_)) if v.is_object() => {
                // `keys` is a free-form map: take it whole.
                if k == "keys" {
                    *d = v.clone();
                } else {
                    merge_valid(d, v);
                }
            }
            Some(d) => {
                let same = matches!((&*d, v), (Value::Bool(_), Value::Bool(_)) | (Value::Number(_), Value::Number(_)) | (Value::String(_), Value::String(_)) | (Value::Array(_), Value::Array(_)));
                if same {
                    *d = v.clone();
                }
            }
            None => {}
        }
    }
}

/// Removes // and /* */ comments outside strings, and trailing commas.
pub fn strip_json_comments(s: &str) -> String {
    strip_trailing_commas(&strip_comments(s))
}

/// Walks `s` outside string literals, letting `f` decide what to emit at
/// each position; `f` returns how many bytes it consumed.
fn scan_outside_strings(s: &str, mut f: impl FnMut(&[u8], usize, &mut Vec<u8>) -> usize) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    let mut in_str = false;
    while i < b.len() {
        let c = b[i];
        if in_str {
            out.push(c);
            if c == b'\\' && i + 1 < b.len() {
                out.push(b[i + 1]);
                i += 2;
                continue;
            }
            if c == b'"' {
                in_str = false;
            }
            i += 1;
        } else if c == b'"' {
            in_str = true;
            out.push(c);
            i += 1;
        } else {
            i += f(b, i, &mut out).max(1);
        }
    }
    String::from_utf8(out).unwrap_or_default()
}

fn strip_comments(s: &str) -> String {
    scan_outside_strings(s, |b, i, out| {
        if b[i] == b'/' && b.get(i + 1) == Some(&b'/') {
            let mut j = i;
            while j < b.len() && b[j] != b'\n' {
                j += 1;
            }
            j - i
        } else if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
            let mut j = i + 2;
            while j + 1 < b.len() && !(b[j] == b'*' && b[j + 1] == b'/') {
                j += 1;
            }
            (j + 2).min(b.len()) - i
        } else {
            out.push(b[i]);
            1
        }
    })
}

fn strip_trailing_commas(s: &str) -> String {
    scan_outside_strings(s, |b, i, out| {
        if b[i] == b',' {
            let mut j = i + 1;
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < b.len() && (b[j] == b'}' || b[j] == b']') {
                return 1;
            }
        }
        out.push(b[i]);
        1
    })
}

/// Start-with-Windows via the per-user Run key. Nib starts hidden in the tray.
pub mod autostart {
    use std::ptr::null_mut;
    use windows_sys::w;
    use windows_sys::Win32::System::Registry::{RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ};

    const RUN: *const u16 = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
    const NAME: *const u16 = w!("Nib");

    pub fn enabled() -> bool {
        unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN, NAME, RRF_RT_REG_SZ, null_mut(), null_mut(), null_mut()) == 0 }
    }

    pub fn set(on: bool) -> bool {
        unsafe {
            if !on {
                let r = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN, NAME);
                return r == 0 || r == 2;
            }
            let Ok(exe) = std::env::current_exe() else { return false };
            let cmd: Vec<u16> = format!("\"{}\" --tray", exe.display()).encode_utf16().chain(Some(0)).collect();
            RegSetKeyValueW(HKEY_CURRENT_USER, RUN, NAME, REG_SZ, cmd.as_ptr().cast(), (cmd.len() * 2) as u32) == 0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bad_fields_fall_back_individually() {
        let s = Settings::parse(
            r#"{
            // comment
            "theme": "dracula",
            "font_size": "big",
            "word_wrap": true,
            "tab_width": 99,
            "ai": { "enabled": true, "provider": "nope" },
            "keys": { "Ctrl+Alt+D": "edit.duplicateLine" },
        }"#,
        )
        .map(|mut s| {
            s.sanitize();
            s
        })
        .unwrap();
        assert_eq!(s.theme, "dracula");
        assert_eq!(s.font_size, 11.0);
        assert!(s.word_wrap);
        assert_eq!(s.tab_width, 16);
        assert!(s.ai.enabled);
        assert_eq!(s.ai.provider, "local");
        assert_eq!(s.ai.anthropic_model, "claude-opus-5-5");
        assert_eq!(s.keys.get("Ctrl+Alt+D").map(String::as_str), Some("edit.duplicateLine"));
    }

    #[test]
    fn comments_inside_strings_survive() {
        assert_eq!(strip_json_comments(r#"{"a":"http://x//y", /* c */ "b":[1,2,],}"#), r#"{"a":"http://x//y",  "b":[1,2]}"#);
    }

    #[test]
    fn round_trip() {
        let s = Settings { theme: "nord".into(), zoom: 3, ..Default::default() };
        let back = Settings::parse(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }
}

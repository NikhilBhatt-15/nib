//! The notes folder and instant search.
//!
//! A background thread indexes file names under the notes folder and any
//! extra search roots (for Quick Open, like Everything does for the whole
//! disk). "Search all notes" scans open tabs and indexed files on another
//! thread; file contents are cached by modification time so repeat searches
//! are instant, and a newer query cancels an older one.

use crate::app::{self, app, WM_APP_INDEX, WM_APP_NOTES};
use crate::gfx::icon;
use crate::palette::{self, Item, Kind};
use crate::util;
use regex::{Regex, RegexBuilder};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;
use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;

const MAX_FILES: usize = 60_000;
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_CACHE_BYTES: usize = 192 * 1024 * 1024;
const MAX_HITS: usize = 400;

static INDEX: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
static INDEX_GEN: AtomicU64 = AtomicU64::new(0);
static LATEST_SEARCH: AtomicU64 = AtomicU64::new(0);

type Cache = Mutex<(HashMap<PathBuf, (SystemTime, Arc<String>)>, usize)>;
fn cache() -> &'static Cache {
    static C: OnceLock<Cache> = OnceLock::new();
    C.get_or_init(|| Mutex::new((HashMap::new(), 0)))
}

/// Main window handle as an integer so worker threads can post to it.
fn main_hwnd() -> usize {
    app().hwnd.get() as usize
}

pub fn notes_dir() -> PathBuf {
    let s = app::settings().notes_dir.clone();
    if s.trim().is_empty() {
        app().paths.default_notes()
    } else {
        PathBuf::from(s)
    }
}

const SKIP_DIRS: &[&str] = &[".git", "node_modules", "target", ".obsidian", ".trash", "bin", "obj", ".venv", "__pycache__", ".idea", ".vs", "dist", "build"];
const TEXT_EXTS: &[&str] = &[
    "txt", "md", "markdown", "mdx", "log", "json", "jsonc", "yaml", "yml", "toml", "ini", "cfg", "conf", "csv", "tsv", "xml", "html", "htm", "css", "scss", "js", "ts", "jsx", "tsx", "py", "rs", "go", "java", "kt", "c", "h", "cpp", "hpp", "cs", "sql", "sh", "ps1", "bat", "cmd", "lua", "rb", "php", "org", "rst", "tex", "env", "properties", "reg", "diff", "patch", "todo", "note", "nfo",
];

fn is_text_name(p: &Path) -> bool {
    match p.extension().and_then(|e| e.to_str()) {
        Some(e) => TEXT_EXTS.contains(&e.to_ascii_lowercase().as_str()),
        None => true,
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 12 || out.len() >= MAX_FILES {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            if !name.starts_with('.') && !SKIP_DIRS.contains(&name.as_str()) {
                walk(&e.path(), out, depth + 1);
            }
        } else if ft.is_file() && is_text_name(&e.path()) {
            out.push(e.path());
            if out.len() >= MAX_FILES {
                return;
            }
        }
    }
}

pub fn roots() -> Vec<PathBuf> {
    let mut r = vec![notes_dir()];
    for s in &app::settings().search_roots {
        if !s.trim().is_empty() {
            r.push(PathBuf::from(s));
        }
    }
    r
}

/// Rebuilds the file-name index in the background.
pub fn start_index() {
    let roots = roots();
    let _ = std::fs::create_dir_all(&roots[0]);
    let gen = INDEX_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let hwnd = main_hwnd();
    std::thread::spawn(move || {
        let mut files = Vec::new();
        for r in &roots {
            walk(r, &mut files, 0);
        }
        files.sort();
        files.dedup();
        if INDEX_GEN.load(Ordering::SeqCst) == gen {
            *INDEX.lock().unwrap() = files;
            unsafe { PostMessageW(hwnd as _, WM_APP_INDEX, 0, 0) };
        }
    });
}

pub fn indexed_files() -> Vec<PathBuf> {
    INDEX.lock().map(|v| v.clone()).unwrap_or_default()
}

pub fn on_index(_lp: isize) {}

// ------------------------------------------------------------------ content search

struct Source {
    doc: u64,
    path: Option<PathBuf>,
    name: String,
    text: Option<Arc<String>>,
}

fn build_matcher(query: &str) -> Result<Vec<Regex>, String> {
    let q = query.trim();
    if q.len() >= 2 && q.starts_with('/') && q.ends_with('/') {
        return RegexBuilder::new(&q[1..q.len() - 1]).case_insensitive(true).build().map(|r| vec![r]).map_err(|e| e.to_string());
    }
    q.split_whitespace().map(|t| RegexBuilder::new(&regex::escape(t)).case_insensitive(true).build().map_err(|e| e.to_string())).collect()
}

fn read_cached(p: &Path) -> Option<Arc<String>> {
    let meta = std::fs::metadata(p).ok()?;
    if meta.len() > MAX_FILE_BYTES {
        return None;
    }
    let mtime = meta.modified().ok()?;
    if let Some((t, s)) = cache().lock().ok()?.0.get(p) {
        if *t == mtime {
            return Some(s.clone());
        }
    }
    let bytes = std::fs::read(p).ok()?;
    let enc = crate::textenc::detect(&bytes);
    if crate::textenc::looks_binary(&bytes, enc) {
        return None;
    }
    let text = Arc::new(String::from_utf8_lossy(&crate::textenc::decode(&bytes, enc)).into_owned());
    if let Ok(mut c) = cache().lock() {
        if c.1 + text.len() <= MAX_CACHE_BYTES {
            c.1 += text.len();
            c.0.insert(p.to_path_buf(), (mtime, text.clone()));
        }
    }
    Some(text)
}

/// Searches open tabs and indexed files; results arrive via WM_APP_NOTES.
pub fn search(query: String, gen: u64) {
    LATEST_SEARCH.store(gen, Ordering::SeqCst);
    let matchers = match build_matcher(&query) {
        Ok(m) => m,
        Err(e) => {
            palette::set_search_results(gen, Vec::new(), format!("Invalid pattern: {e}"));
            return;
        }
    };
    // Open documents are read on this thread (their text lives in Scintilla).
    let mut sources = Vec::new();
    let mut open_paths = Vec::new();
    let mut budget: usize = 64 * 1024 * 1024;
    let ids: Vec<(u64, Option<PathBuf>, String, bool, bool)> = app().st.borrow().docs.iter().filter(|d| !d.virt).map(|d| (d.id, d.path.clone(), d.name(), d.sci != 0, d.has_buffer)).collect();
    for (id, path, name, loaded, has_buffer) in ids {
        if let Some(p) = &path {
            open_paths.push(p.display().to_string().to_lowercase());
        }
        // Unloaded tabs are read from their session buffer rather than loaded.
        let raw = if loaded {
            Some(crate::docs::doc_text(id))
        } else if has_buffer {
            app().store.read_buffer(id)
        } else {
            None
        };
        let text = if let Some(t) = raw {
            if t.len() <= budget {
                budget -= t.len();
                Some(Arc::new(String::from_utf8_lossy(&t).into_owned()))
            } else {
                None
            }
        } else {
            None
        };
        sources.push(Source { doc: id, path, name, text });
    }
    for p in indexed_files() {
        if !open_paths.contains(&p.display().to_string().to_lowercase()) {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            sources.push(Source { doc: 0, path: Some(p), name, text: None });
        }
    }
    let hwnd = main_hwnd();
    std::thread::spawn(move || {
        let mut items = Vec::new();
        let mut files_hit = 0;
        let mut scanned = 0;
        for s in sources {
            if LATEST_SEARCH.load(Ordering::SeqCst) != gen {
                return;
            }
            let text = match s.text.clone().or_else(|| s.path.as_deref().and_then(read_cached)) {
                Some(t) => t,
                None => continue,
            };
            scanned += 1;
            let mut hit_here = false;
            // A file-name match counts too.
            let name_match = matchers.iter().all(|m| m.is_match(&s.name));
            for (ln, line) in text.split('\n').enumerate() {
                if !matchers.iter().all(|m| m.is_match(line)) {
                    continue;
                }
                let m = matchers[0].find(line).unwrap();
                let shown: String = line.trim().chars().take(160).collect();
                let detail = format!("{}:{}", s.name, ln + 1);
                let kind = Kind::Hit { path: s.path.clone(), doc: s.doc, line: ln, col: m.start(), len: m.end() - m.start() };
                items.push(Item::new(icon::NOTE, shown, kind).detail(detail).hint(s.path.as_ref().and_then(|p| p.parent()).map(|d| d.display().to_string()).unwrap_or_else(|| "open tab".into())));
                hit_here = true;
                if items.len() >= MAX_HITS {
                    break;
                }
            }
            if !hit_here && name_match {
                let kind = Kind::Hit { path: s.path.clone(), doc: s.doc, line: 0, col: 0, len: 0 };
                items.push(Item::new(icon::DOCUMENT, s.name.clone(), kind).detail("file name match"));
                hit_here = true;
            }
            if hit_here {
                files_hit += 1;
            }
            if items.len() >= MAX_HITS {
                break;
            }
        }
        let msg = if items.is_empty() {
            format!("No matches in {scanned} files")
        } else {
            format!("{} match{} in {files_hit} file{} ({scanned} searched)", util::group_digits(items.len()), if items.len() == 1 { "" } else { "es" }, if files_hit == 1 { "" } else { "s" })
        };
        let payload = Box::new((items, msg));
        unsafe { PostMessageW(hwnd as _, WM_APP_NOTES, gen as usize, Box::into_raw(payload) as isize) };
    });
}

pub fn on_results(wp: usize, lp: isize) {
    let payload = unsafe { Box::from_raw(lp as *mut (Vec<Item>, String)) };
    let (items, msg) = *payload;
    palette::set_search_results(wp as u64, items, msg);
}

pub fn open_notes_folder() {
    let d = notes_dir();
    let _ = std::fs::create_dir_all(&d);
    util::shell_open(&d.display().to_string());
}

pub fn choose_notes_folder() {
    let cur = notes_dir().display().to_string();
    palette::prompt("Notes folder (full path)", &cur, false, |s| {
        let s = s.trim().trim_matches('"').to_string();
        if s.is_empty() {
            return;
        }
        let p = PathBuf::from(&s);
        if std::fs::create_dir_all(&p).is_err() {
            app::set_status(&format!("Can't use {s} as the notes folder"));
            return;
        }
        let mut st = app::settings().clone();
        st.notes_dir = s;
        app::apply_settings(st);
        app::set_status("Notes folder changed");
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matcher_terms_and_regex() {
        let m = build_matcher("foo BAR").unwrap();
        assert!(m.iter().all(|r| r.is_match("bar then foo")));
        assert!(!m.iter().all(|r| r.is_match("foo only")));
        let r = build_matcher("/err(or)?\\s+\\d+/").unwrap();
        assert!(r[0].is_match("ERROR 42"));
        assert!(build_matcher("/(/").is_err());
    }

    #[test]
    fn text_names() {
        assert!(is_text_name(Path::new("a/b/notes.MD")));
        assert!(is_text_name(Path::new("README")));
        assert!(!is_text_name(Path::new("photo.jpg")));
    }
}

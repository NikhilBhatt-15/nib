//! Document operations: open, show, save, reload, close; the session
//! snapshot and restore; external-change detection and auto-save.

use crate::app::{self, app, invalidate, set_status, view, Closed, Info, InfoAction, R_ALL, R_LAYOUT, R_STATUS, R_TABS};
use crate::doc::{self, Doc, Source, ViewState};
use crate::gfx::icon;
use crate::languages::{self, LANGS};
use crate::sci::{self, Sci};
use crate::sci_consts::*;
use crate::session::{Manifest, SessionDoc};
use crate::textenc::{Enc, Eol};
use crate::util::{self, now_ms, wide};
use crate::{compare, editor, fileio, history, preview};
use std::path::{Path, PathBuf};
use windows_sys::Win32::UI::Controls::Dialogs::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const MAX_BUFFER: usize = 256 * 1024 * 1024;

/// Runs `f` with a view showing document `id`: its own view if visible,
/// otherwise the hidden loader view. None if the document isn't loaded.
pub fn with_doc_view<R>(id: u64, f: impl FnOnce(&Sci) -> R) -> Option<R> {
    let a = app();
    let (sci, shown) = {
        let st = a.st.borrow();
        let d = st.doc(id)?;
        (d.sci, st.shown_in(id))
    };
    if sci == 0 {
        return None;
    }
    if let Some(&g) = shown.first() {
        return Some(f(&view(g)));
    }
    let l = a.loader.get();
    l.call(SCI_SETDOCPOINTER, 0, sci as isize);
    let r = f(&l);
    l.call(SCI_SETDOCPOINTER, 0, a.loader_idle.get() as isize);
    Some(r)
}

pub fn doc_text(id: u64) -> Vec<u8> {
    ensure_loaded(id);
    with_doc_view(id, |v| v.text()).unwrap_or_default()
}

/// Creates the Scintilla document for `id` from its source.
pub fn ensure_loaded(id: u64) {
    let a = app();
    let (source, path, enc, eol, lang, large, virt, has_buffer) = {
        let st = a.st.borrow();
        let Some(d) = st.doc(id) else { return };
        if d.sci != 0 {
            return;
        }
        (d.source.clone(), d.path.clone(), d.enc, d.eol, d.lang, d.large, d.virt, d.has_buffer)
    };
    let s = a.settings.borrow().clone();
    let mut missing = false;
    let mut stamp = None;
    let mut large = large;
    // `base` becomes the save point. For recovered unsaved edits to a file,
    // the file on disk is the base and the recovered text is applied on top
    // as one undoable change: the tab shows as modified and Ctrl+Z returns
    // to the saved version. Text with nothing on disk behind it (untitled
    // notes) is marked unsaved explicitly instead.
    let (base, edit, force_dirty): (Vec<u8>, Option<Vec<u8>>, bool) = match source {
        Source::Buffer => match a.store.read_buffer(id) {
            Some(recovered) if path.is_some() => {
                let disk = read_or_empty(&path, enc, &s, &mut missing, &mut stamp, &mut large);
                if missing {
                    (recovered, None, true)
                } else {
                    (disk, Some(recovered), false)
                }
            }
            Some(recovered) => (recovered, None, true),
            None => (read_or_empty(&path, enc, &s, &mut missing, &mut stamp, &mut large), None, false),
        },
        Source::File => (read_or_empty(&path, enc, &s, &mut missing, &mut stamp, &mut large), None, false),
        Source::Text(t) => (t, None, !virt),
        Source::Empty | Source::Loaded => (Vec::new(), None, false),
    };
    let _ = has_buffer;
    let l = a.loader.get();
    let sci_doc = sci::create_document(&l, large);
    l.call(SCI_SETDOCPOINTER, 0, sci_doc as isize);
    l.m(SCI_SETEOLMODE, eol.sci_mode(), 0);
    l.set_text(&base);
    editor::set_language(&l, lang, large);
    l.m(SCI_EMPTYUNDOBUFFER, 0, 0);
    l.m(SCI_SETSAVEPOINT, 0, 0);
    let mut edited = false;
    if let Some(e) = edit.as_ref().filter(|e| **e != base) {
        l.begin_undo();
        l.replace_range(0, l.len(), e);
        l.end_undo();
        edited = true;
    }
    let len = l.len();
    l.m(SCI_SETREADONLY, virt as i64, 0);
    l.m(SCI_GOTOPOS, 0, 0);
    l.call(SCI_SETDOCPOINTER, 0, a.loader_idle.get() as isize);
    let mut st = a.st.borrow_mut();
    if let Some(d) = st.doc_mut(id) {
        d.sci = sci_doc;
        d.source = Source::Loaded;
        d.len_hint = len;
        d.large = large;
        d.force_dirty = force_dirty;
        if missing {
            d.missing = true;
        }
        if stamp.is_some() && d.stamp.is_none() {
            d.stamp = stamp;
        }
        d.dirty = force_dirty || edited;
    }
}

fn read_or_empty(path: &Option<PathBuf>, enc: Enc, s: &crate::settings::Settings, missing: &mut bool, stamp: &mut Option<fileio::Stamp>, large: &mut bool) -> Vec<u8> {
    let Some(p) = path else { return Vec::new() };
    match doc::read_file(p, Some(enc), s.eol(), s.large_file_mb) {
        Ok(l) => {
            *stamp = l.stamp;
            *large = l.large;
            l.text
        }
        Err(_) => {
            *missing = true;
            Vec::new()
        }
    }
}

fn capture_view_state(g: usize) -> ViewState {
    let v = view(g);
    ViewState { anchor: v.anchor(), caret: v.pos(), first_line: v.first_visible_line(), x_offset: v.m(SCI_GETXOFFSET, 0, 0) }
}

/// Saves caret/scroll of whatever view `g` shows and moves its edit flag onto the document.
fn stash_view(g: usize) {
    let a = app();
    let id = a.st.borrow().groups[g].active;
    if id == 0 {
        return;
    }
    let vs = capture_view_state(g);
    let modified = a.vflags[g].modified.replace(false);
    let last = a.vflags[g].last_edit.get();
    let dirty = view(g).m(SCI_GETMODIFY, 0, 0) != 0;
    let mut st = a.st.borrow_mut();
    if let Some(d) = st.doc_mut(id) {
        d.view[g] = vs;
        if modified {
            d.snap_needed = true;
            d.last_edit = last;
        }
        if !d.virt {
            d.dirty = dirty || d.force_dirty;
        }
    }
}

/// Shows document `id` in view `g`.
pub fn show(g: usize, id: u64) {
    let a = app();
    let prev = a.st.borrow().groups[g].active;
    let had_preview = a.st.borrow().doc(prev).is_some_and(|d| d.preview);
    if prev != id {
        stash_view(g);
    }
    ensure_loaded(id);
    let (sci_doc, vs, preview_on) = {
        let mut st = a.st.borrow_mut();
        let grp = &mut st.groups[g];
        if !grp.tabs.contains(&id) {
            let at = grp.tabs.iter().position(|&t| t == prev).map_or(grp.tabs.len(), |i| i + 1);
            grp.tabs.insert(at, id);
        }
        grp.active = id;
        grp.mru.retain(|&t| t != id);
        grp.mru.insert(0, id);
        let d = st.doc(id).unwrap();
        (d.sci, d.view[g], d.preview)
    };
    let v = view(g);
    v.call(SCI_SETDOCPOINTER, 0, sci_doc as isize);
    app::restyle_view(g);
    v.set_sel(vs.anchor.min(v.len()), vs.caret.min(v.len()));
    v.set_first_visible_line(vs.first_line);
    v.m(SCI_SETXOFFSET, vs.x_offset, 0);
    a.vflags[g].modified.set(false);
    a.manifest_dirty.set(true);
    if had_preview != preview_on || prev == 0 {
        invalidate(R_LAYOUT);
    }
    preview::refresh(g);
    compare::on_show(g);
    update_counts();
    crate::findbar::on_doc_changed();
    refresh_info_for(id);
    invalidate(R_ALL);
}

pub fn activate(g: usize, id: u64) {
    let a = app();
    {
        let mut st = a.st.borrow_mut();
        if g == 1 && !st.split {
            st.split = true;
        }
        st.active_group = g;
    }
    if a.st.borrow().groups[g].active != id {
        show(g, id);
    }
    invalidate(R_ALL | R_LAYOUT);
    unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus(view(g).hwnd) };
}

/// Keeps `dirty` in sync with Scintilla's save point for visible documents.
pub fn sync_dirty_flags() {
    let a = app();
    let shown: Vec<(usize, u64)> = {
        let st = a.st.borrow();
        (0..2).filter(|&g| g == 0 || st.split).map(|g| (g, st.groups[g].active)).filter(|(_, id)| *id != 0).collect()
    };
    for (g, id) in shown {
        let dirty = view(g).m(SCI_GETMODIFY, 0, 0) != 0;
        let mut st = a.st.borrow_mut();
        if let Some(d) = st.doc_mut(id) {
            let dirty = dirty || d.force_dirty;
            if !d.virt && d.dirty != dirty {
                d.dirty = dirty;
                d.snap_needed = true;
            }
        }
    }
}

fn add_doc(d: Doc) -> u64 {
    let id = d.id;
    app().st.borrow_mut().docs.push(d);
    id
}

fn insert_tab(g: usize, id: u64) {
    let mut st = app().st.borrow_mut();
    let grp = &mut st.groups[g];
    if grp.tabs.contains(&id) {
        return;
    }
    let at = grp.tabs.iter().position(|&t| t == grp.active).map_or(grp.tabs.len(), |i| i + 1);
    grp.tabs.insert(at, id);
}

/// A new untitled note in the active group.
pub fn new_doc() -> u64 {
    new_doc_with(Vec::new(), None)
}

pub fn new_doc_with(text: Vec<u8>, lang: Option<usize>) -> u64 {
    let a = app();
    let (enc, eol) = {
        let s = a.settings.borrow();
        (s.encoding(), s.eol())
    };
    let mut d = Doc::new(util::new_id());
    d.untitled = {
        let mut st = a.st.borrow_mut();
        let n = st.next_untitled;
        st.next_untitled += 1;
        n
    };
    d.enc = enc;
    d.eol = if text.is_empty() { eol } else { crate::textenc::detect_eol(&text, eol) };
    d.lang = lang.unwrap_or(0);
    d.source = if text.is_empty() { Source::Empty } else { Source::Text(text) };
    let id = add_doc(d);
    let g = app::active_group();
    insert_tab(g, id);
    activate(g, id);
    id
}

/// A read-only document for comparisons; not part of the session.
pub fn new_virtual(title: &str, text: Vec<u8>, lang: usize) -> u64 {
    let mut d = Doc::new(util::new_id());
    d.title = Some(title.to_string());
    d.virt = true;
    d.readonly = true;
    d.lang = lang;
    d.eol = crate::textenc::detect_eol(&text, Eol::Crlf);
    d.source = Source::Text(text);
    add_doc(d)
}

fn same_path(a: &Path, b: &Path) -> bool {
    a.as_os_str().to_string_lossy().eq_ignore_ascii_case(&b.as_os_str().to_string_lossy())
}

pub fn find_open(path: &Path) -> Option<u64> {
    let st = app().st.borrow();
    st.docs.iter().find(|d| !d.virt && d.path.as_deref().is_some_and(|p| same_path(p, path))).map(|d| d.id)
}

/// The active doc if it's a blank, untouched note that opening a file can replace.
fn replaceable_blank() -> Option<u64> {
    let st = app().st.borrow();
    let id = st.active_id();
    let d = st.doc(id)?;
    let blank = d.path.is_none() && !d.virt && !d.dirty && !d.has_buffer && matches!(d.source, Source::Empty | Source::Loaded) && d.len_hint == 0 && st.groups[st.active_group].tabs.len() == 1;
    (blank && view(st.active_group).len() == 0).then_some(id)
}

pub fn open_path(path: &Path, group: Option<usize>) -> Option<u64> {
    let a = app();
    let path = std::fs::canonicalize(path).map(|p| PathBuf::from(p.to_string_lossy().trim_start_matches(r"\\?\"))).unwrap_or_else(|_| path.to_path_buf());
    if let Some(id) = find_open(&path) {
        let g = {
            let st = a.st.borrow();
            group.or_else(|| (0..2).find(|&g| (g == 0 || st.split) && st.groups[g].tabs.contains(&id))).unwrap_or(st.active_group)
        };
        activate(g, id);
        return Some(id);
    }
    if path.is_dir() {
        util::error_box(a.hwnd.get(), &format!("{} is a folder.", path.display()));
        return None;
    }
    let s = a.settings.borrow().clone();
    let loaded = match doc::read_file(&path, None, s.eol(), s.large_file_mb) {
        Ok(l) => l,
        Err(e) => {
            util::error_box(a.hwnd.get(), &format!("Couldn't open {}\n\n{e}", path.display()));
            return None;
        }
    };
    if loaded.binary && util::message_box(a.hwnd.get(), &format!("{} looks like a binary file. Open it as text anyway?", path.display()), MB_YESNO | MB_ICONQUESTION) != IDYES {
        return None;
    }
    let blank = replaceable_blank();
    let mut d = Doc::new(util::new_id());
    d.lang = languages::detect(&path.to_string_lossy(), &loaded.text);
    d.path = Some(path.clone());
    d.enc = loaded.enc;
    d.eol = loaded.eol;
    d.stamp = loaded.stamp;
    d.large = loaded.large;
    d.source = Source::File;
    // Read once: hand the already-decoded text to the loader.
    let large = loaded.large;
    d.source = Source::Text(loaded.text);
    let id = add_doc(d);
    ensure_loaded(id);
    with_doc_view(id, |v| {
        v.m(SCI_SETREADONLY, 0, 0);
        v.m(SCI_SETSAVEPOINT, 0, 0);
    });
    {
        let mut st = a.st.borrow_mut();
        if let Some(d) = st.doc_mut(id) {
            d.dirty = false;
            d.force_dirty = false;
        }
    }
    let g = group.unwrap_or_else(app::active_group);
    insert_tab(g, id);
    activate(g, id);
    if let Some(old) = blank {
        remove_tab(g, old, false);
    }
    add_recent(&path);
    if large {
        set_info(Info {
            icon: icon::INFO,
            text: "Large file: syntax highlighting, word wrap and indentation guides are off to keep editing fast.".into(),
            actions: vec![("OK".into(), InfoAction::Dismiss)],
            doc: id,
            warning: false,
        });
    }
    Some(id)
}

pub fn add_recent(path: &Path) {
    let s = path.display().to_string();
    {
        let mut st = app().st.borrow_mut();
        let r = &mut st.persist.recent;
        r.retain(|p| !p.eq_ignore_ascii_case(&s));
        r.insert(0, s);
        r.truncate(40);
    }
    app::save_persist();
}

pub fn open_dialog() {
    let a = app();
    let mut buf = vec![0u16; 32 * 1024];
    let filter = wide("All files (*.*)\0*.*\0Text (*.txt;*.log)\0*.txt;*.log\0Markdown (*.md)\0*.md;*.markdown\0\0");
    let initial = current_dir_hint();
    let initial_w = wide(&initial);
    let mut ofn: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    ofn.hwndOwner = a.hwnd.get();
    ofn.lpstrFilter = filter.as_ptr();
    ofn.lpstrFile = buf.as_mut_ptr();
    ofn.nMaxFile = buf.len() as u32;
    ofn.lpstrInitialDir = if initial.is_empty() { std::ptr::null() } else { initial_w.as_ptr() };
    ofn.Flags = OFN_EXPLORER | OFN_ALLOWMULTISELECT | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST;
    if unsafe { GetOpenFileNameW(&mut ofn) } == 0 {
        return;
    }
    // Multi-select: dir\0file1\0file2\0\0; single: full path\0\0.
    let parts: Vec<String> = buf.split(|&c| c == 0).take_while(|s| !s.is_empty()).map(String::from_utf16_lossy).collect();
    let files: Vec<PathBuf> = if parts.len() <= 1 {
        parts.into_iter().map(PathBuf::from).collect()
    } else {
        let dir = PathBuf::from(&parts[0]);
        parts[1..].iter().map(|f| dir.join(f)).collect()
    };
    for f in files {
        open_path(&f, None);
    }
}

/// A single file chosen with the Open dialog.
pub fn pick_file_to_compare() -> Option<PathBuf> {
    let a = app();
    let mut buf = vec![0u16; 4096];
    let filter = wide("All files (*.*)\0*.*\0\0");
    let title = wide("Compare with file");
    let mut ofn: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    ofn.hwndOwner = a.hwnd.get();
    ofn.lpstrFilter = filter.as_ptr();
    ofn.lpstrFile = buf.as_mut_ptr();
    ofn.nMaxFile = buf.len() as u32;
    ofn.lpstrTitle = title.as_ptr();
    ofn.Flags = OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST;
    if unsafe { GetOpenFileNameW(&mut ofn) } == 0 {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(0);
    Some(PathBuf::from(String::from_utf16_lossy(&buf[..len])))
}

fn current_dir_hint() -> String {
    let st = app().st.borrow();
    st.doc(st.active_id()).map(|d| d.dir_hint()).unwrap_or_default()
}

/// Asks where to save. Returns the chosen path.
pub fn save_dialog(id: u64) -> Option<PathBuf> {
    let a = app();
    let (name, dir, lang) = {
        let st = a.st.borrow();
        let d = st.doc(id)?;
        (suggest_name(d, id), d.dir_hint(), d.lang)
    };
    let dir = if dir.is_empty() { a.settings.borrow().notes_dir.clone() } else { dir };
    let dir = if dir.is_empty() { a.paths.default_notes().display().to_string() } else { dir };
    let _ = std::fs::create_dir_all(&dir);
    let mut buf = vec![0u16; 4096];
    for (i, c) in name.encode_utf16().take(4000).enumerate() {
        buf[i] = c;
    }
    let filter = wide("Text (*.txt)\0*.txt\0Markdown (*.md)\0*.md\0All files (*.*)\0*.*\0\0");
    let dir_w = wide(&dir);
    let def_ext = wide(if LANGS[lang].id == "markdown" { "md" } else { "txt" });
    let mut ofn: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    ofn.hwndOwner = a.hwnd.get();
    ofn.lpstrFilter = filter.as_ptr();
    ofn.nFilterIndex = if LANGS[lang].id == "markdown" { 2 } else if name.contains('.') { 3 } else { 1 };
    ofn.lpstrFile = buf.as_mut_ptr();
    ofn.nMaxFile = buf.len() as u32;
    ofn.lpstrInitialDir = dir_w.as_ptr();
    ofn.lpstrDefExt = def_ext.as_ptr();
    ofn.Flags = OFN_EXPLORER | OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST;
    if unsafe { GetSaveFileNameW(&mut ofn) } == 0 {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(0);
    Some(PathBuf::from(String::from_utf16_lossy(&buf[..len])))
}

/// File name suggestion: the existing name, or the first line of the note.
fn suggest_name(d: &Doc, id: u64) -> String {
    if let Some(p) = &d.path {
        return p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    }
    let first = with_doc_view(id, |v| v.line_text(0)).unwrap_or_default();
    let clean: String = first.trim_start_matches(['#', ' ', '-', '*', '>']).chars().filter(|c| !"\\/:*?\"<>|".contains(*c)).take(48).collect();
    let clean = clean.trim().to_string();
    if clean.is_empty() {
        d.name()
    } else {
        clean
    }
}

pub fn save(id: u64) -> bool {
    save_impl(id, false, None)
}

pub fn save_as(id: u64) -> bool {
    match save_dialog(id) {
        Some(p) => save_impl(id, false, Some(p)),
        None => false,
    }
}

pub fn save_all() {
    let ids: Vec<u64> = app().st.borrow().docs.iter().filter(|d| d.dirty && !d.virt).map(|d| d.id).collect();
    for id in ids {
        if !save(id) {
            break;
        }
    }
}

/// Saves without dialogs; used by auto-save. Untitled notes are skipped.
fn save_quiet(id: u64) -> bool {
    save_impl(id, true, None)
}

fn save_impl(id: u64, quiet: bool, new_path: Option<PathBuf>) -> bool {
    let a = app();
    sync_dirty_flags();
    let (path, enc, virt) = {
        let st = a.st.borrow();
        let Some(d) = st.doc(id) else { return false };
        (new_path.clone().or_else(|| d.path.clone()), d.enc, d.virt)
    };
    if virt {
        return false;
    }
    let Some(path) = path else {
        if quiet {
            return false;
        }
        return save_as(id);
    };
    ensure_loaded(id);
    let text = doc_text(id);
    let mut enc = enc;
    let (_, lossy) = crate::textenc::encode(&text, enc);
    if lossy {
        if quiet {
            set_status("Not auto-saved: some characters can't be stored in the file's encoding");
            return false;
        }
        let r = util::message_box(
            a.hwnd.get(),
            &format!("Some characters can't be represented in {}. Save as UTF-8 instead?\n\nChoose No to save anyway (they become '?').", enc.label()),
            MB_YESNOCANCEL | MB_ICONWARNING,
        );
        if r == IDCANCEL {
            return false;
        }
        if r == IDYES {
            enc = Enc::Utf8;
        }
    }
    let existed = path.exists();
    let previous = if existed && a.settings.borrow().history.enabled { std::fs::read(&path).ok().map(|b| (b, fileio::stamp(&path).map_or(0, |s| s.mtime))) } else { None };
    match doc::write_file(&path, &text, enc) {
        Ok((stamp, _, bytes)) => {
            with_doc_view(id, |v| v.m(SCI_SETSAVEPOINT, 0, 0));
            let renamed = new_path.is_some();
            let (old_buffer, new_lang) = {
                let mut st = a.st.borrow_mut();
                let d = st.doc_mut(id).unwrap();
                let lang_changed = renamed && (d.lang == 0 || d.path.is_none());
                d.path = Some(path.clone());
                d.enc = enc;
                d.stamp = stamp;
                d.dirty = false;
                d.disk_changed = false;
                d.force_dirty = false;
                d.missing = false;
                d.title = None;
                d.snap_needed = false;
                let had = d.has_buffer;
                d.has_buffer = false;
                let nl = lang_changed.then(|| languages::detect(&path.to_string_lossy(), &text));
                (had, nl)
            };
            if old_buffer {
                a.store.remove_buffer(id);
            }
            if let Some(l) = new_lang {
                set_language(id, l);
            }
            history::record(&path, previous.as_ref().map(|(b, t)| (b.as_slice(), *t)), &bytes);
            add_recent(&path);
            if path == a.paths.settings() {
                app::check_settings_file();
            }
            a.manifest_dirty.set(true);
            clear_info_for(id);
            set_status(&format!("Saved {}", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
            invalidate(R_ALL);
            true
        }
        Err(e) => {
            if quiet {
                set_status(&format!("Auto-save failed: {e}"));
            } else {
                util::error_box(a.hwnd.get(), &format!("Couldn't save {}\n\n{e}", path.display()));
            }
            false
        }
    }
}

/// Re-reads a file from disk as one undoable change. `enc` forces an encoding.
pub fn reload(id: u64, enc: Option<Enc>) {
    let a = app();
    let (path, _cur_enc) = {
        let st = a.st.borrow();
        let Some(d) = st.doc(id) else { return };
        let Some(p) = d.path.clone() else { return };
        (p, d.enc)
    };
    let s = a.settings.borrow().clone();
    let loaded = match doc::read_file(&path, enc, s.eol(), s.large_file_mb) {
        Ok(l) => l,
        Err(e) => {
            util::error_box(a.hwnd.get(), &format!("Couldn't reload {}\n\n{e}", path.display()));
            return;
        }
    };
    ensure_loaded(id);
    with_doc_view(id, |v| {
        let caret = v.pos();
        let first = v.first_visible_line();
        let ro = v.readonly();
        v.m(SCI_SETREADONLY, 0, 0);
        v.begin_undo();
        v.replace_range(0, v.len(), &loaded.text);
        v.end_undo();
        v.m(SCI_SETREADONLY, ro as i64, 0);
        v.m(SCI_SETSAVEPOINT, 0, 0);
        v.m(SCI_SETEOLMODE, loaded.eol.sci_mode(), 0);
        v.set_sel(caret.min(v.len()), caret.min(v.len()));
        v.set_first_visible_line(first);
    });
    {
        let mut st = a.st.borrow_mut();
        if let Some(d) = st.doc_mut(id) {
            d.enc = loaded.enc;
            d.eol = loaded.eol;
            d.stamp = loaded.stamp;
            d.dirty = false;
            d.force_dirty = false;
            d.disk_changed = false;
            d.missing = false;
            d.snap_needed = true;
        }
    }
    clear_info_for(id);
    invalidate(R_ALL);
}

/// Removes `id` from group `g`; releases it when no group shows it any more.
fn remove_tab(g: usize, id: u64, remember: bool) {
    let a = app();
    let showing = a.st.borrow().groups[g].active == id;
    if showing {
        stash_view(g);
    }
    if remember {
        remember_closed(id, g);
    }
    let (next, still_used) = {
        let mut st = a.st.borrow_mut();
        let grp = &mut st.groups[g];
        let pos = grp.tabs.iter().position(|&t| t == id).unwrap_or(0);
        grp.tabs.retain(|&t| t != id);
        grp.mru.retain(|&t| t != id);
        let next = if grp.active == id {
            grp.mru.first().copied().or_else(|| grp.tabs.get(pos.min(grp.tabs.len().saturating_sub(1))).copied())
        } else {
            Some(grp.active).filter(|&x| x != 0)
        };
        if grp.active == id {
            grp.active = 0;
        }
        let used = st.groups.iter().any(|gr| gr.tabs.contains(&id));
        (next, used)
    };
    if !still_used {
        release_doc(id);
    }
    match next {
        Some(n) => show(g, n),
        None => {
            // Group emptied.
            let split = a.st.borrow().split;
            if g == 1 || split {
                unsplit_into(0);
            }
        }
    }
    if a.st.borrow().groups[0].tabs.is_empty() {
        new_doc();
    }
    a.manifest_dirty.set(true);
    invalidate(R_ALL | R_LAYOUT);
}

fn release_doc(id: u64) {
    let a = app();
    let d = {
        let mut st = a.st.borrow_mut();
        let Some(i) = st.docs.iter().position(|d| d.id == id) else { return };
        st.docs.remove(i)
    };
    if d.sci != 0 {
        sci::release_document(&a.loader.get(), d.sci);
    }
    if d.has_buffer {
        a.store.remove_buffer(id);
    }
    compare::on_doc_closed(id);
}

fn remember_closed(id: u64, g: usize) {
    let a = app();
    let (path, vs, virt, name) = {
        let st = a.st.borrow();
        let Some(d) = st.doc(id) else { return };
        (d.path.clone(), d.view[g], d.virt, d.name())
    };
    if virt {
        return;
    }
    let entry = match path {
        Some(p) => Closed::Path(p, vs),
        None => {
            let text = doc_text(id);
            if text.is_empty() {
                return;
            }
            Closed::Text(name, text, vs.caret)
        }
    };
    let mut st = a.st.borrow_mut();
    st.closed.push(entry);
    if st.closed.len() > 30 {
        st.closed.remove(0);
    }
}

/// Closes a tab, asking about unsaved changes when the setting says so.
pub fn close_tab(g: usize, id: u64) -> bool {
    let a = app();
    sync_dirty_flags();
    let (dirty, virt, name, in_other) = {
        let st = a.st.borrow();
        let Some(d) = st.doc(id) else { return true };
        let other = st.split && st.groups[1 - g].tabs.contains(&id);
        (d.dirty, d.virt, d.name(), other)
    };
    if dirty && !virt && !in_other && a.settings.borrow().confirm_close_dirty {
        let r = util::message_box(a.hwnd.get(), &format!("Save changes to {name}?"), MB_YESNOCANCEL | MB_ICONQUESTION);
        if r == IDCANCEL {
            return false;
        }
        if r == IDYES && !save(id) {
            return false;
        }
    }
    remove_tab(g, id, true);
    true
}

pub fn close_others(g: usize, keep: u64) {
    let ids: Vec<u64> = app().st.borrow().groups[g].tabs.iter().copied().filter(|&t| t != keep).collect();
    for id in ids {
        if !close_tab(g, id) {
            break;
        }
    }
}

pub fn close_to_right(g: usize, of: u64) {
    let ids: Vec<u64> = {
        let st = app().st.borrow();
        let tabs = &st.groups[g].tabs;
        let i = tabs.iter().position(|&t| t == of).unwrap_or(tabs.len());
        tabs[i + 1..].to_vec()
    };
    for id in ids {
        if !close_tab(g, id) {
            break;
        }
    }
}

pub fn close_saved(g: usize) {
    sync_dirty_flags();
    let ids: Vec<u64> = {
        let st = app().st.borrow();
        st.groups[g].tabs.iter().copied().filter(|&t| st.doc(t).is_some_and(|d| !d.dirty)).collect()
    };
    for id in ids {
        close_tab(g, id);
    }
}

pub fn reopen_closed() {
    let entry = app().st.borrow_mut().closed.pop();
    match entry {
        Some(Closed::Path(p, vs)) => {
            if let Some(id) = open_path(&p, None) {
                let g = app::active_group();
                let v = view(g);
                v.set_sel(vs.anchor.min(v.len()), vs.caret.min(v.len()));
                v.set_first_visible_line(vs.first_line);
                let _ = id;
            }
        }
        Some(Closed::Text(name, text, caret)) => {
            new_doc_with(text, None);
            set_status(&format!("Reopened {name} as a new note"));
            let v = app::active_view();
            v.set_sel(caret.min(v.len()), caret.min(v.len()));
            v.m(SCI_SCROLLCARET, 0, 0);
        }
        None => set_status("No recently closed tabs"),
    }
}

pub fn set_language(id: u64, lang: usize) {
    let a = app();
    let large = {
        let mut st = a.st.borrow_mut();
        let Some(d) = st.doc_mut(id) else { return };
        d.lang = lang;
        if !d.is_markdown() {
            d.preview = false;
        }
        d.large
    };
    with_doc_view(id, |v| editor::set_language(v, lang, large));
    for g in a.st.borrow().shown_in(id) {
        app::restyle_view(g);
    }
    a.manifest_dirty.set(true);
    invalidate(R_ALL | R_LAYOUT);
}

pub fn convert_eol(id: u64, eol: Eol) {
    ensure_loaded(id);
    with_doc_view(id, |v| {
        v.m(SCI_SETEOLMODE, eol.sci_mode(), 0);
        v.m(SCI_CONVERTEOLS, eol.sci_mode(), 0);
    });
    if let Some(d) = app().st.borrow_mut().doc_mut(id) {
        d.eol = eol;
    }
    app().manifest_dirty.set(true);
    invalidate(R_ALL);
}

/// Changes the encoding used when saving (and saves if the doc has a file).
pub fn save_with_encoding(id: u64, enc: Enc) {
    let has_path = {
        let mut st = app().st.borrow_mut();
        let Some(d) = st.doc_mut(id) else { return };
        d.enc = enc;
        d.path.is_some()
    };
    if has_path {
        with_doc_view(id, |v| {
            // Force a write even if the text is unchanged.
            v.m(SCI_SETSAVEPOINT, 0, 0);
        });
        if let Some(d) = app().st.borrow_mut().doc_mut(id) {
            d.dirty = true;
        }
        save(id);
    }
    app().manifest_dirty.set(true);
    invalidate(R_ALL);
}

pub fn reopen_with_encoding(id: u64, enc: Enc) {
    let a = app();
    let dirty = a.st.borrow().doc(id).is_some_and(|d| d.dirty);
    if dirty && util::message_box(a.hwnd.get(), "Reopening discards unsaved changes (you can undo it). Continue?", MB_OKCANCEL | MB_ICONQUESTION) != IDOK {
        return;
    }
    reload(id, Some(enc));
}

pub fn toggle_preview() {
    let a = app();
    let id = app::active_doc();
    {
        let mut st = a.st.borrow_mut();
        let Some(d) = st.doc_mut(id) else { return };
        if !d.is_markdown() {
            drop(st);
            set_status("Preview is for Markdown documents (set the language to Markdown)");
            return;
        }
        d.preview = !d.preview;
    }
    a.manifest_dirty.set(true);
    app::layout();
    for g in a.st.borrow().shown_in(id) {
        preview::refresh(g);
    }
    invalidate(R_ALL);
}

pub fn toggle_split() {
    let a = app();
    let split = a.st.borrow().split;
    if split {
        unsplit_into(0);
    } else {
        let id = app::active_doc();
        a.st.borrow_mut().split = true;
        insert_tab(1, id);
        activate(1, id);
    }
    a.manifest_dirty.set(true);
    invalidate(R_ALL | R_LAYOUT);
}

/// Merges the right group into the left and closes the split.
fn unsplit_into(keep: usize) {
    let a = app();
    let other = 1 - keep;
    stash_view(other);
    let ids: Vec<u64> = a.st.borrow().groups[other].tabs.clone();
    {
        let mut st = a.st.borrow_mut();
        for id in &ids {
            if !st.groups[keep].tabs.contains(id) {
                st.groups[keep].tabs.push(*id);
            }
        }
        let grp = &mut st.groups[other];
        grp.tabs.clear();
        grp.mru.clear();
        grp.active = 0;
        st.split = false;
        st.active_group = 0;
    }
    if keep == 1 {
        // Content of group 1 becomes group 0.
        let mut st = a.st.borrow_mut();
        st.groups.swap(0, 1);
    }
    view(1).call(SCI_SETDOCPOINTER, 0, 0);
    let active = a.st.borrow().groups[0].active;
    let first = a.st.borrow().groups[0].tabs.first().copied();
    if active == 0 {
        if let Some(f) = first {
            show(0, f);
        }
    }
    compare::close();
    invalidate(R_ALL | R_LAYOUT);
}

pub fn move_to_other_group() {
    let a = app();
    let (g, id) = {
        let st = a.st.borrow();
        (st.active_group, st.active_id())
    };
    let target = 1 - g;
    a.st.borrow_mut().split = true;
    insert_tab(target, id);
    activate(target, id);
    let remaining = a.st.borrow().groups[g].tabs.len();
    if remaining > 1 {
        remove_tab(g, id, false);
    }
    invalidate(R_ALL | R_LAYOUT);
}

pub fn focus_other_group() {
    let a = app();
    let (split, g) = {
        let st = a.st.borrow();
        (st.split, st.active_group)
    };
    if split {
        let other = 1 - g;
        let id = a.st.borrow().groups[other].active;
        activate(other, id);
    }
}

pub fn cycle_tab(delta: i32) {
    let (g, next) = {
        let st = app().st.borrow();
        let g = st.active_group;
        let tabs = &st.groups[g].tabs;
        if tabs.is_empty() {
            return;
        }
        let i = tabs.iter().position(|&t| t == st.groups[g].active).unwrap_or(0) as i32;
        let n = tabs.len() as i32;
        (g, tabs[((i + delta) % n + n) as usize % n as usize])
    };
    activate(g, next);
}

pub fn goto_tab(n: usize) {
    let (g, id) = {
        let st = app().st.borrow();
        let g = st.active_group;
        let tabs = &st.groups[g].tabs;
        let i = if n == 9 { tabs.len().saturating_sub(1) } else { n.saturating_sub(1) };
        (g, tabs.get(i).copied())
    };
    if let Some(id) = id {
        activate(g, id);
    }
}

pub fn move_tab(g: usize, id: u64, to_index: usize) {
    let mut st = app().st.borrow_mut();
    let tabs = &mut st.groups[g].tabs;
    if let Some(i) = tabs.iter().position(|&t| t == id) {
        tabs.remove(i);
        tabs.insert(to_index.min(tabs.len()), id);
    }
    drop(st);
    app().manifest_dirty.set(true);
    invalidate(R_TABS);
}

pub fn toggle_pin_tab(g: usize, id: u64) {
    {
        let mut st = app().st.borrow_mut();
        let pinned = {
            let Some(d) = st.doc_mut(id) else { return };
            d.pinned = !d.pinned;
            d.pinned
        };
        let pinned_ids: Vec<u64> = st.docs.iter().filter(|d| d.pinned && d.id != id).map(|d| d.id).collect();
        let tabs = &mut st.groups[g].tabs;
        tabs.retain(|&t| t != id);
        // Pinned tabs sit together at the left; this one goes at the end of that block.
        let n = tabs.iter().take_while(|t| pinned_ids.contains(t)).count();
        tabs.insert(n, id);
        let _ = pinned;
    }
    app().manifest_dirty.set(true);
    invalidate(R_TABS);
}

// ------------------------------------------------------------------ info bar

pub fn set_info(info: Info) {
    app().st.borrow_mut().info = Some(info);
    invalidate(R_LAYOUT);
}

pub fn clear_info() {
    let had = app().st.borrow_mut().info.take().is_some();
    if had {
        invalidate(R_LAYOUT);
    }
}

fn clear_info_for(id: u64) {
    let matches = app().st.borrow().info.as_ref().is_some_and(|i| i.doc == id);
    if matches {
        clear_info();
    }
}

/// Shows the "changed on disk" bar when switching to such a document.
fn refresh_info_for(id: u64) {
    let (changed, missing, name) = {
        let st = app().st.borrow();
        let Some(d) = st.doc(id) else { return };
        (d.disk_changed, d.missing && d.path.is_some(), d.name())
    };
    if changed {
        set_info(Info {
            icon: icon::WARNING,
            text: format!("{name} was changed by another program, and you have unsaved edits here."),
            actions: vec![("Reload from disk".into(), InfoAction::Reload(id)), ("Compare".into(), InfoAction::CompareDisk(id)), ("Keep mine".into(), InfoAction::KeepMine(id))],
            doc: id,
            warning: true,
        });
    } else if missing {
        set_info(Info {
            icon: icon::WARNING,
            text: format!("{name} no longer exists on disk. Your text is kept here."),
            actions: vec![("Save".into(), InfoAction::SaveAs(id)), ("Close tab".into(), InfoAction::CloseDoc(id))],
            doc: id,
            warning: true,
        });
    } else {
        let st = app().st.borrow();
        let stale = st.info.as_ref().is_some_and(|i| i.doc != 0 && !st.shown_in(i.doc).iter().any(|_| true));
        drop(st);
        if stale {
            clear_info();
        }
    }
}

pub fn run_info_action(act: InfoAction) {
    match act {
        InfoAction::Dismiss => clear_info(),
        InfoAction::Reload(id) => reload(id, None),
        InfoAction::KeepMine(id) => {
            let mut st = app().st.borrow_mut();
            if let Some(d) = st.doc_mut(id) {
                d.disk_changed = false;
                d.stamp = d.path.as_deref().and_then(fileio::stamp);
            }
            st.info = None;
            drop(st);
            invalidate(R_LAYOUT);
        }
        InfoAction::CompareDisk(id) => compare::with_disk(id),
        InfoAction::SaveAs(id) => {
            if save(id) {
                clear_info();
            }
        }
        InfoAction::CloseDoc(id) => {
            let g = app::active_group();
            clear_info();
            close_tab(g, id);
        }
        InfoAction::CompareNext => compare::next(true),
        InfoAction::ComparePrev => compare::next(false),
        InfoAction::CompareClose => compare::close(),
        InfoAction::CompareRestore => compare::restore_right_into_left(),
        InfoAction::EnableAi(p) => crate::ai::enable(p),
        InfoAction::AiCompareBefore => crate::ai::compare_before(),
        InfoAction::Undo => {
            app::active_view().m(SCI_UNDO, 0, 0);
            clear_info();
        }
    }
}

// ------------------------------------------------------------------ disk changes and auto-save

pub fn check_disk_changes() {
    let a = app();
    let candidates: Vec<(u64, PathBuf, Option<fileio::Stamp>, bool, bool)> =
        a.st.borrow().docs.iter().filter(|d| !d.virt).filter_map(|d| d.path.clone().map(|p| (d.id, p, d.stamp, d.dirty, d.sci != 0))).collect();
    let mut reloaded = Vec::new();
    for (id, path, old, dirty, loaded) in candidates {
        let now = fileio::stamp(&path);
        if now == old {
            continue;
        }
        match now {
            None => {
                if let Some(d) = a.st.borrow_mut().doc_mut(id) {
                    d.missing = true;
                }
            }
            Some(_) if !loaded => {
                if let Some(d) = a.st.borrow_mut().doc_mut(id) {
                    if d.has_buffer {
                        d.disk_changed = true;
                    } else {
                        d.stamp = now;
                    }
                }
            }
            Some(_) if !dirty => {
                reload(id, None);
                reloaded.push(path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
            }
            Some(_) => {
                if let Some(d) = a.st.borrow_mut().doc_mut(id) {
                    d.disk_changed = true;
                    d.missing = false;
                }
            }
        }
    }
    if !reloaded.is_empty() {
        set_status(&format!("Reloaded {} (changed on disk)", reloaded.join(", ")));
    }
    let active = app::active_doc();
    refresh_info_for(active);
    invalidate(R_ALL);
}

pub fn on_focus_lost() {
    if app().settings.borrow().autosave == "focus" {
        save_dirty_files_quietly(0);
    }
    snapshot(true);
}

pub fn autosave_tick() {
    let (mode, delay) = {
        let s = app().settings.borrow();
        (s.autosave.clone(), s.autosave_delay_ms as u64)
    };
    if mode == "delay" {
        save_dirty_files_quietly(delay);
    }
}

fn save_dirty_files_quietly(min_idle_ms: u64) {
    sync_dirty_flags();
    let now = now_ms();
    let ids: Vec<u64> = app()
        .st
        .borrow()
        .docs
        .iter()
        .filter(|d| d.dirty && !d.virt && d.path.is_some() && !d.disk_changed && !d.missing && now.saturating_sub(d.last_edit) >= min_idle_ms)
        .map(|d| d.id)
        .collect();
    for id in ids {
        save_quiet(id);
    }
}

// ------------------------------------------------------------------ session

/// Writes unsaved text of edited documents to their session buffers.
pub fn snapshot(force: bool) {
    let a = app();
    for g in 0..2 {
        if a.vflags[g].modified.replace(false) {
            let id = a.st.borrow().groups[g].active;
            let last = a.vflags[g].last_edit.get();
            if let Some(d) = a.st.borrow_mut().doc_mut(id) {
                d.snap_needed = true;
                d.last_edit = last;
            }
        }
    }
    sync_dirty_flags();
    let now = now_ms();
    let due: Vec<(u64, bool, bool)> = a
        .st
        .borrow()
        .docs
        .iter()
        .filter(|d| d.snap_needed && !d.virt && (force || now.saturating_sub(d.last_edit) >= 1000))
        .map(|d| (d.id, d.dirty, d.has_buffer))
        .collect();
    if !a.settings.borrow().restore_session && !force {
        return;
    }
    for (id, dirty, has_buffer) in due {
        let mut wrote = false;
        if dirty {
            if let Some(text) = with_doc_view(id, |v| (v.len() <= MAX_BUFFER).then(|| v.text())).flatten() {
                a.store.write_buffer(id, text);
                wrote = true;
            }
        } else if has_buffer {
            a.store.remove_buffer(id);
        }
        if let Some(d) = a.st.borrow_mut().doc_mut(id) {
            d.snap_needed = false;
            if dirty {
                d.has_buffer = wrote || d.has_buffer;
            } else {
                d.has_buffer = false;
            }
        }
        a.manifest_dirty.set(true);
    }
    if a.manifest_dirty.get() || force {
        write_manifest_now();
    }
}

pub fn write_manifest_now() {
    let a = app();
    for g in 0..2 {
        let id = a.st.borrow().groups[g].active;
        if id != 0 && (g == 0 || a.st.borrow().split) {
            let vs = capture_view_state(g);
            if let Some(d) = a.st.borrow_mut().doc_mut(id) {
                d.view[g] = vs;
            }
        }
    }
    let m = {
        let st = a.st.borrow();
        let in_tabs = |id: u64| st.groups.iter().any(|g| g.tabs.contains(&id));
        Manifest {
            seq: 0,
            docs: st
                .docs
                .iter()
                .filter(|d| !d.virt && in_tabs(d.id))
                .map(|d| SessionDoc {
                    id: d.id,
                    path: d.path.as_ref().map(|p| p.display().to_string()),
                    untitled: d.untitled,
                    enc: d.enc.id(),
                    eol: d.eol.label().to_string(),
                    lang: LANGS[d.lang].id.to_string(),
                    dirty: d.dirty,
                    buffer: d.has_buffer,
                    stamp: d.stamp,
                    view: d.view,
                    preview: d.preview,
                    pinned: d.pinned,
                })
                .collect(),
            groups: [
                st.groups[0].tabs.iter().copied().filter(|&id| st.doc(id).is_some_and(|d| !d.virt)).collect(),
                st.groups[1].tabs.iter().copied().filter(|&id| st.doc(id).is_some_and(|d| !d.virt)).collect(),
            ],
            active: [st.groups[0].active, st.groups[1].active],
            active_group: st.active_group,
            split: st.split,
            split_ratio: st.split_ratio,
            next_untitled: st.next_untitled,
        }
    };
    a.store.write_manifest(m);
    a.manifest_dirty.set(false);
}

pub fn restore_session(files: &[PathBuf]) {
    let a = app();
    let restore = a.settings.borrow().restore_session;
    let manifest = if restore { a.store.load_manifest() } else { None };
    let mut recovered = 0;
    if let Some(m) = manifest {
        let mut st = a.st.borrow_mut();
        for sd in &m.docs {
            let mut d = Doc::new(sd.id);
            d.path = sd.path.as_ref().map(PathBuf::from);
            d.untitled = sd.untitled;
            d.enc = Enc::from_id(&sd.enc).unwrap_or(Enc::Utf8);
            d.eol = Eol::from_label(&sd.eol).unwrap_or(Eol::Crlf);
            d.lang = languages::by_id(&sd.lang);
            d.dirty = sd.dirty;
            d.has_buffer = sd.buffer;
            d.stamp = sd.stamp;
            d.view = sd.view;
            d.preview = sd.preview;
            d.pinned = sd.pinned;
            d.source = if sd.buffer {
                recovered += 1;
                Source::Buffer
            } else if d.path.is_some() {
                Source::File
            } else {
                Source::Empty
            };
            st.docs.push(d);
        }
        let ids: Vec<u64> = st.docs.iter().map(|d| d.id).collect();
        for g in 0..2 {
            st.groups[g].tabs = m.groups[g].iter().copied().filter(|id| ids.contains(id)).collect();
            st.groups[g].active = 0;
            st.groups[g].mru = st.groups[g].tabs.clone();
        }
        st.split = m.split && !st.groups[1].tabs.is_empty();
        st.split_ratio = if m.split_ratio > 0.0 { m.split_ratio } else { 0.5 };
        st.active_group = if st.split { m.active_group.min(1) } else { 0 };
        st.next_untitled = m.next_untitled.max(1);
        // Docs referenced by no group are dropped.
        let used: Vec<u64> = st.groups.iter().flat_map(|g| g.tabs.iter().copied()).collect();
        st.docs.retain(|d| used.contains(&d.id));
        let keep: Vec<u64> = st.docs.iter().filter(|d| d.has_buffer).map(|d| d.id).collect();
        a.store.prune_buffers(&keep);
        drop(st);
        for g in 0..2 {
            let (active, first) = {
                let st = a.st.borrow();
                (m.active[g], st.groups[g].tabs.first().copied())
            };
            let pick = if a.st.borrow().groups[g].tabs.contains(&active) { Some(active) } else { first };
            if let Some(id) = pick {
                if g == 0 || a.st.borrow().split {
                    show(g, id);
                }
            }
        }
    } else {
        a.store.prune_buffers(&[]);
    }
    if a.crashed.get() && recovered > 0 {
        set_info(Info {
            icon: icon::INFO,
            text: format!("Nib didn't close properly last time. Your unsaved work was recovered ({recovered} tab{}).", if recovered == 1 { "" } else { "s" }),
            actions: vec![("OK".into(), InfoAction::Dismiss)],
            doc: 0,
            warning: false,
        });
    }
    for f in files {
        open_path(f, None);
    }
    let active = a.st.borrow().active_id();
    if active == 0 {
        let first = a.st.borrow().groups[0].tabs.first().copied();
        if let Some(f) = first {
            show(0, f);
        }
    }
    a.manifest_dirty.set(true);
}

/// With session restore off, asks about each unsaved document before quitting.
pub fn confirm_quit() -> bool {
    let a = app();
    if a.settings.borrow().restore_session {
        return true;
    }
    sync_dirty_flags();
    let ids: Vec<(u64, String)> = a.st.borrow().docs.iter().filter(|d| d.dirty && !d.virt).map(|d| (d.id, d.name())).collect();
    for (id, name) in ids {
        let r = util::message_box(a.hwnd.get(), &format!("Save changes to {name}?"), MB_YESNOCANCEL | MB_ICONQUESTION);
        if r == IDCANCEL || (r == IDYES && !save(id)) {
            return false;
        }
        if r == IDNO {
            if let Some(d) = a.st.borrow_mut().doc_mut(id) {
                d.dirty = false;
                d.snap_needed = true;
            }
        }
    }
    true
}

/// Word and character counts for visible documents, cached by edit revision.
pub fn update_counts() {
    let a = app();
    for g in 0..2 {
        let id = a.st.borrow().groups[g].active;
        if id == 0 {
            continue;
        }
        let rev = a.vflags[g].rev.get();
        let cached = a.st.borrow().counts.get(&id).map(|c| c.0);
        if cached == Some(rev) {
            continue;
        }
        let v = view(g);
        if v.len() > 32 * 1024 * 1024 {
            continue;
        }
        let (w, c) = util::count_words_chars(v.bytes());
        a.st.borrow_mut().counts.insert(id, (rev, w, c));
    }
    invalidate(R_STATUS);
}

pub fn reveal_active() {
    let p = {
        let st = app().st.borrow();
        st.doc(st.active_id()).and_then(|d| d.path.clone())
    };
    match p {
        Some(p) => util::reveal_in_explorer(&p),
        None => set_status("This note hasn't been saved to a file yet"),
    }
}

pub fn copy_path() {
    let p = {
        let st = app().st.borrow();
        st.doc(st.active_id()).and_then(|d| d.path.clone())
    };
    if let Some(p) = p {
        util::set_clipboard_text(app().hwnd.get(), &p.display().to_string());
        set_status("Path copied");
    }
}

pub fn toggle_pin_file() {
    let p = {
        let st = app().st.borrow();
        st.doc(st.active_id()).and_then(|d| d.path.clone())
    };
    let Some(p) = p else {
        set_status("Save the note to pin it");
        return;
    };
    let s = p.display().to_string();
    let pinned = {
        let mut st = app().st.borrow_mut();
        let list = &mut st.persist.pinned;
        if let Some(i) = list.iter().position(|x| x.eq_ignore_ascii_case(&s)) {
            list.remove(i);
            false
        } else {
            list.insert(0, s);
            true
        }
    };
    app::save_persist();
    set_status(if pinned { "Pinned to Quick Open" } else { "Unpinned" });
}


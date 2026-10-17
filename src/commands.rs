//! Every user action is a command with an id, a title and default keys.
//! The same table drives the menu bar, the command palette and key
//! bindings, so all three always agree. Users can rebind keys in
//! settings.json (`"keys": { "Ctrl+Alt+D": "edit.duplicate" }`).

use crate::app::{self, app, set_status, view, R_ALL};
use crate::gfx::icon;
use crate::keys::Chord;
use crate::languages::LANGS;
use crate::palette::{self, Item, Kind};
use crate::sci_consts::*;
use crate::textenc::{Enc, Eol};
use crate::theme::THEMES;
use crate::util::{self, wide};
use crate::{ai, compare, docs, editor, findbar, history, notes, scratchpad, shell, snippets, textops};
use std::collections::HashMap;
use std::ptr::null;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub struct Cmd {
    pub id: &'static str,
    pub title: &'static str,
    pub keys: &'static [&'static str],
}

macro_rules! cmds {
    ($( $id:literal, $title:literal, [$($k:literal),*] ;)*) => {
        pub static CMDS: &[Cmd] = &[ $( Cmd { id: $id, title: $title, keys: &[$($k),*] } ),* ];
    };
}

cmds! {
    "file.new", "File: New note", ["Ctrl+N"];
    "file.newFromTemplate", "File: New from template…", ["Ctrl+Shift+N"];
    "file.open", "File: Open…", ["Ctrl+O"];
    "file.quickOpen", "Go to file…", ["Ctrl+P", "Ctrl+E"];
    "file.save", "File: Save", ["Ctrl+S"];
    "file.saveAs", "File: Save as…", ["Ctrl+Shift+S"];
    "file.saveAll", "File: Save all", ["Ctrl+Alt+S"];
    "file.reload", "File: Reload from disk", [];
    "file.close", "File: Close tab", ["Ctrl+W", "Ctrl+F4"];
    "file.closeOthers", "File: Close other tabs", [];
    "file.closeSaved", "File: Close saved tabs", [];
    "file.reopenClosed", "File: Reopen closed tab", ["Ctrl+Shift+T"];
    "file.history", "File: Version history…", ["Ctrl+Alt+H"];
    "file.encoding", "File: Encoding…", [];
    "file.lineEndings", "File: Line endings…", [];
    "file.recent", "File: Open recent…", ["Ctrl+R"];
    "file.reveal", "File: Reveal in File Explorer", [];
    "file.copyPath", "File: Copy path", [];
    "file.pin", "File: Pin / unpin in Quick Open", [];
    "file.readonly", "File: Toggle read-only", [];
    "file.exit", "File: Exit", [];
    "edit.undo", "Edit: Undo", ["Ctrl+Z"];
    "edit.redo", "Edit: Redo", ["Ctrl+Y", "Ctrl+Shift+Z"];
    "edit.cut", "Edit: Cut", ["Ctrl+X"];
    "edit.copy", "Edit: Copy", ["Ctrl+C"];
    "edit.paste", "Edit: Paste", ["Ctrl+V"];
    "edit.pasteClean", "Edit: Paste cleaned text", ["Ctrl+Shift+V"];
    "edit.cleanClipboard", "Edit: Clean up clipboard text", [];
    "edit.cleanSelection", "Edit: Clean up selection", [];
    "edit.selectAll", "Edit: Select all", ["Ctrl+A"];
    "edit.selectNext", "Selection: Add next occurrence", ["Ctrl+D"];
    "edit.selectAllOccurrences", "Selection: Select all occurrences", ["Ctrl+Shift+L"];
    "edit.duplicate", "Edit: Duplicate line or selection", ["Ctrl+Shift+D"];
    "edit.deleteLine", "Edit: Delete line", ["Ctrl+Shift+K"];
    "edit.moveUp", "Edit: Move line up", ["Alt+Up"];
    "edit.moveDown", "Edit: Move line down", ["Alt+Down"];
    "edit.toggleComment", "Edit: Toggle comment", ["Ctrl+/"];
    "edit.joinLines", "Edit: Join lines", ["Ctrl+J"];
    "edit.upper", "Text: UPPERCASE", ["Ctrl+Shift+U"];
    "edit.lower", "Text: lowercase", ["Ctrl+U"];
    "edit.title", "Text: Title Case", [];
    "edit.sentence", "Text: Sentence case", [];
    "edit.sortAsc", "Lines: Sort ascending", [];
    "edit.sortDesc", "Lines: Sort descending", [];
    "edit.unique", "Lines: Remove duplicates", [];
    "edit.reverse", "Lines: Reverse order", [];
    "edit.trimTrailing", "Lines: Trim trailing whitespace", [];
    "edit.dedent", "Lines: Remove common indentation", [];
    "edit.unwrap", "Lines: Join wrapped paragraphs", [];
    "edit.formatJson", "JSON: Format", [];
    "edit.minifyJson", "JSON: Minify", [];
    "edit.dateTime", "Edit: Insert date and time", ["F5"];
    "edit.snippet", "Snippets: Insert snippet…", ["Ctrl+Shift+I"];
    "edit.editSnippets", "Snippets: Edit snippets", [];
    "edit.bookmark", "Bookmarks: Toggle", ["Ctrl+F2"];
    "edit.nextBookmark", "Bookmarks: Next", ["F2"];
    "edit.prevBookmark", "Bookmarks: Previous", ["Shift+F2"];
    "find.find", "Find", ["Ctrl+F"];
    "find.replace", "Replace", ["Ctrl+H"];
    "find.next", "Find next", ["F3"];
    "find.prev", "Find previous", ["Shift+F3"];
    "find.notes", "Search all notes…", ["Ctrl+Shift+F"];
    "go.line", "Go to line…", ["Ctrl+G"];
    "view.palette", "Command palette…", ["Ctrl+Shift+P", "F1"];
    "view.wordWrap", "View: Word wrap", ["Alt+Z"];
    "view.lineNumbers", "View: Line numbers", [];
    "view.whitespace", "View: Show whitespace", [];
    "view.currentLine", "View: Highlight current line", [];
    "view.zoomIn", "View: Zoom in", ["Ctrl+=", "Ctrl+NumPlus"];
    "view.zoomOut", "View: Zoom out", ["Ctrl+-", "Ctrl+NumMinus"];
    "view.zoomReset", "View: Reset zoom", ["Ctrl+0"];
    "view.split", "View: Split editor", ["Ctrl+\\"];
    "view.moveToOther", "View: Move tab to other side", ["Ctrl+Alt+\\"];
    "view.focusOther", "View: Focus other side", ["F6"];
    "view.nextTab", "View: Next tab", ["Ctrl+Tab", "Ctrl+PageDown"];
    "view.prevTab", "View: Previous tab", ["Ctrl+Shift+Tab", "Ctrl+PageUp"];
    "view.tab1", "View: Go to tab 1", ["Alt+1"];
    "view.tab2", "View: Go to tab 2", ["Alt+2"];
    "view.tab3", "View: Go to tab 3", ["Alt+3"];
    "view.tab4", "View: Go to tab 4", ["Alt+4"];
    "view.tab5", "View: Go to tab 5", ["Alt+5"];
    "view.tab6", "View: Go to tab 6", ["Alt+6"];
    "view.tab7", "View: Go to tab 7", ["Alt+7"];
    "view.tab8", "View: Go to tab 8", ["Alt+8"];
    "view.tab9", "View: Go to last tab", ["Alt+9"];
    "view.preview", "Markdown: Toggle live preview", ["Ctrl+Shift+M"];
    "view.onTop", "View: Always on top", ["Ctrl+Alt+T"];
    "view.fullscreen", "View: Full screen", ["F11"];
    "view.theme", "Preferences: Theme…", [];
    "view.font", "Preferences: Font…", [];
    "view.language", "View: Language mode…", ["Ctrl+Shift+G"];
    "view.indentation", "View: Indentation…", [];
    "md.bold", "Markdown: Bold", ["Ctrl+B"];
    "md.italic", "Markdown: Italic", ["Ctrl+I"];
    "md.code", "Markdown: Inline code", ["Ctrl+`"];
    "md.link", "Markdown: Insert link", ["Ctrl+K"];
    "md.task", "Markdown: Toggle checkbox", ["Ctrl+Shift+X"];
    "compare.with", "Compare: Compare with…", ["Ctrl+Alt+C"];
    "compare.clipboard", "Compare: With clipboard", [];
    "compare.disk", "Compare: With saved version", [];
    "compare.next", "Compare: Next difference", ["F8"];
    "compare.prev", "Compare: Previous difference", ["Shift+F8"];
    "compare.close", "Compare: Close comparison", [];
    "app.scratchpad", "Scratchpad: Show / hide", ["Ctrl+Alt+N"];
    "app.scratchToTab", "Scratchpad: Move text to a new tab", [];
    "ai.explain", "AI: Explain this (logs, errors, code)", [];
    "ai.summarize", "AI: Summarize", [];
    "ai.rewrite", "AI: Rewrite selection…", [];
    "ai.format", "AI: Format messy notes", [];
    "ai.provider", "AI: Choose provider…", [];
    "ai.setKey", "AI: Set Claude API key…", [];
    "app.settings", "Preferences: Open settings file", ["Ctrl+,"];
    "app.autosave", "Preferences: Auto save…", [];
    "app.restoreSession", "Preferences: Restore session at start-up", [];
    "app.confirmClose", "Preferences: Ask before closing unsaved tabs", [];
    "app.tray", "Preferences: Show tray icon", [];
    "app.closeToTray", "Preferences: Close to tray", [];
    "app.startup", "Preferences: Start with Windows", [];
    "app.explorer", "Preferences: 'Edit with Nib' in File Explorer", [];
    "app.notesFolder", "Notes: Open notes folder", [];
    "app.setNotesFolder", "Notes: Choose notes folder…", [];
    "app.dataFolder", "Preferences: Open data folder", [];
    "app.shortcuts", "Help: Keyboard shortcuts", [];
    "app.about", "Help: About Nib", [];
}

pub fn index_of(id: &str) -> Option<usize> {
    CMDS.iter().position(|c| c.id == id)
}

pub fn run_index(i: usize) {
    if let Some(c) = CMDS.get(i) {
        run(c.id);
    }
}

/// Effective chords for each command (defaults plus settings overrides).
pub fn rebuild_keymap() {
    let a = app();
    let mut map: HashMap<Chord, usize> = HashMap::new();
    for (i, c) in CMDS.iter().enumerate() {
        for k in c.keys {
            if let Some(ch) = Chord::parse(k) {
                map.insert(ch, i);
            }
        }
    }
    for (k, id) in &a.settings.borrow().keys {
        let Some(ch) = Chord::parse(k) else { continue };
        if id.is_empty() {
            map.remove(&ch);
        } else if let Some(i) = index_of(id) {
            map.insert(ch, i);
        }
    }
    *a.keymap.borrow_mut() = map;
}

pub fn shortcut_for(i: usize) -> String {
    let km = app().keymap.borrow();
    let mut chords: Vec<Chord> = km.iter().filter(|(_, &v)| v == i).map(|(k, _)| *k).collect();
    // Prefer the order the defaults list them in.
    let defaults: Vec<Chord> = CMDS[i].keys.iter().filter_map(|k| Chord::parse(k)).collect();
    chords.sort_by_key(|c| defaults.iter().position(|d| d == c).unwrap_or(99));
    chords.first().map(|c| c.display()).unwrap_or_default()
}

pub fn palette_items() -> Vec<Item> {
    CMDS.iter()
        .enumerate()
        .filter(|(_, c)| !matches!(c.id, "edit.undo" | "edit.redo" | "edit.cut" | "edit.copy" | "edit.paste" | "edit.selectAll" | "view.palette"))
        .map(|(i, c)| {
            let mut it = Item::new(icon::COMMAND, c.title, Kind::Command(i)).hint(shortcut_for(i));
            if let Some(on) = checked(c.id) {
                it.detail = if on { "on".into() } else { "off".into() };
            }
            it
        })
        .collect()
}

fn class_name(h: HWND) -> String {
    let mut buf = [0u16; 64];
    let n = unsafe { GetClassNameW(h, buf.as_mut_ptr(), 64) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// Keyboard dispatch, before TranslateMessage. Returns true when consumed.
pub fn pre_translate(msg: &windows_sys::Win32::UI::WindowsAndMessaging::MSG) -> bool {
    if msg.message != WM_KEYDOWN && msg.message != WM_SYSKEYDOWN {
        return false;
    }
    let a = app();
    let main = a.hwnd.get();
    let inside = msg.hwnd == main || unsafe { IsChild(main, msg.hwnd) } != 0;
    if !inside || palette::is_open() {
        return false;
    }
    let vk = msg.wParam as u16;
    if matches!(vk, VK_CONTROL | VK_SHIFT | VK_MENU | VK_LWIN | VK_RWIN) {
        return false;
    }
    let chord = Chord::current(vk);
    let in_edit = class_name(msg.hwnd).eq_ignore_ascii_case("Edit");
    let in_view = (0..2).any(|g| view(g).hwnd == msg.hwnd);
    if in_edit {
        let typing = chord.mods & (crate::keys::M_CTRL | crate::keys::M_ALT) == 0 && !(VK_F1..=VK_F24).contains(&vk);
        let native = chord.mods & crate::keys::M_CTRL != 0 && matches!(vk as u8, b'A' | b'C' | b'V' | b'X' | b'Z' | b'Y') || matches!(vk, VK_LEFT | VK_RIGHT | VK_HOME | VK_END | VK_BACK | VK_DELETE);
        if typing || native {
            return false;
        }
    }
    if in_view && chord.mods == 0 && vk == VK_TAB && snippets::try_expand(&view(app::active_group())) {
        return true;
    }
    if in_view && chord.mods == 0 && vk == VK_ESCAPE {
        if findbar::is_visible() {
            findbar::close();
            return true;
        }
        if compare::active() {
            compare::close();
            return true;
        }
        if is_fullscreen() {
            toggle_fullscreen();
            return true;
        }
        return false;
    }
    let idx = a.keymap.borrow().get(&chord).copied();
    match idx {
        Some(i) => {
            run_index(i);
            true
        }
        None => false,
    }
}

// ------------------------------------------------------------------ toggles

fn checked(id: &str) -> Option<bool> {
    let s = app::settings();
    Some(match id {
        "view.wordWrap" => s.word_wrap,
        "view.lineNumbers" => s.line_numbers,
        "view.whitespace" => s.show_whitespace,
        "view.currentLine" => s.highlight_current_line,
        "view.onTop" => s.always_on_top,
        "view.fullscreen" => is_fullscreen(),
        "app.restoreSession" => s.restore_session,
        "app.confirmClose" => s.confirm_close_dirty,
        "app.tray" => s.tray_icon,
        "app.closeToTray" => s.close_to_tray,
        "app.startup" => crate::settings::autostart::enabled(),
        "app.explorer" => shell::explorer_menu_installed(),
        "view.split" => app().st.borrow().split,
        "view.preview" => {
            let st = app().st.borrow();
            st.doc(st.active_id()).is_some_and(|d| d.preview)
        }
        "file.readonly" => {
            let st = app().st.borrow();
            st.doc(st.active_id()).is_some_and(|d| d.readonly)
        }
        _ => return None,
    })
}

fn toggle_setting(f: impl FnOnce(&mut crate::settings::Settings)) {
    let mut s = app::settings().clone();
    f(&mut s);
    app::apply_settings(s);
}

// ------------------------------------------------------------------ running

pub fn run(id: &str) {
    let g = app::active_group();
    let v = view(g);
    let doc = app::active_doc();
    let is_md = app().st.borrow().doc(doc).is_some_and(|d| d.is_markdown());
    match id {
        "file.new" => {
            docs::new_doc();
        }
        "file.newFromTemplate" => snippets::pick_template(),
        "file.open" => docs::open_dialog(),
        "file.quickOpen" => palette::open_quick(""),
        "file.save" => {
            docs::save(doc);
        }
        "file.saveAs" => {
            docs::save_as(doc);
        }
        "file.saveAll" => docs::save_all(),
        "file.reload" => docs::reload(doc, None),
        "file.close" => {
            docs::close_tab(g, doc);
        }
        "file.closeOthers" => docs::close_others(g, doc),
        "file.closeSaved" => docs::close_saved(g),
        "file.reopenClosed" => docs::reopen_closed(),
        "file.history" => history::show_for(doc),
        "file.encoding" => pick_encoding(doc),
        "file.lineEndings" => pick_eol(doc),
        "file.recent" => pick_recent(),
        "file.reveal" => docs::reveal_active(),
        "file.copyPath" => docs::copy_path(),
        "file.pin" => docs::toggle_pin_file(),
        "file.readonly" => {
            let ro = {
                let mut st = app().st.borrow_mut();
                let Some(d) = st.doc_mut(doc) else { return };
                if d.virt {
                    return;
                }
                d.readonly = !d.readonly;
                d.readonly
            };
            v.m(SCI_SETREADONLY, ro as i64, 0);
            app::invalidate(R_ALL);
        }
        "file.exit" => app::quit(),
        "edit.undo" => {
            v.m(SCI_UNDO, 0, 0);
        }
        "edit.redo" => {
            v.m(SCI_REDO, 0, 0);
        }
        "edit.cut" => {
            v.m(SCI_CUT, 0, 0);
        }
        "edit.copy" => {
            v.m(SCI_COPY, 0, 0);
        }
        "edit.paste" => {
            v.m(SCI_PASTE, 0, 0);
        }
        "edit.selectAll" => {
            v.m(SCI_SELECTALL, 0, 0);
        }
        "edit.pasteClean" => {
            if let Some(text) = util::clipboard_text(app().hwnd.get()) {
                let cleaned = textops::cleanup(&text, &app::settings().cleanup);
                let eol = String::from_utf8_lossy(eol_of(&v)).into_owned();
                v.replace_sel(cleaned.replace('\n', &eol).as_bytes());
                v.m(SCI_SCROLLCARET, 0, 0);
            }
        }
        "edit.cleanClipboard" => {
            if let Some(text) = util::clipboard_text(app().hwnd.get()) {
                let cleaned = textops::cleanup(&text, &app::settings().cleanup).replace('\n', "\r\n");
                util::set_clipboard_text(app().hwnd.get(), &cleaned);
                set_status(&format!("Clipboard cleaned ({} characters)", cleaned.chars().count()));
            }
        }
        "edit.cleanSelection" => {
            let c = app::settings().cleanup.clone();
            editor::transform(&v, true, |s| textops::cleanup(s, &c));
        }
        "edit.selectNext" => editor::select_next_occurrence(&v),
        "edit.selectAllOccurrences" => editor::select_all_occurrences(&v),
        "edit.duplicate" => editor::duplicate(&v),
        "edit.deleteLine" => editor::delete_lines(&v),
        "edit.moveUp" => {
            v.m(SCI_MOVESELECTEDLINESUP, 0, 0);
        }
        "edit.moveDown" => {
            v.m(SCI_MOVESELECTEDLINESDOWN, 0, 0);
        }
        "edit.toggleComment" => {
            let lang = app().st.borrow().doc(doc).map_or(0, |d| d.lang);
            editor::toggle_comment(&v, lang);
        }
        "edit.joinLines" => editor::join_lines(&v),
        "edit.upper" => {
            v.m(SCI_UPPERCASE, 0, 0);
        }
        "edit.lower" => {
            v.m(SCI_LOWERCASE, 0, 0);
        }
        "edit.title" => editor::transform(&v, false, textops::title_case),
        "edit.sentence" => editor::transform(&v, false, textops::sentence_case),
        "edit.sortAsc" => editor::transform(&v, true, |s| textops::sort_lines(s, false, true)),
        "edit.sortDesc" => editor::transform(&v, true, |s| textops::sort_lines(s, true, true)),
        "edit.unique" => editor::transform(&v, true, textops::unique_lines),
        "edit.reverse" => editor::transform(&v, true, textops::reverse_lines),
        "edit.trimTrailing" => editor::transform(&v, true, textops::trim_trailing),
        "edit.dedent" => editor::transform(&v, true, textops::dedent),
        "edit.unwrap" => editor::transform(&v, true, textops::unwrap_paragraphs),
        "edit.formatJson" | "edit.minifyJson" => {
            let text = if v.sel_empty() { v.text_string() } else { String::from_utf8_lossy(&v.selected()).into_owned() };
            let tab = app::settings().tab_width as usize;
            let r = if id == "edit.formatJson" { textops::json_format(&text, tab) } else { textops::json_minify(&text) };
            match r {
                Ok(_) => {
                    editor::transform(&v, false, |s| if id == "edit.formatJson" { textops::json_format(s, tab).unwrap_or_else(|_| s.to_string()) } else { textops::json_minify(s).unwrap_or_else(|_| s.to_string()) });
                    if id == "edit.formatJson" && app().st.borrow().doc(doc).is_some_and(|d| d.lang == 0) {
                        docs::set_language(doc, crate::languages::by_id("json"));
                    }
                }
                Err(e) => set_status(&format!("Not valid JSON: {e}")),
            }
        }
        "edit.dateTime" => {
            let (y, mo, d, h, mi, ..) = util::local_time(util::now_ms());
            v.replace_sel(format!("{y}-{mo:02}-{d:02} {h:02}:{mi:02}").as_bytes());
        }
        "edit.snippet" => snippets::pick_snippet(),
        "edit.editSnippets" => snippets::edit_file(),
        "edit.bookmark" => editor::toggle_bookmark(&v),
        "edit.nextBookmark" => editor::goto_bookmark(&v, true),
        "edit.prevBookmark" => editor::goto_bookmark(&v, false),
        "find.find" => findbar::open(false),
        "find.replace" => findbar::open(true),
        "find.next" => findbar::find_next(true),
        "find.prev" => findbar::find_next(false),
        "find.notes" => {
            let sel = String::from_utf8_lossy(&v.selected()).into_owned();
            let q = if sel.len() < 100 && !sel.contains('\n') { sel } else { String::new() };
            palette::open_search(&q);
        }
        "go.line" => palette::open_quick(":"),
        "view.palette" => palette::open_quick(">"),
        "view.wordWrap" => toggle_setting(|s| s.word_wrap = !s.word_wrap),
        "view.lineNumbers" => toggle_setting(|s| s.line_numbers = !s.line_numbers),
        "view.whitespace" => toggle_setting(|s| s.show_whitespace = !s.show_whitespace),
        "view.currentLine" => toggle_setting(|s| s.highlight_current_line = !s.highlight_current_line),
        "view.zoomIn" => toggle_setting(|s| s.zoom = (s.zoom + 1).min(20)),
        "view.zoomOut" => toggle_setting(|s| s.zoom = (s.zoom - 1).max(-10)),
        "view.zoomReset" => toggle_setting(|s| s.zoom = 0),
        "view.split" => docs::toggle_split(),
        "view.moveToOther" => docs::move_to_other_group(),
        "view.focusOther" => docs::focus_other_group(),
        "view.nextTab" => docs::cycle_tab(1),
        "view.prevTab" => docs::cycle_tab(-1),
        id if id.starts_with("view.tab") => docs::goto_tab(id[8..].parse().unwrap_or(1)),
        "view.preview" => docs::toggle_preview(),
        "view.onTop" => toggle_setting(|s| s.always_on_top = !s.always_on_top),
        "view.fullscreen" => toggle_fullscreen(),
        "view.theme" => pick_theme(),
        "view.font" => choose_font(),
        "view.language" => pick_language(doc),
        "view.indentation" => pick_indentation(),
        "md.bold" | "md.italic" | "md.code" | "md.link" | "md.task" if !is_md => set_status("Markdown formatting works in Markdown documents (View › Language mode)"),
        "md.bold" => editor::toggle_wrap(&v, "**"),
        "md.italic" => editor::toggle_wrap(&v, "*"),
        "md.code" => editor::toggle_wrap(&v, "`"),
        "md.link" => editor::insert_link(&v),
        "md.task" => editor::transform(&v, true, toggle_tasks),
        "compare.with" => compare::pick_target(),
        "compare.clipboard" => compare::with_clipboard(doc),
        "compare.disk" => compare::with_disk(doc),
        "compare.next" => compare::next(true),
        "compare.prev" => compare::next(false),
        "compare.close" => compare::close(),
        "app.scratchpad" => scratchpad::toggle(),
        "app.scratchToTab" => scratchpad::move_to_tab(),
        "ai.explain" => ai::run_task(ai::Task::Explain),
        "ai.summarize" => ai::run_task(ai::Task::Summarize),
        "ai.rewrite" => ai::pick_rewrite(),
        "ai.format" => ai::run_task(ai::Task::Format),
        "ai.provider" => ai::pick_provider(),
        "ai.setKey" => ai::prompt_key(),
        "app.settings" => open_settings_file(),
        "app.autosave" => pick_autosave(),
        "app.restoreSession" => toggle_setting(|s| s.restore_session = !s.restore_session),
        "app.confirmClose" => toggle_setting(|s| s.confirm_close_dirty = !s.confirm_close_dirty),
        "app.tray" => toggle_setting(|s| s.tray_icon = !s.tray_icon),
        "app.closeToTray" => toggle_setting(|s| {
            s.close_to_tray = !s.close_to_tray;
            if s.close_to_tray {
                s.tray_icon = true;
            }
        }),
        "app.startup" => {
            let on = !crate::settings::autostart::enabled();
            if !crate::settings::autostart::set(on) {
                set_status("Couldn't change the start-up setting");
            }
        }
        "app.explorer" => shell::toggle_explorer_menu(),
        "app.notesFolder" => notes::open_notes_folder(),
        "app.setNotesFolder" => notes::choose_notes_folder(),
        "app.dataFolder" => util::shell_open(&app().paths.data.display().to_string()),
        "app.shortcuts" => show_shortcuts(),
        "app.about" => about(),
        _ => {}
    }
}

fn eol_of(v: &crate::sci::Sci) -> &'static [u8] {
    match v.m(SCI_GETEOLMODE, 0, 0) {
        SC_EOL_CRLF => b"\r\n",
        SC_EOL_CR => b"\r",
        _ => b"\n",
    }
}

fn toggle_tasks(s: &str) -> String {
    s.split('\n')
        .map(|l| {
            let indent = &l[..l.len() - l.trim_start().len()];
            let t = l.trim_start();
            if let Some(r) = t.strip_prefix("- [ ] ") {
                format!("{indent}- [x] {r}")
            } else if let Some(r) = t.strip_prefix("- [x] ").or_else(|| t.strip_prefix("- [X] ")) {
                format!("{indent}- [ ] {r}")
            } else if let Some(r) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
                format!("{indent}- [ ] {r}")
            } else if t.is_empty() {
                l.to_string()
            } else {
                format!("{indent}- [ ] {t}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn open_settings_file() {
    let a = app();
    let path = a.paths.settings();
    if !path.exists() {
        let _ = a.settings.borrow().save(&path);
        a.settings_stamp.set(crate::fileio::stamp(&path));
    }
    docs::open_path(&path, None);
    set_status("Edit and save to apply. Command ids for \"keys\" are listed under Help › Keyboard shortcuts.");
}

// ------------------------------------------------------------------ pickers

fn pick_theme() {
    let current = app::settings().theme.clone();
    let items: Vec<Item> = THEMES.iter().enumerate().map(|(i, (id, name))| Item::new(icon::SETTINGS, *name, Kind::Index(i)).hint(if *id == current { "current" } else { "" })).collect();
    let sel = THEMES.iter().position(|(id, _)| *id == current).unwrap_or(0);
    palette::pick_selected("Choose a theme", items, sel, |i| {
        let id = THEMES[i].0.to_string();
        toggle_setting(|s| s.theme = id);
    });
}

fn pick_language(doc: u64) {
    let cur = app().st.borrow().doc(doc).map_or(0, |d| d.lang);
    let items: Vec<Item> = LANGS.iter().enumerate().map(|(i, l)| Item::new(icon::CODE, l.name, Kind::Index(i)).detail(l.exts.iter().take(4).map(|e| format!(".{e}")).collect::<Vec<_>>().join(" "))).collect();
    palette::pick_selected("Language mode", items, cur, move |i| docs::set_language(doc, i));
}

fn pick_encoding(doc: u64) {
    let has_path = app().st.borrow().doc(doc).is_some_and(|d| d.path.is_some());
    let mut items = Vec::new();
    let encs = Enc::all();
    for (i, e) in encs.iter().enumerate() {
        items.push(Item::new(icon::SAVE_ICON, format!("Save with {}", e.label()), Kind::Index(i)));
    }
    if has_path {
        for (i, e) in encs.iter().enumerate() {
            items.push(Item::new(icon::DOCUMENT, format!("Reopen with {}", e.label()), Kind::Index(1000 + i)));
        }
    }
    palette::pick("Save with / reopen with encoding", items, true, move |i| {
        let encs = Enc::all();
        if i >= 1000 {
            docs::reopen_with_encoding(doc, encs[i - 1000]);
        } else {
            docs::save_with_encoding(doc, encs[i]);
        }
    });
}

fn pick_eol(doc: u64) {
    let opts = [(Eol::Crlf, "CRLF (Windows)"), (Eol::Lf, "LF (Unix, macOS)"), (Eol::Cr, "CR (classic Mac)")];
    let items = opts.iter().enumerate().map(|(i, (_, l))| Item::new(icon::DOCUMENT, *l, Kind::Index(i))).collect();
    palette::pick("Convert line endings to…", items, true, move |i| docs::convert_eol(doc, opts[i].0));
}

fn pick_indentation() {
    let opts: Vec<(bool, u32, &str)> = vec![(true, 4, "Spaces: 4"), (true, 2, "Spaces: 2"), (true, 8, "Spaces: 8"), (false, 4, "Tabs, width 4"), (false, 8, "Tabs, width 8")];
    let items = opts.iter().enumerate().map(|(i, (_, _, l))| Item::new(icon::SETTINGS, *l, Kind::Index(i))).collect();
    palette::pick("Indentation", items, true, move |i| {
        let (spaces, w, _) = opts[i];
        toggle_setting(|s| {
            s.insert_spaces = spaces;
            s.tab_width = w;
        });
    });
}

fn pick_autosave() {
    let cur = app::settings().autosave.clone();
    let modes = crate::settings::AUTOSAVE_MODES;
    let items = modes.iter().enumerate().map(|(i, (id, l))| Item::new(icon::SAVE_ICON, *l, Kind::Index(i)).hint(if *id == cur { "current" } else { "" })).collect();
    palette::pick("Auto save to the file (unsaved work is always kept in the session)", items, true, |i| {
        let id = crate::settings::AUTOSAVE_MODES[i].0.to_string();
        toggle_setting(|s| s.autosave = id);
    });
}

fn pick_recent() {
    let list = app().st.borrow().persist.recent.clone();
    let items = list
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let pb = std::path::Path::new(p);
            Item::new(icon::CLOCK, pb.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), Kind::Index(i)).detail(pb.parent().map(|d| d.display().to_string()).unwrap_or_default())
        })
        .collect();
    palette::pick("Open recent", items, true, move |i| {
        docs::open_path(std::path::Path::new(&list[i]), None);
    });
}

fn choose_font() {
    use windows_sys::Win32::Graphics::Gdi::LOGFONTW;
    use windows_sys::Win32::UI::Controls::Dialogs::{ChooseFontW, CF_FIXEDPITCHONLY, CF_INITTOLOGFONTSTRUCT, CF_NOVERTFONTS, CF_SCREENFONTS, CHOOSEFONTW};
    let s = app::settings().clone();
    let mut lf: LOGFONTW = unsafe { std::mem::zeroed() };
    let face = if s.font.is_empty() { editor::default_font().to_string() } else { s.font.clone() };
    for (i, c) in face.encode_utf16().take(31).enumerate() {
        lf.lfFaceName[i] = c;
    }
    let dpi = app::dpi();
    lf.lfHeight = -((s.font_size * dpi as f32 / 72.0).round() as i32);
    let mut cf: CHOOSEFONTW = unsafe { std::mem::zeroed() };
    cf.lStructSize = std::mem::size_of::<CHOOSEFONTW>() as u32;
    cf.hwndOwner = app().hwnd.get();
    cf.lpLogFont = &mut lf;
    cf.Flags = CF_SCREENFONTS | CF_INITTOLOGFONTSTRUCT | CF_NOVERTFONTS;
    let _ = CF_FIXEDPITCHONLY;
    if unsafe { ChooseFontW(&mut cf) } != 0 {
        let len = lf.lfFaceName.iter().position(|&c| c == 0).unwrap_or(32);
        let face = String::from_utf16_lossy(&lf.lfFaceName[..len]);
        let size = cf.iPointSize as f32 / 10.0;
        toggle_setting(|s| {
            s.font = face;
            s.font_size = size;
        });
    }
}

// ------------------------------------------------------------------ full screen

thread_local! {
    static FULLSCREEN: std::cell::Cell<Option<(i32, windows_sys::Win32::Foundation::RECT)>> = const { std::cell::Cell::new(None) };
}

fn is_fullscreen() -> bool {
    FULLSCREEN.with(|f| f.get().is_some())
}

fn toggle_fullscreen() {
    let hwnd = app().hwnd.get();
    unsafe {
        match FULLSCREEN.with(|f| f.take()) {
            Some((style, r)) => {
                SetWindowLongW(hwnd, GWL_STYLE, style);
                SetMenu(hwnd, MENU.with(|m| m.get()));
                SetWindowPos(hwnd, std::ptr::null_mut(), r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_FRAMECHANGED);
            }
            None => {
                let style = GetWindowLongW(hwnd, GWL_STYLE);
                let mut r = std::mem::zeroed();
                GetWindowRect(hwnd, &mut r);
                FULLSCREEN.with(|f| f.set(Some((style, r))));
                let mon = windows_sys::Win32::Graphics::Gdi::MonitorFromWindow(hwnd, windows_sys::Win32::Graphics::Gdi::MONITOR_DEFAULTTONEAREST);
                let mut mi: windows_sys::Win32::Graphics::Gdi::MONITORINFO = std::mem::zeroed();
                mi.cbSize = std::mem::size_of::<windows_sys::Win32::Graphics::Gdi::MONITORINFO>() as u32;
                windows_sys::Win32::Graphics::Gdi::GetMonitorInfoW(mon, &mut mi);
                SetWindowLongW(hwnd, GWL_STYLE, style & !(WS_OVERLAPPEDWINDOW as i32));
                SetMenu(hwnd, std::ptr::null_mut());
                let m = mi.rcMonitor;
                SetWindowPos(hwnd, HWND_TOP, m.left, m.top, m.right - m.left, m.bottom - m.top, SWP_FRAMECHANGED);
                set_status("Full screen — press F11 or Esc to leave");
            }
        }
    }
}

// ------------------------------------------------------------------ menus

enum M {
    C(&'static str),
    Sep,
    Sub(&'static str, &'static [M]),
    Recent,
    Themes,
}

static MENU_BAR: &[(&str, &[M])] = &[
    (
        "&File",
        &[
            M::C("file.new"),
            M::C("file.newFromTemplate"),
            M::C("file.open"),
            M::C("file.quickOpen"),
            M::Recent,
            M::Sep,
            M::C("file.save"),
            M::C("file.saveAs"),
            M::C("file.saveAll"),
            M::C("app.autosave"),
            M::Sep,
            M::C("file.history"),
            M::C("file.reload"),
            M::C("file.encoding"),
            M::C("file.lineEndings"),
            M::C("file.readonly"),
            M::Sep,
            M::C("file.reveal"),
            M::C("file.copyPath"),
            M::C("file.pin"),
            M::Sep,
            M::C("file.close"),
            M::C("file.closeOthers"),
            M::C("file.reopenClosed"),
            M::Sep,
            M::C("file.exit"),
        ],
    ),
    (
        "&Edit",
        &[
            M::C("edit.undo"),
            M::C("edit.redo"),
            M::Sep,
            M::C("edit.cut"),
            M::C("edit.copy"),
            M::C("edit.paste"),
            M::C("edit.pasteClean"),
            M::C("edit.cleanClipboard"),
            M::Sep,
            M::C("edit.selectAll"),
            M::C("edit.selectNext"),
            M::C("edit.selectAllOccurrences"),
            M::Sep,
            M::Sub(
                "&Line",
                &[M::C("edit.duplicate"), M::C("edit.deleteLine"), M::C("edit.moveUp"), M::C("edit.moveDown"), M::C("edit.joinLines"), M::Sep, M::C("edit.sortAsc"), M::C("edit.sortDesc"), M::C("edit.unique"), M::C("edit.reverse"), M::C("edit.trimTrailing"), M::C("edit.dedent"), M::C("edit.unwrap")],
            ),
            M::Sub("&Text", &[M::C("edit.upper"), M::C("edit.lower"), M::C("edit.title"), M::C("edit.sentence"), M::Sep, M::C("edit.cleanSelection"), M::C("edit.formatJson"), M::C("edit.minifyJson")]),
            M::C("edit.toggleComment"),
            M::Sep,
            M::C("edit.dateTime"),
            M::C("edit.snippet"),
            M::C("edit.editSnippets"),
            M::Sep,
            M::C("edit.bookmark"),
            M::C("edit.nextBookmark"),
            M::C("edit.prevBookmark"),
        ],
    ),
    ("F&ind", &[M::C("find.find"), M::C("find.replace"), M::C("find.next"), M::C("find.prev"), M::Sep, M::C("find.notes"), M::C("go.line")]),
    (
        "&View",
        &[
            M::C("view.palette"),
            M::Sep,
            M::Themes,
            M::C("view.font"),
            M::C("view.language"),
            M::C("view.indentation"),
            M::Sep,
            M::C("view.wordWrap"),
            M::C("view.lineNumbers"),
            M::C("view.whitespace"),
            M::C("view.currentLine"),
            M::Sep,
            M::C("view.zoomIn"),
            M::C("view.zoomOut"),
            M::C("view.zoomReset"),
            M::Sep,
            M::C("view.split"),
            M::C("view.moveToOther"),
            M::C("view.preview"),
            M::Sep,
            M::C("view.onTop"),
            M::C("view.fullscreen"),
        ],
    ),
    (
        "&Tools",
        &[
            M::C("app.scratchpad"),
            M::C("app.scratchToTab"),
            M::Sep,
            M::Sub("&Compare", &[M::C("compare.with"), M::C("compare.clipboard"), M::C("compare.disk"), M::Sep, M::C("compare.next"), M::C("compare.prev"), M::C("compare.close")]),
            M::Sub("&Markdown", &[M::C("view.preview"), M::Sep, M::C("md.bold"), M::C("md.italic"), M::C("md.code"), M::C("md.link"), M::C("md.task")]),
            M::Sub("&AI (optional)", &[M::C("ai.explain"), M::C("ai.summarize"), M::C("ai.rewrite"), M::C("ai.format"), M::Sep, M::C("ai.provider"), M::C("ai.setKey")]),
            M::Sep,
            M::C("app.notesFolder"),
            M::C("app.setNotesFolder"),
            M::Sep,
            M::Sub("&Preferences", &[M::C("app.settings"), M::Sep, M::C("app.restoreSession"), M::C("app.confirmClose"), M::C("app.tray"), M::C("app.closeToTray"), M::C("app.startup"), M::C("app.explorer"), M::Sep, M::C("app.dataFolder")]),
        ],
    ),
    ("&Help", &[M::C("app.shortcuts"), M::C("app.about")]),
];

const ID_BASE: usize = 1000;
const ID_RECENT: usize = 5000;
const ID_THEME: usize = 6000;

thread_local! {
    static MENU: std::cell::Cell<HMENU> = const { std::cell::Cell::new(std::ptr::null_mut()) };
    static RECENT_MENU: std::cell::Cell<HMENU> = const { std::cell::Cell::new(std::ptr::null_mut()) };
    static THEME_MENU: std::cell::Cell<HMENU> = const { std::cell::Cell::new(std::ptr::null_mut()) };
}

fn menu_label(i: usize) -> String {
    let t = CMDS[i].title;
    let short = t.split_once(": ").map(|(_, r)| r).unwrap_or(t);
    let mut s = short.to_string();
    if let Some(c) = s.get(0..1) {
        s = format!("{}{}", c.to_uppercase(), &s[1..]);
    }
    s.replace('&', "&&")
}

fn build(items: &[M]) -> HMENU {
    unsafe {
        let m = CreatePopupMenu();
        for it in items {
            match it {
                M::C(id) => {
                    if let Some(i) = index_of(id) {
                        let w = wide(&menu_label(i));
                        AppendMenuW(m, MF_STRING, ID_BASE + i, w.as_ptr());
                    }
                }
                M::Sep => {
                    AppendMenuW(m, MF_SEPARATOR, 0, null());
                }
                M::Sub(name, sub) => {
                    let w = wide(name);
                    AppendMenuW(m, MF_POPUP, build(sub) as usize, w.as_ptr());
                }
                M::Recent => {
                    let sub = CreatePopupMenu();
                    RECENT_MENU.with(|r| r.set(sub));
                    AppendMenuW(m, MF_POPUP, sub as usize, windows_sys::w!("Open &recent"));
                }
                M::Themes => {
                    let sub = CreatePopupMenu();
                    for (i, (_, name)) in THEMES.iter().enumerate() {
                        let w = wide(name);
                        AppendMenuW(sub, MF_STRING, ID_THEME + i, w.as_ptr());
                    }
                    THEME_MENU.with(|r| r.set(sub));
                    AppendMenuW(m, MF_POPUP, sub as usize, windows_sys::w!("&Theme"));
                }
            }
        }
        m
    }
}

pub fn build_menu() -> HMENU {
    unsafe {
        let bar = CreateMenu();
        for (name, items) in MENU_BAR {
            let w = wide(name);
            AppendMenuW(bar, MF_POPUP, build(items) as usize, w.as_ptr());
        }
        MENU.with(|m| m.set(bar));
        bar
    }
}

/// Refreshes shortcuts, check marks and the recent-files list as a menu opens.
pub fn init_menu_popup(m: HMENU) {
    unsafe {
        if RECENT_MENU.with(|r| r.get()) == m {
            while GetMenuItemCount(m) > 0 {
                DeleteMenu(m, 0, MF_BYPOSITION);
            }
            let recent = app().st.borrow().persist.recent.clone();
            if recent.is_empty() {
                AppendMenuW(m, MF_STRING | MF_GRAYED, 0, windows_sys::w!("No recent files"));
            }
            for (i, p) in recent.iter().take(15).enumerate() {
                let w = wide(&format!("&{} {}", (i + 1) % 10, p.replace('&', "&&")));
                AppendMenuW(m, MF_STRING, ID_RECENT + i, w.as_ptr());
            }
            return;
        }
        if THEME_MENU.with(|r| r.get()) == m {
            let cur = app::settings().theme.clone();
            for (i, (id, _)) in THEMES.iter().enumerate() {
                CheckMenuItem(m, (ID_THEME + i) as u32, if *id == cur { MF_CHECKED } else { MF_UNCHECKED });
            }
            return;
        }
        let n = GetMenuItemCount(m);
        for pos in 0..n {
            let id = GetMenuItemID(m, pos);
            if id == u32::MAX || (id as usize) < ID_BASE || id as usize >= ID_BASE + CMDS.len() {
                continue;
            }
            let i = id as usize - ID_BASE;
            let key = shortcut_for(i);
            let label = if key.is_empty() { menu_label(i) } else { format!("{}\t{key}", menu_label(i)) };
            let w = wide(&label);
            ModifyMenuW(m, pos as u32, MF_BYPOSITION | MF_STRING, id as usize, w.as_ptr());
            if let Some(on) = checked(CMDS[i].id) {
                CheckMenuItem(m, id, if on { MF_CHECKED } else { MF_UNCHECKED });
            }
            let enabled = match CMDS[i].id {
                "compare.next" | "compare.prev" | "compare.close" => compare::active(),
                _ => true,
            };
            EnableMenuItem(m, id, if enabled { MF_ENABLED } else { MF_GRAYED });
        }
    }
}

pub fn on_menu(id: usize) {
    if (ID_BASE..ID_BASE + CMDS.len()).contains(&id) {
        run_index(id - ID_BASE);
    } else if (ID_RECENT..ID_RECENT + 100).contains(&id) {
        let p = app().st.borrow().persist.recent.get(id - ID_RECENT).cloned();
        if let Some(p) = p {
            docs::open_path(std::path::Path::new(&p), None);
        }
    } else if (ID_THEME..ID_THEME + THEMES.len()).contains(&id) {
        let t = THEMES[id - ID_THEME].0.to_string();
        toggle_setting(|s| s.theme = t);
    }
}

// ------------------------------------------------------------------ help

fn show_shortcuts() {
    let mut out = String::from("# Nib keyboard shortcuts\n\nRebind any command in settings.json, e.g. `\"keys\": { \"Ctrl+Alt+D\": \"edit.duplicate\" }`.\nAn empty id (`\"Ctrl+D\": \"\"`) removes a binding.\n\n| Command | Shortcut | Id |\n|---|---|---|\n");
    for (i, c) in CMDS.iter().enumerate() {
        out.push_str(&format!("| {} | {} | `{}` |\n", c.title, shortcut_for(i).replace('|', "\\|"), c.id));
    }
    out.push_str(&format!("\nGlobal: **{}** shows the floating scratchpad from any app.\n", app::settings().scratchpad_hotkey));
    out.push_str("\nIn Quick Open (Ctrl+P): type `>` for commands, `:` for a line number, `#` to search inside notes.\n");
    let id = docs::new_virtual("Keyboard shortcuts", out.into_bytes(), crate::languages::by_id("markdown"));
    let g = app::active_group();
    {
        let mut st = app().st.borrow_mut();
        if let Some(d) = st.doc_mut(id) {
            d.preview = true;
        }
    }
    docs::activate(g, id);
    app::layout();
}

/// "567" -> "5.6.7" (Scintilla's version.txt format).
fn dotted(v: &str) -> String {
    v.trim().chars().map(String::from).collect::<Vec<_>>().join(".")
}

fn about() {
    let a = app();
    let mode = if a.paths.portable { "portable" } else { "installed" };
    util::message_box(
        a.hwnd.get(),
        &format!(
            "Nib {}\nA fast, private, keyboard-first text editor for Windows.\n\nData folder ({mode}): {}\n\nEditing component: Scintilla {} and Lexilla {} (Neil Hodgson, HPND-style licence).\nNo account, no telemetry. AI features are off unless you turn them on.",
            env!("CARGO_PKG_VERSION"),
            a.paths.data.display(),
            dotted(include_str!("../vendor/scintilla/version.txt")),
            dotted(include_str!("../vendor/lexilla/version.txt"))
        ),
        MB_OK | MB_ICONINFORMATION,
    );
}

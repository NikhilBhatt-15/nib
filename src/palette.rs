//! One popup for every keyboard picker, in the style of Sublime Text and
//! VS Code:
//!
//! * Quick Open (Ctrl+P): open tabs, pinned and recent files, and every file
//!   in the notes folder and search roots, fuzzy-matched by name.
//!   `>` switches to commands, `:` goes to a line, `#` searches note contents.
//! * Command palette (Ctrl+Shift+P).
//! * Generic pickers (theme, language, encoding, snippets, versions...).
//! * Text prompts (snippet names, API key...).
//!
//! Keyboard focus stays in the text box; navigation keys are intercepted in
//! `pre_translate` before they reach it.

use crate::app::{self, app, px};
use crate::fuzzy;
use crate::gfx::{self, contains, icon, rect, Canvas};
use crate::util::{self, wide, wide_nz};
use crate::{commands, docs, notes};
use std::cell::RefCell;
use std::mem::{size_of, zeroed};
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use windows_sys::w;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{EM_GETSEL, EM_REPLACESEL, EM_SETCUEBANNER, EM_SETSEL, WM_MOUSELEAVE};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const EM_SETPASSWORDCHAR: u32 = 0x00CC;
const MAX_ROWS: usize = 12;

#[derive(Clone)]
pub enum Kind {
    Doc(u64),
    Path(PathBuf),
    Command(usize),
    Line(usize),
    /// A content-search hit: file or open doc, line, byte range in the line.
    Hit { path: Option<PathBuf>, doc: u64, line: usize, col: usize, len: usize },
    Index(usize),
}

#[derive(Clone)]
pub struct Item {
    pub icon: &'static str,
    pub title: String,
    pub detail: String,
    pub hint: String,
    pub kind: Kind,
}

impl Item {
    pub fn new(icon: &'static str, title: impl Into<String>, kind: Kind) -> Item {
        Item { icon, title: title.into(), detail: String::new(), hint: String::new(), kind }
    }
    pub fn detail(mut self, d: impl Into<String>) -> Item {
        self.detail = d.into();
        self
    }
    pub fn hint(mut self, h: impl Into<String>) -> Item {
        self.hint = h.into();
        self
    }
}

type PickFn = Box<dyn Fn(usize)>;
type SubmitFn = Box<dyn Fn(String)>;

enum Mode {
    Quick,
    Pick { items: Vec<Item>, on_pick: Option<PickFn>, keep_order: bool },
    Prompt { hint: String, on_submit: Option<SubmitFn> },
}

struct Row {
    item: Item,
    hl: Vec<usize>,
}

struct State {
    hwnd: HWND,
    edit: HWND,
    mode: Mode,
    rows: Vec<Row>,
    sel: usize,
    scroll: usize,
    hover: Option<usize>,
    edit_brush: HBRUSH,
    note_gen: u64,
    note_query: String,
    message: String,
    /// Items for Quick Open, gathered once per opening.
    quick_files: Vec<Item>,
}

thread_local! {
    static ST: RefCell<State> = const { RefCell::new(State {
        hwnd: null_mut(),
        edit: null_mut(),
        mode: Mode::Quick,
        rows: Vec::new(),
        sel: 0,
        scroll: 0,
        hover: None,
        edit_brush: null_mut(),
        note_gen: 0,
        note_query: String::new(),
        message: String::new(),
        quick_files: Vec::new(),
    }) };
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    ST.with(|s| f(&mut s.borrow_mut()))
}

fn ensure_window() {
    if !with(|s| s.hwnd.is_null()) {
        return;
    }
    unsafe {
        let hinst = GetModuleHandleW(null());
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            lpszClassName: w!("NibPalette"),
            ..zeroed()
        };
        RegisterClassExW(&wc);
        let hwnd = CreateWindowExW(WS_EX_TOOLWINDOW, w!("NibPalette"), null(), WS_POPUP | WS_BORDER, 0, 0, 600, 400, app().hwnd.get(), null_mut(), hinst, null());
        let edit = CreateWindowExW(0, w!("EDIT"), null(), WS_CHILD | WS_VISIBLE | ES_AUTOHSCROLL as u32, 0, 0, 100, 20, hwnd, null_mut(), hinst, null());
        with(|s| {
            s.hwnd = hwnd;
            s.edit = edit;
        });
        apply_theme();
    }
}

pub fn apply_theme() {
    let (hwnd, edit) = with(|s| (s.hwnd, s.edit));
    if hwnd.is_null() {
        return;
    }
    let t = app::theme();
    unsafe {
        gfx::style_frame(hwnd, &t, false);
        let brush = CreateSolidBrush(t.surface.colorref());
        let old = with(|s| std::mem::replace(&mut s.edit_brush, brush));
        if !old.is_null() {
            DeleteObject(old);
        }
        SendMessageW(edit, WM_SETFONT, app().fonts.borrow().ui as WPARAM, 1);
        InvalidateRect(hwnd, null(), 1);
    }
}

pub fn is_open() -> bool {
    with(|s| !s.hwnd.is_null() && unsafe { IsWindowVisible(s.hwnd) } != 0)
}

fn edit_text() -> String {
    let edit = with(|s| s.edit);
    unsafe {
        let len = GetWindowTextLengthW(edit);
        let mut buf = vec![0u16; len as usize + 1];
        GetWindowTextW(edit, buf.as_mut_ptr(), len + 1);
        String::from_utf16_lossy(&buf[..len as usize])
    }
}

fn set_edit_text(s: &str) {
    let edit = with(|st| st.edit);
    let w = wide(s);
    unsafe {
        SetWindowTextW(edit, w.as_ptr());
        let n = s.encode_utf16().count();
        SendMessageW(edit, EM_SETSEL, n, n as LPARAM);
    }
}

fn show(initial: &str, placeholder: &str, password: bool) {
    ensure_window();
    let a = app();
    let (hwnd, edit) = with(|s| (s.hwnd, s.edit));
    let fonts = a.fonts.borrow();
    unsafe {
        let mut rc: RECT = zeroed();
        GetWindowRect(a.hwnd.get(), &mut rc);
        let mut client_origin = POINT { x: 0, y: 0 };
        ClientToScreen(a.hwnd.get(), &mut client_origin);
        let w = px(640).min(rc.right - rc.left - px(40));
        let row_h = fonts.ui_h + px(14);
        let h = px(16) + fonts.ui_h + px(16) + px(8) + row_h * MAX_ROWS as i32 + px(10);
        let x = rc.left + (rc.right - rc.left - w) / 2;
        let y = client_origin.y + px(6);
        SetWindowPos(hwnd, HWND_TOP, x, y, w, h, SWP_NOACTIVATE);
        let cue = wide(placeholder);
        SendMessageW(edit, EM_SETCUEBANNER, 1, cue.as_ptr() as LPARAM);
        SendMessageW(edit, EM_SETPASSWORDCHAR, if password { '●' as usize } else { 0 }, 0);
        SendMessageW(edit, WM_SETFONT, fonts.ui as WPARAM, 1);
    }
    drop(fonts);
    layout_edit();
    with(|s| {
        s.sel = 0;
        s.scroll = 0;
        s.hover = None;
        s.message.clear();
    });
    set_edit_text(initial);
    refresh();
    unsafe {
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        SetFocus(edit);
    }
}

fn layout_edit() {
    let (hwnd, edit) = with(|s| (s.hwnd, s.edit));
    let fonts = app().fonts.borrow();
    unsafe {
        let mut rc: RECT = zeroed();
        GetClientRect(hwnd, &mut rc);
        let top = px(8) + px(8);
        MoveWindow(edit, px(40), top, rc.right - px(56), fonts.ui_h + px(2), 1);
    }
}

pub fn hide() {
    let hwnd = with(|s| s.hwnd);
    if hwnd.is_null() {
        return;
    }
    unsafe {
        ShowWindow(hwnd, SW_HIDE);
        let main = app().hwnd.get();
        if GetForegroundWindow() == hwnd || GetForegroundWindow() == main {
            SetForegroundWindow(main);
        }
        SetFocus(app::active_view().hwnd);
    }
    with(|s| {
        s.mode = Mode::Quick;
        s.rows.clear();
        s.quick_files.clear();
    });
}

// ------------------------------------------------------------------ public entry points

pub fn open_quick(prefix: &str) {
    with(|s| {
        s.mode = Mode::Quick;
        s.quick_files = gather_files();
    });
    show(prefix, "Search files by name  ·  > commands  ·  : line  ·  # search in notes", false);
}

pub fn pick(placeholder: &str, items: Vec<Item>, keep_order: bool, on_pick: impl Fn(usize) + 'static) {
    with(|s| s.mode = Mode::Pick { items, on_pick: Some(Box::new(on_pick)), keep_order });
    show("", placeholder, false);
}

/// Like `pick`, with the list pre-selected at `selected`.
pub fn pick_selected(placeholder: &str, items: Vec<Item>, selected: usize, on_pick: impl Fn(usize) + 'static) {
    pick(placeholder, items, true, on_pick);
    with(|s| {
        s.sel = selected.min(s.rows.len().saturating_sub(1));
        s.scroll = s.sel.saturating_sub(MAX_ROWS / 2);
    });
    invalidate();
}

pub fn prompt(placeholder: &str, initial: &str, password: bool, on_submit: impl Fn(String) + 'static) {
    with(|s| s.mode = Mode::Prompt { hint: placeholder.into(), on_submit: Some(Box::new(on_submit)) });
    show(initial, placeholder, password);
}

/// Content search across notes, open tabs and search roots.
pub fn open_search(initial: &str) {
    open_quick(&format!("#{initial}"));
}

// ------------------------------------------------------------------ filtering

fn gather_files() -> Vec<Item> {
    let a = app();
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    {
        let st = a.st.borrow();
        let g = st.active_group;
        let mut order: Vec<u64> = st.groups[g].mru.clone();
        for grp in &st.groups {
            for id in &grp.tabs {
                if !order.contains(id) {
                    order.push(*id);
                }
            }
        }
        for id in order {
            if let Some(d) = st.doc(id) {
                if let Some(p) = &d.path {
                    seen.insert(p.display().to_string().to_lowercase());
                }
                let item = Item::new(icon::DOCUMENT, d.name(), Kind::Doc(id)).detail(d.dir_hint()).hint(if d.dirty { "open · unsaved" } else { "open" });
                out.push(item);
            }
        }
        for (list, label, ic) in [(&st.persist.pinned, "pinned", icon::PIN), (&st.persist.recent, "recent", icon::CLOCK)] {
            for p in list {
                if seen.insert(p.to_lowercase()) {
                    let pb = PathBuf::from(p);
                    let name = pb.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.clone());
                    let dir = pb.parent().map(|d| d.display().to_string()).unwrap_or_default();
                    out.push(Item::new(ic, name, Kind::Path(pb)).detail(dir).hint(label));
                }
            }
        }
    }
    for p in notes::indexed_files() {
        let key = p.display().to_string().to_lowercase();
        if seen.insert(key) {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let dir = p.parent().map(|d| d.display().to_string()).unwrap_or_default();
            out.push(Item::new(icon::NOTE, name, Kind::Path(p)).detail(dir));
        }
    }
    out
}

fn score_items(items: &[Item], q: &str, keep_order: bool, cap: usize) -> Vec<Row> {
    if q.trim().is_empty() {
        return items.iter().take(cap).map(|i| Row { item: i.clone(), hl: Vec::new() }).collect();
    }
    let mut scored: Vec<(i32, usize, Vec<usize>)> = Vec::new();
    for (i, it) in items.iter().enumerate() {
        if let Some((sc, hl)) = fuzzy::score(q, &it.title) {
            scored.push((sc + 20, i, hl));
        } else if let Some((sc, _)) = fuzzy::score(q, &format!("{} {}", it.detail, it.title)) {
            scored.push((sc - 10, i, Vec::new()));
        }
    }
    if !keep_order || !q.is_empty() {
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    }
    scored.into_iter().take(cap).map(|(_, i, hl)| Row { item: items[i].clone(), hl }).collect()
}

fn refresh() {
    let q = edit_text();
    enum Next {
        Rows(Vec<Row>, String),
        Search(String),
    }
    let next = with(|s| match &s.mode {
        Mode::Quick => {
            if let Some(rest) = q.strip_prefix('>') {
                let items: Vec<Item> = commands::palette_items();
                Next::Rows(score_items(&items, rest.trim(), true, 400), String::new())
            } else if let Some(rest) = q.strip_prefix(':') {
                let v = app::active_view();
                let total = v.line_count();
                let rows = match rest.trim().parse::<usize>() {
                    Ok(n) if n >= 1 => vec![Row { item: Item::new(icon::DOWN, format!("Go to line {n}"), Kind::Line(n.min(total))).detail(format!("of {total}")), hl: Vec::new() }],
                    _ => Vec::new(),
                };
                Next::Rows(rows, format!("Type a line number between 1 and {total}"))
            } else if let Some(rest) = q.strip_prefix('#') {
                Next::Search(rest.to_string())
            } else {
                Next::Rows(score_items(&s.quick_files, q.trim(), true, 300), String::new())
            }
        }
        Mode::Pick { items, keep_order, .. } => Next::Rows(score_items(items, q.trim(), *keep_order, 2000), String::new()),
        Mode::Prompt { hint, .. } => Next::Rows(Vec::new(), format!("{hint} — press Enter")),
    });
    match next {
        Next::Rows(rows, msg) => with(|s| {
            s.rows = rows;
            s.message = msg;
            s.sel = s.sel.min(s.rows.len().saturating_sub(1));
            if !matches!(s.mode, Mode::Pick { .. }) || !q.trim().is_empty() {
                s.sel = 0;
                s.scroll = 0;
            }
        }),
        Next::Search(query) => {
            let gen = with(|s| {
                s.note_gen += 1;
                s.note_query = query.clone();
                if query.trim().is_empty() {
                    s.rows.clear();
                    s.message = "Type to search inside your notes, open tabs and search folders (regex: /pattern/)".into();
                } else {
                    s.message = "Searching…".into();
                }
                s.note_gen
            });
            if !query.trim().is_empty() {
                notes::search(query, gen);
            }
        }
    }
    invalidate();
}

/// Results from the notes search thread.
pub fn set_search_results(gen: u64, items: Vec<Item>, done_msg: String) {
    with(|s| {
        if gen != s.note_gen {
            return;
        }
        s.rows = items.into_iter().map(|item| Row { item, hl: Vec::new() }).collect();
        s.sel = 0;
        s.scroll = 0;
        s.message = done_msg;
    });
    invalidate();
}

/// Repaints, shrinking or growing the popup to fit its rows.
fn invalidate() {
    let (hwnd, rows, has_msg) = with(|s| (s.hwnd, s.rows.len(), !s.message.is_empty()));
    if hwnd.is_null() {
        return;
    }
    let fonts = app().fonts.borrow();
    let row_h = fonts.ui_h + px(14);
    let shown = rows.clamp(1, MAX_ROWS) as i32;
    let footer = if rows > MAX_ROWS || (has_msg && rows > 0) { px(22) } else { px(10) };
    let h = px(16) + fonts.ui_h + px(18) + px(8) + row_h * shown + footer;
    drop(fonts);
    unsafe {
        let mut rc: RECT = zeroed();
        GetWindowRect(hwnd, &mut rc);
        if rc.bottom - rc.top != h {
            SetWindowPos(hwnd, null_mut(), 0, 0, rc.right - rc.left, h, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
        InvalidateRect(hwnd, null(), 0);
    }
}

// ------------------------------------------------------------------ choosing

fn choose(i: usize) {
    let q = edit_text();
    let is_prompt = with(|s| matches!(s.mode, Mode::Prompt { .. }));
    if is_prompt {
        let cb = with(|s| match &mut s.mode {
            Mode::Prompt { on_submit, .. } => on_submit.take(),
            _ => None,
        });
        hide();
        if let Some(cb) = cb {
            cb(q);
        }
        return;
    }
    let Some(item) = with(|s| s.rows.get(i).map(|r| r.item.clone())) else { return };
    let pick_cb = with(|s| match &mut s.mode {
        Mode::Pick { on_pick, .. } => on_pick.take(),
        _ => None,
    });
    hide();
    match item.kind {
        Kind::Index(n) => {
            if let Some(cb) = pick_cb {
                cb(n);
            }
        }
        Kind::Doc(id) => {
            let g = {
                let st = app().st.borrow();
                if st.groups[st.active_group].tabs.contains(&id) {
                    st.active_group
                } else if st.split && st.groups[1 - st.active_group].tabs.contains(&id) {
                    1 - st.active_group
                } else {
                    st.active_group
                }
            };
            docs::activate(g, id);
        }
        Kind::Path(p) => {
            docs::open_path(&p, None);
        }
        Kind::Command(c) => commands::run_index(c),
        Kind::Line(n) => {
            let v = app::active_view();
            v.goto_line(n.saturating_sub(1));
        }
        Kind::Hit { path, doc, line, col, len } => {
            let id = if doc != 0 { Some(doc) } else { path.as_ref().and_then(|p| docs::open_path(p, None)) };
            if let Some(id) = id {
                if doc != 0 {
                    docs::activate(app::active_group(), id);
                }
                let v = app::active_view();
                let start = v.line_start(line) + col;
                v.goto_pos_centered(start, start + len);
            }
        }
    }
}

fn select(delta: isize) {
    with(|s| {
        if s.rows.is_empty() {
            return;
        }
        let n = s.rows.len() as isize;
        let next = (s.sel as isize + delta).clamp(0, n - 1) as usize;
        s.sel = next;
        if s.sel < s.scroll {
            s.scroll = s.sel;
        } else if s.sel >= s.scroll + MAX_ROWS {
            s.scroll = s.sel + 1 - MAX_ROWS;
        }
    });
    invalidate();
}

pub fn pre_translate(msg: &MSG) -> bool {
    let (edit, hwnd) = with(|s| (s.edit, s.hwnd));
    if edit.is_null() || msg.hwnd != edit || msg.message != WM_KEYDOWN {
        return false;
    }
    let ctrl = unsafe { GetKeyState(VK_CONTROL as i32) } < 0;
    match msg.wParam as u16 {
        VK_RETURN => {
            let sel = with(|s| s.sel);
            choose(sel);
        }
        VK_ESCAPE => hide(),
        VK_UP => select(-1),
        VK_DOWN => select(1),
        VK_PRIOR => select(-(MAX_ROWS as isize)),
        VK_NEXT => select(MAX_ROWS as isize),
        VK_TAB => {
            // Tab completes a Quick Open command prefix switch, otherwise moves down.
            select(1);
        }
        VK_BACK if ctrl => delete_word_before_caret(edit),
        _ => return false,
    }
    let _ = hwnd;
    true
}

fn delete_word_before_caret(edit: HWND) {
    let text: Vec<char> = edit_text().chars().collect();
    let mut start = 0u32;
    let mut end = 0u32;
    unsafe { SendMessageW(edit, EM_GETSEL, &mut start as *mut u32 as WPARAM, &mut end as *mut u32 as LPARAM) };
    if start != end {
        unsafe { SendMessageW(edit, EM_REPLACESEL, 1, w!("") as LPARAM) };
        return;
    }
    let mut units = 0usize;
    let mut caret = 0usize;
    while caret < text.len() && units < end as usize {
        units += text[caret].len_utf16();
        caret += 1;
    }
    let mut i = caret;
    while i > 0 && text[i - 1].is_whitespace() {
        i -= 1;
    }
    while i > 0 && !text[i - 1].is_whitespace() {
        i -= 1;
    }
    let from: usize = text[..i].iter().map(|c| c.len_utf16()).sum();
    unsafe {
        SendMessageW(edit, EM_SETSEL, from, end as LPARAM);
        SendMessageW(edit, EM_REPLACESEL, 1, w!("") as LPARAM);
    }
}

// ------------------------------------------------------------------ painting

struct Geo {
    search: RECT,
    list_top: i32,
    row_h: i32,
}

fn geo(hwnd: HWND) -> Geo {
    let fonts = app().fonts.borrow();
    let mut rc: RECT = unsafe { zeroed() };
    unsafe { GetClientRect(hwnd, &mut rc) };
    let search = rect(px(8), px(8), rc.right - px(8), px(8) + fonts.ui_h + px(18));
    Geo { search, list_top: search.bottom + px(8), row_h: fonts.ui_h + px(14) }
}

fn paint(hwnd: HWND) {
    let a = app();
    let t = a.theme.get();
    let g = geo(hwnd);
    unsafe {
        let mut ps: PAINTSTRUCT = zeroed();
        let dc = BeginPaint(hwnd, &mut ps);
        let mut rc: RECT = zeroed();
        GetClientRect(hwnd, &mut rc);
        let c = Canvas::new(dc, rc.right, rc.bottom);
        let bg = t.chrome.mix(t.bg, 0.5);
        c.fill(rc, bg);
        c.round(g.search, px(6), t.surface);
        c.round_stroke(g.search, px(6), t.accent, 1.0);
        let fonts = a.fonts.borrow();
        c.text(icon::SEARCH, rect(g.search.left + px(6), g.search.top, g.search.left + px(30), g.search.bottom), fonts.icon_small, t.subtext, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
        with(|s| {
            let mut y = g.list_top;
            if s.rows.is_empty() {
                let msg = if s.message.is_empty() { "No matches".to_string() } else { s.message.clone() };
                c.text(&msg, rect(px(20), y, rc.right - px(20), y + g.row_h), fonts.small, t.subtext, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
            }
            for (i, row) in s.rows.iter().enumerate().skip(s.scroll).take(MAX_ROWS) {
                let r = rect(px(6), y, rc.right - px(6), y + g.row_h);
                if i == s.sel {
                    c.round(r, px(5), t.selection.mix(bg, 0.7));
                } else if s.hover == Some(i) {
                    c.round(r, px(5), t.fg.mix(bg, 0.05));
                }
                c.text(row.item.icon, rect(r.left + px(6), r.top, r.left + px(30), r.bottom), fonts.icon_small, t.subtext, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
                // Hints (shortcuts, folders) never take more than 40% of the row.
                let hint_w = if row.item.hint.is_empty() { 0 } else { (c.extent(&wide_nz(&row.item.hint), fonts.small) + px(16)).min((r.right - r.left) * 2 / 5) };
                let title_r = rect(r.left + px(36), r.top, r.right - hint_w - px(8), r.bottom);
                let chars: Vec<char> = row.item.title.chars().collect();
                // Consecutive matched characters become one highlight.
                let mut ranges: Vec<(usize, usize)> = Vec::new();
                for &i in &row.hl {
                    match ranges.last_mut() {
                        Some(r) if r.1 == i => r.1 = i + 1,
                        _ => ranges.push((i, i + 1)),
                    }
                }
                let line_y = r.top + (g.row_h - fonts.ui_h) / 2;
                let title_text_r = rect(title_r.left, line_y, title_r.right, line_y + fonts.ui_h);
                c.highlighted_line(&chars, &ranges, title_text_r, fonts.ui, t.fg, t.accent.mix(bg, 0.35), fonts.ui_h);
                if !row.item.detail.is_empty() {
                    let tw = c.extent(&wide_nz(&row.item.title), fonts.ui).min(title_r.right - title_r.left);
                    let dr = rect(title_r.left + tw + px(10), r.top, title_r.right, r.bottom);
                    if dr.right - dr.left > px(40) {
                        c.text(&row.item.detail, dr, fonts.small, t.subtext, DT_SINGLELINE | DT_VCENTER | DT_PATH_ELLIPSIS);
                    }
                }
                if hint_w > 0 {
                    c.text(&row.item.hint, rect(r.right - hint_w, r.top, r.right - px(8), r.bottom), fonts.small, t.subtext, DT_SINGLELINE | DT_VCENTER | DT_RIGHT | DT_PATH_ELLIPSIS);
                }
                y += g.row_h;
            }
            if s.rows.len() > MAX_ROWS {
                let more = format!("{} of {}", (s.sel + 1).min(s.rows.len()), s.rows.len());
                c.text(&more, rect(px(10), rc.bottom - px(20), rc.right - px(14), rc.bottom - px(2)), fonts.small, t.subtext, DT_SINGLELINE | DT_RIGHT);
            } else if !s.message.is_empty() && !s.rows.is_empty() {
                c.text(&s.message, rect(px(10), rc.bottom - px(20), rc.right - px(14), rc.bottom - px(2)), fonts.small, t.subtext, DT_SINGLELINE | DT_RIGHT);
            }
        });
        drop(fonts);
        c.present();
        EndPaint(hwnd, &ps);
    }
}

fn row_at(hwnd: HWND, y: i32) -> Option<usize> {
    let g = geo(hwnd);
    if y < g.list_top {
        return None;
    }
    let i = ((y - g.list_top) / g.row_h) as usize;
    with(|s| {
        let idx = s.scroll + i;
        (i < MAX_ROWS && idx < s.rows.len()).then_some(idx)
    })
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            paint(hwnd);
            0
        }
        WM_CTLCOLOREDIT => {
            let t = app::theme();
            let dc = wp as HDC;
            SetTextColor(dc, t.fg.colorref());
            SetBkColor(dc, t.surface.colorref());
            with(|s| s.edit_brush) as LRESULT
        }
        WM_COMMAND => {
            if ((wp >> 16) & 0xffff) as u32 == EN_CHANGE {
                refresh();
            }
            0
        }
        WM_MOUSEMOVE => {
            let h = row_at(hwnd, util::hi(lp));
            if with(|s| std::mem::replace(&mut s.hover, h)) != h {
                let mut tme = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                TrackMouseEvent(&mut tme);
                invalidate();
            }
            0
        }
        WM_MOUSELEAVE => {
            with(|s| s.hover = None);
            invalidate();
            0
        }
        WM_LBUTTONUP => {
            if let Some(i) = row_at(hwnd, util::hi(lp)) {
                choose(i);
            } else {
                SetFocus(with(|s| s.edit));
            }
            0
        }
        WM_MOUSEWHEEL => {
            let delta = ((wp >> 16) & 0xffff) as i16 as i32;
            with(|s| {
                let max = s.rows.len().saturating_sub(MAX_ROWS);
                s.scroll = if delta > 0 { s.scroll.saturating_sub(3) } else { (s.scroll + 3).min(max) };
            });
            invalidate();
            0
        }
        WM_ACTIVATE => {
            if (wp & 0xffff) as u32 == WA_INACTIVE {
                // Clicking elsewhere dismisses the palette.
                ShowWindow(hwnd, SW_HIDE);
                with(|s| {
                    s.mode = Mode::Quick;
                    s.rows.clear();
                });
            }
            0
        }
        _ => {
            let _ = contains;
            DefWindowProcW(hwnd, msg, wp, lp)
        }
    }
}

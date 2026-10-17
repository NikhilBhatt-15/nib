//! Find and replace bar.
//!
//! Matching uses the `regex` crate directly on Scintilla's UTF-8 buffer
//! (no copy), so byte offsets are editor positions. Literal searches are
//! escaped regexes; whole-word wraps the pattern in `\b`. Every match is
//! highlighted, the bar shows "n of m", and Replace all is one undo step.

use crate::app::{self, app, px};
use crate::editor::{IND_FIND, IND_FIND_CURRENT};
use crate::gfx::{contains, icon, rect, Canvas};
use crate::sci::Sci;
use crate::sci_consts::*;
use crate::util::{self, wide};
use regex::bytes::{Regex, RegexBuilder};
use std::cell::RefCell;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use windows_sys::w;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{EM_SETCUEBANNER, EM_SETSEL, WM_MOUSELEAVE};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const MAX_MATCHES: usize = 200_000;
const MAX_HIGHLIGHTS: usize = 20_000;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Btn {
    ToggleReplace,
    Case,
    Word,
    Regex,
    InSel,
    Prev,
    Next,
    Close,
    Replace,
    ReplaceAll,
}

struct State {
    hwnd: HWND,
    find: HWND,
    repl: HWND,
    visible: bool,
    replace: bool,
    case: bool,
    word: bool,
    regex: bool,
    in_sel: bool,
    scope: Option<(usize, usize)>,
    matches: Vec<(usize, usize)>,
    current: Option<usize>,
    error: Option<String>,
    hover: Option<Btn>,
    brush: HBRUSH,
    /// Caret position when the search started, for incremental find.
    origin: usize,
}

thread_local! {
    static ST: RefCell<State> = const { RefCell::new(State {
        hwnd: null_mut(),
        find: null_mut(),
        repl: null_mut(),
        visible: false,
        replace: false,
        case: false,
        word: false,
        regex: false,
        in_sel: false,
        scope: None,
        matches: Vec::new(),
        current: None,
        error: None,
        hover: None,
        brush: null_mut(),
        origin: 0,
    }) };
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    ST.with(|s| f(&mut s.borrow_mut()))
}

pub fn hwnd() -> HWND {
    with(|s| s.hwnd)
}

pub fn height() -> Option<i32> {
    with(|s| s.visible.then(|| if s.replace { px(76) } else { px(40) }))
}

pub fn create(parent: HWND) {
    unsafe {
        let hinst = GetModuleHandleW(null());
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            lpszClassName: w!("NibFind"),
            ..zeroed()
        };
        RegisterClassExW(&wc);
        let hwnd = CreateWindowExW(0, w!("NibFind"), null(), WS_CHILD | WS_CLIPCHILDREN, 0, 0, 0, 0, parent, 403 as _, hinst, null());
        let style = WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL as u32;
        let find = CreateWindowExW(0, w!("EDIT"), null(), style, 0, 0, 10, 10, hwnd, 1 as _, hinst, null());
        let repl = CreateWindowExW(0, w!("EDIT"), null(), style, 0, 0, 10, 10, hwnd, 2 as _, hinst, null());
        SendMessageW(find, EM_SETCUEBANNER, 1, w!("Find") as LPARAM);
        SendMessageW(repl, EM_SETCUEBANNER, 1, w!("Replace (regex: $1 for groups)") as LPARAM);
        with(|s| {
            s.hwnd = hwnd;
            s.find = find;
            s.repl = repl;
        });
        apply_theme();
    }
}

pub fn apply_theme() {
    let (hwnd, find, repl) = with(|s| (s.hwnd, s.find, s.repl));
    if hwnd.is_null() {
        return;
    }
    let t = app::theme();
    unsafe {
        let b = CreateSolidBrush(t.surface.colorref());
        let old = with(|s| std::mem::replace(&mut s.brush, b));
        if !old.is_null() {
            DeleteObject(old);
        }
        let f = app().fonts.borrow().ui;
        SendMessageW(find, WM_SETFONT, f as WPARAM, 1);
        SendMessageW(repl, WM_SETFONT, f as WPARAM, 1);
        InvalidateRect(hwnd, null(), 1);
    }
}

pub fn on_dpi() {
    apply_theme();
    layout_children();
}

fn edit_text(h: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(h);
        let mut buf = vec![0u16; len as usize + 1];
        GetWindowTextW(h, buf.as_mut_ptr(), len + 1);
        String::from_utf16_lossy(&buf[..len as usize])
    }
}

/// Opens the bar, seeding it with the selection when that is a short single line.
pub fn open(replace: bool) {
    let v = app::active_view();
    let sel = String::from_utf8_lossy(&v.selected()).into_owned();
    let (find, repl) = with(|s| (s.find, s.repl));
    if !sel.is_empty() && sel.len() < 300 && !sel.contains('\n') {
        let mut text = sel;
        if with(|s| s.regex) {
            text = regex::escape(&text);
        }
        let w = wide(&text);
        unsafe { SetWindowTextW(find, w.as_ptr()) };
    }
    with(|s| {
        s.visible = true;
        s.replace = replace;
        s.origin = v.sel_start();
    });
    unsafe {
        ShowWindow(repl, if replace { SW_SHOW } else { SW_HIDE });
    }
    app::layout();
    layout_children();
    unsafe {
        SetFocus(find);
        SendMessageW(find, EM_SETSEL, 0, -1);
    }
    search(false);
}

pub fn close() {
    let was = with(|s| std::mem::replace(&mut s.visible, false));
    clear_highlights(&app::view(0));
    clear_highlights(&app::view(1));
    with(|s| {
        s.matches.clear();
        s.current = None;
        s.in_sel = false;
        s.scope = None;
    });
    if was {
        app::layout();
    }
    unsafe { SetFocus(app::active_view().hwnd) };
}

pub fn is_visible() -> bool {
    with(|s| s.visible)
}

fn clear_highlights(v: &Sci) {
    for ind in [IND_FIND, IND_FIND_CURRENT] {
        v.m(SCI_SETINDICATORCURRENT, ind, 0);
        v.m(SCI_INDICATORCLEARRANGE, 0, v.len() as i64);
    }
}

fn build(query: &str) -> Result<Option<Regex>, String> {
    if query.is_empty() {
        return Ok(None);
    }
    let (case, word, rx) = with(|s| (s.case, s.word, s.regex));
    let mut pat = if rx { query.to_string() } else { regex::escape(query) };
    if word {
        pat = format!(r"\b(?:{pat})\b");
    }
    RegexBuilder::new(&pat).case_insensitive(!case).multi_line(true).size_limit(50 << 20).build().map(Some).map_err(|e| {
        let msg = e.to_string();
        msg.lines().last().unwrap_or("invalid pattern").trim().to_string()
    })
}

/// Recomputes matches. With `keep_position` the selection isn't moved (used
/// after edits); otherwise the first match after the search origin is selected.
pub fn search(keep_position: bool) {
    if !is_visible() {
        return;
    }
    let v = app::active_view();
    clear_highlights(&v);
    let query = edit_text(with(|s| s.find));
    let re = match build(&query) {
        Ok(r) => r,
        Err(e) => {
            with(|s| {
                s.error = Some(e);
                s.matches.clear();
                s.current = None;
            });
            invalidate();
            return;
        }
    };
    with(|s| s.error = None);
    let Some(re) = re else {
        with(|s| {
            s.matches.clear();
            s.current = None;
        });
        invalidate();
        return;
    };
    let (lo, hi) = with(|s| s.scope).filter(|_| with(|s| s.in_sel)).unwrap_or((0, v.len()));
    let hi = hi.min(v.len());
    let bytes = v.bytes();
    let hay = &bytes[lo.min(hi)..hi];
    let mut matches = Vec::new();
    for m in re.find_iter(hay) {
        if m.start() == m.end() {
            continue;
        }
        matches.push((lo + m.start(), lo + m.end()));
        if matches.len() >= MAX_MATCHES {
            break;
        }
    }
    v.m(SCI_SETINDICATORCURRENT, IND_FIND, 0);
    for &(a, b) in matches.iter().take(MAX_HIGHLIGHTS) {
        v.m(SCI_INDICATORFILLRANGE, a as i64, (b - a) as i64);
    }
    let current = if keep_position {
        let (sa, sb) = (v.sel_start(), v.sel_end());
        matches.iter().position(|&(a, b)| a == sa && b == sb)
    } else {
        let origin = with(|s| s.origin);
        let i = matches.iter().position(|&(a, _)| a >= origin).or(if matches.is_empty() { None } else { Some(0) });
        if let Some(i) = i {
            let (a, b) = matches[i];
            v.goto_pos_centered(a, b);
        }
        i
    };
    with(|s| {
        s.matches = matches;
        s.current = current;
    });
    mark_current(&v);
    invalidate();
}

fn mark_current(v: &Sci) {
    v.m(SCI_SETINDICATORCURRENT, IND_FIND_CURRENT, 0);
    v.m(SCI_INDICATORCLEARRANGE, 0, v.len() as i64);
    if let Some((a, b)) = with(|s| s.current.and_then(|i| s.matches.get(i).copied())) {
        v.m(SCI_INDICATORFILLRANGE, a as i64, (b - a) as i64);
    }
}

/// F3 / Shift+F3; opens the bar with the selection if it isn't showing.
pub fn find_next(forward: bool) {
    if !is_visible() {
        open(false);
        return;
    }
    let v = app::active_view();
    if with(|s| s.matches.is_empty()) {
        search(true);
    }
    let (sa, sb) = (v.sel_start(), v.sel_end());
    let next = with(|s| {
        if s.matches.is_empty() {
            return None;
        }
        let n = s.matches.len();
        let i = if forward {
            s.matches.iter().position(|&(a, _)| a >= sb && !(a == sa && sa != sb && a < sb)).unwrap_or(0)
        } else {
            s.matches.iter().rposition(|&(_, b)| b <= sa).unwrap_or(n - 1)
        };
        s.current = Some(i);
        Some(s.matches[i])
    });
    if let Some((a, b)) = next {
        v.goto_pos_centered(a, b);
        mark_current(&v);
    } else {
        unsafe { windows_sys::Win32::System::Diagnostics::Debug::MessageBeep(MB_OK) };
    }
    with(|s| s.origin = v.sel_end());
    invalidate();
}

fn replacement(re: &Regex, matched: &[u8]) -> Vec<u8> {
    let rep = edit_text(with(|s| s.repl));
    if !with(|s| s.regex) {
        return rep.into_bytes();
    }
    let rep = unescape(&rep);
    let mut out = Vec::new();
    if let Some(caps) = re.captures(matched) {
        caps.expand(rep.as_bytes(), &mut out);
    }
    out
}

/// \n, \t and \\ in regex-mode replacements.
fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.peek() {
                Some('n') => {
                    out.push('\n');
                    it.next();
                }
                Some('t') => {
                    out.push('\t');
                    it.next();
                }
                Some('r') => {
                    out.push('\r');
                    it.next();
                }
                Some('\\') => {
                    out.push('\\');
                    it.next();
                }
                _ => out.push(c),
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn replace_one() {
    let v = app::active_view();
    if v.readonly() {
        return;
    }
    let query = edit_text(with(|s| s.find));
    let Ok(Some(re)) = build(&query) else { return };
    let (sa, sb) = (v.sel_start(), v.sel_end());
    let is_match = with(|s| s.matches.iter().any(|&(a, b)| a == sa && b == sb));
    if is_match {
        let matched = v.range(sa, sb);
        let rep = replacement(&re, &matched);
        v.replace_range(sa, sb, &rep);
        v.set_sel(sa + rep.len(), sa + rep.len());
        with(|s| s.origin = sa + rep.len());
        search(true);
        find_next(true);
    } else {
        find_next(true);
    }
}

fn replace_all() {
    let v = app::active_view();
    if v.readonly() {
        return;
    }
    let query = edit_text(with(|s| s.find));
    let Ok(Some(re)) = build(&query) else { return };
    let (lo, hi) = with(|s| s.scope).filter(|_| with(|s| s.in_sel)).unwrap_or((0, v.len()));
    let hi = hi.min(v.len());
    let original = v.range(lo, hi);
    let regex_mode = with(|s| s.regex);
    let rep_text = edit_text(with(|s| s.repl));
    let count = re.find_iter(&original).filter(|m| m.start() != m.end() || regex_mode).count();
    if count == 0 {
        app::set_status("No matches to replace");
        return;
    }
    let result = if regex_mode { re.replace_all(&original, unescape(&rep_text).as_bytes()).into_owned() } else { re.replace_all(&original, regex::bytes::NoExpand(rep_text.as_bytes())).into_owned() };
    v.begin_undo();
    v.replace_range(lo, hi, &result);
    v.end_undo();
    if with(|s| s.in_sel) {
        with(|s| s.scope = Some((lo, lo + result.len())));
    }
    app::set_status(&format!("Replaced {} occurrence{}", util::group_digits(count), if count == 1 { "" } else { "s" }));
    search(true);
}

fn toggle(b: Btn) {
    with(|s| match b {
        Btn::Case => s.case = !s.case,
        Btn::Word => s.word = !s.word,
        Btn::Regex => s.regex = !s.regex,
        Btn::InSel => {
            s.in_sel = !s.in_sel;
            if s.in_sel {
                let v = app::active_view();
                s.scope = Some((v.sel_start(), v.sel_end())).filter(|(a, b)| a != b);
                if s.scope.is_none() {
                    s.in_sel = false;
                }
            }
        }
        _ => {}
    });
    search(true);
}

fn press(b: Btn) {
    match b {
        Btn::ToggleReplace => {
            let r = !with(|s| s.replace);
            open(r);
        }
        Btn::Case | Btn::Word | Btn::Regex | Btn::InSel => toggle(b),
        Btn::Prev => find_next(false),
        Btn::Next => find_next(true),
        Btn::Close => close(),
        Btn::Replace => replace_one(),
        Btn::ReplaceAll => replace_all(),
    }
}

/// After edits or switching tabs: refresh highlights without moving.
pub fn on_doc_changed() {
    if is_visible() {
        search(true);
    }
}

// ------------------------------------------------------------------ keyboard

pub fn pre_translate(msg: &MSG) -> bool {
    let (find, repl, visible) = with(|s| (s.find, s.repl, s.visible));
    if !visible || (msg.hwnd != find && msg.hwnd != repl) {
        return false;
    }
    let key = |k: u16| unsafe { GetKeyState(k as i32) } < 0;
    if msg.message == WM_SYSKEYDOWN {
        let b = match msg.wParam as u8 {
            b'C' => Btn::Case,
            b'W' => Btn::Word,
            b'R' => Btn::Regex,
            b'L' => Btn::InSel,
            _ => return false,
        };
        toggle(b);
        return true;
    }
    if msg.message != WM_KEYDOWN {
        return false;
    }
    match msg.wParam as u16 {
        VK_RETURN => {
            if msg.hwnd == repl {
                if key(VK_CONTROL) {
                    replace_all();
                } else {
                    replace_one();
                }
            } else {
                find_next(!key(VK_SHIFT));
            }
        }
        VK_ESCAPE => close(),
        VK_TAB => unsafe {
            if with(|s| s.replace) {
                let to = if msg.hwnd == find { repl } else { find };
                SetFocus(to);
                SendMessageW(to, EM_SETSEL, 0, -1);
            }
        },
        _ => return false,
    }
    true
}

pub fn on_command(child: HWND, code: u32) -> bool {
    let find = with(|s| s.find);
    if child == find && code == EN_CHANGE {
        search(false);
        return true;
    }
    false
}

// ------------------------------------------------------------------ painting

struct Geo {
    find: RECT,
    repl: RECT,
    buttons: Vec<(Btn, RECT)>,
    count: RECT,
}

fn geometry() -> Geo {
    let (hwnd, replace) = with(|s| (s.hwnd, s.replace));
    let mut rc: RECT = unsafe { zeroed() };
    unsafe { GetClientRect(hwnd, &mut rc) };
    let row_h = px(40);
    let bh = px(26);
    let y1 = (row_h - bh) / 2;
    let edit_w = px(380).min(rc.right / 2).max(px(160));
    let x0 = px(34);
    let find = rect(x0, y1, x0 + edit_w, y1 + bh);
    let mut b = vec![(Btn::ToggleReplace, rect(px(6), y1, px(30), y1 + bh))];
    let mut x = find.right - bh * 4 - px(4);
    for t in [Btn::Case, Btn::Word, Btn::Regex, Btn::InSel] {
        b.push((t, rect(x, y1 + px(2), x + bh - px(4), y1 + bh - px(2))));
        x += bh;
    }
    let mut x = find.right + px(8);
    let count = rect(x, y1, x + px(110), y1 + bh);
    x = count.right + px(4);
    for t in [Btn::Prev, Btn::Next] {
        b.push((t, rect(x, y1, x + bh, y1 + bh)));
        x += bh + px(2);
    }
    b.push((Btn::Close, rect(rc.right - px(6) - bh, y1, rc.right - px(6), y1 + bh)));
    let y2 = row_h - px(4) + y1;
    let repl = rect(x0, y2, find.right, y2 + bh);
    if replace {
        let mut x = repl.right + px(8);
        for (t, w) in [(Btn::Replace, px(78)), (Btn::ReplaceAll, px(94))] {
            b.push((t, rect(x, y2, x + w, y2 + bh)));
            x += w + px(6);
        }
    }
    Geo { find, repl, buttons: b, count }
}

fn layout_children() {
    let (find, repl) = with(|s| (s.find, s.repl));
    if find.is_null() {
        return;
    }
    let g = geometry();
    let fh = app().fonts.borrow().ui_h + px(2);
    unsafe {
        let place = |h: HWND, r: RECT, right_reserve: i32| {
            let y = r.top + (r.bottom - r.top - fh) / 2;
            MoveWindow(h, r.left + px(8), y, r.right - r.left - px(16) - right_reserve, fh, 1);
        };
        place(find, g.find, px(26) * 4);
        place(repl, g.repl, 0);
    }
}

fn invalidate() {
    let h = with(|s| s.hwnd);
    unsafe { InvalidateRect(h, null(), 0) };
}

fn paint(hwnd: HWND) {
    let a = app();
    let t = a.theme.get();
    let g = geometry();
    unsafe {
        let mut ps: PAINTSTRUCT = zeroed();
        let dc = BeginPaint(hwnd, &mut ps);
        let mut rc: RECT = zeroed();
        GetClientRect(hwnd, &mut rc);
        let c = Canvas::new(dc, rc.right, rc.bottom);
        c.fill(rc, t.chrome);
        c.fill(rect(0, 0, rc.right, 1), t.border);
        let fonts = a.fonts.borrow();
        with(|s| {
            let err = s.error.is_some();
            for (r, show) in [(g.find, true), (g.repl, s.replace)] {
                if show {
                    c.round(r, px(5), t.surface);
                    c.round_stroke(r, px(5), if err && r.top == g.find.top { t.syntax.error } else { t.border }, 1.0);
                }
            }
            for (b, r) in &g.buttons {
                let on = match b {
                    Btn::Case => s.case,
                    Btn::Word => s.word,
                    Btn::Regex => s.regex,
                    Btn::InSel => s.in_sel,
                    _ => false,
                };
                if on {
                    c.round(*r, px(4), t.accent.mix(t.surface, 0.35));
                } else if s.hover == Some(*b) {
                    c.round(*r, px(4), t.fg.mix(t.chrome, 0.1));
                }
                let (label, is_icon) = match b {
                    Btn::ToggleReplace => (if s.replace { icon::DOWN } else { icon::CHEVRON_RIGHT }, true),
                    Btn::Case => ("Aa", false),
                    Btn::Word => ("ab", false),
                    Btn::Regex => (".*", false),
                    Btn::InSel => (icon::SELECTION, true),
                    Btn::Prev => (icon::UP, true),
                    Btn::Next => (icon::DOWN, true),
                    Btn::Close => (icon::CLOSE, true),
                    Btn::Replace => ("Replace", false),
                    Btn::ReplaceAll => ("Replace all", false),
                };
                if matches!(b, Btn::Replace | Btn::ReplaceAll) {
                    c.round_stroke(*r, px(4), t.border, 1.0);
                }
                let font = if is_icon { fonts.icon_small } else if matches!(b, Btn::Word) { fonts.ui_bold } else { fonts.small };
                c.text(label, *r, font, if on { t.fg } else { t.subtext }, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
            }
            let count_text = if let Some(e) = &s.error {
                e.clone()
            } else if s.matches.is_empty() {
                if edit_text(s.find).is_empty() { String::new() } else { "No results".into() }
            } else {
                let n = s.matches.len();
                let more = if n >= MAX_MATCHES { "+" } else { "" };
                match s.current {
                    Some(i) => format!("{} of {}{more}", util::group_digits(i + 1), util::group_digits(n)),
                    None => format!("{}{more} found", util::group_digits(n)),
                }
            };
            c.text(&count_text, g.count, fonts.small, if s.error.is_some() || (s.matches.is_empty() && !count_text.is_empty()) { t.syntax.error } else { t.subtext }, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
        });
        drop(fonts);
        c.present();
        EndPaint(hwnd, &ps);
    }
}

fn hit(x: i32, y: i32) -> Option<Btn> {
    geometry().buttons.into_iter().find(|(_, r)| contains(r, x, y)).map(|(b, _)| b)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            paint(hwnd);
            0
        }
        WM_SIZE => {
            layout_children();
            0
        }
        WM_CTLCOLOREDIT => {
            let t = app::theme();
            let dc = wp as HDC;
            SetTextColor(dc, t.fg.colorref());
            SetBkColor(dc, t.surface.colorref());
            with(|s| s.brush) as LRESULT
        }
        WM_COMMAND => {
            let code = ((wp >> 16) & 0xffff) as u32;
            on_command(lp as HWND, code);
            0
        }
        WM_MOUSEMOVE => {
            let h = hit(util::lo(lp), util::hi(lp));
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
            if let Some(b) = hit(util::lo(lp), util::hi(lp)) {
                press(b);
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

#[cfg(test)]
mod tests {
    use super::unescape;

    #[test]
    fn replacement_escapes() {
        assert_eq!(unescape(r"a\nb\tc\\d\q"), "a\nb\tc\\d\\q");
    }
}

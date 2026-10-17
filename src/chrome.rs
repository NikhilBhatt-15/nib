//! The status bar and the info bar.
//!
//! The status bar's segments are clickable: position opens Go to line,
//! indentation, encoding, line endings and language open pickers.
//! The info bar shows one message with buttons (recovered session, file
//! changed on disk, comparison controls...).

use crate::app::{self, app, px, InfoAction};
use crate::gfx::{contains, icon, rect, Canvas};
use crate::languages::LANGS;
use crate::sci_consts::*;
use crate::util::{self, group_digits, wide_nz};
use crate::{commands, docs};
use std::cell::Cell;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use windows_sys::w;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::WM_MOUSELEAVE;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

thread_local! {
    static STATUS_HOVER: Cell<i32> = const { Cell::new(-1) };
    static INFO_HOVER: Cell<i32> = const { Cell::new(-1) };
}

fn register(class: *const u16, proc_: unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT) {
    unsafe {
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(proc_),
            hInstance: GetModuleHandleW(null()),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            lpszClassName: class,
            ..zeroed()
        };
        RegisterClassExW(&wc);
    }
}

pub fn create_status(parent: HWND) -> HWND {
    register(w!("NibStatus"), status_proc);
    unsafe { CreateWindowExW(0, w!("NibStatus"), null(), WS_CHILD | WS_VISIBLE, 0, 0, 0, 0, parent, 401 as _, GetModuleHandleW(null()), null()) }
}

pub fn create_infobar(parent: HWND) -> HWND {
    register(w!("NibInfo"), info_proc);
    unsafe { CreateWindowExW(0, w!("NibInfo"), null(), WS_CHILD, 0, 0, 0, 0, parent, 402 as _, GetModuleHandleW(null()), null()) }
}

fn client(hwnd: HWND) -> RECT {
    let mut rc: RECT = unsafe { zeroed() };
    unsafe { GetClientRect(hwnd, &mut rc) };
    rc
}

fn text_width(s: &str, font: HFONT) -> i32 {
    if s.is_empty() {
        return 0;
    }
    let w = wide_nz(s);
    unsafe {
        let dc = GetDC(null_mut());
        let old = SelectObject(dc, font);
        let mut sz = SIZE { cx: 0, cy: 0 };
        GetTextExtentPoint32W(dc, w.as_ptr(), w.len() as i32, &mut sz);
        SelectObject(dc, old);
        ReleaseDC(null_mut(), dc);
        sz.cx
    }
}

// ------------------------------------------------------------------ status bar

fn plural(n: usize, word: &str) -> String {
    format!("{} {word}{}", group_digits(n), if n == 1 { "" } else { "s" })
}

struct Segment {
    r: RECT,
    text: String,
    cmd: Option<&'static str>,
}

fn segments(width: i32, height: i32) -> (Vec<Segment>, String) {
    let a = app();
    let g = app::active_group();
    let v = app::view(g);
    let st = a.st.borrow();
    let id = st.active_id();
    let Some(d) = st.doc(id) else { return (Vec::new(), String::new()) };
    let s = a.settings.borrow();
    let mut items: Vec<(String, Option<&'static str>)> = Vec::new();
    let pos = v.pos();
    let line = v.line_from_pos(pos) + 1;
    let col = v.m(SCI_GETCOLUMN, pos as i64, 0) + 1;
    items.push((format!("Ln {}, Col {}", group_digits(line), col), Some("go.line")));
    let sels = v.m(SCI_GETSELECTIONS, 0, 0);
    if sels > 1 {
        items.push((format!("{sels} selections"), None));
    } else if !v.sel_empty() {
        let (a0, b0) = (v.sel_start(), v.sel_end());
        if b0 - a0 <= 4 * 1024 * 1024 {
            let (w, c) = util::count_words_chars(&v.range(a0, b0));
            let lines = v.line_from_pos(b0) - v.line_from_pos(a0) + 1;
            let l = if lines > 1 { format!(", {lines} lines") } else { String::new() };
            items.push((format!("{}, {} selected{l}", plural(w, "word"), plural(c, "char")), None));
        }
    } else if let Some((_, w, c)) = st.counts.get(&id) {
        items.push((format!("{} · {}", plural(*w, "word"), plural(*c, "char")), None));
    }
    let indent = if v.m(SCI_GETUSETABS, 0, 0) != 0 { format!("Tab size: {}", s.tab_width) } else { format!("Spaces: {}", s.tab_width) };
    items.push((indent, Some("view.indentation")));
    items.push((d.enc.label(), Some("file.encoding")));
    items.push((d.eol.label().to_string(), Some("file.lineEndings")));
    items.push((if d.large { "Large file".into() } else { LANGS[d.lang].name.to_string() }, Some("view.language")));
    if s.zoom != 0 {
        let pct = 100 + s.zoom * 10;
        items.push((format!("{pct}%"), Some("view.zoomReset")));
    }
    if d.readonly && !d.virt {
        items.push(("Read-only".into(), None));
    }
    let fonts = a.fonts.borrow();
    let pad = px(10);
    let mut x = width - px(6);
    let mut out = Vec::new();
    for (text, cmd) in items.into_iter().rev() {
        let w = text_width(&text, fonts.small) + pad * 2;
        out.push(Segment { r: rect(x - w, 0, x, height), text, cmd });
        x -= w;
    }
    out.reverse();
    let left = if let Some((m, _)) = &st.status_msg {
        m.clone()
    } else if d.virt {
        "Read-only view".into()
    } else if let Some(p) = &d.path {
        if d.dirty {
            "Unsaved changes (kept in the session)".into()
        } else {
            p.display().to_string()
        }
    } else if d.dirty {
        "Not saved to a file yet — kept safely in the session".into()
    } else {
        String::new()
    };
    (out, left)
}

fn paint_status(hwnd: HWND) {
    let a = app();
    let t = a.theme.get();
    let rc = client(hwnd);
    let (segs, left) = segments(rc.right, rc.bottom);
    let hover = STATUS_HOVER.with(|h| h.get());
    unsafe {
        let mut ps: PAINTSTRUCT = zeroed();
        let dc = BeginPaint(hwnd, &mut ps);
        let c = Canvas::new(dc, rc.right, rc.bottom);
        c.fill(rc, t.chrome);
        c.fill(rect(0, 0, rc.right, 1), t.border);
        let fonts = a.fonts.borrow();
        let first_left = segs.first().map_or(rc.right, |s| s.r.left);
        c.text(&left, rect(px(10), 0, first_left - px(8), rc.bottom), fonts.small, t.subtext, DT_SINGLELINE | DT_VCENTER | DT_PATH_ELLIPSIS);
        for (i, s) in segs.iter().enumerate() {
            if hover == i as i32 && s.cmd.is_some() {
                c.fill(rect(s.r.left, 1, s.r.right, s.r.bottom), t.fg.mix(t.chrome, 0.08));
            }
            c.text(&s.text, s.r, fonts.small, t.subtext, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
        }
        drop(fonts);
        c.present();
        EndPaint(hwnd, &ps);
    }
}

unsafe extern "system" fn status_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            paint_status(hwnd);
            0
        }
        WM_MOUSEMOVE => {
            let rc = client(hwnd);
            let (segs, _) = segments(rc.right, rc.bottom);
            let i = segs.iter().position(|s| contains(&s.r, util::lo(lp), util::hi(lp))).map_or(-1, |i| i as i32);
            if STATUS_HOVER.with(|h| h.replace(i)) != i {
                InvalidateRect(hwnd, null(), 0);
                let mut tme = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                TrackMouseEvent(&mut tme);
            }
            0
        }
        WM_MOUSELEAVE => {
            STATUS_HOVER.with(|h| h.set(-1));
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_LBUTTONUP => {
            let rc = client(hwnd);
            let (segs, _) = segments(rc.right, rc.bottom);
            if let Some(cmd) = segs.iter().find(|s| contains(&s.r, util::lo(lp), util::hi(lp))).and_then(|s| s.cmd) {
                commands::run(cmd);
            }
            0
        }
        WM_SETCURSOR => {
            if STATUS_HOVER.with(|h| h.get()) >= 0 {
                SetCursor(LoadCursorW(null_mut(), IDC_HAND));
                return 1;
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

// ------------------------------------------------------------------ info bar

struct InfoLayout {
    icon: RECT,
    text: RECT,
    buttons: Vec<(RECT, String, InfoAction)>,
    close: RECT,
}

fn info_layout(width: i32, height: i32) -> Option<(InfoLayout, String, &'static str, bool)> {
    let a = app();
    let st = a.st.borrow();
    let info = st.info.as_ref()?;
    let fonts = a.fonts.borrow();
    let close = rect(width - px(36), (height - px(24)) / 2, width - px(12), (height + px(24)) / 2);
    let mut x = close.left - px(8);
    let mut buttons = Vec::new();
    for (label, act) in info.actions.iter().rev() {
        let w = text_width(label, fonts.small) + px(24);
        buttons.push((rect(x - w, (height - px(24)) / 2, x, (height + px(24)) / 2), label.clone(), act.clone()));
        x -= w + px(6);
    }
    buttons.reverse();
    let icon_r = rect(px(10), 0, px(34), height);
    Some((InfoLayout { icon: icon_r, text: rect(px(38), 0, x - px(8), height), buttons, close }, info.text.clone(), info.icon, info.warning))
}

fn paint_info(hwnd: HWND) {
    let a = app();
    let t = a.theme.get();
    let rc = client(hwnd);
    let hover = INFO_HOVER.with(|h| h.get());
    unsafe {
        let mut ps: PAINTSTRUCT = zeroed();
        let dc = BeginPaint(hwnd, &mut ps);
        let c = Canvas::new(dc, rc.right, rc.bottom);
        if let Some((l, text, ic, warning)) = info_layout(rc.right, rc.bottom) {
            let tint = if warning { t.warning } else { t.accent };
            c.fill(rc, tint.mix(t.chrome, if t.dark { 0.16 } else { 0.10 }));
            c.fill(rect(0, rc.bottom - 1, rc.right, rc.bottom), t.border);
            let fonts = a.fonts.borrow();
            c.text(ic, l.icon, fonts.icon, tint, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
            c.text(&text, l.text, fonts.ui, t.fg, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
            for (i, (r, label, _)) in l.buttons.iter().enumerate() {
                let bg = if hover == i as i32 { t.fg.mix(t.chrome, 0.16) } else { t.surface };
                c.round(*r, px(4), bg);
                c.round_stroke(*r, px(4), t.border, 1.0);
                c.text(label, *r, fonts.small, t.fg, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
            }
            if hover == 1000 {
                c.round(l.close, px(4), t.fg.mix(t.chrome, 0.14));
            }
            c.text(icon::CLOSE, l.close, fonts.icon_small, t.subtext, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
        } else {
            c.fill(rc, t.chrome);
        }
        c.present();
        EndPaint(hwnd, &ps);
    }
}

fn info_hit(hwnd: HWND, x: i32, y: i32) -> i32 {
    let rc = client(hwnd);
    let Some((l, ..)) = info_layout(rc.right, rc.bottom) else { return -1 };
    if contains(&l.close, x, y) {
        return 1000;
    }
    l.buttons.iter().position(|(r, ..)| contains(r, x, y)).map_or(-1, |i| i as i32)
}

unsafe extern "system" fn info_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            paint_info(hwnd);
            0
        }
        WM_MOUSEMOVE => {
            let i = info_hit(hwnd, util::lo(lp), util::hi(lp));
            if INFO_HOVER.with(|h| h.replace(i)) != i {
                InvalidateRect(hwnd, null(), 0);
                let mut tme = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                TrackMouseEvent(&mut tme);
            }
            0
        }
        WM_MOUSELEAVE => {
            INFO_HOVER.with(|h| h.set(-1));
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_LBUTTONUP => {
            let i = info_hit(hwnd, util::lo(lp), util::hi(lp));
            if i == 1000 {
                // Closing the comparison bar ends the comparison.
                if crate::compare::active() {
                    crate::compare::close();
                } else {
                    docs::clear_info();
                }
            } else if i >= 0 {
                let rc = client(hwnd);
                let act = info_layout(rc.right, rc.bottom).and_then(|(l, ..)| l.buttons.get(i as usize).map(|b| b.2.clone()));
                if let Some(act) = act {
                    docs::run_info_action(act);
                }
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

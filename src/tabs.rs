//! The tab strip above each editor group. Custom-drawn so it follows the
//! theme: the active tab merges into the editor, unsaved tabs show a dot in
//! place of the close button, pinned tabs stay at the left.

use crate::app::{self, app, px, R_ALL, R_LAYOUT};
use crate::gfx::{contains, icon, rect, Canvas};
use crate::util::{self, wide, wide_nz};
use crate::{compare, docs};
use std::cell::RefCell;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use windows_sys::w;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::WM_MOUSELEAVE;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Hit {
    #[default]
    None,
    Tab(usize),
    Close(usize),
    Plus,
}

#[derive(Default)]
struct Strip {
    hover: Hit,
    tracking: bool,
    pressed_close: Option<u64>,
    drag: Option<(u64, i32)>,
    dragging: bool,
    last_active: u64,
    last_width: i32,
}

thread_local! {
    static STRIPS: RefCell<[Strip; 2]> = RefCell::new([Strip::default(), Strip::default()]);
}

fn with<R>(g: usize, f: impl FnOnce(&mut Strip) -> R) -> R {
    STRIPS.with(|s| f(&mut s.borrow_mut()[g]))
}

pub fn create(parent: HWND, g: usize) -> HWND {
    unsafe {
        let hinst = GetModuleHandleW(null());
        let class = w!("NibTabs");
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_DBLCLKS,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            lpszClassName: class,
            ..zeroed()
        };
        RegisterClassExW(&wc);
        let h = CreateWindowExW(0, class, null(), WS_CHILD | WS_CLIPSIBLINGS, 0, 0, 0, 0, parent, (201 + g) as _, hinst, null());
        SetWindowLongPtrW(h, GWLP_USERDATA, g as isize);
        h
    }
}

struct TabGeom {
    id: u64,
    r: RECT,
    close: RECT,
}

struct Geometry {
    tabs: Vec<TabGeom>,
    plus: RECT,
}

/// Tab rectangles in client coordinates, already shifted by the scroll offset.
fn geometry(g: usize, width: i32, height: i32) -> Geometry {
    let a = app();
    let fonts = a.fonts.borrow();
    let st = a.st.borrow();
    let grp = &st.groups[g];
    let dc = unsafe { GetDC(null_mut()) };
    // Measured in bold: the active tab is drawn bold and must not truncate.
    let old = unsafe { SelectObject(dc, fonts.ui_bold) };
    let pad = px(12);
    let close_w = px(20);
    let mut x = 0;
    let mut out = Vec::new();
    for &id in &grp.tabs {
        let Some(d) = st.doc(id) else { continue };
        let name = wide_nz(&d.name());
        let mut sz = SIZE { cx: 0, cy: 0 };
        if !name.is_empty() {
            unsafe { GetTextExtentPoint32W(dc, name.as_ptr(), name.len() as i32, &mut sz) };
        }
        let pin_w = if d.pinned { px(18) } else { 0 };
        let w = (pad + pin_w + sz.cx + px(6) + close_w + px(6)).clamp(px(72), px(260));
        let r = rect(x - grp.scroll, 0, x - grp.scroll + w, height);
        let close = rect(r.right - px(6) - close_w, (height - close_w) / 2, r.right - px(6), (height + close_w) / 2);
        out.push(TabGeom { id, r, close });
        x += w;
    }
    unsafe {
        SelectObject(dc, old);
        ReleaseDC(null_mut(), dc);
    }
    let pw = px(32);
    let px0 = (x - grp.scroll).min(width - pw).max(0);
    let plus = rect(px0, 0, px0 + pw, height);
    Geometry { tabs: out, plus }
}

fn total_width(g: usize, h: i32) -> i32 {
    let geo = geometry(g, i32::MAX / 2, h);
    let scroll = app().st.borrow().groups[g].scroll;
    geo.tabs.last().map_or(0, |t| t.r.right + scroll)
}

/// Keeps the active tab inside the visible part of the strip.
fn ensure_active_visible(g: usize, width: i32, height: i32) {
    let avail = width - px(32);
    let total = total_width(g, height);
    let geo = geometry(g, width, height);
    let a = app();
    let mut st = a.st.borrow_mut();
    let active = st.groups[g].active;
    let grp = &mut st.groups[g];
    let max_scroll = (total - avail).max(0);
    if let Some(t) = geo.tabs.iter().find(|t| t.id == active) {
        if t.r.left < 0 {
            grp.scroll += t.r.left;
        } else if t.r.right > avail {
            grp.scroll += t.r.right - avail;
        }
    }
    grp.scroll = grp.scroll.clamp(0, max_scroll);
}

fn client(hwnd: HWND) -> RECT {
    let mut rc: RECT = unsafe { zeroed() };
    unsafe { GetClientRect(hwnd, &mut rc) };
    rc
}

fn hit_test(g: usize, hwnd: HWND, x: i32, y: i32) -> Hit {
    let rc = client(hwnd);
    let geo = geometry(g, rc.right, rc.bottom);
    if contains(&geo.plus, x, y) {
        return Hit::Plus;
    }
    for (i, t) in geo.tabs.iter().enumerate() {
        if contains(&t.r, x, y) && t.r.left < geo.plus.left + 1 {
            let pinned = app().st.borrow().doc(t.id).is_some_and(|d| d.pinned);
            if contains(&t.close, x, y) && !pinned {
                return Hit::Close(i);
            }
            return Hit::Tab(i);
        }
    }
    Hit::None
}

fn tab_id(g: usize, i: usize) -> Option<u64> {
    app().st.borrow().groups[g].tabs.get(i).copied()
}

fn paint(hwnd: HWND, g: usize) {
    let a = app();
    let t = a.theme.get();
    let rc = client(hwnd);
    let active_now = a.st.borrow().groups[g].active;
    let (prev_active, prev_w) = with(g, |s| (std::mem::replace(&mut s.last_active, active_now), std::mem::replace(&mut s.last_width, rc.right)));
    if prev_active != active_now || prev_w != rc.right {
        ensure_active_visible(g, rc.right, rc.bottom);
    }
    let geo = geometry(g, rc.right, rc.bottom);
    let hover = with(g, |s| s.hover);
    unsafe {
        let mut ps: PAINTSTRUCT = zeroed();
        let dc = BeginPaint(hwnd, &mut ps);
        let c = Canvas::new(dc, rc.right, rc.bottom);
        c.fill(rc, t.chrome);
        let fonts = a.fonts.borrow();
        let st = a.st.borrow();
        let group_active = st.active_group == g || !st.split;
        let active = st.groups[g].active;
        let comparing = compare::active();
        // Bottom border; the active tab covers it.
        c.fill(rect(0, rc.bottom - 1, rc.right, rc.bottom), t.border);
        let clip_right = geo.plus.left;
        for (i, tg) in geo.tabs.iter().enumerate() {
            let Some(d) = st.doc(tg.id) else { continue };
            if tg.r.left >= clip_right {
                break;
            }
            let r = RECT { right: tg.r.right.min(clip_right), ..tg.r };
            let is_active = tg.id == active;
            let hovered = matches!(hover, Hit::Tab(h) | Hit::Close(h) if h == i);
            if is_active {
                c.fill(r, t.bg);
                c.fill(rect(r.left, 0, r.left + 1, r.bottom), t.border);
                c.fill(rect(r.right - 1, 0, r.right, r.bottom), t.border);
                let line = if group_active { t.accent } else { t.border };
                c.fill(rect(r.left, 0, r.right, px(2)), line);
            } else if hovered {
                c.fill(rect(r.left, 0, r.right, r.bottom - 1), t.fg.mix(t.chrome, 0.06));
            }
            let mut x = r.left + px(12);
            if d.pinned {
                c.text(icon::PIN, rect(x - px(3), 0, x + px(14), r.bottom), fonts.icon_small, t.subtext, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
                x += px(18);
            }
            let fg = if is_active { t.fg } else { t.subtext };
            let name = if d.virt && comparing { format!("{} ⇄", d.name()) } else { d.name() };
            let text_r = rect(x, 0, tg.close.left - px(4), r.bottom);
            c.text(&name, text_r, if is_active { fonts.ui_bold } else { fonts.ui }, if d.missing { t.syntax.error } else { fg }, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
            if !d.pinned && tg.close.right <= clip_right {
                let close_hover = hover == Hit::Close(i);
                if close_hover {
                    c.round(tg.close, px(4), t.fg.mix(t.chrome, 0.14));
                }
                if d.dirty && !hovered {
                    c.circle((tg.close.left + tg.close.right) / 2, (tg.close.top + tg.close.bottom) / 2, px(8), fg);
                } else if hovered || is_active {
                    c.text(icon::CLOSE, tg.close, fonts.icon_small, fg, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
                }
            }
            if !is_active && geo.tabs.get(i + 1).is_none_or(|n| n.id != active) {
                c.fill(rect(r.right - 1, px(9), r.right, r.bottom - px(9)), t.border);
            }
        }
        if hover == Hit::Plus {
            c.round(rect(geo.plus.left + px(4), px(5), geo.plus.right - px(4), rc.bottom - px(5)), px(4), t.fg.mix(t.chrome, 0.1));
        }
        c.text(icon::ADD, geo.plus, fonts.icon_small, t.subtext, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
        drop(st);
        drop(fonts);
        c.present();
        EndPaint(hwnd, &ps);
    }
}

fn context_menu(hwnd: HWND, g: usize, id: u64) {
    let (pinned, has_path, split) = {
        let st = app().st.borrow();
        let d = st.doc(id);
        (d.is_some_and(|d| d.pinned), d.is_some_and(|d| d.path.is_some()), st.split)
    };
    unsafe {
        let m = CreatePopupMenu();
        let add = |id: usize, s: &str| {
            let w = wide(s);
            AppendMenuW(m, MF_STRING, id, w.as_ptr());
        };
        add(1, "Close\tCtrl+W");
        add(2, "Close others");
        add(3, "Close to the right");
        add(4, "Close saved");
        AppendMenuW(m, MF_SEPARATOR, 0, null());
        add(5, if pinned { "Unpin tab" } else { "Pin tab" });
        add(6, if split { "Move to other side" } else { "Open to the side" });
        add(9, "Compare with active tab");
        AppendMenuW(m, MF_SEPARATOR, 0, null());
        add(7, "Copy path");
        add(8, "Reveal in File Explorer");
        if !has_path {
            EnableMenuItem(m, 7, MF_GRAYED);
            EnableMenuItem(m, 8, MF_GRAYED);
        }
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        let cmd = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, 0, hwnd, null());
        DestroyMenu(m);
        match cmd {
            1 => {
                docs::close_tab(g, id);
            }
            2 => docs::close_others(g, id),
            3 => docs::close_to_right(g, id),
            4 => docs::close_saved(g),
            5 => docs::toggle_pin_tab(g, id),
            6 => {
                docs::activate(g, id);
                docs::move_to_other_group();
            }
            7 | 8 => {
                docs::activate(g, id);
                if cmd == 7 {
                    docs::copy_path();
                } else {
                    docs::reveal_active();
                }
            }
            9 => {
                let active = app::active_doc();
                if active != id {
                    compare::start(active, id);
                }
            }
            _ => {}
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let g = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as usize;
    let (x, y) = (util::lo(lp), util::hi(lp));
    match msg {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            paint(hwnd, g);
            0
        }
        WM_MOUSEMOVE => {
            let hit = hit_test(g, hwnd, x, y);
            let (changed, track) = with(g, |s| {
                let ch = s.hover != hit;
                s.hover = hit;
                let tr = !s.tracking;
                s.tracking = true;
                (ch, tr)
            });
            if track {
                let mut tme = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                TrackMouseEvent(&mut tme);
            }
            // Drag to reorder within the strip.
            if let Some((id, x0)) = with(g, |s| s.drag) {
                if (x - x0).abs() > px(6) {
                    with(g, |s| s.dragging = true);
                }
                if with(g, |s| s.dragging) && y >= 0 && y < client(hwnd).bottom {
                    if let Hit::Tab(i) | Hit::Close(i) = hit {
                        let cur = app().st.borrow().groups[g].tabs.iter().position(|&t| t == id);
                        if cur.is_some_and(|c| c != i) {
                            docs::move_tab(g, id, i);
                        }
                    }
                }
            }
            if changed {
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        WM_MOUSELEAVE => {
            with(g, |s| {
                s.hover = Hit::None;
                s.tracking = false;
            });
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_LBUTTONDOWN => {
            match hit_test(g, hwnd, x, y) {
                Hit::Close(i) => {
                    with(g, |s| s.pressed_close = tab_id(g, i));
                    SetCapture(hwnd);
                }
                Hit::Tab(i) => {
                    if let Some(id) = tab_id(g, i) {
                        docs::activate(g, id);
                        with(g, |s| {
                            s.drag = Some((id, x));
                            s.dragging = false;
                        });
                        SetCapture(hwnd);
                    }
                }
                Hit::Plus => {
                    app().st.borrow_mut().active_group = g;
                    docs::new_doc();
                }
                Hit::None => {}
            }
            0
        }
        WM_LBUTTONUP => {
            ReleaseCapture();
            let pressed = with(g, |s| s.pressed_close.take());
            if let Some(id) = pressed {
                if let Hit::Close(i) = hit_test(g, hwnd, x, y) {
                    if tab_id(g, i) == Some(id) {
                        docs::close_tab(g, id);
                    }
                }
            }
            let drag = with(g, |s| {
                let d = s.drag.take();
                let was = std::mem::replace(&mut s.dragging, false);
                d.filter(|_| was)
            });
            if let Some((id, _)) = drag {
                // Dropped on the other strip (or the other editor): move it across.
                let mut pt = POINT { x, y };
                ClientToScreen(hwnd, &mut pt);
                let target = WindowFromPoint(pt);
                let other = 1 - g;
                let a = app();
                // Dropped on the other side, or dragged well below the strip when not split.
                let onto_other = target == a.strips[other].get() || target == app::view(other).hwnd;
                let pulled_down = y > client(hwnd).bottom + px(60) && !a.st.borrow().split;
                if onto_other || pulled_down {
                    docs::activate(g, id);
                    docs::move_to_other_group();
                }
            }
            app::invalidate(R_ALL);
            0
        }
        WM_MBUTTONUP => {
            if let Hit::Tab(i) | Hit::Close(i) = hit_test(g, hwnd, x, y) {
                if let Some(id) = tab_id(g, i) {
                    docs::close_tab(g, id);
                }
            }
            0
        }
        WM_LBUTTONDBLCLK => {
            if hit_test(g, hwnd, x, y) == Hit::None {
                app().st.borrow_mut().active_group = g;
                docs::new_doc();
            }
            0
        }
        WM_RBUTTONUP => {
            if let Hit::Tab(i) | Hit::Close(i) = hit_test(g, hwnd, x, y) {
                if let Some(id) = tab_id(g, i) {
                    context_menu(hwnd, g, id);
                }
            }
            0
        }
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
            let delta = ((wp >> 16) & 0xffff) as i16 as i32;
            let step = if msg == WM_MOUSEWHEEL { -delta } else { delta };
            let rc = client(hwnd);
            let total = total_width(g, rc.bottom);
            {
                let mut st = app().st.borrow_mut();
                let grp = &mut st.groups[g];
                grp.scroll = (grp.scroll + step * px(60) / 120).clamp(0, (total - rc.right + px(32)).max(0));
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        _ => {
            let _ = R_LAYOUT;
            DefWindowProcW(hwnd, msg, wp, lp)
        }
    }
}

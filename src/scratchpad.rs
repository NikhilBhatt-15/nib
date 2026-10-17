//! The floating scratchpad: a small always-on-top Markdown pad toggled by a
//! global shortcut (default Win+Alt+N) from any app, like PowerToys' utilities.
//! Its text lives in scratchpad.md in the data folder and is saved as you type.

use crate::app::{self, app, set_status};
use crate::editor;
use crate::gfx;
use crate::languages;
use crate::sci::{Notification, Sci};
use crate::sci_consts::*;
use crate::util;
use std::cell::{Cell, RefCell};
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use windows_sys::w;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const TIMER_SAVE: usize = 1;

thread_local! {
    static WND: Cell<HWND> = const { Cell::new(null_mut()) };
    static VIEW: RefCell<Option<Sci>> = const { RefCell::new(None) };
    static DIRTY: Cell<bool> = const { Cell::new(false) };
}

fn view() -> Option<Sci> {
    VIEW.with(|v| *v.borrow())
}

fn create() {
    unsafe {
        let hinst = GetModuleHandleW(null());
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst,
            hIcon: app::app_icon(32),
            hIconSm: app::app_icon(16),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            lpszClassName: w!("NibScratch"),
            ..zeroed()
        };
        RegisterClassExW(&wc);
        let (x, y, wdt, h) = match app().st.borrow().persist.scratch_rect {
            Some([x, y, w, h]) if w > 150 && h > 100 => (x, y, w, h),
            _ => {
                let mut wa: RECT = zeroed();
                SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut wa as *mut RECT as _, 0);
                let (w, h) = (app::px(460), app::px(420));
                (wa.right - w - app::px(24), wa.top + app::px(24), w, h)
            }
        };
        let hwnd = CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_TOPMOST, w!("NibScratch"), w!("Scratchpad"), WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_CLIPCHILDREN, x, y, wdt, h, null_mut(), null_mut(), hinst, null());
        WND.with(|wn| wn.set(hwnd));
        let v = Sci::create(hwnd, 1);
        editor::init_view(&v);
        v.show(true);
        let text = std::fs::read(app().paths.scratchpad()).unwrap_or_default();
        v.set_text(&text);
        v.m(SCI_SETEOLMODE, SC_EOL_CRLF, 0);
        editor::set_language(&v, languages::by_id("markdown"), false);
        v.m(SCI_EMPTYUNDOBUFFER, 0, 0);
        v.m(SCI_SETSAVEPOINT, 0, 0);
        v.m(SCI_DOCUMENTEND, 0, 0);
        VIEW.with(|vw| *vw.borrow_mut() = Some(v));
        apply_theme();
        layout();
    }
}

pub fn apply_theme() {
    let Some(v) = view() else { return };
    let hwnd = WND.with(|w| w.get());
    let t = app::theme();
    let s = app::settings().clone();
    editor::style_view(&v, &t, &s, languages::by_id("markdown"), false);
    // A compact gutter: no line numbers here.
    for margin in 0..4 {
        v.m(SCI_SETMARGINWIDTHN, margin, 0);
    }
    v.m(SCI_SETWRAPMODE, SC_WRAP_WORD, 0);
    v.m(SCI_SETMARGINLEFT, 0, app::px(8) as i64);
    crate::darkmode::set_window_dark(v.hwnd, t.dark);
    gfx::style_frame(hwnd, &t, true);
}

fn layout() {
    let hwnd = WND.with(|w| w.get());
    let Some(v) = view() else { return };
    unsafe {
        let mut rc: RECT = zeroed();
        GetClientRect(hwnd, &mut rc);
        MoveWindow(v.hwnd, 0, 0, rc.right, rc.bottom, 1);
    }
}

pub fn toggle() {
    if WND.with(|w| w.get()).is_null() {
        create();
    }
    let hwnd = WND.with(|w| w.get());
    unsafe {
        if IsWindowVisible(hwnd) != 0 && GetForegroundWindow() == hwnd {
            hide();
            return;
        }
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        if let Some(v) = view() {
            SetFocus(v.hwnd);
        }
    }
}

fn hide() {
    save_now();
    let hwnd = WND.with(|w| w.get());
    unsafe { ShowWindow(hwnd, SW_HIDE) };
}

pub fn save_now() {
    let Some(v) = view() else { return };
    if !DIRTY.with(|d| d.replace(false)) {
        return;
    }
    let _ = crate::fileio::save_atomic(&app().paths.scratchpad(), &v.text());
    remember_rect();
}

fn remember_rect() {
    let hwnd = WND.with(|w| w.get());
    if hwnd.is_null() {
        return;
    }
    unsafe {
        let mut wp: WINDOWPLACEMENT = zeroed();
        wp.length = size_of::<WINDOWPLACEMENT>() as u32;
        if GetWindowPlacement(hwnd, &mut wp) != 0 {
            let r = wp.rcNormalPosition;
            app().st.borrow_mut().persist.scratch_rect = Some([r.left, r.top, r.right - r.left, r.bottom - r.top]);
        }
    }
}

/// Moves the scratchpad text into a new tab and clears the pad.
pub fn move_to_tab() {
    if view().is_none() {
        create();
    }
    let Some(v) = view() else { return };
    let text = v.text();
    if text.iter().all(|b| b.is_ascii_whitespace()) {
        set_status("The scratchpad is empty");
        return;
    }
    crate::docs::new_doc_with(text, Some(languages::by_id("markdown")));
    v.set_text(b"");
    DIRTY.with(|d| d.set(true));
    save_now();
    let hwnd = WND.with(|w| w.get());
    unsafe { ShowWindow(hwnd, SW_HIDE) };
    app::show_main();
}

pub fn pre_translate(msg: &MSG) -> bool {
    let Some(v) = view() else { return false };
    if msg.hwnd != v.hwnd || msg.message != WM_KEYDOWN {
        return false;
    }
    let ctrl = unsafe { GetKeyState(VK_CONTROL as i32) } < 0;
    match msg.wParam as u16 {
        VK_ESCAPE => hide(),
        0x53 if ctrl => {
            DIRTY.with(|d| d.set(true));
            save_now();
        }
        VK_RETURN if ctrl && unsafe { GetKeyState(VK_SHIFT as i32) } < 0 => move_to_tab(),
        _ => return false,
    }
    true
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_SIZE => {
            layout();
            0
        }
        WM_SETFOCUS => {
            if let Some(v) = view() {
                SetFocus(v.hwnd);
            }
            0
        }
        WM_NOTIFY => {
            let n = &*(lp as *const Notification);
            if n.code as i64 == SCN_MODIFIED && n.modification_type as i64 & (SC_MOD_INSERTTEXT | SC_MOD_DELETETEXT) != 0 {
                DIRTY.with(|d| d.set(true));
                SetTimer(hwnd, TIMER_SAVE, 600, None);
            } else if n.code as i64 == SCN_CHARADDED && n.ch == '\n' as i32 {
                if let Some(v) = view() {
                    editor::on_newline(&v, true);
                }
            }
            0
        }
        WM_TIMER => {
            KillTimer(hwnd, TIMER_SAVE);
            save_now();
            0
        }
        WM_ACTIVATE => {
            if (wp & 0xffff) as u32 == WA_INACTIVE {
                save_now();
                remember_rect();
            }
            0
        }
        WM_ERASEBKGND => {
            let mut rc: RECT = zeroed();
            GetClientRect(hwnd, &mut rc);
            let b = CreateSolidBrush(app::theme().bg.colorref());
            FillRect(wp as HDC, &rc, b);
            DeleteObject(b);
            1
        }
        WM_CLOSE => {
            hide();
            0
        }
        _ => {
            let _ = util::lo(0);
            DefWindowProcW(hwnd, msg, wp, lp)
        }
    }
}

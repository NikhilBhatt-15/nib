//! Theming for the parts Win32 draws itself: the menu bar, popup menus and
//! scrollbars.
//!
//! Popup menus and scrollbars follow uxtheme's undocumented app mode
//! (SetPreferredAppMode, ordinal 135) and the "DarkMode_Explorer" visual
//! style. The menu bar has no dark style at all, so it is painted here from
//! the undocumented WM_UAHDRAWMENU / WM_UAHDRAWMENUITEM messages, the way
//! Notepad++ and Windows Terminal's prototypes do it.

use crate::theme::Theme;
use std::mem::{size_of, zeroed};
use std::ptr::null;
use windows_sys::w;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_sys::Win32::UI::Controls::{SetWindowTheme, DRAWITEMSTRUCT, ODS_HOTLIGHT, ODS_NOACCEL, ODS_SELECTED, ODS_GRAYED, ODS_DISABLED};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub const WM_UAHDRAWMENU: u32 = 0x0091;
pub const WM_UAHDRAWMENUITEM: u32 = 0x0092;

#[repr(C)]
struct UahMenu {
    hmenu: HMENU,
    hdc: HDC,
    flags: u32,
}

#[repr(C)]
struct UahMenuItem {
    position: i32,
    metrics: [u32; 8],
    popup_metrics: [u32; 5],
}

#[repr(C)]
struct UahDrawMenuItem {
    dis: DRAWITEMSTRUCT,
    um: UahMenu,
    umi: UahMenuItem,
}

type SetPreferredAppMode = unsafe extern "system" fn(i32) -> i32;
type FlushMenuThemes = unsafe extern "system" fn();
type AllowDarkModeForWindow = unsafe extern "system" fn(HWND, i32) -> i32;

fn uxtheme_fn(ordinal: usize) -> Option<unsafe extern "system" fn() -> isize> {
    unsafe {
        let lib = LoadLibraryW(w!("uxtheme.dll"));
        if lib.is_null() {
            return None;
        }
        GetProcAddress(lib, ordinal as *const u8)
    }
}

/// Switches popup menus and common controls between light and dark.
pub fn set_app_dark(dark: bool) {
    unsafe {
        if let Some(f) = uxtheme_fn(135) {
            let f: SetPreferredAppMode = std::mem::transmute(f);
            f(if dark { 2 } else { 3 }); // ForceDark / ForceLight
        }
        if let Some(f) = uxtheme_fn(136) {
            let f: FlushMenuThemes = std::mem::transmute(f);
            f();
        }
    }
}

/// Dark or light scrollbars (and other themed parts) for one window.
pub fn set_window_dark(hwnd: HWND, dark: bool) {
    unsafe {
        if let Some(f) = uxtheme_fn(133) {
            let f: AllowDarkModeForWindow = std::mem::transmute(f);
            f(hwnd, dark as i32);
        }
        SetWindowTheme(hwnd, if dark { w!("DarkMode_Explorer") } else { w!("Explorer") }, null());
    }
}

fn menu_bar_rect(hwnd: HWND) -> Option<RECT> {
    unsafe {
        let mut mbi: MENUBARINFO = zeroed();
        mbi.cbSize = size_of::<MENUBARINFO>() as u32;
        if GetMenuBarInfo(hwnd, OBJID_MENU, 0, &mut mbi) == 0 {
            return None;
        }
        let mut wr: RECT = zeroed();
        GetWindowRect(hwnd, &mut wr);
        let mut r = mbi.rcBar;
        OffsetRect(&mut r, -wr.left, -wr.top);
        Some(r)
    }
}

/// Handles the menu-bar paint messages. Returns Some when the message was consumed.
pub fn handle_menu_bar(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM, t: &Theme) -> Option<LRESULT> {
    unsafe {
        match msg {
            WM_UAHDRAWMENU => {
                let um = &*(lp as *const UahMenu);
                if let Some(mut r) = menu_bar_rect(hwnd) {
                    r.top -= 1;
                    let b = CreateSolidBrush(t.chrome.colorref());
                    FillRect(um.hdc, &r, b);
                    DeleteObject(b);
                }
                Some(0)
            }
            WM_UAHDRAWMENUITEM => {
                let d = &*(lp as *const UahDrawMenuItem);
                let mut buf = [0u16; 256];
                let mut mii: MENUITEMINFOW = zeroed();
                mii.cbSize = size_of::<MENUITEMINFOW>() as u32;
                mii.fMask = MIIM_STRING;
                mii.dwTypeData = buf.as_mut_ptr();
                mii.cch = buf.len() as u32 - 1;
                GetMenuItemInfoW(d.um.hmenu, d.umi.position as u32, 1, &mut mii);
                let state = d.dis.itemState;
                let hot = state & (ODS_HOTLIGHT | ODS_SELECTED) != 0;
                let grey = state & (ODS_GRAYED | ODS_DISABLED) != 0;
                let bg = if hot { t.fg.mix(t.chrome, 0.12) } else { t.chrome };
                let b = CreateSolidBrush(bg.colorref());
                FillRect(d.um.hdc, &d.dis.rcItem, b);
                DeleteObject(b);
                let mut flags = DT_CENTER | DT_SINGLELINE | DT_VCENTER;
                if state & ODS_NOACCEL != 0 {
                    flags |= DT_HIDEPREFIX;
                }
                SetBkMode(d.um.hdc, TRANSPARENT as i32);
                SetTextColor(d.um.hdc, if grey { t.subtext.colorref() } else { t.fg.colorref() });
                let mut r = d.dis.rcItem;
                DrawTextW(d.um.hdc, buf.as_ptr(), -1, &mut r, flags);
                Some(0)
            }
            _ => {
                let _ = wp;
                None
            }
        }
    }
}

/// Paints over the 1px light line Windows draws under the menu bar. Call
/// after DefWindowProc for WM_NCPAINT and WM_NCACTIVATE.
pub fn paint_menu_bar_line(hwnd: HWND, t: &Theme) {
    unsafe {
        let mut client: RECT = zeroed();
        GetClientRect(hwnd, &mut client);
        let mut pts = [windows_sys::Win32::Foundation::POINT { x: client.left, y: client.top }, windows_sys::Win32::Foundation::POINT { x: client.right, y: client.bottom }];
        MapWindowPoints(hwnd, std::ptr::null_mut(), pts.as_mut_ptr(), 2);
        let mut wr: RECT = zeroed();
        GetWindowRect(hwnd, &mut wr);
        let left = pts[0].x - wr.left;
        let right = pts[1].x - wr.left;
        let top = pts[0].y - wr.top;
        let line = RECT { left, top: top - 1, right, bottom: top };
        let dc = GetWindowDC(hwnd);
        let b = CreateSolidBrush(t.chrome.colorref());
        FillRect(dc, &line, b);
        DeleteObject(b);
        ReleaseDC(hwnd, dc);
    }
}


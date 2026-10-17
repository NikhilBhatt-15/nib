//! Notification-area icon: open Nib, new note, Quick Open, scratchpad, exit.

use crate::app::{self, app};
use crate::{docs, palette, scratchpad};
use std::mem::{size_of, zeroed};
use std::ptr::null;
use std::sync::OnceLock;
use windows_sys::w;
use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT};
use windows_sys::Win32::UI::Shell::{Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub const WM_TRAY: u32 = WM_APP + 1;

pub fn taskbar_created() -> u32 {
    static MSG: OnceLock<u32> = OnceLock::new();
    *MSG.get_or_init(|| unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) })
}

fn data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut nid: NOTIFYICONDATAW = unsafe { zeroed() };
    nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = 1;
    nid
}

pub fn add(hwnd: HWND) {
    let mut nid = data(hwnd);
    nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    nid.uCallbackMessage = WM_TRAY;
    nid.hIcon = app::app_icon(16);
    let tip: Vec<u16> = format!("Nib — scratchpad: {}", app::settings().scratchpad_hotkey).encode_utf16().take(127).collect();
    nid.szTip[..tip.len()].copy_from_slice(&tip);
    unsafe { Shell_NotifyIconW(NIM_ADD, &nid) };
}

pub fn remove(hwnd: HWND) {
    let nid = data(hwnd);
    unsafe { Shell_NotifyIconW(NIM_DELETE, &nid) };
}

pub fn on_message(hwnd: HWND, lp: LPARAM) {
    match (lp & 0xffff) as u32 {
        WM_LBUTTONUP => app::show_main(),
        WM_RBUTTONUP | WM_CONTEXTMENU => show_menu(hwnd),
        _ => {}
    }
}

fn show_menu(hwnd: HWND) {
    unsafe {
        let m = CreatePopupMenu();
        let scratch: Vec<u16> = format!("Scratchpad\t{}", app::settings().scratchpad_hotkey).encode_utf16().chain(Some(0)).collect();
        AppendMenuW(m, MF_STRING, 1, w!("Open Nib"));
        AppendMenuW(m, MF_STRING, 2, w!("New note"));
        AppendMenuW(m, MF_STRING, 3, w!("Go to file…"));
        AppendMenuW(m, MF_STRING, 4, scratch.as_ptr());
        AppendMenuW(m, MF_SEPARATOR, 0, null());
        AppendMenuW(m, MF_STRING, 5, w!("Exit"));
        SetMenuDefaultItem(m, 1, 0);
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, 0, hwnd, null());
        DestroyMenu(m);
        match cmd {
            1 => app::show_main(),
            2 => {
                app::show_main();
                docs::new_doc();
            }
            3 => {
                app::show_main();
                palette::open_quick("");
            }
            4 => scratchpad::toggle(),
            5 => {
                app().quitting.set(false);
                app::quit();
            }
            _ => {}
        }
    }
}

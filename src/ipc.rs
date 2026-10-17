//! Single instance: a second launch hands its arguments to the running Nib
//! over WM_COPYDATA and exits, so opening files from Explorer is instant.

use crate::app::{self, CLASS};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::ptr::null;
use windows_sys::w;
use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows_sys::Win32::System::DataExchange::COPYDATASTRUCT;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const MAGIC: usize = 0x4E49_4201;

#[derive(Serialize, Deserialize)]
struct Payload {
    cwd: String,
    args: Vec<String>,
}

/// True if another instance is running and took the arguments.
pub fn try_forward(args: &[String], cwd: &std::path::Path) -> bool {
    unsafe {
        CreateMutexW(null(), 1, w!("Local\\Nib-single-instance"));
        if GetLastError() != ERROR_ALREADY_EXISTS {
            return false;
        }
        // The first instance may still be starting: give its window a moment.
        let mut hwnd = std::ptr::null_mut();
        for _ in 0..50 {
            hwnd = FindWindowW(CLASS, null());
            if !hwnd.is_null() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        if hwnd.is_null() {
            return false;
        }
        let json = serde_json::to_vec(&Payload { cwd: cwd.display().to_string(), args: args.to_vec() }).unwrap_or_default();
        let cds = COPYDATASTRUCT { dwData: MAGIC, cbData: json.len() as u32, lpData: json.as_ptr() as *mut _ };
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        AllowSetForegroundWindow(pid);
        SendMessageTimeoutW(hwnd, WM_COPYDATA, 0, &cds as *const COPYDATASTRUCT as isize, SMTO_ABORTIFHUNG, 5000, std::ptr::null_mut());
        true
    }
}

pub fn on_copydata(cds: &COPYDATASTRUCT) {
    if cds.dwData != MAGIC || cds.lpData.is_null() {
        return;
    }
    let bytes = unsafe { std::slice::from_raw_parts(cds.lpData as *const u8, cds.cbData as usize) };
    let Ok(p) = serde_json::from_slice::<Payload>(bytes) else { return };
    let a = app::parse_args(&p.args, &PathBuf::from(&p.cwd));
    let hwnd = app::app().hwnd.get();
    if a.scratch {
        unsafe { PostMessageW(hwnd, WM_HOTKEY, 1, 0) };
    }
    if a.new_note {
        if let Some(i) = crate::commands::index_of("file.new") {
            unsafe { PostMessageW(hwnd, WM_COMMAND, 1000 + i, 0) };
        }
    }
    for c in &a.commands {
        if let Some(i) = crate::commands::index_of(c) {
            unsafe { PostMessageW(hwnd, WM_COMMAND, 1000 + i, 0) };
        }
    }
    if !a.files.is_empty() {
        app::post_open(a.files);
    } else if !a.scratch && !a.tray && a.commands.is_empty() {
        unsafe { PostMessageW(hwnd, app::WM_APP_OPEN, 0, Box::into_raw(Box::new(Vec::<PathBuf>::new())) as isize) };
    }
}

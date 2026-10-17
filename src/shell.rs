//! File Explorer integration, per user (no admin rights): an "Edit with Nib"
//! entry on every file's context menu, and Nib in the "Open with" list.

use crate::app::set_status;
use crate::util::wide;
use std::ptr::{null, null_mut};
use windows_sys::Win32::System::Registry::*;
use windows_sys::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};

const VERB_KEY: &str = r"Software\Classes\*\shell\Nib";
const APP_KEY: &str = r"Software\Classes\Applications\nib.exe";

fn set_value(key: &str, name: Option<&str>, value: &str) -> bool {
    let k = wide(key);
    let n = name.map(wide);
    let v = wide(value);
    unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, k.as_ptr(), n.as_ref().map_or(null(), |n| n.as_ptr()), REG_SZ, v.as_ptr().cast(), (v.len() * 2) as u32) == 0 }
}

pub fn explorer_menu_installed() -> bool {
    let k = wide(VERB_KEY);
    let mut h = null_mut();
    unsafe {
        let ok = RegOpenKeyExW(HKEY_CURRENT_USER, k.as_ptr(), 0, KEY_READ, &mut h) == 0;
        if ok {
            RegCloseKey(h);
        }
        ok
    }
}

pub fn install() -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let exe = exe.display().to_string();
    let cmd = format!("\"{exe}\" \"%1\"");
    let ok = set_value(VERB_KEY, None, "Edit with &Nib")
        && set_value(VERB_KEY, Some("Icon"), &format!("\"{exe}\",0"))
        && set_value(&format!(r"{VERB_KEY}\command"), None, &cmd)
        && set_value(&format!(r"{APP_KEY}\shell\open\command"), None, &cmd)
        && set_value(APP_KEY, Some("FriendlyAppName"), "Nib");
    for ext in [".txt", ".md", ".log", ".json", ".ini", ".csv", ".xml", ".yaml", ".yml", ".toml"] {
        set_value(&format!(r"{APP_KEY}\SupportedTypes"), Some(ext), "");
        set_value(&format!(r"Software\Classes\{ext}\OpenWithList\nib.exe"), None, "");
    }
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED as i32, SHCNF_IDLIST, null(), null()) };
    ok
}

pub fn uninstall() {
    for key in [VERB_KEY, APP_KEY] {
        let k = wide(key);
        unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, k.as_ptr()) };
    }
    for ext in [".txt", ".md", ".log", ".json", ".ini", ".csv", ".xml", ".yaml", ".yml", ".toml"] {
        let k = wide(&format!(r"Software\Classes\{ext}\OpenWithList\nib.exe"));
        unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, k.as_ptr()) };
    }
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED as i32, SHCNF_IDLIST, null(), null()) };
}

pub fn toggle_explorer_menu() {
    if explorer_menu_installed() {
        uninstall();
        set_status("Removed 'Edit with Nib' from File Explorer");
    } else if install() {
        set_status("Added 'Edit with Nib' to File Explorer (Windows 11: under 'Show more options')");
    } else {
        set_status("Couldn't update the File Explorer menu");
    }
}

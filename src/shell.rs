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

// ------------------------------------------------------------------ Start menu shortcut
//
// Windows search only finds apps with a Start menu shortcut, and Nib has no
// installer, so it can create its own: a .lnk in the per-user Programs folder
// written through IShellLinkW / IPersistFile (called through their vtables;
// windows-sys has no COM wrappers).

use std::ffi::c_void;
use std::path::PathBuf;
use windows_sys::core::GUID;
use windows_sys::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};

const CLSID_SHELL_LINK: GUID = GUID::from_u128(0x00021401_0000_0000_c000_000000000046);
const IID_ISHELL_LINK_W: GUID = GUID::from_u128(0x000214f9_0000_0000_c000_000000000046);
const IID_IPERSIST_FILE: GUID = GUID::from_u128(0x0000010b_0000_0000_c000_000000000046);

type QueryInterface = unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> i32;
type Release = unsafe extern "system" fn(*mut c_void) -> u32;
type SetString = unsafe extern "system" fn(*mut c_void, *const u16) -> i32;
type SetIcon = unsafe extern "system" fn(*mut c_void, *const u16, i32) -> i32;
type Save = unsafe extern "system" fn(*mut c_void, *const u16, i32) -> i32;

/// The `index`-th entry of a COM object's vtable.
unsafe fn method(obj: *mut c_void, index: usize) -> *const c_void {
    let vtbl = *(obj as *const *const *const c_void);
    *vtbl.add(index)
}

pub fn start_menu_shortcut() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(appdata).join(r"Microsoft\Windows\Start Menu\Programs\Nib.lnk"))
}

pub fn start_menu_installed() -> bool {
    start_menu_shortcut().is_some_and(|p| p.exists())
}

fn create_shortcut(lnk: &std::path::Path, target: &std::path::Path) -> bool {
    let dir = target.parent().map(|d| d.display().to_string()).unwrap_or_default();
    unsafe {
        let mut link: *mut c_void = null_mut();
        if CoCreateInstance(&CLSID_SHELL_LINK, null_mut(), CLSCTX_INPROC_SERVER, &IID_ISHELL_LINK_W, &mut link) != 0 || link.is_null() {
            return false;
        }
        let target_w = wide(&target.display().to_string());
        let dir_w = wide(&dir);
        let desc_w = wide("Nib — fast, private text editor");
        // IShellLinkW: SetDescription = 7, SetWorkingDirectory = 9, SetIconLocation = 17, SetPath = 20.
        std::mem::transmute::<_, SetString>(method(link, 20))(link, target_w.as_ptr());
        std::mem::transmute::<_, SetString>(method(link, 9))(link, dir_w.as_ptr());
        std::mem::transmute::<_, SetString>(method(link, 7))(link, desc_w.as_ptr());
        std::mem::transmute::<_, SetIcon>(method(link, 17))(link, target_w.as_ptr(), 0);
        let mut file: *mut c_void = null_mut();
        let ok = std::mem::transmute::<_, QueryInterface>(method(link, 0))(link, &IID_IPERSIST_FILE, &mut file) == 0 && !file.is_null() && {
            let lnk_w = wide(&lnk.display().to_string());
            // IPersistFile::Save = 6.
            let saved = std::mem::transmute::<_, Save>(method(file, 6))(file, lnk_w.as_ptr(), 1) == 0;
            std::mem::transmute::<_, Release>(method(file, 2))(file);
            saved
        };
        std::mem::transmute::<_, Release>(method(link, 2))(link);
        ok
    }
}

pub fn toggle_start_menu() {
    let Some(lnk) = start_menu_shortcut() else { return };
    if lnk.exists() {
        let _ = std::fs::remove_file(&lnk);
        set_status("Removed Nib from the Start menu");
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    if create_shortcut(&lnk, &exe) {
        set_status("Added Nib to the Start menu: it now shows up in Windows search");
    } else {
        set_status("Couldn't create the Start menu shortcut");
    }
}

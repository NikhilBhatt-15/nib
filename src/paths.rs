//! Where Nib keeps its data. Normally %APPDATA%\Nib; in portable mode (a file
//! named `portable` next to nib.exe) everything lives in a `data` folder
//! beside the executable so the whole thing can travel on a USB stick.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Paths {
    pub data: PathBuf,
    pub portable: bool,
}

impl Paths {
    pub fn resolve() -> Paths {
        let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf));
        if let Some(dir) = &exe_dir {
            if dir.join("portable").exists() || dir.join("portable.txt").exists() {
                let data = dir.join("data");
                if std::fs::create_dir_all(&data).is_ok() {
                    return Paths { data, portable: true };
                }
            }
        }
        let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        let data = base.join("Nib");
        let _ = std::fs::create_dir_all(&data);
        Paths { data, portable: false }
    }

    fn sub(&self, name: &str) -> PathBuf {
        let p = self.data.join(name);
        let _ = std::fs::create_dir_all(&p);
        p
    }

    pub fn settings(&self) -> PathBuf {
        self.data.join("settings.json")
    }
    pub fn session(&self) -> PathBuf {
        self.sub("session")
    }
    pub fn history(&self) -> PathBuf {
        self.sub("history")
    }
    pub fn templates(&self) -> PathBuf {
        self.sub("templates")
    }
    pub fn snippets(&self) -> PathBuf {
        self.data.join("snippets.json")
    }
    pub fn scratchpad(&self) -> PathBuf {
        self.data.join("scratchpad.md")
    }
    pub fn state(&self) -> PathBuf {
        self.data.join("state.json")
    }

    /// Default notes folder: Documents\Nib Notes, or data\notes when portable.
    pub fn default_notes(&self) -> PathBuf {
        if self.portable {
            return self.data.join("notes");
        }
        documents_dir().map(|d| d.join("Nib Notes")).unwrap_or_else(|| self.data.join("notes"))
    }
}

fn documents_dir() -> Option<PathBuf> {
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{FOLDERID_Documents, SHGetKnownFolderPath};
    unsafe {
        let mut p = std::ptr::null_mut();
        if SHGetKnownFolderPath(&FOLDERID_Documents, 0, std::ptr::null_mut(), &mut p) != 0 {
            return None;
        }
        let s = crate::util::from_wide(p);
        CoTaskMemFree(p as _);
        (!s.is_empty()).then(|| PathBuf::from(s))
    }
}

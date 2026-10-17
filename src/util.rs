//! Small helpers shared across modules.

use std::ptr::{null, null_mut};
use std::time::{SystemTime, UNIX_EPOCH};
use windows_sys::Win32::Foundation::{GlobalFree, HWND};
use windows_sys::Win32::System::DataExchange::*;
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub const CF_UNICODETEXT: u32 = 13;

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// UTF-16 without the terminating NUL.
pub fn wide_nz(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

pub fn from_wide(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    unsafe {
        let mut len = 0;
        while *p.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Random 64-bit id, from the OS-seeded hasher keys.
pub fn new_id() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(now_ms());
    h.write_u64(COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    h.finish() | 1
}

pub fn human_bytes(n: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else if v < 10.0 {
        format!("{v:.1} {}", U[i])
    } else {
        format!("{v:.0} {}", U[i])
    }
}

/// Local date and time from ms since the epoch: (y, mo, d, h, mi, s, weekday).
pub fn local_time(ms: u64) -> (i32, u32, u32, u32, u32, u32, u32) {
    use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows_sys::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
    let ft_val = ms * 10_000 + 116_444_736_000_000_000;
    let ft = FILETIME { dwLowDateTime: ft_val as u32, dwHighDateTime: (ft_val >> 32) as u32 };
    unsafe {
        let mut utc: SYSTEMTIME = std::mem::zeroed();
        let mut local: SYSTEMTIME = std::mem::zeroed();
        FileTimeToSystemTime(&ft, &mut utc);
        SystemTimeToTzSpecificLocalTime(null(), &utc, &mut local);
        (local.wYear as i32, local.wMonth as u32, local.wDay as u32, local.wHour as u32, local.wMinute as u32, local.wSecond as u32, local.wDayOfWeek as u32)
    }
}

pub fn fmt_datetime(ms: u64) -> String {
    let (y, mo, d, h, mi, _, _) = local_time(ms);
    format!("{y}-{mo:02}-{d:02} {h:02}:{mi:02}")
}

/// "just now", "5 min ago", "Yesterday 14:03", or a date.
pub fn fmt_ago(ms: u64) -> String {
    let now = now_ms();
    let diff = now.saturating_sub(ms) / 1000;
    let (_, _, d_then, h, mi, _, _) = local_time(ms);
    let (_, _, d_now, ..) = local_time(now);
    if diff < 45 {
        "just now".into()
    } else if diff < 3600 {
        format!("{} min ago", (diff + 30) / 60)
    } else if diff < 86_400 && d_then == d_now {
        format!("Today {h:02}:{mi:02}")
    } else if diff < 2 * 86_400 {
        format!("Yesterday {h:02}:{mi:02}")
    } else {
        fmt_datetime(ms)
    }
}

pub fn clipboard_text(owner: HWND) -> Option<String> {
    unsafe {
        if IsClipboardFormatAvailable(CF_UNICODETEXT) == 0 {
            return None;
        }
        for _ in 0..10 {
            if OpenClipboard(owner) != 0 {
                let h = GetClipboardData(CF_UNICODETEXT);
                let mut out = None;
                if !h.is_null() {
                    let p = GlobalLock(h as _) as *const u16;
                    if !p.is_null() {
                        out = Some(from_wide(p));
                        GlobalUnlock(h as _);
                    }
                }
                CloseClipboard();
                return out;
            }
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        None
    }
}

pub fn set_clipboard_text(owner: HWND, text: &str) -> bool {
    let w = wide(text);
    unsafe {
        for _ in 0..10 {
            if OpenClipboard(owner) != 0 {
                EmptyClipboard();
                let h = GlobalAlloc(GMEM_MOVEABLE, w.len() * 2);
                let mut ok = false;
                if !h.is_null() {
                    let p = GlobalLock(h) as *mut u16;
                    std::ptr::copy_nonoverlapping(w.as_ptr(), p, w.len());
                    GlobalUnlock(h);
                    ok = !SetClipboardData(CF_UNICODETEXT, h as _).is_null();
                    if !ok {
                        GlobalFree(h);
                    }
                }
                CloseClipboard();
                return ok;
            }
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        false
    }
}

pub fn message_box(owner: HWND, text: &str, flags: MESSAGEBOX_STYLE) -> MESSAGEBOX_RESULT {
    let t = wide(text);
    unsafe { MessageBoxW(owner, t.as_ptr(), windows_sys::w!("Nib"), flags) }
}

pub fn error_box(owner: HWND, text: &str) {
    message_box(owner, text, MB_OK | MB_ICONWARNING);
}

pub fn lo(lp: isize) -> i32 {
    (lp & 0xffff) as i16 as i32
}
pub fn hi(lp: isize) -> i32 {
    ((lp >> 16) & 0xffff) as i16 as i32
}

/// Opens a URL or file with its default handler.
pub fn shell_open(target: &str) {
    let t = wide(target);
    unsafe { windows_sys::Win32::UI::Shell::ShellExecuteW(null_mut(), windows_sys::w!("open"), t.as_ptr(), null(), null(), SW_SHOWNORMAL) };
}

/// Shows a file selected in Explorer.
pub fn reveal_in_explorer(path: &std::path::Path) {
    let args = wide(&format!("/select,\"{}\"", path.display()));
    unsafe { windows_sys::Win32::UI::Shell::ShellExecuteW(null_mut(), windows_sys::w!("open"), windows_sys::w!("explorer.exe"), args.as_ptr(), null(), SW_SHOWNORMAL) };
}

/// Counts words (runs of letters/digits/apostrophes) and Unicode characters.
pub fn count_words_chars(bytes: &[u8]) -> (usize, usize) {
    let mut words = 0;
    let mut chars = 0;
    let mut in_word = false;
    let s = String::from_utf8_lossy(bytes);
    for c in s.chars() {
        if c != '\r' && c != '\n' {
            chars += 1;
        }
        let w = c.is_alphanumeric() || c == '\'' || c == '_' || c == '’';
        if w && !in_word {
            words += 1;
        }
        in_word = w;
    }
    (words, chars)
}

pub fn group_digits(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts() {
        assert_eq!(count_words_chars(b"Hello, world! It's 2026."), (4, 24));
        assert_eq!(count_words_chars(b""), (0, 0));
        assert_eq!(count_words_chars("naïve café\r\n".as_bytes()), (2, 10));
    }

    #[test]
    fn formatting() {
        assert_eq!(group_digits(1234567), "1,234,567");
        assert_eq!(group_digits(12), "12");
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1536), "1.5 KB");
        assert_ne!(new_id(), new_id());
    }
}

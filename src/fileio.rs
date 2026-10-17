//! Disk I/O that can't leave a half-written file behind.
//!
//! User files are written to a temporary sibling, flushed, then swapped in
//! with ReplaceFileW (which keeps the original's attributes, ACLs and
//! creation time). Nib's own state files use the sequence Windows Notepad
//! uses for its TabState snapshots: write `x.tmp`, rename the old `x` to
//! `x.bak`, rename `x.tmp` to `x`, then delete `x.bak`. Each state file
//! carries a CRC so a torn or corrupted copy is rejected and the `.bak` is
//! read instead.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr::null;
use std::time::UNIX_EPOCH;
use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, ReplaceFileW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, REPLACEFILE_IGNORE_MERGE_ERRORS};

pub fn wide_path(p: &Path) -> Vec<u16> {
    p.as_os_str().encode_wide().chain(Some(0)).collect()
}

/// Modification time (ms since the epoch) and size, used to notice changes made by other programs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Stamp {
    pub mtime: u64,
    pub size: u64,
}

pub fn stamp(path: &Path) -> Option<Stamp> {
    let m = fs::metadata(path).ok()?;
    let mtime = m.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_millis() as u64;
    Some(Stamp { mtime, size: m.len() })
}

fn temp_sibling(path: &Path, tag: &str) -> PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let nonce = std::process::id() as u64 ^ crate::util::now_ms();
    path.with_file_name(format!(".{name}.{tag}-{nonce:x}"))
}

fn write_synced(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut f = OpenOptions::new().write(true).create(true).truncate(true).open(path)?;
    f.write_all(bytes)?;
    f.sync_all()
}

fn move_replace(from: &Path, to: &Path) -> io::Result<()> {
    let ok = unsafe { MoveFileExW(wide_path(from).as_ptr(), wide_path(to).as_ptr(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH) };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Saves a user's file without a window where it is empty or partial.
pub fn save_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = temp_sibling(path, "nib-save");
    if write_synced(&tmp, bytes).is_err() {
        // No permission to create files in that folder: write in place.
        let _ = fs::remove_file(&tmp);
        return write_synced(path, bytes);
    }
    let result = if path.exists() {
        let ok = unsafe { ReplaceFileW(wide_path(path).as_ptr(), wide_path(&tmp).as_ptr(), null(), REPLACEFILE_IGNORE_MERGE_ERRORS, null(), null()) };
        if ok != 0 {
            Ok(())
        } else {
            // ReplaceFileW fails on some network shares and FAT volumes.
            move_replace(&tmp, path)
        }
    } else {
        move_replace(&tmp, path)
    };
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
        return write_synced(path, bytes);
    }
    Ok(())
}

const MAGIC: &[u8; 4] = b"NIB1";

/// Writes an internal state file: magic, CRC32 and length header, then data.
pub fn write_state(path: &Path, data: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension(ext_plus(path, "tmp"));
    let bak = path.with_extension(ext_plus(path, "bak"));
    {
        let mut f = File::create(&tmp)?;
        let mut header = Vec::with_capacity(16);
        header.extend_from_slice(MAGIC);
        header.extend_from_slice(&crc32(data).to_le_bytes());
        header.extend_from_slice(&(data.len() as u64).to_le_bytes());
        f.write_all(&header)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    if path.exists() {
        let _ = fs::remove_file(&bak);
        move_replace(path, &bak)?;
    }
    move_replace(&tmp, path)?;
    let _ = fs::remove_file(&bak);
    Ok(())
}

/// Reads a state file, falling back to the `.bak` left by an interrupted
/// write when the main file is missing or fails its CRC.
pub fn read_state(path: &Path) -> Option<Vec<u8>> {
    let bak = path.with_extension(ext_plus(path, "bak"));
    fs::read(path).ok().and_then(|b| parse_state(&b)).or_else(|| fs::read(bak).ok().and_then(|b| parse_state(&b)))
}

pub fn remove_state(path: &Path) {
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(path.with_extension(ext_plus(path, "bak")));
    let _ = fs::remove_file(path.with_extension(ext_plus(path, "tmp")));
}

fn ext_plus(path: &Path, extra: &str) -> String {
    match path.extension() {
        Some(e) => format!("{}.{extra}", e.to_string_lossy()),
        None => extra.to_string(),
    }
}

fn parse_state(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() < 16 || &bytes[..4] != MAGIC {
        return None;
    }
    let crc = u32::from_le_bytes(bytes[4..8].try_into().ok()?);
    let len = u64::from_le_bytes(bytes[8..16].try_into().ok()?) as usize;
    let data = bytes.get(16..16 + len)?;
    (crc32(data) == crc).then(|| data.to_vec())
}

pub fn crc32(data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let t = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, e) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            *e = c;
        }
        t
    });
    let mut c = !0u32;
    for &b in data {
        c = t[((c ^ b as u32) & 0xff) as usize] ^ (c >> 8);
    }
    !c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("nib-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn crc_matches_reference() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn state_round_trip_and_corruption_falls_back_to_bak() {
        let d = tmpdir("state");
        let p = d.join("tab.bin");
        write_state(&p, b"first").unwrap();
        assert_eq!(read_state(&p).unwrap(), b"first");
        write_state(&p, b"second").unwrap();
        assert_eq!(read_state(&p).unwrap(), b"second");
        assert!(!p.with_extension("bin.bak").exists());
        assert!(!p.with_extension("bin.tmp").exists());

        // Simulate a crash between "rename old to .bak" and "rename tmp into place".
        fs::rename(&p, p.with_extension("bin.bak")).unwrap();
        assert_eq!(read_state(&p).unwrap(), b"second");

        // A torn main file is rejected by its CRC.
        fs::write(&p, b"NIB1garbage-garbage").unwrap();
        assert_eq!(read_state(&p).unwrap(), b"second");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn save_atomic_creates_and_replaces() {
        let d = tmpdir("save");
        let p = d.join("note.txt");
        save_atomic(&p, b"one").unwrap();
        save_atomic(&p, b"two").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"two");
        let leftovers = fs::read_dir(&d).unwrap().count();
        assert_eq!(leftovers, 1);
        let _ = fs::remove_dir_all(&d);
    }
}

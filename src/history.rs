//! Version history: every save keeps a copy of the file as written, in
//! `history/<hash of path>/<unix ms>.bin`, with identical consecutive
//! versions skipped and the oldest pruned. Versions open in a comparison
//! against the current text, from where they can be restored.

use crate::app::{self, app, set_status};
use crate::fileio::crc32;
use crate::gfx::icon;
use crate::palette::{self, Item, Kind};
use crate::util::{self, now_ms};
use std::path::{Path, PathBuf};

fn key(path: &Path) -> String {
    let s = path.display().to_string().to_lowercase();
    format!("{:08x}{:08x}", crc32(s.as_bytes()), s.len() as u32 ^ crc32(s.chars().rev().collect::<String>().as_bytes()))
}

fn dir_for(path: &Path) -> PathBuf {
    app().paths.history().join(key(path))
}

/// (ms, size, file) newest first.
pub fn versions(path: &Path) -> Vec<(u64, u64, PathBuf)> {
    let Ok(rd) = std::fs::read_dir(dir_for(path)) else { return Vec::new() };
    let mut v: Vec<(u64, u64, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let ms = p.file_stem()?.to_str()?.parse::<u64>().ok()?;
            (p.extension()? == "bin").then(|| (ms, e.metadata().map(|m| m.len()).unwrap_or(0), p))
        })
        .collect();
    v.sort_by(|a, b| b.0.cmp(&a.0));
    v
}

/// Called after a successful save. `previous` is the file's content and
/// modification time before the save, kept as the first version when the
/// file has no history yet.
pub fn record(path: &Path, previous: Option<(&[u8], u64)>, written: &[u8]) {
    let (enabled, max_versions, max_bytes) = {
        let s = app::settings();
        (s.history.enabled, s.history.max_versions as usize, s.history.max_file_mb as usize * 1024 * 1024)
    };
    if !enabled || written.len() > max_bytes || path.starts_with(&app().paths.data) {
        return;
    }
    let dir = dir_for(path);
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let _ = std::fs::write(dir.join("path.txt"), path.display().to_string());
    let existing = versions(path);
    let now = now_ms();
    if existing.is_empty() {
        // The version from before Nib first saved, dated by its own modification time.
        if let Some((prev, mtime)) = previous.filter(|(p, _)| *p != written && p.len() <= max_bytes) {
            let when = if mtime > 0 && mtime < now { mtime } else { now - 1 };
            let _ = std::fs::write(dir.join(format!("{when}.bin")), prev);
        }
    }
    let same_as_last = existing.first().is_some_and(|(_, size, p)| *size == written.len() as u64 && std::fs::read(p).is_ok_and(|b| b == written));
    if !same_as_last {
        let _ = std::fs::write(dir.join(format!("{now}.bin")), written);
    }
    for (_, _, p) in versions(path).into_iter().skip(max_versions) {
        let _ = std::fs::remove_file(p);
    }
}

pub fn show_for(doc: u64) {
    let (path, enc, name) = {
        let st = app().st.borrow();
        let Some(d) = st.doc(doc) else { return };
        (d.path.clone(), d.enc, d.name())
    };
    let Some(path) = path else {
        set_status("Version history starts once the note is saved to a file");
        return;
    };
    let list = versions(&path);
    if list.is_empty() {
        set_status("No saved versions yet. Nib keeps one each time you save.");
        return;
    }
    let items: Vec<Item> = list
        .iter()
        .enumerate()
        .map(|(i, (ms, size, _))| Item::new(icon::CLOCK, util::fmt_ago(*ms), Kind::Index(i)).detail(util::fmt_datetime(*ms)).hint(util::human_bytes(*size)))
        .collect();
    palette::pick(&format!("Versions of {name} — pick one to compare with the current text"), items, true, move |i| {
        let (ms, _, p) = &list[i];
        match std::fs::read(p) {
            Ok(bytes) => {
                let text = crate::textenc::decode(&bytes, enc);
                crate::compare::start_with_text(doc, &format!("{name} @ {}", util::fmt_datetime(*ms)), text, true);
            }
            Err(e) => set_status(&format!("Couldn't read that version: {e}")),
        }
    });
}

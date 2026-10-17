//! Session persistence and crash recovery.
//!
//! Modelled on what Windows Notepad does with its TabState/WindowState
//! folders (see notepad-analysis/FINDINGS.md in the workspace):
//!
//! * Each tab with unsaved text has its own buffer file, `buffers/<id>.bin`,
//!   written with the tmp → bak → rename sequence and a CRC header, so a
//!   crash mid-write leaves either the old or the new copy readable.
//! * The tab list (the manifest) alternates between two slots,
//!   `session.0.json` and `session.1.json`, each stamped with a sequence
//!   number. Loading takes the newest slot that passes its CRC, so a torn
//!   manifest write falls back to the previous one.
//! * `running.lock` exists while Nib runs. Finding it at start-up means the
//!   last run ended without shutting down cleanly.
//!
//! Writes happen on a background thread so typing never waits on the disk.

use crate::doc::ViewState;
use crate::fileio::{self, Stamp};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Sender};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionDoc {
    pub id: u64,
    pub path: Option<String>,
    pub untitled: u32,
    pub enc: String,
    pub eol: String,
    pub lang: String,
    pub dirty: bool,
    /// Text lives in buffers/<id>.bin.
    pub buffer: bool,
    pub stamp: Option<Stamp>,
    pub view: [ViewState; 2],
    pub preview: bool,
    pub pinned: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Manifest {
    pub seq: u64,
    pub docs: Vec<SessionDoc>,
    pub groups: [Vec<u64>; 2],
    pub active: [u64; 2],
    pub active_group: usize,
    pub split: bool,
    pub split_ratio: f32,
    pub next_untitled: u32,
}

enum Job {
    Write(PathBuf, Vec<u8>),
    Remove(PathBuf),
    Flush(Sender<()>),
}

pub struct Store {
    dir: PathBuf,
    tx: Sender<Job>,
    seq: std::cell::Cell<u64>,
}

impl Store {
    pub fn open(dir: &Path) -> Store {
        let _ = std::fs::create_dir_all(dir.join("buffers"));
        let (tx, rx) = channel::<Job>();
        std::thread::spawn(move || {
            for job in rx {
                match job {
                    Job::Write(p, data) => {
                        let _ = fileio::write_state(&p, &data);
                    }
                    Job::Remove(p) => fileio::remove_state(&p),
                    Job::Flush(done) => {
                        let _ = done.send(());
                    }
                }
            }
        });
        Store { dir: dir.to_path_buf(), tx, seq: std::cell::Cell::new(0) }
    }

    fn slot(&self, n: u64) -> PathBuf {
        self.dir.join(format!("session.{}.json", n % 2))
    }

    pub fn buffer_path(&self, id: u64) -> PathBuf {
        self.dir.join("buffers").join(format!("{id:016x}.bin"))
    }

    pub fn write_buffer(&self, id: u64, text: Vec<u8>) {
        let _ = self.tx.send(Job::Write(self.buffer_path(id), text));
    }

    pub fn remove_buffer(&self, id: u64) {
        let _ = self.tx.send(Job::Remove(self.buffer_path(id)));
    }

    /// Reads a buffer synchronously, waiting for queued writes first.
    pub fn read_buffer(&self, id: u64) -> Option<Vec<u8>> {
        self.flush();
        fileio::read_state(&self.buffer_path(id))
    }

    pub fn write_manifest(&self, mut m: Manifest) {
        let seq = self.seq.get() + 1;
        self.seq.set(seq);
        m.seq = seq;
        if let Ok(json) = serde_json::to_vec(&m) {
            let _ = self.tx.send(Job::Write(self.slot(seq), json));
        }
    }

    /// The newest manifest that reads back intact.
    pub fn load_manifest(&self) -> Option<Manifest> {
        let best = (0..2)
            .filter_map(|n| fileio::read_state(&self.slot(n)))
            .filter_map(|b| serde_json::from_slice::<Manifest>(&b).ok())
            .max_by_key(|m| m.seq)?;
        self.seq.set(best.seq);
        Some(best)
    }

    /// Blocks until every queued write has reached the disk.
    pub fn flush(&self) {
        let (tx, rx) = channel();
        if self.tx.send(Job::Flush(tx)).is_ok() {
            let _ = rx.recv_timeout(std::time::Duration::from_secs(20));
        }
    }

    /// Deletes buffer files no manifest entry refers to (left by closed tabs or crashes).
    pub fn prune_buffers(&self, keep: &[u64]) {
        let Ok(rd) = std::fs::read_dir(self.dir.join("buffers")) else { return };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let id = name.split('.').next().and_then(|h| u64::from_str_radix(h, 16).ok());
            if id.is_some_and(|id| !keep.contains(&id)) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }

    fn lock_path(&self) -> PathBuf {
        self.dir.join("running.lock")
    }

    /// Marks the session as running. Returns true if the previous run didn't exit cleanly.
    pub fn begin(&self) -> bool {
        let crashed = self.lock_path().exists();
        let _ = std::fs::write(self.lock_path(), std::process::id().to_string());
        crashed
    }

    pub fn end(&self) {
        self.flush();
        let _ = std::fs::remove_file(self.lock_path());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_valid_slot_wins_and_torn_slot_is_skipped() {
        let dir = std::env::temp_dir().join(format!("nib-session-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let s = Store::open(&dir);
        assert!(!s.begin());
        for i in 1..=3u32 {
            s.write_manifest(Manifest { next_untitled: i, ..Default::default() });
        }
        s.flush();
        let s2 = Store::open(&dir);
        assert_eq!(s2.load_manifest().unwrap().next_untitled, 3);
        // Corrupt the newest slot (seq 3 lives in slot 1): the older one is used.
        std::fs::write(dir.join("session.1.json"), b"NIB1-torn").unwrap();
        let _ = std::fs::remove_file(dir.join("session.1.json.bak"));
        let s3 = Store::open(&dir);
        assert_eq!(s3.load_manifest().unwrap().next_untitled, 2);
        // An unclean exit is detected.
        let s4 = Store::open(&dir);
        assert!(s4.begin());
        s4.end();
        assert!(!Store::open(&dir).begin());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn buffers_round_trip_and_prune() {
        let dir = std::env::temp_dir().join(format!("nib-buffers-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let s = Store::open(&dir);
        s.write_buffer(7, b"seven".to_vec());
        s.write_buffer(8, b"eight".to_vec());
        assert_eq!(s.read_buffer(7).unwrap(), b"seven");
        s.prune_buffers(&[8]);
        assert!(s.read_buffer(7).is_none());
        assert_eq!(s.read_buffer(8).unwrap(), b"eight");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

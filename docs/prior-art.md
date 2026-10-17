# Prior art: how Windows Notepad keeps unsaved tabs

Nib's session and crash-recovery design follows what Windows Notepad
(11.2607.14.0) does. I studied its shipped `Notepad.exe` statically with
[REA](https://github.com/morluto/rea) and Ghidra 12.1.4, and looked at the
files it keeps in
`%LOCALAPPDATA%\Packages\Microsoft.WindowsNotepad_8wekyb3d8bbwe\LocalState`.
This page summarises the findings; no Notepad code is included in Nib.

## What Notepad stores

| Path | Purpose |
|---|---|
| `TabState\{guid}.bin` | One full snapshot per tab (file reference and unsaved text). |
| `TabState\{guid}.0.bin`, `{guid}.1.bin` | A pair of small state files per tab that are written alternately. |
| `TabState\{guid}.bin.bak` | Left behind only when a write was interrupted. |
| `WindowState\{guid}.0.bin`, `.1.bin` | Window and tab order, with the same two-slot scheme. |

The binary contains the file-name patterns `{}.bin` and `{}.{}.bin`, and the
suffixes `.tmp` and `.bak`. It also contains the regular expressions
`^(guid)\.([01])\.bin$` and `^(guid)\.([01])\.bin.bak$`, which it uses to find
these files at start-up.

## How a snapshot is written

The routine that writes a tab snapshot does the following:

1. Builds `TabState\{guid}.bin`, then the same path with `.tmp` and with `.bak`.
2. Streams a small header (`"NP"`, a version byte, flags) and the tab's
   sections into the `.tmp` file.
3. If the snapshot already exists, removes any stale `.bak` and renames the
   current snapshot to `.bak`.
4. Deletes both alternating slot files, because the fresh snapshot supersedes them.
5. Renames `.tmp` to the snapshot name.
6. Writes fresh slot state, then removes the `.bak`.

The alternating `.0` / `.1` files absorb frequent small updates. A torn write
damages at most one slot, and the other still holds the previous state.

Not verified: the per-record checksum algorithm, the slot sequence field, and
the exact text encoding inside the snapshot.

## What Nib does with it

- **Unsaved text per tab:** `session/buffers/<id>.bin`, written with the same
  tmp → bak → rename → delete-bak sequence. Each file starts with a magic
  number, a CRC32 and a length, so a damaged copy is detected and the `.bak`
  is read instead (`src/fileio.rs`).
- **Tab list:** two slots, `session.0.json` and `session.1.json`, written
  alternately with an increasing sequence number. Start-up loads the newest
  slot that passes its CRC (`src/session.rs`).
- **Unclean exits:** `session/running.lock` exists while Nib runs. Finding it
  at start-up shows the "recovered your work" bar.
- **No waiting on the disk:** buffer writes go to a background thread, about
  a second after typing stops. Exit, focus loss and Windows shutdown flush
  synchronously.
- **Recovered edits stay honest:** a file's recovered changes are applied on
  top of the on-disk text as one undoable step. The tab shows as modified, and
  Ctrl+Z returns to the saved version.

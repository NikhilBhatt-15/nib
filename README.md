# Nib

A fast, private, keyboard-first text editor for Windows. Native Win32 with
[Scintilla](https://www.scintilla.org/) as the editing component: one small
executable, no runtime, no account, no cloud, no telemetry.

Nib keeps everything you type. Close it with unsaved tabs and they come back
next time. Kill it from Task Manager and the last second of typing is still
there.

## Features

**Editing**
- Tabs, with drag to reorder, pinned tabs, and reopening closed tabs (Ctrl+Shift+T).
- Split view (Ctrl+\\). The same document can be open on both sides.
- Unlimited undo/redo per document, multiple cursors (Ctrl+D adds the next match, Ctrl+Shift+L selects all matches), and rectangular selection (Alt+drag).
- Find and replace (Ctrl+F / Ctrl+H) with regular expressions, case and whole-word toggles, in-selection scope, a live match count, highlighting of every match, `$1` capture groups in replacements, and Replace all as one undo step.
- Syntax highlighting and folding for 29 languages: C/C++, C#, Java/Kotlin, JavaScript/TypeScript, Go, Rust, Python, JSON, HTML, XML, CSS, YAML, TOML, INI, Markdown, SQL, PowerShell, Batch, Shell, Lua, Ruby, Perl, Diff, Makefile, CMake, Registry, Dart, Zig and Visual Basic.
- Line numbers, a change bar for lines edited since the last save, bookmarks, brace matching, highlighting of other occurrences, and an optional column ruler.
- Line tools: duplicate, move, delete, join, sort, remove duplicates, reverse, and trim whitespace. Case changes, JSON format/minify, toggle comment, and insert date/time (F5).
- Encodings: UTF-8 (with or without BOM), UTF-16 LE/BE and 25 legacy code pages, detected on open. Reopen or save with any of them. Line endings are CRLF, LF or CR, with conversion.
- Large files: a 1-million-line, 86 MB log opens in about a third of a second. Above a configurable size, files open in large-file mode, which turns off highlighting and wrapping.

**Markdown**
- Live preview beside the editor (Ctrl+Shift+M). It renders headings, emphasis, code blocks, quotes, nested and task lists, tables and links, and scrolls with the editor.
- Bold, italic, inline code, links and checkboxes with Ctrl+B, Ctrl+I, Ctrl+\`, Ctrl+K and Ctrl+Shift+X. Lists and quotes continue when you press Enter.

**Finding things**
- Quick Open (Ctrl+P) fuzzy-searches open tabs, pinned and recent files, and every file in your notes folder and extra search folders. In the same box, type `>` for commands, `:` for a line number, and `#` to search inside notes.
- Search all notes (Ctrl+Shift+F) runs instantly across open tabs, the notes folder and search folders. Results are cached, and `/regex/` patterns work.
- Command palette (Ctrl+Shift+P) lists every command with its shortcut.

**Reliability**
- Unsaved work is always kept in the session, Windows Notepad style. Untitled notes and unsaved edits are written to disk within about a second of when you stop typing.
- Crash recovery: after an unclean exit, Nib restores every tab and tells you what it recovered.
- Atomic saves: files are written to a temporary file and swapped in, so a crash or full disk can't leave a half-written file.
- Version history: every save keeps a copy (File › Version history, Ctrl+Alt+H). Pick a version to compare with the current text, then restore it if you want.
- Optional auto-save to the file itself, after a delay or when Nib loses focus.
- Changes made by other programs are picked up. Clean tabs reload; tabs with your own edits ask you to reload, keep yours, or compare.

**Comparing**
- Side-by-side comparison with the clipboard, the saved version on disk, another tab, any file, or an earlier version. Changed lines and characters are highlighted, both sides stay aligned and scroll together, and F8 / Shift+F8 jump between differences.

**Windows integration**
- Floating scratchpad on a global shortcut (Win+Alt+N, from any app). It stays on top and saves as you type.
- Always on top (Ctrl+Alt+T), full screen (F11), and a tray icon with an optional close-to-tray mode.
- "Edit with Nib" in the File Explorer context menu, Nib in "Open with", and a Start menu shortcut so Windows search finds it (per user, no admin rights needed).
- Drag and drop files onto the window. Only one instance runs: opening a file from Explorer reuses the running window.
- Start with Windows (in the tray).
- Portable mode: put an empty file named `portable` next to `nib.exe` and all data stays in a `data` folder beside it.
- Follows Windows light/dark mode and accent colour, including a dark menu bar and dark scrollbars, and supports per-monitor DPI.

**Clipboard and text cleanup**
- Paste cleaned text (Ctrl+Shift+V) and Clean up clipboard. These straighten smart quotes, remove zero-width and non-breaking characters, trim trailing spaces, remove shared indentation and collapse blank lines. Optionally they also unwrap hard-wrapped paragraphs or strip bullets.

**Snippets and templates**
- Snippets in `snippets.json`: type a trigger and press Tab, or pick one with Ctrl+Shift+I. Snippets can use variables such as `$DATE`, `$TIME`, `$CLIPBOARD`, `$SELECTION` and `$CURSOR`.
- New from template (Ctrl+Shift+N) uses plain files in the templates folder. Meeting notes, a daily journal, a to-do list and a bug report are included.

**Optional AI** (off until you turn it on)
- Explain this (logs, errors, code), Summarize, Rewrite selection (clearer, shorter, more formal, friendlier, fix grammar, or a custom instruction), and Format messy notes into clean Markdown.
- Two providers:
  - **Local model**: any OpenAI-compatible server on your PC, such as Ollama or LM Studio. Text never leaves the machine.
  - **Claude API**: Anthropic's Messages API, defaulting to `claude-opus-5-5` with server-side refusal fallbacks enabled. The API key is encrypted for your Windows account with DPAPI. Nib asks before the first time text is sent, and never sends anything except the selection or document you run a command on.
- Rewrites replace the selection as one undo step, with "Compare with before" and "Undo" right there. Inputs over the size limit are refused with a message, never silently cut.

**Themes and fonts**
- System (light/dark), Light, Dark, GitHub Light, Solarized Light/Dark, One Dark, Dracula, Nord, Monokai, Gruvbox Dark and Catppuccin Mocha. Choose any font and size, and zoom with Ctrl+= / Ctrl+-.

## Install

Download `Nib-vX.Y.Z.zip` from [Releases](../../releases), extract it anywhere, and run `nib.exe`. No installer and no admin rights are needed.

- To find Nib in Windows search: put `nib.exe` somewhere permanent (for example `%LOCALAPPDATA%\Programs\Nib`), run it, then choose Tools › Preferences › Add Nib to the Start menu.
- For the Explorer menu entry: Tools › Preferences › 'Edit with Nib' in File Explorer.
- To start with Windows: Tools › Preferences › Start with Windows.
- For a portable copy: create an empty file called `portable` next to `nib.exe`.

Settings live in `%APPDATA%\Nib\settings.json` (Tools › Preferences › Open settings file, or Ctrl+,). Edit and save it in Nib to apply changes immediately. Notes go to `Documents\Nib Notes` unless you choose another folder.

### Command line

```
nib.exe [files...]            open files (in the running window if there is one)
nib.exe --new                 new note
nib.exe --scratch             toggle the scratchpad
nib.exe --tray                start hidden in the tray
nib.exe --cmd <command-id>    run any command, e.g. --cmd view.palette
```

Command ids are listed under Help › Keyboard shortcuts. Any of them can be
bound to keys in `settings.json`:

```json
"keys": { "Ctrl+Alt+D": "edit.duplicate", "Ctrl+D": "" }
```

## Keyboard

| Keys | Action |
|---|---|
| Ctrl+P / Ctrl+Shift+P | Go to file / command palette |
| Ctrl+Shift+F | Search all notes |
| Ctrl+F / Ctrl+H / F3 | Find / replace / next match |
| Ctrl+G | Go to line |
| Ctrl+N / Ctrl+O / Ctrl+S | New note / open / save |
| Ctrl+W / Ctrl+Shift+T | Close tab / reopen closed tab |
| Ctrl+Tab, Alt+1…9 | Switch tabs |
| Ctrl+\\ / F6 | Split editor / focus other side |
| Ctrl+D / Ctrl+Shift+L | Add next match / select all matches |
| Alt+Up / Alt+Down | Move line |
| Ctrl+Shift+D / Ctrl+Shift+K | Duplicate / delete line |
| Ctrl+/ | Toggle comment |
| Ctrl+Shift+M | Markdown preview |
| Ctrl+Shift+V | Paste cleaned text |
| Ctrl+Alt+C, F8 | Compare with…, next difference |
| Ctrl+Alt+H | Version history |
| Ctrl+Alt+T / F11 | Always on top / full screen |
| Win+Alt+N | Scratchpad (global) |

## Build

Requires Rust (MSVC toolchain) and the Visual Studio C++ build tools, which compile the vendored Scintilla and Lexilla sources.

```
cargo build --release
cargo test
```

`cargo build --profile fast` gives a non-LTO optimized build with symbols, for iterating.

## How it works

| Piece | File | Mechanism |
|---|---|---|
| Editing component | `vendor/`, `build.rs`, `src/sci.rs` | Scintilla 5.6.7 and 28 Lexilla 5.5.4 lexer modules, compiled into the exe with the `cc` crate. `build.rs` generates Rust constants from `Scintilla.h` and `SciLexer.h`. Calls go through Scintilla's direct function pointer, not SendMessage. |
| Documents and tabs | `src/docs.rs`, `src/tabs.rs` | One Scintilla view per side. Switching tabs swaps the view's document pointer, so each document keeps its undo history and styling. Documents are created on first display, so restoring a 50-tab session costs almost nothing. |
| Session and crash recovery | `src/session.rs`, `src/fileio.rs` | Modelled on Windows Notepad's TabState folder (see [docs/prior-art.md](docs/prior-art.md)). Each unsaved tab has a buffer file written as `tmp` → rename old to `bak` → rename `tmp` into place → delete `bak`, with a CRC header. The tab list alternates between two sequence-numbered slots, so a torn write falls back to the previous one. A lock file detects unclean exits. Writes happen on a background thread. |
| Saving | `src/fileio.rs` | Writes to a temporary sibling file, flushes it, then swaps it in with `ReplaceFileW`, which keeps attributes, ACLs and creation time. Falls back to `MoveFileExW` or an in-place write on file systems that can't. |
| Encodings | `src/textenc.rs` | BOM detection, a BOM-less UTF-16 heuristic, a UTF-8 validity check, then the system ANSI code page. Conversions use `encoding_rs`. Characters that can't be encoded trigger a prompt instead of silent loss. |
| Find | `src/findbar.rs` | The `regex` crate runs directly on Scintilla's UTF-8 buffer (`SCI_GETCHARACTERPOINTER`), so byte offsets are editor positions and nothing is copied. |
| Search and Quick Open | `src/notes.rs`, `src/palette.rs`, `src/fuzzy.rs` | A background thread indexes file names. Content search runs on a worker thread with an mtime-keyed cache; newer queries cancel older ones. Fuzzy scoring favours word starts, consecutive runs and file-name matches. |
| Markdown preview | `src/preview.rs` | pulldown-cmark turns the Markdown into RTF in the theme's colours, displayed in a read-only RichEdit 4.1 control. No browser engine. |
| Comparison | `src/compare.rs` | `similar` (Myers diff) on lines, then on characters within changed lines. Scintilla markers draw line backgrounds, indicators mark characters, and annotations pad the shorter side so both stay aligned. |
| Version history | `src/history.rs` | A copy of each saved file under `history/<hash of path>/<ms>.bin`. Identical consecutive versions are skipped and old ones pruned. |
| UI chrome | `src/chrome.rs`, `src/gfx.rs`, `src/darkmode.rs` | Custom-drawn tab strip, status bar, info bar, find bar and palette (GDI+ shapes, GDI ClearType text). The dark menu bar is painted from the undocumented `WM_UAHDRAWMENU*` messages, and popup menus and scrollbars use uxtheme's dark mode. |
| Commands and keys | `src/commands.rs`, `src/keys.rs` | One table of commands drives the menu bar, the palette and key bindings, with user overrides from settings. |
| AI | `src/ai.rs`, `src/http.rs` | WinHTTP POST on a worker thread: the Claude Messages API, or `/chat/completions` on a local OpenAI-compatible server. The API key is stored with `CryptProtectData`. |
| Single instance and shell | `src/ipc.rs`, `src/shell.rs`, `src/tray.rs`, `src/scratchpad.rs` | A named mutex plus `WM_COPYDATA` forward arguments to the running window. HKCU registry keys provide the Explorer verb and the "Open with" entry. `RegisterHotKey` provides the scratchpad shortcut. |

If something goes wrong, Nib writes `crash.log` to its data folder. It also
logs native faults, and the session on disk is at most a second old.

## License

MIT. Scintilla and Lexilla are © Neil Hodgson under their own permissive
licence; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

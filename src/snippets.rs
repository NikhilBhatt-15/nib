//! Reusable snippets and note templates.
//!
//! Snippets live in snippets.json; type a snippet's trigger and press Tab,
//! or pick one from Edit › Insert snippet. Templates are plain files in the
//! templates folder, used by File › New from template. Both expand
//! variables: $DATE $TIME $DATETIME $WEEKDAY $CLIPBOARD $SELECTION $FILENAME
//! and $CURSOR (where the caret ends up).

use crate::app::{self, app, set_status};
use crate::fileio::Stamp;
use crate::gfx::icon;
use crate::languages;
use crate::palette::{self, Item, Kind};
use crate::sci::Sci;
use crate::sci_consts::*;
use crate::util;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Snippet {
    pub name: String,
    #[serde(default)]
    pub trigger: String,
    pub body: String,
}

thread_local! {
    static CACHE: RefCell<(Option<Stamp>, Vec<Snippet>)> = const { RefCell::new((None, Vec::new())) };
}

fn defaults() -> Vec<Snippet> {
    let s = |name: &str, trigger: &str, body: &str| Snippet { name: name.into(), trigger: trigger.into(), body: body.into() };
    vec![
        s("Today's date", "date", "$DATE"),
        s("Timestamp", "now", "$DATETIME"),
        s("Dated heading", "dh", "## $WEEKDAY, $DATE\n\n$CURSOR"),
        s("Task", "todo", "- [ ] $CURSOR"),
        s("Code block", "code", "```$CURSOR\n$SELECTION\n```"),
        s("Markdown table", "table", "| Column | Column |\n|---|---|\n| $CURSOR |  |"),
        s("Meeting notes", "meet", "# Meeting — $DATE\n\n**Attendees:** \n\n## Agenda\n\n- $CURSOR\n\n## Notes\n\n## Action items\n\n- [ ] "),
        s("Log entry", "log", "[$DATETIME] $CURSOR"),
        s("Signature", "sig", "—\n$CURSOR"),
    ]
}

pub fn load() -> Vec<Snippet> {
    let path = app().paths.snippets();
    if !path.exists() {
        let json = serde_json::to_string_pretty(&defaults()).unwrap_or_default();
        let _ = crate::fileio::save_atomic(&path, json.as_bytes());
    }
    let stamp = crate::fileio::stamp(&path);
    if let Some(v) = CACHE.with(|c| (c.borrow().0 == stamp && stamp.is_some()).then(|| c.borrow().1.clone())) {
        return v;
    }
    let list: Vec<Snippet> = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&crate::settings::strip_json_comments(&t)).ok()).unwrap_or_else(defaults);
    CACHE.with(|c| *c.borrow_mut() = (stamp, list.clone()));
    list
}

pub fn check_reload() {
    let _ = load();
}

/// Expands variables. Returns the text and the caret offset ($CURSOR) in it.
pub fn expand(body: &str, selection: &str) -> (String, Option<usize>) {
    let (y, mo, d, h, mi, _, wd) = util::local_time(util::now_ms());
    let days = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
    let file = {
        let st = app().st.borrow();
        st.doc(st.active_id()).map(|d| d.name()).unwrap_or_default()
    };
    let mut s = body
        .replace("$DATETIME", &format!("{y}-{mo:02}-{d:02} {h:02}:{mi:02}"))
        .replace("$DATE", &format!("{y}-{mo:02}-{d:02}"))
        .replace("$TIME", &format!("{h:02}:{mi:02}"))
        .replace("$WEEKDAY", days[wd as usize % 7])
        .replace("$FILENAME", &file)
        .replace("$SELECTION", selection);
    if s.contains("$CLIPBOARD") {
        s = s.replace("$CLIPBOARD", &util::clipboard_text(app().hwnd.get()).unwrap_or_default());
    }
    let cursor = s.find("$CURSOR");
    (s.replacen("$CURSOR", "", 1).replace("$CURSOR", ""), cursor)
}

fn insert(v: &Sci, body: &str, replace: Option<(usize, usize)>) {
    let sel = String::from_utf8_lossy(&v.selected()).into_owned();
    let (text, cursor) = expand(body, &sel);
    let eol = match v.m(SCI_GETEOLMODE, 0, 0) {
        SC_EOL_CRLF => "\r\n",
        SC_EOL_CR => "\r",
        _ => "\n",
    };
    let fixed = text.replace("\r\n", "\n").replace('\n', eol);
    let cursor = cursor.map(|c| text[..c].replace("\r\n", "\n").replace('\n', eol).len());
    let (a, b) = replace.unwrap_or((v.sel_start(), v.sel_end()));
    v.begin_undo();
    v.replace_range(a, b, fixed.as_bytes());
    v.end_undo();
    let caret = a + cursor.unwrap_or(fixed.len());
    v.set_sel(caret, caret);
    v.m(SCI_SCROLLCARET, 0, 0);
}

/// Tab after a trigger word expands the snippet. Returns true if it did.
pub fn try_expand(v: &Sci) -> bool {
    if !v.sel_empty() || v.m(SCI_GETSELECTIONS, 0, 0) > 1 || v.readonly() {
        return false;
    }
    let p = v.pos();
    let line_start = v.line_start(v.line_from_pos(p));
    let before = v.range_string(line_start, p);
    let word: String = before.chars().rev().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-').collect::<Vec<_>>().into_iter().rev().collect();
    if word.is_empty() {
        return false;
    }
    let Some(sn) = load().into_iter().find(|s| !s.trigger.is_empty() && s.trigger == word) else { return false };
    insert(v, &sn.body, Some((p - word.len(), p)));
    true
}

pub fn pick_snippet() {
    let list = load();
    let items = list.iter().enumerate().map(|(i, s)| Item::new(icon::SNIPPET, s.name.clone(), Kind::Index(i)).detail(s.body.lines().next().unwrap_or("").to_string()).hint(if s.trigger.is_empty() { String::new() } else { format!("{} ⇥", s.trigger) })).collect();
    palette::pick("Insert snippet (or type its trigger and press Tab)", items, true, move |i| insert(&app::active_view(), &list[i].body, None));
}

pub fn edit_file() {
    let _ = load();
    crate::docs::open_path(&app().paths.snippets(), None);
    set_status("Snippets reload when you save this file");
}

// ------------------------------------------------------------------ templates

fn seed_templates() {
    let dir = app().paths.templates();
    let empty = std::fs::read_dir(&dir).map(|mut d| d.next().is_none()).unwrap_or(true);
    if !empty {
        return;
    }
    let files: &[(&str, &str)] = &[
        ("Meeting notes.md", "# $TITLE — $DATE\n\n**Attendees:**\n\n## Agenda\n\n1. $CURSOR\n\n## Discussion\n\n## Decisions\n\n## Action items\n\n- [ ] \n"),
        ("Daily journal.md", "# $WEEKDAY, $DATE\n\n## Today I want to\n\n- [ ] $CURSOR\n\n## Notes\n\n## Grateful for\n\n"),
        ("To-do list.md", "# To do\n\n## Now\n\n- [ ] $CURSOR\n\n## Next\n\n- [ ] \n\n## Later\n\n- [ ] \n"),
        ("Bug report.md", "# Bug: $CURSOR\n\n**Date:** $DATE\n\n## Steps to reproduce\n\n1. \n\n## Expected\n\n## Actual\n\n## Logs\n\n```\n\n```\n"),
        ("Plain note.txt", "$DATE\n\n$CURSOR"),
    ];
    for (name, body) in files {
        let _ = std::fs::write(dir.join(name), body);
    }
}

pub fn pick_template() {
    seed_templates();
    let dir = app().paths.templates();
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir).map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect()).unwrap_or_default();
    files.sort();
    let mut items: Vec<Item> = files.iter().enumerate().map(|(i, p)| Item::new(icon::TEMPLATE, p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(), Kind::Index(i)).detail(p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default())).collect();
    items.push(Item::new(icon::FOLDER, "Open templates folder…", Kind::Index(usize::MAX)).detail("add your own files here"));
    palette::pick("New from template", items, true, move |i| {
        if i == usize::MAX {
            util::shell_open(&app().paths.templates().display().to_string());
            return;
        }
        let p = &files[i];
        let body = std::fs::read_to_string(p).unwrap_or_default();
        let title = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let (text, cursor) = expand(&body.replace("$TITLE", &title), "");
        let lang = languages::detect(&p.to_string_lossy(), text.as_bytes());
        crate::docs::new_doc_with(text.replace('\n', "\r\n").into_bytes(), Some(lang));
        if let Some(c) = cursor {
            let v = app::active_view();
            let pos = text[..c].replace('\n', "\r\n").len();
            v.set_sel(pos, pos);
            v.m(SCI_SCROLLCARET, 0, 0);
        }
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn default_snippets_have_unique_triggers() {
        let d = super::defaults();
        let mut t: Vec<_> = d.iter().map(|s| s.trigger.clone()).collect();
        t.sort();
        t.dedup();
        assert_eq!(t.len(), d.len());
    }
}

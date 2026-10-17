//! Side-by-side comparison in the split view, in the spirit of Beyond
//! Compare: changed lines get a background, changed characters inside them
//! are highlighted, blank padding (annotations) keeps the two sides aligned
//! line for line, scrolling is linked, and F8 / Shift+F8 jump between
//! differences. The left side stays editable and the diff updates as you type.

use crate::app::{self, app, set_status, view, Info, InfoAction};
use crate::editor::{IND_DIFF, MK_DIFF_ADD, MK_DIFF_CHG, MK_DIFF_DEL, STYLE_PAD};
use crate::gfx::icon;
use crate::palette::{self, Item, Kind};
use crate::sci::Sci;
use crate::sci_consts::*;
use crate::util;
use crate::docs;
use similar::{DiffOp, TextDiff};
use std::cell::{Cell, RefCell};
use std::time::Duration;

#[derive(Clone, Copy)]
struct Hunk {
    left: usize,
    right: usize,
}

struct Cmp {
    left: u64,
    right: u64,
    hunks: Vec<Hunk>,
    was_split: bool,
    /// The right side can be copied over the left ("Restore this version").
    restorable: bool,
    added: usize,
    removed: usize,
}

thread_local! {
    static CMP: RefCell<Option<Cmp>> = const { RefCell::new(None) };
    static BUSY: Cell<bool> = const { Cell::new(false) };
}

pub fn active() -> bool {
    CMP.with(|c| c.borrow().is_some())
}

fn ids() -> Option<(u64, u64)> {
    CMP.with(|c| c.borrow().as_ref().map(|c| (c.left, c.right)))
}

/// Compares `left` (editable) with `right`.
pub fn start(left: u64, right: u64) {
    start_ex(left, right, false);
}

fn start_ex(left: u64, right: u64, restorable: bool) {
    if active() {
        close();
    }
    let a = app();
    let was_split = a.st.borrow().split;
    BUSY.with(|b| b.set(true));
    docs::activate(0, left);
    {
        let mut st = a.st.borrow_mut();
        st.split = true;
        if !st.groups[1].tabs.contains(&right) {
            st.groups[1].tabs.push(right);
        }
    }
    docs::show(1, right);
    docs::activate(0, left);
    BUSY.with(|b| b.set(false));
    CMP.with(|c| *c.borrow_mut() = Some(Cmp { left, right, hunks: Vec::new(), was_split, restorable, added: 0, removed: 0 }));
    app::layout();
    recompute();
    next(true);
}

/// Compares document `doc` with some other text shown read-only on the right.
pub fn start_with_text(doc: u64, title: &str, text: Vec<u8>, restorable: bool) {
    let lang = app().st.borrow().doc(doc).map_or(0, |d| d.lang);
    let right = docs::new_virtual(title, text, lang);
    start_ex(doc, right, restorable);
}

fn clear(v: &Sci) {
    for m in [MK_DIFF_ADD, MK_DIFF_DEL, MK_DIFF_CHG] {
        v.m(SCI_MARKERDELETEALL, m, 0);
    }
    v.m(SCI_ANNOTATIONCLEARALL, 0, 0);
    v.m(SCI_SETINDICATORCURRENT, IND_DIFF, 0);
    v.m(SCI_INDICATORCLEARRANGE, 0, v.len() as i64);
}

fn pad(v: &Sci, after_line: usize, n: usize) {
    if n == 0 {
        return;
    }
    let text = vec![" "; n].join("\n");
    v.set_str(SCI_ANNOTATIONSETTEXT, after_line, &text);
    v.m(SCI_ANNOTATIONSETSTYLE, after_line as i64, STYLE_PAD);
}

/// Splits lines the way Scintilla numbers them: CRLF, LF and a lone CR all
/// end a line, so diff line indices match editor line numbers.
fn split_lines(s: &str) -> Vec<&str> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\r' => {
                out.push(&s[start..i]);
                i += if b.get(i + 1) == Some(&b'\n') { 2 } else { 1 };
                start = i;
            }
            b'\n' => {
                out.push(&s[start..i]);
                i += 1;
                start = i;
            }
            _ => i += 1,
        }
    }
    out.push(&s[start..]);
    out
}

/// Highlights the characters that differ between a pair of lines.
fn intraline(lv: &Sci, ll: usize, rv: &Sci, rl: usize) {
    let a = lv.line_text(ll);
    let b = rv.line_text(rl);
    if a.len() > 4000 || b.len() > 4000 {
        return;
    }
    let diff = TextDiff::configure().timeout(Duration::from_millis(50)).diff_chars(&a, &b);
    let (la, ra) = (lv.line_start(ll), rv.line_start(rl));
    let byte_at = |s: &str, ci: usize| s.char_indices().nth(ci).map_or(s.len(), |(i, _)| i);
    for op in diff.ops() {
        match *op {
            DiffOp::Delete { old_index, old_len, .. } => {
                let (s, e) = (byte_at(&a, old_index), byte_at(&a, old_index + old_len));
                lv.m(SCI_INDICATORFILLRANGE, (la + s) as i64, (e - s) as i64);
            }
            DiffOp::Insert { new_index, new_len, .. } => {
                let (s, e) = (byte_at(&b, new_index), byte_at(&b, new_index + new_len));
                rv.m(SCI_INDICATORFILLRANGE, (ra + s) as i64, (e - s) as i64);
            }
            DiffOp::Replace { old_index, old_len, new_index, new_len } => {
                let (s, e) = (byte_at(&a, old_index), byte_at(&a, old_index + old_len));
                lv.m(SCI_INDICATORFILLRANGE, (la + s) as i64, (e - s) as i64);
                let (s, e) = (byte_at(&b, new_index), byte_at(&b, new_index + new_len));
                rv.m(SCI_INDICATORFILLRANGE, (ra + s) as i64, (e - s) as i64);
            }
            DiffOp::Equal { .. } => {}
        }
    }
}

/// Re-runs the diff and redraws all decorations.
pub fn recompute() {
    if !active() {
        return;
    }
    let (lv, rv) = (view(0), view(1));
    clear(&lv);
    clear(&rv);
    let lt = lv.text_string();
    let rt = rv.text_string();
    let ll = split_lines(&lt);
    let rl = split_lines(&rt);
    let diff = TextDiff::configure().timeout(Duration::from_secs(2)).diff_slices(&ll, &rl);
    let mut hunks = Vec::new();
    let (mut added, mut removed) = (0, 0);
    lv.m(SCI_SETINDICATORCURRENT, IND_DIFF, 0);
    rv.m(SCI_SETINDICATORCURRENT, IND_DIFF, 0);
    for op in diff.ops() {
        match *op {
            DiffOp::Equal { .. } => {}
            DiffOp::Delete { old_index, old_len, new_index } => {
                // Lines only on the left (the current text): additions.
                for i in old_index..old_index + old_len {
                    lv.m(SCI_MARKERADD, i as i64, MK_DIFF_ADD);
                }
                pad(&rv, new_index.saturating_sub(1), old_len);
                hunks.push(Hunk { left: old_index, right: new_index });
                added += old_len;
            }
            DiffOp::Insert { old_index, new_index, new_len } => {
                // Lines only on the right (the earlier version): removals.
                for i in new_index..new_index + new_len {
                    rv.m(SCI_MARKERADD, i as i64, MK_DIFF_DEL);
                }
                pad(&lv, old_index.saturating_sub(1), new_len);
                hunks.push(Hunk { left: old_index, right: new_index });
                removed += new_len;
            }
            DiffOp::Replace { old_index, old_len, new_index, new_len } => {
                for i in old_index..old_index + old_len {
                    lv.m(SCI_MARKERADD, i as i64, MK_DIFF_CHG);
                }
                for i in new_index..new_index + new_len {
                    rv.m(SCI_MARKERADD, i as i64, MK_DIFF_CHG);
                }
                if old_len > new_len {
                    pad(&rv, new_index + new_len - 1, old_len - new_len);
                } else if new_len > old_len {
                    pad(&lv, old_index + old_len - 1, new_len - old_len);
                }
                for k in 0..old_len.min(new_len) {
                    intraline(&lv, old_index + k, &rv, new_index + k);
                }
                hunks.push(Hunk { left: old_index, right: new_index });
                added += old_len;
                removed += new_len;
            }
        }
    }
    let n = hunks.len();
    CMP.with(|c| {
        if let Some(c) = c.borrow_mut().as_mut() {
            c.hunks = hunks;
            c.added = added;
            c.removed = removed;
        }
    });
    update_info(n, added, removed);
    sync_from(0);
}

fn update_info(n: usize, added: usize, removed: usize) {
    let Some((l, r)) = ids() else { return };
    let restorable = CMP.with(|c| c.borrow().as_ref().is_some_and(|c| c.restorable));
    let (ln, rn) = {
        let st = app().st.borrow();
        (st.doc(l).map(|d| d.name()).unwrap_or_default(), st.doc(r).map(|d| d.name()).unwrap_or_default())
    };
    let text = if n == 0 {
        format!("{ln} ⇄ {rn}: identical")
    } else {
        format!("{ln} ⇄ {rn}: {n} difference{} (+{added} −{removed} lines)", if n == 1 { "" } else { "s" })
    };
    let mut actions = vec![("◀ Previous".to_string(), InfoAction::ComparePrev), ("Next ▶".to_string(), InfoAction::CompareNext)];
    if restorable {
        actions.push(("Restore this version".into(), InfoAction::CompareRestore));
    }
    actions.push(("Close".into(), InfoAction::CompareClose));
    docs::set_info(Info { icon: icon::REPLACE, text, actions, doc: 0, warning: false });
}

fn sync_from(g: usize) {
    if !active() || BUSY.with(|b| b.get()) {
        return;
    }
    BUSY.with(|b| b.set(true));
    let (src, dst) = (view(g), view(1 - g));
    dst.set_first_visible_line(src.first_visible_line());
    dst.m(SCI_SETXOFFSET, src.m(SCI_GETXOFFSET, 0, 0), 0);
    BUSY.with(|b| b.set(false));
}

pub fn on_scroll(g: usize) {
    sync_from(g);
}

pub fn on_edit() {
    if active() {
        recompute();
    }
}

/// Showing a different document on either side ends the comparison.
pub fn on_show(g: usize) {
    if BUSY.with(|b| b.get()) {
        return;
    }
    let Some((l, r)) = ids() else { return };
    let shown = app().st.borrow().groups[g].active;
    if (g == 0 && shown != l) || (g == 1 && shown != r) {
        close();
    }
}

pub fn on_doc_closed(id: u64) {
    if ids().is_some_and(|(l, r)| l == id || r == id) {
        close();
    }
}

pub fn close() {
    let Some(c) = CMP.with(|c| c.borrow_mut().take()) else { return };
    clear(&view(0));
    clear(&view(1));
    docs::clear_info();
    // Remove the read-only right side if it was a temporary document.
    let virt = app().st.borrow().doc(c.right).is_some_and(|d| d.virt);
    if virt {
        BUSY.with(|b| b.set(true));
        docs::close_tab(1, c.right);
        BUSY.with(|b| b.set(false));
    }
    if !c.was_split && app().st.borrow().split {
        docs::toggle_split();
    }
    app::invalidate(app::R_ALL | app::R_LAYOUT);
}

pub fn next(forward: bool) {
    let Some(hunks) = CMP.with(|c| c.borrow().as_ref().map(|c| c.hunks.clone())) else {
        set_status("No comparison open (Tools › Compare)");
        return;
    };
    if hunks.is_empty() {
        set_status("No differences");
        return;
    }
    let g = app::active_group();
    let v = view(g);
    let line = v.line_from_pos(v.pos());
    let key = |h: &Hunk| if g == 0 { h.left } else { h.right };
    let target = if forward {
        hunks.iter().find(|h| key(h) > line).or(hunks.first())
    } else {
        hunks.iter().rev().find(|h| key(h) < line).or(hunks.last())
    };
    if let Some(h) = target {
        let l = key(h).min(v.line_count().saturating_sub(1));
        let p = v.line_start(l);
        v.set_sel(p, p);
        let n = v.lines_on_screen();
        v.set_first_visible_line(v.visible_from_doc_line(l).saturating_sub(n / 3));
        sync_from(g);
        let idx = hunks.iter().position(|x| std::ptr::eq(x, h)).unwrap_or(0);
        set_status(&format!("Difference {} of {}", idx + 1, hunks.len()));
    }
}

/// Replaces the left document's text with the right side (undoable).
pub fn restore_right_into_left() {
    if !active() {
        return;
    }
    let text = view(1).text();
    let lv = view(0);
    lv.begin_undo();
    lv.replace_range(0, lv.len(), &text);
    lv.end_undo();
    close();
    set_status("Restored. Press Ctrl+Z to undo.");
}

// ------------------------------------------------------------------ targets

pub fn with_clipboard(doc: u64) {
    match util::clipboard_text(app().hwnd.get()) {
        Some(t) => start_with_text(doc, "Clipboard", t.into_bytes(), true),
        None => set_status("The clipboard has no text"),
    }
}

pub fn with_disk(doc: u64) {
    let (path, enc, name) = {
        let st = app().st.borrow();
        let Some(d) = st.doc(doc) else { return };
        (d.path.clone(), d.enc, d.name())
    };
    let Some(path) = path else {
        set_status("This note isn't saved to a file yet");
        return;
    };
    match std::fs::read(&path) {
        Ok(bytes) => start_with_text(doc, &format!("{name} (on disk)"), crate::textenc::decode(&bytes, enc), true),
        Err(e) => util::error_box(app().hwnd.get(), &format!("Couldn't read {}\n\n{e}", path.display())),
    }
}

pub fn pick_target() {
    let doc = app::active_doc();
    let (has_path, others) = {
        let st = app().st.borrow();
        let has = st.doc(doc).is_some_and(|d| d.path.is_some());
        let others: Vec<(u64, String, String)> = st.docs.iter().filter(|d| d.id != doc && !d.virt).map(|d| (d.id, d.name(), d.dir_hint())).collect();
        (has, others)
    };
    let mut items = vec![Item::new(icon::PASTE_ICON, "Clipboard", Kind::Index(0)), Item::new(icon::FOLDER, "A file…", Kind::Index(1))];
    if has_path {
        items.push(Item::new(icon::SAVE_ICON, "Saved version on disk", Kind::Index(2)));
        items.push(Item::new(icon::CLOCK, "An earlier version (history)…", Kind::Index(3)));
    }
    for (i, (_, name, dir)) in others.iter().enumerate() {
        items.push(Item::new(icon::DOCUMENT, name.clone(), Kind::Index(100 + i)).detail(dir.clone()).hint("open tab"));
    }
    palette::pick("Compare the current document with…", items, true, move |i| match i {
        0 => with_clipboard(doc),
        1 => {
            if let Some(p) = crate::docs::pick_file_to_compare() {
                if let Some(other) = docs::open_path(&p, Some(1)) {
                    start(doc, other);
                }
            }
        }
        2 => with_disk(doc),
        3 => crate::history::show_for(doc),
        n if n >= 100 => {
            if let Some((id, ..)) = others.get(n - 100) {
                start(doc, *id);
            }
        }
        _ => {}
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_split_like_scintilla() {
        assert_eq!(split_lines("a\r\nb\nc\rd"), vec!["a", "b", "c", "d"]);
        assert_eq!(split_lines("x\n"), vec!["x", ""]);
        assert_eq!(split_lines(""), vec![""]);
    }

    #[test]
    fn replace_hunk_spans_all_changed_lines() {
        let right = "a\nb\n    *m.entry(w);\nc\n";
        let left = "a\nb\n    *\r// added line\r.entry(w);\nc\n";
        let (ll, rl) = (split_lines(left), split_lines(right));
        let diff = TextDiff::configure().diff_slices(&ll, &rl);
        let ops: Vec<_> = diff.ops().iter().filter(|o| !matches!(o, DiffOp::Equal { .. })).cloned().collect();
        assert!(matches!(ops[..], [DiffOp::Replace { old_index: 2, old_len: 3, new_index: 2, new_len: 1 }]));
    }
}

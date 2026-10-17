//! Configuring Scintilla views (fonts, colours, margins, language styles)
//! and the editing operations that work on a view's selection.

use crate::languages::{Lang, LANGS};
use crate::sci::Sci;
use crate::sci_consts::*;
use crate::settings::Settings;
use crate::theme::{Theme, Tok};

/// Indicator numbers. Scintilla reserves 0-7 for lexers.
pub const IND_FIND: i64 = 8;
pub const IND_OCCURRENCE: i64 = 9;
pub const IND_DIFF: i64 = 10;
pub const IND_FIND_CURRENT: i64 = 11;

/// Marker numbers.
pub const MK_BOOKMARK: i64 = 1;
pub const MK_DIFF_DEL: i64 = 2;
pub const MK_DIFF_ADD: i64 = 3;
pub const MK_DIFF_CHG: i64 = 4;

const MARGIN_NUMBERS: i64 = 0;
const MARGIN_MARKS: i64 = 1;
const MARGIN_FOLD: i64 = 2;
const MARGIN_HISTORY: i64 = 3;

/// Style used for comparison padding lines (annotations).
pub const STYLE_PAD: i64 = 200;

pub fn default_font() -> &'static str {
    use std::sync::OnceLock;
    static F: OnceLock<&str> = OnceLock::new();
    F.get_or_init(|| {
        let fonts = ["Cascadia Mono", "Cascadia Code", "Consolas"];
        fonts.iter().copied().find(|f| font_exists(f)).unwrap_or("Consolas")
    })
}

fn font_exists(name: &str) -> bool {
    use windows_sys::Win32::Graphics::Gdi::*;
    unsafe extern "system" fn found(_: *const LOGFONTW, _: *const TEXTMETRICW, _: u32, lp: isize) -> i32 {
        *(lp as *mut bool) = true;
        0
    }
    unsafe {
        let mut lf: LOGFONTW = std::mem::zeroed();
        lf.lfCharSet = DEFAULT_CHARSET;
        for (i, c) in name.encode_utf16().take(31).enumerate() {
            lf.lfFaceName[i] = c;
        }
        let dc = GetDC(std::ptr::null_mut());
        let mut ok = false;
        EnumFontFamiliesExW(dc, &lf, Some(found), &mut ok as *mut bool as isize, 0);
        ReleaseDC(std::ptr::null_mut(), dc);
        ok
    }
}

/// One-time setup of a new view.
pub fn init_view(v: &Sci) {
    v.m(SCI_SETCODEPAGE, SC_CP_UTF8, 0);
    v.m(SCI_SETTECHNOLOGY, SC_TECHNOLOGY_DIRECTWRITE, 0);
    v.m(SCI_SETFONTQUALITY, SC_EFF_QUALITY_LCD_OPTIMIZED, 0);
    v.m(SCI_SETBUFFEREDDRAW, 0, 0);
    v.m(SCI_SETMODEVENTMASK, SC_MOD_INSERTTEXT | SC_MOD_DELETETEXT, 0);
    v.m(SCI_SETMULTIPLESELECTION, 1, 0);
    v.m(SCI_SETADDITIONALSELECTIONTYPING, 1, 0);
    v.m(SCI_SETMULTIPASTE, SC_MULTIPASTE_EACH, 0);
    v.m(SCI_SETVIRTUALSPACEOPTIONS, SCVS_RECTANGULARSELECTION, 0);
    v.m(SCI_SETMOUSESELECTIONRECTANGULARSWITCH, 1, 0);
    v.m(SCI_SETSCROLLWIDTHTRACKING, 1, 0);
    v.m(SCI_SETSCROLLWIDTH, 1, 0);
    v.m(SCI_SETENDATLASTLINE, 0, 0);
    v.m(SCI_SETCARETLINEVISIBLEALWAYS, 1, 0);
    v.m(SCI_SETCARETWIDTH, 2, 0);
    v.m(SCI_SETSELECTIONLAYER, SC_LAYER_UNDER_TEXT, 0);
    v.m(SCI_SETCARETLINELAYER, SC_LAYER_UNDER_TEXT, 0);
    v.m(SCI_USEPOPUP, SC_POPUP_TEXT, 0);
    v.m(SCI_SETWRAPVISUALFLAGS, SC_WRAPVISUALFLAG_END, 0);
    v.m(SCI_SETWRAPINDENTMODE, SC_WRAPINDENT_INDENT, 0);
    v.m(SCI_SETMOUSEDWELLTIME, SC_TIME_FOREVER, 0);
    v.m(SCI_SETEXTRADESCENT, 1, 0);
    v.m(SCI_SETEXTRAASCENT, 1, 0);
    v.m(SCI_SETXCARETPOLICY, CARET_SLOP | CARET_EVEN, 40);
    v.m(SCI_SETYCARETPOLICY, CARET_SLOP | CARET_EVEN | CARET_STRICT, 2);

    // Margins: line numbers, bookmarks/diff marks, change history, folding.
    v.m(SCI_SETMARGINS, 4, 0);
    v.m(SCI_SETMARGINTYPEN, MARGIN_NUMBERS, SC_MARGIN_NUMBER);
    v.m(SCI_SETMARGINTYPEN, MARGIN_MARKS, SC_MARGIN_SYMBOL);
    v.m(SCI_SETMARGINMASKN, MARGIN_MARKS, (1 << MK_BOOKMARK) | (1 << MK_DIFF_DEL) | (1 << MK_DIFF_ADD) | (1 << MK_DIFF_CHG));
    v.m(SCI_SETMARGINSENSITIVEN, MARGIN_MARKS, 1);
    v.m(SCI_SETMARGINTYPEN, MARGIN_HISTORY, SC_MARGIN_SYMBOL);
    let history_mask = (1 << SC_MARKNUM_HISTORY_REVERTED_TO_ORIGIN) | (1 << SC_MARKNUM_HISTORY_SAVED) | (1 << SC_MARKNUM_HISTORY_MODIFIED) | (1 << SC_MARKNUM_HISTORY_REVERTED_TO_MODIFIED);
    v.m(SCI_SETMARGINMASKN, MARGIN_HISTORY, history_mask);
    v.m(SCI_SETMARGINTYPEN, MARGIN_FOLD, SC_MARGIN_SYMBOL);
    v.m(SCI_SETMARGINMASKN, MARGIN_FOLD, SC_MASK_FOLDERS);
    v.m(SCI_SETMARGINSENSITIVEN, MARGIN_FOLD, 1);
    v.m(SCI_SETAUTOMATICFOLD, SC_AUTOMATICFOLD_SHOW | SC_AUTOMATICFOLD_CLICK | SC_AUTOMATICFOLD_CHANGE, 0);
    v.m(SCI_SETFOLDFLAGS, SC_FOLDFLAG_LINEAFTER_CONTRACTED, 0);
    for (n, sym) in [
        (SC_MARKNUM_FOLDEROPEN, SC_MARK_ARROWDOWN),
        (SC_MARKNUM_FOLDER, SC_MARK_ARROW),
        (SC_MARKNUM_FOLDERSUB, SC_MARK_EMPTY),
        (SC_MARKNUM_FOLDERTAIL, SC_MARK_EMPTY),
        (SC_MARKNUM_FOLDEREND, SC_MARK_ARROW),
        (SC_MARKNUM_FOLDEROPENMID, SC_MARK_ARROWDOWN),
        (SC_MARKNUM_FOLDERMIDTAIL, SC_MARK_EMPTY),
    ] {
        v.m(SCI_MARKERDEFINE, n, sym);
    }
    v.m(SCI_MARKERDEFINE, MK_BOOKMARK, SC_MARK_BOOKMARK);
    v.m(SCI_MARKERDEFINE, MK_DIFF_DEL, SC_MARK_BACKGROUND);
    v.m(SCI_MARKERDEFINE, MK_DIFF_ADD, SC_MARK_BACKGROUND);
    v.m(SCI_MARKERDEFINE, MK_DIFF_CHG, SC_MARK_BACKGROUND);
    for n in [SC_MARKNUM_HISTORY_REVERTED_TO_ORIGIN, SC_MARKNUM_HISTORY_SAVED, SC_MARKNUM_HISTORY_MODIFIED, SC_MARKNUM_HISTORY_REVERTED_TO_MODIFIED] {
        v.m(SCI_MARKERDEFINE, n, SC_MARK_FULLRECT);
    }
    v.m(SCI_SETCHANGEHISTORY, SC_CHANGE_HISTORY_ENABLED | SC_CHANGE_HISTORY_MARKERS, 0);

    v.m(SCI_INDICSETSTYLE, IND_FIND, INDIC_ROUNDBOX);
    v.m(SCI_INDICSETSTYLE, IND_FIND_CURRENT, INDIC_ROUNDBOX);
    v.m(SCI_INDICSETSTYLE, IND_OCCURRENCE, INDIC_ROUNDBOX);
    v.m(SCI_INDICSETSTYLE, IND_DIFF, INDIC_FULLBOX);
    for i in [IND_FIND, IND_FIND_CURRENT, IND_OCCURRENCE, IND_DIFF] {
        v.m(SCI_INDICSETUNDER, i, 1);
    }
    v.m(SCI_ANNOTATIONSETVISIBLE, ANNOTATION_STANDARD, 0);
    v.m(SCI_SETSEARCHFLAGS, SCFIND_MATCHCASE, 0);
}

/// Colours, fonts and language styles. Call after switching document or theme.
pub fn style_view(v: &Sci, t: &Theme, s: &Settings, lang: usize, large: bool) {
    let face = if s.font.is_empty() { default_font() } else { s.font.as_str() };
    v.set_str(SCI_STYLESETFONT, STYLE_DEFAULT as usize, face);
    v.m(SCI_STYLESETSIZEFRACTIONAL, STYLE_DEFAULT, (s.font_size * SC_FONT_SIZE_MULTIPLIER as f32) as i64);
    v.style_fore(STYLE_DEFAULT, t.fg);
    v.style_back(STYLE_DEFAULT, t.bg);
    v.m(SCI_STYLESETBOLD, STYLE_DEFAULT, 0);
    v.m(SCI_STYLESETITALIC, STYLE_DEFAULT, 0);
    v.m(SCI_STYLECLEARALL, 0, 0);

    let l: &Lang = &LANGS[lang];
    if !large {
        for &(style, tok) in l.styles {
            let (c, bold, italic) = t.tok(tok);
            v.style_fore(style, c);
            if bold {
                v.m(SCI_STYLESETBOLD, style, 1);
            }
            if italic {
                v.m(SCI_STYLESETITALIC, style, 1);
            }
            if tok == Tok::Code && l.id == "markdown" {
                v.style_back(style, t.fg.mix(t.bg, 0.06));
            }
        }
    }
    v.style_fore(STYLE_LINENUMBER, t.gutter_fg);
    v.style_back(STYLE_LINENUMBER, t.bg);
    v.style_fore(STYLE_INDENTGUIDE, t.whitespace.mix(t.bg, 0.6));
    v.style_back(STYLE_BRACELIGHT, t.selection.mix(t.bg, 0.6));
    v.style_fore(STYLE_BRACELIGHT, t.fg);
    v.m(SCI_STYLESETBOLD, STYLE_BRACELIGHT, 1);
    v.style_fore(STYLE_BRACEBAD, t.syntax.error);
    v.style_fore(STYLE_CONTROLCHAR, t.syntax.error);
    v.style_back(STYLE_PAD, t.fg.mix(t.bg, 0.04));
    v.style_fore(STYLE_PAD, t.subtext);

    v.element(SC_ELEMENT_CARET, t.accent, 255);
    v.element(SC_ELEMENT_SELECTION_BACK, t.selection, 255);
    v.element(SC_ELEMENT_SELECTION_ADDITIONAL_BACK, t.selection.mix(t.bg, 0.8), 255);
    v.element(SC_ELEMENT_SELECTION_INACTIVE_BACK, t.selection.mix(t.bg, 0.55), 255);
    v.element(SC_ELEMENT_SELECTION_SECONDARY_BACK, t.selection.mix(t.bg, 0.55), 255);
    // Translucent, so diff and bookmark backgrounds still show on the caret line.
    v.element(SC_ELEMENT_CARET_LINE_BACK, t.fg, if t.dark { 14 } else { 10 });
    v.element(SC_ELEMENT_WHITE_SPACE, t.whitespace, 255);
    v.element(SC_ELEMENT_FOLD_LINE, t.gutter_fg, 255);
    v.m(SCI_SETFOLDMARGINCOLOUR, 1, t.bg.colorref() as i64);
    v.m(SCI_SETFOLDMARGINHICOLOUR, 1, t.bg.colorref() as i64);
    for n in 25..=31 {
        v.m(SCI_MARKERSETFORE, n, t.gutter_fg.colorref() as i64);
        v.m(SCI_MARKERSETBACK, n, t.gutter_fg.colorref() as i64);
    }
    v.m(SCI_MARKERSETBACK, MK_BOOKMARK, t.accent.colorref() as i64);
    v.m(SCI_MARKERSETFORE, MK_BOOKMARK, t.accent.colorref() as i64);
    let (add, del) = (t.syntax.added, t.syntax.deleted);
    v.m(SCI_MARKERSETBACK, MK_DIFF_ADD, add.mix(t.bg, 0.18).colorref() as i64);
    v.m(SCI_MARKERSETBACK, MK_DIFF_DEL, del.mix(t.bg, 0.18).colorref() as i64);
    v.m(SCI_MARKERSETBACK, MK_DIFF_CHG, t.warning.mix(t.bg, 0.14).colorref() as i64);
    let hist = [
        (SC_MARKNUM_HISTORY_MODIFIED, t.warning),
        (SC_MARKNUM_HISTORY_SAVED, add),
        (SC_MARKNUM_HISTORY_REVERTED_TO_ORIGIN, t.accent),
        (SC_MARKNUM_HISTORY_REVERTED_TO_MODIFIED, t.accent),
    ];
    for (n, c) in hist {
        v.m(SCI_MARKERSETFORE, n, c.colorref() as i64);
        v.m(SCI_MARKERSETBACK, n, c.colorref() as i64);
    }
    v.m(SCI_INDICSETFORE, IND_FIND, t.match_bg.colorref() as i64);
    v.m(SCI_INDICSETALPHA, IND_FIND, if t.dark { 110 } else { 120 });
    v.m(SCI_INDICSETOUTLINEALPHA, IND_FIND, 200);
    v.m(SCI_INDICSETFORE, IND_FIND_CURRENT, t.accent.colorref() as i64);
    v.m(SCI_INDICSETALPHA, IND_FIND_CURRENT, 90);
    v.m(SCI_INDICSETOUTLINEALPHA, IND_FIND_CURRENT, 255);
    v.m(SCI_INDICSETFORE, IND_OCCURRENCE, t.fg.colorref() as i64);
    v.m(SCI_INDICSETALPHA, IND_OCCURRENCE, 28);
    v.m(SCI_INDICSETOUTLINEALPHA, IND_OCCURRENCE, 60);
    v.m(SCI_INDICSETFORE, IND_DIFF, t.warning.colorref() as i64);
    v.m(SCI_INDICSETALPHA, IND_DIFF, 70);
    v.m(SCI_SETEDGECOLOUR, t.whitespace.mix(t.bg, 0.5).colorref() as i64, 0);
    for i in 0..4 {
        v.m(SCI_SETMARGINBACKN, i, t.bg.colorref() as i64);
    }
    v.m(SCI_SETEXTRADESCENT, 1, 0);
    apply_settings(v, s, large);
}

/// Language: lexer, keywords and properties. The lexer belongs to the document,
/// so this runs once per document load or language change.
pub fn set_language(v: &Sci, lang: usize, large: bool) {
    let l = &LANGS[lang];
    if l.lexer.is_empty() || large {
        v.call(SCI_SETILEXER, 0, 0);
        return;
    }
    v.set_lexer(l.lexer);
    for (i, kw) in l.keywords.iter().enumerate() {
        v.set_str(SCI_SETKEYWORDS, i, kw);
    }
    for (k, val) in l.props {
        let key: Vec<u8> = k.bytes().chain(Some(0)).collect();
        let value: Vec<u8> = val.bytes().chain(Some(0)).collect();
        v.call(SCI_SETPROPERTY, key.as_ptr() as usize, value.as_ptr() as isize);
    }
    v.m(SCI_COLOURISE, 0, -1);
}

pub fn apply_settings(v: &Sci, s: &Settings, large: bool) {
    let wrap = s.word_wrap && !large;
    v.m(SCI_SETWRAPMODE, if wrap { SC_WRAP_WORD } else { SC_WRAP_NONE }, 0);
    v.m(SCI_SETVIEWWS, if s.show_whitespace { SCWS_VISIBLEALWAYS } else { SCWS_INVISIBLE }, 0);
    v.m(SCI_SETVIEWEOL, 0, 0);
    v.m(SCI_SETTABWIDTH, s.tab_width as i64, 0);
    v.m(SCI_SETINDENT, 0, 0);
    v.m(SCI_SETUSETABS, (!s.insert_spaces) as i64, 0);
    v.m(SCI_SETTABINDENTS, 1, 0);
    v.m(SCI_SETBACKSPACEUNINDENTS, 1, 0);
    v.m(SCI_SETINDENTATIONGUIDES, if large { SC_IV_NONE } else { SC_IV_LOOKBOTH }, 0);
    v.m(SCI_SETCARETLINEVISIBLE, s.highlight_current_line as i64, 0);
    v.m(SCI_SETZOOM, s.zoom as i64, 0);
    if s.ruler > 0 {
        v.m(SCI_SETEDGEMODE, EDGE_LINE, 0);
        v.m(SCI_SETEDGECOLUMN, s.ruler as i64, 0);
    } else {
        v.m(SCI_SETEDGEMODE, EDGE_NONE, 0);
    }
    v.m(SCI_SETLAYOUTCACHE, if large { SC_CACHE_CARET } else { SC_CACHE_PAGE }, 0);
    update_margins(v, s);
}

/// Widths for the line-number, marks, history and fold margins.
pub fn update_margins(v: &Sci, s: &Settings) {
    let w = |text: &str| -> i64 {
        let b: Vec<u8> = text.bytes().chain(Some(0)).collect();
        v.call(SCI_TEXTWIDTH, STYLE_LINENUMBER as usize, b.as_ptr() as isize) as i64
    };
    if s.line_numbers {
        let digits = v.line_count().max(1).to_string().len().max(3);
        let width = w(&"9".repeat(digits)) + w("9") / 2 + 6;
        v.m(SCI_SETMARGINWIDTHN, MARGIN_NUMBERS, width);
    } else {
        v.m(SCI_SETMARGINWIDTHN, MARGIN_NUMBERS, 0);
    }
    let unit = w("9");
    v.m(SCI_SETMARGINWIDTHN, MARGIN_MARKS, unit + 2);
    v.m(SCI_SETMARGINWIDTHN, MARGIN_HISTORY, (unit / 3).max(3));
    let folding = v.call(SCI_GETLEXER, 0, 0) != SCLEX_NULL as isize && v.call(SCI_GETLEXER, 0, 0) != SCLEX_CONTAINER as isize;
    v.m(SCI_SETMARGINWIDTHN, MARGIN_FOLD, if folding { unit + 4 } else { 0 });
    v.m(SCI_SETMARGINLEFT, 0, 2);
}

// ------------------------------------------------------------ editing operations

/// Selected line range (inclusive), ignoring a final line that the selection only touches at column 0.
pub fn selected_lines(v: &Sci) -> (usize, usize) {
    let (a, b) = (v.sel_start(), v.sel_end());
    let first = v.line_from_pos(a);
    let mut last = v.line_from_pos(b);
    if last > first && v.line_start(last) == b {
        last -= 1;
    }
    (first, last)
}

pub fn toggle_comment(v: &Sci, lang: usize) {
    let l = &LANGS[lang];
    if !l.line_comment.is_empty() {
        toggle_line_comment(v, l.line_comment);
    } else if !l.block_comment.0.is_empty() {
        toggle_block_comment(v, l.block_comment.0, l.block_comment.1);
    }
}

fn toggle_line_comment(v: &Sci, token: &str) {
    let (first, last) = selected_lines(v);
    let tok = token.trim_end();
    let lines: Vec<String> = (first..=last).map(|i| v.line_text(i)).collect();
    let content: Vec<&String> = lines.iter().filter(|l| !l.trim().is_empty()).collect();
    if content.is_empty() {
        return;
    }
    let all_commented = content.iter().all(|l| l.trim_start().starts_with(tok));
    let indent = content.iter().map(|l| l.len() - l.trim_start().len()).min().unwrap_or(0);
    v.begin_undo();
    for (k, line) in lines.iter().enumerate().rev() {
        let i = first + k;
        let start = v.line_start(i);
        if line.trim().is_empty() {
            continue;
        }
        if all_commented {
            let lead = line.len() - line.trim_start().len();
            let rest = &line[lead + tok.len()..];
            let remove = tok.len() + usize::from(rest.starts_with(' '));
            v.replace_range(start + lead, start + lead + remove, b"");
        } else {
            let ins = if token.ends_with(' ') { token.to_string() } else { format!("{token} ") };
            v.insert(start + indent, ins.as_bytes());
        }
    }
    v.end_undo();
}

fn toggle_block_comment(v: &Sci, open: &str, close: &str) {
    let (mut a, mut b) = (v.sel_start(), v.sel_end());
    if a == b {
        let line = v.line_from_pos(a);
        a = v.line_start(line);
        b = v.line_end(line);
    }
    let text = v.range_string(a, b);
    let t = text.trim();
    v.begin_undo();
    if t.starts_with(open) && t.ends_with(close) && t.len() >= open.len() + close.len() {
        let lead = text.len() - text.trim_start().len();
        let inner = t[open.len()..t.len() - close.len()].trim();
        v.replace_range(a + lead, a + lead + t.len(), inner.as_bytes());
    } else {
        v.replace_range(a, b, format!("{open} {text} {close}").as_bytes());
    }
    v.end_undo();
}

pub fn duplicate(v: &Sci) {
    if v.sel_empty() {
        v.m(SCI_LINEDUPLICATE, 0, 0);
    } else {
        v.m(SCI_SELECTIONDUPLICATE, 0, 0);
    }
}

pub fn delete_lines(v: &Sci) {
    let (first, last) = selected_lines(v);
    let a = v.line_start(first);
    let b = if last + 1 < v.line_count() { v.line_start(last + 1) } else { v.len() };
    // Deleting the last line also removes the newline before it.
    let a = if b == v.len() && first > 0 { v.line_end(first - 1) } else { a };
    v.replace_range(a, b, b"");
}

pub fn join_lines(v: &Sci) {
    let (first, mut last) = selected_lines(v);
    if first == last {
        last = (first + 1).min(v.line_count().saturating_sub(1));
    }
    if first == last {
        return;
    }
    v.begin_undo();
    for i in (first..last).rev() {
        let end = v.line_end(i);
        let next = v.line_start(i + 1);
        let next_text = v.line_text(i + 1);
        let skip = next_text.len() - next_text.trim_start().len();
        v.replace_range(end, next + skip, b" ");
    }
    v.end_undo();
}

/// Ctrl+D: select the word at the caret, then add the next occurrence.
pub fn select_next_occurrence(v: &Sci) {
    if v.sel_empty() {
        let p = v.pos();
        let a = v.m(SCI_WORDSTARTPOSITION, p as i64, 1);
        let b = v.m(SCI_WORDENDPOSITION, p as i64, 1);
        if a < b {
            v.set_sel(a as usize, b as usize);
        }
        return;
    }
    v.m(SCI_TARGETWHOLEDOCUMENT, 0, 0);
    v.m(SCI_SETSEARCHFLAGS, SCFIND_MATCHCASE, 0);
    v.m(SCI_MULTIPLESELECTADDNEXT, 0, 0);
    v.m(SCI_SCROLLCARET, 0, 0);
}

pub fn select_all_occurrences(v: &Sci) {
    if v.sel_empty() {
        select_next_occurrence(v);
    }
    v.m(SCI_TARGETWHOLEDOCUMENT, 0, 0);
    v.m(SCI_SETSEARCHFLAGS, SCFIND_MATCHCASE, 0);
    v.m(SCI_MULTIPLESELECTADDEACH, 0, 0);
}

/// Applies `f` to the selection, or to the whole document when nothing is
/// selected. Whole lines are used when `lines` is set.
pub fn transform(v: &Sci, lines: bool, f: impl Fn(&str) -> String) {
    let (a, b) = if v.sel_empty() {
        (0, v.len())
    } else if lines {
        let (first, last) = selected_lines(v);
        (v.line_start(first), v.line_end(last))
    } else {
        (v.sel_start(), v.sel_end())
    };
    let before = v.range_string(a, b);
    // Work on "\n" internally and restore the document's line ending.
    let eol = match v.m(SCI_GETEOLMODE, 0, 0) {
        SC_EOL_CRLF => "\r\n",
        SC_EOL_CR => "\r",
        _ => "\n",
    };
    let normal = before.replace("\r\n", "\n");
    let after = f(&normal);
    let after = if eol == "\n" { after } else { after.replace('\n', eol) };
    if after != before {
        v.replace_range(a, b, after.as_bytes());
        v.set_sel(a, a + after.len());
    }
}

/// Wraps the selection in `mark` (Markdown bold/italic/code), or unwraps it.
pub fn toggle_wrap(v: &Sci, mark: &str) {
    let (a, b) = (v.sel_start(), v.sel_end());
    let m = mark.len();
    if a == b {
        v.replace_sel(format!("{mark}{mark}").as_bytes());
        v.set_sel(a + m, a + m);
        return;
    }
    let before = if a >= m { v.range_string(a - m, a) } else { String::new() };
    let after = v.range_string(b, (b + m).min(v.len()));
    v.begin_undo();
    if before == mark && after == mark {
        v.replace_range(b, b + m, b"");
        v.replace_range(a - m, a, b"");
        v.set_sel(a - m, b - m);
    } else {
        v.insert(b, mark.as_bytes());
        v.insert(a, mark.as_bytes());
        v.set_sel(a + m, b + m);
    }
    v.end_undo();
}

pub fn insert_link(v: &Sci) {
    let sel = String::from_utf8_lossy(&v.selected()).into_owned();
    let a = v.sel_start();
    if sel.starts_with("http://") || sel.starts_with("https://") {
        let text = format!("[]({sel})");
        v.replace_sel(text.as_bytes());
        v.set_sel(a + 1, a + 1);
    } else {
        let text = format!("[{sel}](url)");
        v.replace_sel(text.as_bytes());
        let u = a + sel.len() + 3;
        v.set_sel(u, u + 3);
    }
}

/// After Enter: copy the previous line's indentation and, in Markdown,
/// continue a list or quote (or end it when the item was empty).
pub fn on_newline(v: &Sci, markdown: bool) {
    let line = v.line_from_pos(v.pos());
    if line == 0 || v.m(SCI_GETSELECTIONS, 0, 0) > 1 {
        return;
    }
    let prev = v.line_text(line - 1);
    if markdown {
        if let Some(cont) = crate::textops::list_continuation(&prev) {
            match cont {
                Some(prefix) => {
                    v.replace_sel(prefix.as_bytes());
                }
                None => {
                    // Empty item: remove the marker and end the list.
                    let start = v.line_start(line - 1);
                    let end = v.line_end(line - 1);
                    v.begin_undo();
                    let caret = v.pos();
                    v.replace_range(start, end, b"");
                    let moved = caret - (end - start);
                    v.set_sel(moved, moved);
                    v.end_undo();
                }
            }
            return;
        }
    }
    let indent: String = prev.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
    if !indent.is_empty() {
        let here = v.line_text(line);
        if here.trim().is_empty() || !here.starts_with(&indent) {
            v.replace_sel(indent.as_bytes());
        }
    }
}

/// Highlights other occurrences of a selected whole word in the visible area.
pub fn highlight_occurrences(v: &Sci) {
    v.m(SCI_SETINDICATORCURRENT, IND_OCCURRENCE, 0);
    v.m(SCI_INDICATORCLEARRANGE, 0, v.len() as i64);
    if v.sel_empty() || v.m(SCI_GETSELECTIONS, 0, 0) > 1 {
        return;
    }
    let (a, b) = (v.sel_start(), v.sel_end());
    if b - a < 2 || b - a > 200 {
        return;
    }
    let ws = v.m(SCI_WORDSTARTPOSITION, a as i64, 1) as usize;
    let we = v.m(SCI_WORDENDPOSITION, a as i64, 1) as usize;
    if ws != a || we != b {
        return;
    }
    let word = v.range(a, b);
    let first = v.doc_line_from_visible(v.first_visible_line());
    let last = v.doc_line_from_visible(v.first_visible_line() + v.lines_on_screen() + 1);
    let lo = v.line_start(first);
    let hi = v.line_end(last.min(v.line_count().saturating_sub(1)));
    let text = v.range(lo, hi);
    let is_word = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80;
    let mut i = 0;
    while let Some(off) = find_bytes(&text[i..], &word) {
        let s = i + off;
        let e = s + word.len();
        let left_ok = s == 0 || !is_word(text[s - 1]);
        let right_ok = e >= text.len() || !is_word(text[e]);
        if left_ok && right_ok && lo + s != a {
            v.m(SCI_INDICATORFILLRANGE, (lo + s) as i64, word.len() as i64);
        }
        i = e;
    }
}

pub fn find_bytes(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

pub fn toggle_bookmark(v: &Sci) {
    let line = v.line_from_pos(v.pos());
    if v.m(SCI_MARKERGET, line as i64, 0) & (1 << MK_BOOKMARK) != 0 {
        v.m(SCI_MARKERDELETE, line as i64, MK_BOOKMARK);
    } else {
        v.m(SCI_MARKERADD, line as i64, MK_BOOKMARK);
    }
}

pub fn goto_bookmark(v: &Sci, forward: bool) {
    let line = v.line_from_pos(v.pos()) as i64;
    let mask = 1 << MK_BOOKMARK;
    let found = if forward {
        let n = v.m(SCI_MARKERNEXT, line + 1, mask);
        if n < 0 {
            v.m(SCI_MARKERNEXT, 0, mask)
        } else {
            n
        }
    } else {
        let n = v.m(SCI_MARKERPREVIOUS, line - 1, mask);
        if n < 0 {
            v.m(SCI_MARKERPREVIOUS, v.line_count() as i64, mask)
        } else {
            n
        }
    };
    if found >= 0 {
        v.goto_line(found as usize);
    }
}

/// Brace matching around the caret.
pub fn match_braces(v: &Sci) {
    let p = v.pos();
    let is_brace = |pos: usize| pos < v.len() && matches!(v.m(SCI_GETCHARAT, pos as i64, 0) as u8, b'(' | b')' | b'[' | b']' | b'{' | b'}');
    let at = if is_brace(p) {
        Some(p)
    } else if p > 0 && is_brace(p - 1) {
        Some(p - 1)
    } else {
        None
    };
    match at {
        Some(a) => {
            let b = v.m(SCI_BRACEMATCH, a as i64, 0);
            if b < 0 {
                v.m(SCI_BRACEBADLIGHT, a as i64, 0);
            } else {
                v.m(SCI_BRACEHIGHLIGHT, a as i64, b);
            }
        }
        None => {
            v.m(SCI_BRACEHIGHLIGHT, -1, -1);
        }
    }
}

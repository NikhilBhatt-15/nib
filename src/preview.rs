//! Live Markdown preview: Markdown is parsed with pulldown-cmark, rendered
//! to RTF in the theme's colours, and shown in a read-only RichEdit control
//! next to the editor. No browser engine is involved, so it opens instantly.

use crate::app::{self, app};
use crate::theme::{Rgb, Theme};
use crate::util;
use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use std::cell::RefCell;
use std::mem::zeroed;
use std::ptr::null;
use windows_sys::w;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::InvalidateRect;
use windows_sys::Win32::UI::Controls::NMHDR;
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, LoadLibraryW};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

// RichEdit messages and structures (not in windows-sys).
const EM_STREAMIN: u32 = WM_USER + 73;
const EM_SETBKGNDCOLOR: u32 = WM_USER + 67;
const EM_SETEVENTMASK: u32 = WM_USER + 69;
const EM_AUTOURLDETECT: u32 = WM_USER + 91;
const EM_GETSCROLLPOS: u32 = WM_USER + 221;
const EM_SETSCROLLPOS: u32 = WM_USER + 222;
const EM_GETTEXTRANGE: u32 = WM_USER + 75;
const EM_SETTARGETDEVICE: u32 = WM_USER + 72;
const EM_SETEDITSTYLEEX: u32 = WM_USER + 275;
const SF_RTF: usize = 2;
const ENM_LINK: isize = 0x0400_0000;
const EN_LINK: u32 = 0x070b;
const SES_EX_HANDLEFRIENDLYURL: isize = 0x100;

// richedit.h declares its structures under #pragma pack(4): on x64 the
// callback pointer follows dwError at offset 12, not 16.
#[repr(C, packed(4))]
struct EditStream {
    cookie: usize,
    error: u32,
    callback: unsafe extern "system" fn(usize, *mut u8, i32, *mut i32) -> u32,
}

#[repr(C)]
struct CharRange {
    min: i32,
    max: i32,
}

#[repr(C, packed(4))]
struct EnLink {
    nmhdr: NMHDR,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    chrg: CharRange,
}

#[repr(C, packed(4))]
struct TextRange {
    chrg: CharRange,
    text: *mut u16,
}

thread_local! {
    /// Link text -> URL for each group's preview, to resolve clicks.
    static LINKS: RefCell<[Vec<(String, String)>; 2]> = const { RefCell::new([Vec::new(), Vec::new()]) };
}

pub fn create(parent: HWND, g: usize) -> HWND {
    unsafe {
        LoadLibraryW(w!("Msftedit.dll"));
        let style = WS_CHILD | WS_VSCROLL | ES_MULTILINE as u32 | ES_READONLY as u32 | ES_NOHIDESEL as u32;
        let h = CreateWindowExW(0, w!("RICHEDIT50W"), null(), style, 0, 0, 10, 10, parent, (301 + g) as _, GetModuleHandleW(null()), null());
        SendMessageW(h, EM_SETEVENTMASK, 0, ENM_LINK);
        SendMessageW(h, EM_AUTOURLDETECT, 1, 0);
        SendMessageW(h, EM_SETTARGETDEVICE, 0, 0);
        SendMessageW(h, EM_SETEDITSTYLEEX, 0, SES_EX_HANDLEFRIENDLYURL);
        h
    }
}

pub fn apply_theme(g: usize) {
    let h = app().previews[g].get();
    let t = app::theme();
    unsafe { SendMessageW(h, EM_SETBKGNDCOLOR, 0, t.bg.colorref() as LPARAM) };
    crate::darkmode::set_window_dark(h, t.dark);
    refresh(g);
}

pub fn refresh_all() {
    for g in 0..2 {
        refresh(g);
    }
}

/// Re-renders the preview for group `g` if its document shows one.
pub fn refresh(g: usize) {
    let a = app();
    let h = a.previews[g].get();
    let show = {
        let st = a.st.borrow();
        (g == 0 || st.split) && st.doc(st.groups[g].active).is_some_and(|d| d.preview)
    };
    if !show || h.is_null() {
        return;
    }
    let text = app::view(g).text_string();
    let mut rc: RECT = unsafe { zeroed() };
    unsafe { GetClientRect(h, &mut rc) };
    let width_twips = ((rc.right - rc.left).max(200) as i64 * 1440 / app::dpi() as i64) as i32;
    let (rtf, links) = to_rtf(&text, &app::theme(), width_twips);
    LINKS.with(|l| l.borrow_mut()[g] = links);
    unsafe {
        let mut pos = POINT { x: 0, y: 0 };
        SendMessageW(h, EM_GETSCROLLPOS, 0, &mut pos as *mut POINT as LPARAM);
        SendMessageW(h, WM_SETREDRAW, 0, 0);
        stream_in(h, rtf.as_bytes());
        SendMessageW(h, EM_SETSCROLLPOS, 0, &mut pos as *mut POINT as LPARAM);
        SendMessageW(h, WM_SETREDRAW, 1, 0);
        InvalidateRect(h, null(), 1);
    }
}

struct Feed<'a> {
    data: &'a [u8],
    pos: usize,
}

unsafe extern "system" fn feed(cookie: usize, buf: *mut u8, cb: i32, read: *mut i32) -> u32 {
    let f = &mut *(cookie as *mut Feed);
    let n = (f.data.len() - f.pos).min(cb.max(0) as usize);
    std::ptr::copy_nonoverlapping(f.data.as_ptr().add(f.pos), buf, n);
    f.pos += n;
    *read = n as i32;
    0
}

fn stream_in(h: HWND, data: &[u8]) {
    let mut f = Feed { data, pos: 0 };
    let mut es = EditStream { cookie: &mut f as *mut Feed as usize, error: 0, callback: feed };
    unsafe { SendMessageW(h, EM_STREAMIN, SF_RTF, &mut es as *mut EditStream as LPARAM) };
}

/// Keeps the preview at the same relative position as the editor.
pub fn sync_scroll(g: usize) {
    let a = app();
    let h = a.previews[g].get();
    if h.is_null() || unsafe { IsWindowVisible(h) } == 0 {
        return;
    }
    let v = app::view(g);
    let total = v.visible_from_doc_line(v.line_count()).max(1);
    let page = v.lines_on_screen();
    let range = total.saturating_sub(page).max(1);
    let ratio = (v.first_visible_line() as f64 / range as f64).clamp(0.0, 1.0);
    unsafe {
        let mut si: SCROLLINFO = zeroed();
        si.cbSize = std::mem::size_of::<SCROLLINFO>() as u32;
        si.fMask = SIF_RANGE | SIF_PAGE;
        GetScrollInfo(h, SB_VERT, &mut si);
        let max = (si.nMax - si.nPage as i32).max(0);
        let mut pt = POINT { x: 0, y: (ratio * max as f64) as i32 };
        SendMessageW(h, EM_SETSCROLLPOS, 0, &mut pt as *mut POINT as LPARAM);
    }
}

/// Handles link clicks. Returns Some when the notification was ours.
pub fn on_notify(lp: LPARAM) -> Option<LRESULT> {
    let hdr = unsafe { &*(lp as *const NMHDR) };
    let g = (0..2).find(|&g| app().previews[g].get() == hdr.hwndFrom)?;
    if hdr.code != EN_LINK {
        return Some(0);
    }
    let link = unsafe { &*(lp as *const EnLink) };
    let (msg, min, max) = (link.msg, link.chrg.min, link.chrg.max);
    if msg != WM_LBUTTONUP {
        return Some(0);
    }
    let len = (max - min).max(0) as usize;
    let mut buf = vec![0u16; len + 1];
    let mut tr = TextRange { chrg: CharRange { min, max }, text: buf.as_mut_ptr() };
    unsafe { SendMessageW(hdr.hwndFrom, EM_GETTEXTRANGE, 0, &mut tr as *mut TextRange as LPARAM) };
    let text = String::from_utf16_lossy(&buf[..len]).trim().to_string();
    let target = LINKS.with(|l| l.borrow()[g].iter().find(|(t, _)| *t == text).map(|(_, u)| u.clone())).unwrap_or(text);
    open_link(g, &target);
    Some(0)
}

fn open_link(g: usize, target: &str) {
    let lower = target.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:") {
        util::shell_open(target);
        return;
    }
    if target.starts_with('#') {
        return;
    }
    // Relative links open next to the current file.
    let base = {
        let st = app().st.borrow();
        st.doc(st.groups[g].active).and_then(|d| d.path.clone()).and_then(|p| p.parent().map(|d| d.to_path_buf()))
    };
    let decoded = target.replace("%20", " ");
    let path = match base {
        Some(b) => b.join(&decoded),
        None => std::path::PathBuf::from(&decoded),
    };
    if path.exists() {
        crate::docs::open_path(&path, None);
    } else {
        app::set_status(&format!("Link target not found: {target}"));
    }
}

// ------------------------------------------------------------------ Markdown -> RTF

fn esc(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            '\n' => out.push_str("\\line "),
            '\t' => out.push_str("\\tab "),
            c if (c as u32) < 0x80 => out.push(c),
            c => {
                let mut b = [0u16; 2];
                for u in c.encode_utf16(&mut b) {
                    out.push_str(&format!("\\u{}?", *u as i16));
                }
            }
        }
    }
}

fn colortbl(t: &Theme) -> String {
    let c = |r: Rgb| format!("\\red{}\\green{}\\blue{};", r.0, r.1, r.2);
    // 1 text, 2 muted, 3 link, 4 code background, 5 heading, 6 border, 7 code text
    format!(
        "{{\\colortbl ;{}{}{}{}{}{}{}}}",
        c(t.fg),
        c(t.subtext),
        c(t.syntax.link),
        c(t.fg.mix(t.bg, 0.07)),
        c(t.syntax.heading),
        c(t.border),
        c(t.syntax.string)
    )
}

struct List {
    ordered: bool,
    next: u64,
}

/// Converts Markdown to RTF. Also returns (link text, URL) pairs for clicks.
pub fn to_rtf(md: &str, t: &Theme, width_twips: i32) -> (String, Vec<(String, String)>) {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_FOOTNOTES);
    let body_face = "Segoe UI";
    let mono = crate::editor::default_font();
    let mut out = format!("{{\\rtf1\\ansi\\ansicpg1252\\deff0{{\\fonttbl{{\\f0\\fswiss {body_face};}}{{\\f1\\fmodern {mono};}}}}{}\\viewkind4\\uc1\\pard\\sa160\\sl300\\slmult1\\li240\\ri240\\f0\\fs22\\cf1 ", colortbl(t));
    // Output stack: inline content of links and table cells is collected separately.
    let mut stack: Vec<String> = Vec::new();
    let mut links: Vec<(String, String)> = Vec::new();
    let mut link_dest: Vec<String> = Vec::new();
    let mut link_text: Vec<String> = Vec::new();
    let mut lists: Vec<List> = Vec::new();
    let mut in_code_block = false;
    let mut quote_depth = 0;
    let mut table: Vec<Vec<String>> = Vec::new();
    let mut aligns: Vec<Alignment> = Vec::new();
    let mut header_rows = 0;
    let mut in_head = false;
    let mut item_start = false;

    let cur = |stack: &mut Vec<String>, out: &mut String| -> *mut String {
        match stack.last_mut() {
            Some(s) => s as *mut String,
            None => out as *mut String,
        }
    };
    let para_fmt = |lists: &Vec<List>, quote: usize| -> String {
        let li = 240 + 360 * lists.len() as i32 + 400 * quote as i32;
        let fi = if lists.is_empty() { 0 } else { -260 };
        let cf = if quote > 0 { "\\cf2\\i" } else { "\\cf1" };
        let sa = if lists.is_empty() { 120 } else { 50 };
        // \plain resets character formatting, which \pard leaves alone.
        format!("\\pard\\sa{sa}\\sl300\\slmult1\\li{li}\\fi{fi}\\ri240\\plain\\f0\\fs22{cf} ")
    };

    for ev in Parser::new_ext(md, opts) {
        let o: &mut String = unsafe { &mut *cur(&mut stack, &mut out) };
        match ev {
            Event::Start(tag) => match tag {
                Tag::Paragraph => {
                    if !item_start && stack.is_empty() {
                        o.push_str(&para_fmt(&lists, quote_depth));
                    }
                }
                Tag::Heading { level, .. } => {
                    let size = match level {
                        HeadingLevel::H1 => 40,
                        HeadingLevel::H2 => 32,
                        HeadingLevel::H3 => 28,
                        HeadingLevel::H4 => 24,
                        _ => 22,
                    };
                    o.push_str(&format!("\\pard\\sb200\\sa120\\li240\\ri240\\plain\\f0\\b\\fs{size}\\cf5"));
                }
                Tag::BlockQuote(_) => quote_depth += 1,
                Tag::CodeBlock(kind) => {
                    in_code_block = true;
                    let lang = match kind {
                        CodeBlockKind::Fenced(l) => l.to_string(),
                        CodeBlockKind::Indented => String::new(),
                    };
                    let left = 240 + 360 * lists.len() as i32;
                    let right = (width_twips - 240).max(left + 1000);
                    o.push_str(&format!("\\pard\\trowd\\trgaph160\\trleft{left}\\clcbpat4\\clbrdrt\\brdrs\\brdrw10\\brdrcf6\\clbrdrb\\brdrs\\brdrw10\\brdrcf6\\clbrdrl\\brdrs\\brdrw10\\brdrcf6\\clbrdrr\\brdrs\\brdrw10\\brdrcf6\\cellx{right}\\pard\\intbl\\sa0\\sl0\\plain\\f1\\fs19\\cf1"));
                    if !lang.is_empty() {
                        o.push_str("{\\cf2\\fs16 ");
                        esc(&lang, o);
                        o.push_str("}\\line ");
                    }
                    stack.push(String::new());
                }
                Tag::List(start) => {
                    // A list nested in a tight item starts on its own line.
                    if !lists.is_empty() && !o.ends_with("\\par\n") {
                        o.push_str("\\par\n");
                    }
                    item_start = false;
                    lists.push(List { ordered: start.is_some(), next: start.unwrap_or(1) });
                }
                Tag::Item => {
                    o.push_str(&para_fmt(&lists, quote_depth));
                    if let Some(l) = lists.last_mut() {
                        if l.ordered {
                            o.push_str(&format!("{}.  ", l.next));
                            l.next += 1;
                        } else {
                            o.push_str(if lists.len() % 2 == 1 { "\\u8226?  " } else { "\\u9702?  " });
                        }
                    }
                    item_start = true;
                }
                Tag::Emphasis => o.push_str("\\i "),
                Tag::Strong => o.push_str("\\b "),
                Tag::Strikethrough => o.push_str("\\strike "),
                Tag::Link { dest_url, .. } => {
                    link_dest.push(dest_url.to_string());
                    link_text.push(String::new());
                    stack.push(String::new());
                }
                Tag::Image { dest_url, .. } => {
                    o.push_str("{\\cf2\\i [image");
                    let _ = dest_url;
                    o.push_str(": ");
                }
                Tag::Table(a) => {
                    aligns = a;
                    table.clear();
                    header_rows = 0;
                }
                Tag::TableHead => {
                    in_head = true;
                    table.push(Vec::new());
                }
                Tag::TableRow => table.push(Vec::new()),
                Tag::TableCell => stack.push(String::new()),
                Tag::FootnoteDefinition(name) => {
                    o.push_str("\\pard\\sa60\\li240\\cf2\\fs18 [");
                    esc(&name, o);
                    o.push_str("] ");
                }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph => {
                    if stack.is_empty() {
                        o.push_str("\\par\n");
                    } else {
                        o.push_str("\\line ");
                    }
                    item_start = false;
                }
                TagEnd::Heading(_) => o.push_str("\\b0\\fs22\\cf1\\par\n"),
                TagEnd::BlockQuote(_) => quote_depth = quote_depth.saturating_sub(1),
                TagEnd::CodeBlock => {
                    in_code_block = false;
                    let code = stack.pop().unwrap_or_default();
                    let o: &mut String = unsafe { &mut *cur(&mut stack, &mut out) };
                    let trimmed = code.trim_end_matches("\\line ");
                    o.push_str(trimmed);
                    o.push_str("\\cell\\row\\pard\\sa120\\li240\\f0\\fs22\\cf1\\par\n");
                }
                TagEnd::List(_) => {
                    lists.pop();
                    if lists.is_empty() {
                        o.push_str("\\pard\\sa120\\li240\\fi0 ");
                    }
                }
                TagEnd::Item => {
                    if !o.ends_with("\\par\n") {
                        o.push_str("\\par\n");
                    }
                    item_start = false;
                }
                TagEnd::Emphasis => o.push_str("\\i0 "),
                TagEnd::Strong => o.push_str("\\b0 "),
                TagEnd::Strikethrough => o.push_str("\\strike0 "),
                TagEnd::Link => {
                    let inner = stack.pop().unwrap_or_default();
                    let dest = link_dest.pop().unwrap_or_default();
                    let text = link_text.pop().unwrap_or_default();
                    let o: &mut String = unsafe { &mut *cur(&mut stack, &mut out) };
                    let mut d = String::new();
                    esc(&dest, &mut d);
                    let d = d.replace('"', "%22");
                    o.push_str(&format!("{{\\field{{\\*\\fldinst{{HYPERLINK \"{d}\"}}}}{{\\fldrslt{{\\ul\\cf3 {inner}\\ulnone\\cf1}}}}}}"));
                    links.push((text.trim().to_string(), dest));
                }
                TagEnd::Image => {
                    o.push_str("]}");
                }
                TagEnd::TableHead => {
                    in_head = false;
                    header_rows = table.len();
                }
                TagEnd::TableCell => {
                    let cell = stack.pop().unwrap_or_default();
                    if let Some(row) = table.last_mut() {
                        row.push(cell);
                    }
                }
                TagEnd::Table => {
                    let o: &mut String = unsafe { &mut *cur(&mut stack, &mut out) };
                    emit_table(o, &table, &aligns, header_rows, width_twips);
                }
                TagEnd::FootnoteDefinition => o.push_str("\\fs22\\cf1\\par\n"),
                _ => {}
            },
            Event::Text(s) => {
                if in_code_block {
                    esc(&s, o);
                } else {
                    esc(&s, o);
                    if let Some(lt) = link_text.last_mut() {
                        lt.push_str(&s);
                    }
                }
            }
            Event::Code(s) => {
                o.push_str("{\\f1\\fs20\\highlight4\\cf7 ");
                esc(&s, o);
                o.push('}');
                if let Some(lt) = link_text.last_mut() {
                    lt.push_str(&s);
                }
            }
            Event::Html(s) | Event::InlineHtml(s) => {
                o.push_str("{\\f1\\fs19\\cf2 ");
                esc(s.trim_end(), o);
                o.push('}');
                if !in_code_block && stack.is_empty() && s.ends_with('\n') {
                    o.push_str("\\par\n");
                }
            }
            Event::SoftBreak => o.push(' '),
            Event::HardBreak => o.push_str("\\line "),
            Event::Rule => {
                o.push_str("\\pard\\sa120\\li240\\cf6 ");
                o.push_str(&"\\u9472?".repeat(48));
                o.push_str("\\cf1\\par\n");
            }
            Event::TaskListMarker(done) => {
                // Replace the bullet just written with a checkbox.
                for bullet in ["\\u8226?  ", "\\u9702?  "] {
                    if o.ends_with(bullet) {
                        o.truncate(o.len() - bullet.len());
                        break;
                    }
                }
                o.push_str(if done { "\\u9745?  " } else { "\\u9744?  " });
            }
            Event::FootnoteReference(name) => {
                o.push_str("{\\super\\cf3 ");
                esc(&name, o);
                o.push('}');
            }
            _ => {}
        }
    }
    let _ = in_head;
    out.push('}');
    (out, links)
}

fn emit_table(o: &mut String, rows: &[Vec<String>], aligns: &[Alignment], header_rows: usize, width_twips: i32) {
    let cols = rows.iter().map(|r| r.len()).max().unwrap_or(0).max(1);
    let left = 240;
    let usable = (width_twips - 480).max(cols as i32 * 600);
    let cw = usable / cols as i32;
    for (ri, row) in rows.iter().enumerate() {
        let head = ri < header_rows;
        o.push_str(&format!("\\trowd\\trgaph100\\trleft{left}"));
        for c in 0..cols {
            if head {
                o.push_str("\\clcbpat4");
            }
            o.push_str("\\clbrdrt\\brdrs\\brdrw10\\brdrcf6\\clbrdrb\\brdrs\\brdrw10\\brdrcf6\\clbrdrl\\brdrs\\brdrw10\\brdrcf6\\clbrdrr\\brdrs\\brdrw10\\brdrcf6");
            o.push_str(&format!("\\cellx{}", left + cw * (c as i32 + 1)));
        }
        for c in 0..cols {
            let align = match aligns.get(c) {
                Some(Alignment::Center) => "\\qc",
                Some(Alignment::Right) => "\\qr",
                _ => "\\ql",
            };
            o.push_str(&format!("\\pard\\intbl{align}\\sa40\\plain\\f0\\fs21\\cf1"));
            o.push_str(if head { "\\b " } else { " " });
            o.push_str(row.get(c).map(String::as_str).unwrap_or(""));
            o.push_str(if head { "\\b0\\cell" } else { "\\cell" });
        }
        o.push_str("\\row\n");
    }
    o.push_str("\\pard\\sa120\\li240\\par\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn richedit_struct_layouts_match_pack4() {
        assert_eq!(std::mem::size_of::<EditStream>(), 20);
        assert_eq!(std::mem::offset_of!(EditStream, callback), 12);
        assert_eq!(std::mem::offset_of!(EnLink, msg), 24);
        assert_eq!(std::mem::offset_of!(EnLink, wparam), 28);
        assert_eq!(std::mem::offset_of!(EnLink, chrg), 44);
    }

    #[test]
    fn renders_common_markdown() {
        let t = Theme::named("light");
        let md = "# Title\n\nSome **bold**, *it*, `code` and [a link](https://example.com).\n\n- one\n- [x] done\n\n1. first\n\n> quote\n\n```rust\nfn main() {}\n```\n\n| a | b |\n|---|--:|\n| 1 | 2 |\n\n---\n\nUnicode: café ✓\n";
        let (rtf, links) = to_rtf(md, &t, 9000);
        assert!(rtf.starts_with("{\\rtf1"));
        assert!(rtf.ends_with('}'));
        assert_eq!(rtf.matches('{').count() - rtf.matches("\\{").count(), rtf.matches('}').count() - rtf.matches("\\}").count());
        assert!(rtf.contains("\\b bold\\b0"));
        assert!(rtf.contains("HYPERLINK \"https://example.com\""));
        assert!(rtf.contains("\\u9745?"));
        assert!(rtf.contains("\\trowd"));
        assert!(rtf.contains("caf\\u233?"));
        assert_eq!(links, vec![("a link".to_string(), "https://example.com".to_string())]);
    }

    #[test]
    fn escapes_rtf_specials() {
        let (rtf, _) = to_rtf("a {b} \\c", &Theme::named("dark"), 9000);
        assert!(rtf.contains("a \\{b\\} \\\\c"));
    }
}

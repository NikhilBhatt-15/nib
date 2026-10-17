//! Thin wrapper over a Scintilla window. Calls go through Scintilla's direct
//! function pointer instead of SendMessage, which matters for loops over
//! large documents.

use crate::sci_consts::*;
use crate::theme::Rgb;
use std::ffi::c_void;
use std::ptr::{null, null_mut};
use windows_sys::w;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

extern "C" {
    fn Scintilla_RegisterClasses(hinstance: *mut c_void) -> i32;
    fn nib_create_lexer(name: *const u8) -> *mut c_void;
}

type DirectFn = unsafe extern "C" fn(isize, u32, usize, isize) -> isize;

pub fn register_classes() {
    unsafe { Scintilla_RegisterClasses(GetModuleHandleW(null())) };
}

/// Matches `struct SCNotification` in Scintilla.h.
#[repr(C)]
pub struct Notification {
    pub hwnd_from: HWND,
    pub id_from: usize,
    pub code: u32,
    pub position: isize,
    pub ch: i32,
    pub modifiers: i32,
    pub modification_type: i32,
    pub text: *const u8,
    pub length: isize,
    pub lines_added: isize,
    pub message: i32,
    pub wparam: usize,
    pub lparam: isize,
    pub line: isize,
    pub fold_level_now: i32,
    pub fold_level_prev: i32,
    pub margin: i32,
    pub list_type: i32,
    pub x: i32,
    pub y: i32,
    pub token: i32,
    pub annotation_lines_added: isize,
    pub updated: i32,
    pub list_completion_method: i32,
    pub character_source: i32,
}

#[derive(Clone, Copy)]
pub struct Sci {
    pub hwnd: HWND,
    f: DirectFn,
    p: isize,
}

impl Sci {
    pub fn create(parent: HWND, id: usize) -> Sci {
        unsafe {
            let hwnd = CreateWindowExW(
                0,
                w!("Scintilla"),
                null(),
                WS_CHILD | WS_CLIPSIBLINGS | WS_TABSTOP,
                0,
                0,
                10,
                10,
                parent,
                id as _,
                GetModuleHandleW(null()),
                null(),
            );
            let f = SendMessageW(hwnd, SCI_GETDIRECTFUNCTION as u32, 0, 0);
            let p = SendMessageW(hwnd, SCI_GETDIRECTPOINTER as u32, 0, 0);
            Sci { hwnd, f: std::mem::transmute::<isize, DirectFn>(f), p }
        }
    }

    #[inline]
    pub fn call(&self, msg: i64, w: usize, l: isize) -> isize {
        unsafe { (self.f)(self.p, msg as u32, w, l) }
    }

    /// Numeric convenience form of `call`.
    #[inline]
    pub fn m(&self, msg: i64, w: i64, l: i64) -> i64 {
        self.call(msg, w as usize, l as isize) as i64
    }

    pub fn len(&self) -> usize {
        self.m(SCI_GETLENGTH, 0, 0) as usize
    }

    /// The document text without copying. Valid only until the next change.
    pub fn bytes(&self) -> &[u8] {
        let len = self.len();
        let p = self.call(SCI_GETCHARACTERPOINTER, 0, 0) as *const u8;
        if p.is_null() || len == 0 {
            return &[];
        }
        unsafe { std::slice::from_raw_parts(p, len) }
    }

    pub fn text(&self) -> Vec<u8> {
        self.bytes().to_vec()
    }

    pub fn text_string(&self) -> String {
        String::from_utf8_lossy(self.bytes()).into_owned()
    }

    pub fn range(&self, a: usize, b: usize) -> Vec<u8> {
        let (a, b) = (a.min(b), a.max(b).min(self.len()));
        if a >= b {
            return Vec::new();
        }
        let p = self.call(SCI_GETRANGEPOINTER, a, (b - a) as isize) as *const u8;
        if p.is_null() {
            return Vec::new();
        }
        unsafe { std::slice::from_raw_parts(p, b - a).to_vec() }
    }

    pub fn range_string(&self, a: usize, b: usize) -> String {
        String::from_utf8_lossy(&self.range(a, b)).into_owned()
    }

    /// Replaces the whole text; accepts embedded NULs.
    pub fn set_text(&self, bytes: &[u8]) {
        self.m(SCI_CLEARALL, 0, 0);
        if !bytes.is_empty() {
            self.call(SCI_APPENDTEXT, bytes.len(), bytes.as_ptr() as isize);
        }
    }

    pub fn replace_range(&self, a: usize, b: usize, bytes: &[u8]) {
        self.m(SCI_SETTARGETRANGE, a as i64, b as i64);
        self.call(SCI_REPLACETARGET, bytes.len(), bytes.as_ptr() as isize);
    }

    pub fn replace_sel(&self, bytes: &[u8]) {
        let (a, b) = (self.sel_start(), self.sel_end());
        self.replace_range(a, b, bytes);
        let end = a + bytes.len();
        self.set_sel(end, end);
    }

    pub fn insert(&self, pos: usize, bytes: &[u8]) {
        let mut v = bytes.to_vec();
        v.push(0);
        self.call(SCI_INSERTTEXT, pos, v.as_ptr() as isize);
    }

    pub fn pos(&self) -> usize {
        self.m(SCI_GETCURRENTPOS, 0, 0) as usize
    }
    pub fn anchor(&self) -> usize {
        self.m(SCI_GETANCHOR, 0, 0) as usize
    }
    pub fn sel_start(&self) -> usize {
        self.m(SCI_GETSELECTIONSTART, 0, 0) as usize
    }
    pub fn sel_end(&self) -> usize {
        self.m(SCI_GETSELECTIONEND, 0, 0) as usize
    }
    pub fn sel_empty(&self) -> bool {
        self.m(SCI_GETSELECTIONEMPTY, 0, 0) != 0
    }
    pub fn set_sel(&self, anchor: usize, caret: usize) {
        self.m(SCI_SETSEL, anchor as i64, caret as i64);
    }
    pub fn selected(&self) -> Vec<u8> {
        self.range(self.sel_start(), self.sel_end())
    }
    pub fn line_from_pos(&self, pos: usize) -> usize {
        self.m(SCI_LINEFROMPOSITION, pos as i64, 0) as usize
    }
    pub fn line_start(&self, line: usize) -> usize {
        self.m(SCI_POSITIONFROMLINE, line as i64, 0) as usize
    }
    pub fn line_end(&self, line: usize) -> usize {
        self.m(SCI_GETLINEENDPOSITION, line as i64, 0) as usize
    }
    pub fn line_count(&self) -> usize {
        self.m(SCI_GETLINECOUNT, 0, 0) as usize
    }
    pub fn line_text(&self, line: usize) -> String {
        self.range_string(self.line_start(line), self.line_end(line))
    }
    pub fn first_visible_line(&self) -> usize {
        self.m(SCI_GETFIRSTVISIBLELINE, 0, 0) as usize
    }
    pub fn lines_on_screen(&self) -> usize {
        self.m(SCI_LINESONSCREEN, 0, 0) as usize
    }
    pub fn doc_line_from_visible(&self, v: usize) -> usize {
        self.m(SCI_DOCLINEFROMVISIBLE, v as i64, 0) as usize
    }
    pub fn visible_from_doc_line(&self, line: usize) -> usize {
        self.m(SCI_VISIBLEFROMDOCLINE, line as i64, 0) as usize
    }
    pub fn set_first_visible_line(&self, v: usize) {
        self.m(SCI_SETFIRSTVISIBLELINE, v as i64, 0);
    }

    /// Moves the caret to `pos` and scrolls it into view, centred if it was off-screen.
    pub fn goto_pos_centered(&self, anchor: usize, caret: usize) {
        let line = self.line_from_pos(caret);
        self.m(SCI_ENSUREVISIBLEENFORCEPOLICY, line as i64, 0);
        let first = self.first_visible_line();
        let vis = self.visible_from_doc_line(line);
        let n = self.lines_on_screen().max(1);
        if vis < first || vis >= first + n {
            self.set_first_visible_line(vis.saturating_sub(n / 3));
        }
        self.set_sel(anchor, caret);
        self.m(SCI_SCROLLCARET, 0, 0);
    }

    pub fn goto_line(&self, line: usize) {
        let p = self.line_start(line.min(self.line_count().saturating_sub(1)));
        self.goto_pos_centered(p, p);
    }

    pub fn begin_undo(&self) {
        self.m(SCI_BEGINUNDOACTION, 0, 0);
    }
    pub fn end_undo(&self) {
        self.m(SCI_ENDUNDOACTION, 0, 0);
    }
    pub fn readonly(&self) -> bool {
        self.m(SCI_GETREADONLY, 0, 0) != 0
    }

    pub fn set_str(&self, msg: i64, w: usize, s: &str) {
        let mut v = s.as_bytes().to_vec();
        v.push(0);
        self.call(msg, w, v.as_ptr() as isize);
    }

    pub fn set_lexer(&self, name: &str) {
        let mut v = name.as_bytes().to_vec();
        v.push(0);
        let lexer = unsafe { nib_create_lexer(v.as_ptr()) };
        self.call(SCI_SETILEXER, 0, lexer as isize);
    }

    pub fn style_fore(&self, style: i64, c: Rgb) {
        self.m(SCI_STYLESETFORE, style, c.colorref() as i64);
    }
    pub fn style_back(&self, style: i64, c: Rgb) {
        self.m(SCI_STYLESETBACK, style, c.colorref() as i64);
    }
    pub fn element(&self, element: i64, c: Rgb, alpha: u8) {
        self.m(SCI_SETELEMENTCOLOUR, element, c.argb_abgr(alpha) as i64);
    }

    pub fn show(&self, on: bool) {
        unsafe { ShowWindow(self.hwnd, if on { SW_SHOWNA } else { SW_HIDE }) };
    }

    pub fn null() -> Sci {
        unsafe extern "C" fn nop(_: isize, _: u32, _: usize, _: isize) -> isize {
            0
        }
        Sci { hwnd: null_mut(), f: nop, p: 0 }
    }
}

/// Creates a new empty Scintilla document with one reference held by the caller.
pub fn create_document(view: &Sci, large: bool) -> usize {
    let opts = if large { SC_DOCUMENTOPTION_TEXT_LARGE | SC_DOCUMENTOPTION_STYLES_NONE } else { SC_DOCUMENTOPTION_DEFAULT };
    view.m(SCI_CREATEDOCUMENT, 0, opts) as usize
}

pub fn release_document(view: &Sci, doc: usize) {
    if doc != 0 {
        view.call(SCI_RELEASEDOCUMENT, 0, doc as isize);
    }
}

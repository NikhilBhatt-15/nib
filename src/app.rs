//! Application state, the main window and its layout, and the routing of
//! Scintilla notifications and timers.
//!
//! Everything runs on the UI thread. Re-entrancy rule: Scintilla notifies
//! synchronously (WM_NOTIFY) while it is being called, so notification
//! handlers only touch `Cell`s and post a deferred refresh; they never borrow
//! `App::st`. Code holding a borrow of `st` must not call Win32 functions that
//! send messages to our own windows.

use crate::doc::Doc;
use crate::gfx::{self, Fonts};
use crate::keys::Chord;
use crate::paths::Paths;
use crate::sci::{Notification, Sci};
use crate::sci_consts::*;
use crate::settings::Settings;
use crate::theme::Theme;
use crate::util::{self, now_ms, wide};
use crate::{chrome, commands, compare, darkmode, docs, editor, findbar, palette, preview, scratchpad, session, tabs, tray};
use serde::{Deserialize, Serialize};
use std::cell::{Cell, OnceCell, RefCell};
use std::collections::HashMap;
use std::mem::{size_of, zeroed};
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use windows_sys::w;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::DataExchange::COPYDATASTRUCT;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Controls::NMHDR;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::Shell::{DragFinish, DragQueryFileW, HDROP};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub const CLASS: *const u16 = w!("NibMain");

pub const WM_APP_REFRESH: u32 = WM_APP + 10;
pub const WM_APP_OPEN: u32 = WM_APP + 11;
pub const WM_APP_NOTES: u32 = WM_APP + 12;
pub const WM_APP_AI: u32 = WM_APP + 13;
pub const WM_APP_INDEX: u32 = WM_APP + 14;

pub const ID_VIEW: [usize; 2] = [101, 102];
pub const ID_LOADER: usize = 103;

const TIMER_TICK: usize = 1;
const TIMER_EDIT: usize = 2;
const TIMER_DISK: usize = 3;
const HOTKEY_SCRATCH: i32 = 1;

/// UI parts that need repainting after a change; coalesced into one WM_APP_REFRESH.
pub const R_TABS: u32 = 1;
pub const R_STATUS: u32 = 2;
pub const R_TITLE: u32 = 4;
pub const R_LAYOUT: u32 = 8;
pub const R_DIRTY: u32 = 16;
pub const R_ALL: u32 = 31;

#[derive(Default)]
pub struct ViewFlags {
    /// Text changed since the session snapshot looked at this view.
    pub modified: Cell<bool>,
    pub last_edit: Cell<u64>,
    /// Bumped on every change; keys the word-count cache.
    pub rev: Cell<u64>,
    pub markdown: Cell<bool>,
    pub auto_indent: Cell<bool>,
}

pub struct Group {
    pub tabs: Vec<u64>,
    pub active: u64,
    /// Most recently used first, for picking the next tab after a close.
    pub mru: Vec<u64>,
    /// Horizontal scroll of the tab strip in pixels.
    pub scroll: i32,
}

impl Group {
    fn new() -> Group {
        Group { tabs: Vec::new(), active: 0, mru: Vec::new(), scroll: 0 }
    }
}

pub enum Closed {
    Path(PathBuf, crate::doc::ViewState),
    Text(String, Vec<u8>, usize),
}

#[derive(Clone)]
pub enum InfoAction {
    Dismiss,
    Reload(u64),
    KeepMine(u64),
    CompareDisk(u64),
    SaveAs(u64),
    CloseDoc(u64),
    CompareNext,
    ComparePrev,
    CompareClose,
    CompareRestore,
    EnableAi(&'static str),
    AiCompareBefore,
    Undo,
}

pub struct Info {
    pub icon: &'static str,
    pub text: String,
    pub actions: Vec<(String, InfoAction)>,
    /// The doc it concerns; the bar hides when another doc is shown.
    pub doc: u64,
    pub warning: bool,
}

pub struct State {
    pub docs: Vec<Doc>,
    pub groups: [Group; 2],
    pub active_group: usize,
    pub split: bool,
    pub split_ratio: f32,
    pub next_untitled: u32,
    pub closed: Vec<Closed>,
    pub info: Option<Info>,
    /// (view rev, doc id) -> (words, chars) for the status bar.
    pub counts: HashMap<u64, (u64, usize, usize)>,
    pub status_msg: Option<(String, u64)>,
    pub persist: Persist,
}

impl State {
    pub fn doc(&self, id: u64) -> Option<&Doc> {
        self.docs.iter().find(|d| d.id == id)
    }
    pub fn doc_mut(&mut self, id: u64) -> Option<&mut Doc> {
        self.docs.iter_mut().find(|d| d.id == id)
    }
    pub fn active_id(&self) -> u64 {
        self.groups[self.active_group].active
    }
    /// Groups currently showing `id`.
    pub fn shown_in(&self, id: u64) -> Vec<usize> {
        (0..2).filter(|&g| (g == 0 || self.split) && self.groups[g].active == id).collect()
    }
}

/// Small UI state kept across runs in state.json.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Persist {
    pub recent: Vec<String>,
    pub pinned: Vec<String>,
    pub window: Option<[i32; 4]>,
    pub maximized: bool,
    pub scratch_rect: Option<[i32; 4]>,
    pub ai_remote_ok: bool,
}

pub struct App {
    pub hwnd: Cell<HWND>,
    pub paths: Paths,
    pub settings: RefCell<Settings>,
    pub theme: Cell<Theme>,
    pub fonts: RefCell<Fonts>,
    pub views: [Cell<Sci>; 2],
    pub loader: Cell<Sci>,
    pub loader_idle: Cell<usize>,
    pub previews: [Cell<HWND>; 2],
    pub strips: [Cell<HWND>; 2],
    pub status: Cell<HWND>,
    pub infobar: Cell<HWND>,
    pub vflags: [ViewFlags; 2],
    pub st: RefCell<State>,
    pub store: session::Store,
    pub refresh: Cell<u32>,
    pub manifest_dirty: Cell<bool>,
    pub crashed: Cell<bool>,
    pub keymap: RefCell<HashMap<Chord, usize>>,
    pub quitting: Cell<bool>,
    pub splitter_drag: Cell<bool>,
    pub settings_stamp: Cell<Option<crate::fileio::Stamp>>,
}

thread_local! {
    static APP: OnceCell<&'static App> = const { OnceCell::new() };
}

pub fn app() -> &'static App {
    APP.with(|a| *a.get().expect("app initialised"))
}

pub fn theme() -> Theme {
    app().theme.get()
}

pub fn settings() -> std::cell::Ref<'static, Settings> {
    app().settings.borrow()
}

pub fn view(g: usize) -> Sci {
    app().views[g].get()
}

pub fn active_group() -> usize {
    app().st.borrow().active_group
}

pub fn active_view() -> Sci {
    view(active_group())
}

pub fn active_doc() -> u64 {
    app().st.borrow().active_id()
}

/// Marks parts of the UI for repaint; processed once per message-loop turn.
pub fn invalidate(bits: u32) {
    let a = app();
    let old = a.refresh.get();
    a.refresh.set(old | bits);
    if old == 0 {
        unsafe { PostMessageW(a.hwnd.get(), WM_APP_REFRESH, 0, 0) };
    }
}

pub fn set_status(msg: &str) {
    if let Ok(mut st) = app().st.try_borrow_mut() {
        st.status_msg = Some((msg.to_string(), now_ms()));
    }
    invalidate(R_STATUS);
}

pub fn dpi() -> i32 {
    app().fonts.borrow().dpi
}

/// Device pixels for a 96-DPI length.
pub fn px(v: i32) -> i32 {
    (v as f32 * dpi() as f32 / 96.0).round() as i32
}

pub fn app_icon(size: i32) -> HICON {
    use std::sync::OnceLock;
    static BIG: OnceLock<usize> = OnceLock::new();
    static SMALL: OnceLock<usize> = OnceLock::new();
    let cell = if size <= 20 { &SMALL } else { &BIG };
    *cell.get_or_init(|| gfx::make_icon(size) as usize) as HICON
}

// ------------------------------------------------------------------ start-up

pub struct StartArgs {
    pub files: Vec<PathBuf>,
    /// Command ids to run (`--cmd file.new`), for scripts and shortcuts.
    pub commands: Vec<String>,
    pub tray: bool,
    pub new_note: bool,
    pub scratch: bool,
}

pub fn parse_args(args: &[String], cwd: &std::path::Path) -> StartArgs {
    let mut a = StartArgs { files: Vec::new(), commands: Vec::new(), tray: false, new_note: false, scratch: false };
    let mut it = args.iter();
    while let Some(s) = it.next() {
        match s.as_str() {
            "--cmd" | "--command" => {
                if let Some(c) = it.next() {
                    a.commands.push(c.clone());
                }
            }
            "--tray" => a.tray = true,
            "--new" => a.new_note = true,
            "--scratch" | "--scratchpad" => a.scratch = true,
            _ if s.starts_with("--") => {}
            _ => {
                let p = PathBuf::from(s);
                a.files.push(if p.is_absolute() { p } else { cwd.join(p) });
            }
        }
    }
    a
}

pub fn run(args: StartArgs) {
    let paths = Paths::resolve();
    let settings = Settings::load(&paths.settings());
    let theme = Theme::named(&settings.theme);
    darkmode::set_app_dark(theme.dark);
    let persist: Persist = std::fs::read(paths.state()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let store = session::Store::open(&paths.session());
    let crashed = store.begin();

    let state: &'static App = Box::leak(Box::new(App {
        hwnd: Cell::new(null_mut()),
        settings_stamp: Cell::new(crate::fileio::stamp(&paths.settings())),
        paths,
        settings: RefCell::new(settings),
        theme: Cell::new(theme),
        fonts: RefCell::new(Fonts::new(96, 10)),
        views: [Cell::new(Sci::null()), Cell::new(Sci::null())],
        loader: Cell::new(Sci::null()),
        loader_idle: Cell::new(0),
        previews: [Cell::new(null_mut()), Cell::new(null_mut())],
        strips: [Cell::new(null_mut()), Cell::new(null_mut())],
        status: Cell::new(null_mut()),
        infobar: Cell::new(null_mut()),
        vflags: [ViewFlags::default(), ViewFlags::default()],
        st: RefCell::new(State {
            docs: Vec::new(),
            groups: [Group::new(), Group::new()],
            active_group: 0,
            split: false,
            split_ratio: 0.5,
            next_untitled: 1,
            closed: Vec::new(),
            info: None,
            counts: HashMap::new(),
            status_msg: None,
            persist,
        }),
        store,
        refresh: Cell::new(0),
        manifest_dirty: Cell::new(false),
        crashed: Cell::new(crashed),
        keymap: RefCell::new(HashMap::new()),
        quitting: Cell::new(false),
        splitter_drag: Cell::new(false),
    }));
    APP.with(|a| a.set(state).ok());
    commands::rebuild_keymap();

    unsafe {
        let hinst = GetModuleHandleW(null());
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_DBLCLKS,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst,
            hIcon: app_icon(32),
            hIconSm: app_icon(16),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            lpszClassName: CLASS,
            ..zeroed()
        };
        RegisterClassExW(&wc);
        let (x, y, w, h) = match state.st.borrow().persist.window {
            Some([x, y, w, h]) if w > 200 && h > 150 => (x, y, w, h),
            _ => (CW_USEDEFAULT, CW_USEDEFAULT, 1100, 760),
        };
        let hwnd = CreateWindowExW(
            WS_EX_ACCEPTFILES,
            CLASS,
            w!("Nib"),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            x,
            y,
            w,
            h,
            null_mut(),
            commands::build_menu(),
            hinst,
            null(),
        );
        state.hwnd.set(hwnd);
        *state.fonts.borrow_mut() = Fonts::new(GetDpiForWindow(hwnd) as i32, 10);
        create_children(hwnd);
        apply_theme_all();

        docs::restore_session(&args.files);
        if args.new_note {
            docs::new_doc();
        }
        if state.st.borrow().docs.is_empty() {
            docs::new_doc();
        }
        layout();

        if !args.tray {
            let max = state.st.borrow().persist.maximized;
            ShowWindow(hwnd, if max { SW_SHOWMAXIMIZED } else { SW_SHOW });
            UpdateWindow(hwnd);
            SetFocus(active_view().hwnd);
        }
        if state.settings.borrow().always_on_top {
            set_topmost(true);
        }
        if state.settings.borrow().tray_icon || args.tray {
            tray::add(hwnd);
        }
        register_scratch_hotkey();
        SetTimer(hwnd, TIMER_TICK, 1000, None);
        if args.scratch {
            scratchpad::toggle();
        }
        for c in &args.commands {
            commands::run(c);
        }
        crate::notes::start_index();
        invalidate(R_ALL);
        message_loop();
    }
}

fn create_children(hwnd: HWND) {
    let a = app();
    for g in 0..2 {
        let v = Sci::create(hwnd, ID_VIEW[g]);
        editor::init_view(&v);
        a.views[g].set(v);
        a.strips[g].set(tabs::create(hwnd, g));
        a.previews[g].set(preview::create(hwnd, g));
    }
    let loader = Sci::create(hwnd, ID_LOADER);
    editor::init_view(&loader);
    a.loader.set(loader);
    a.loader_idle.set(loader.m(SCI_GETDOCPOINTER, 0, 0) as usize);
    loader.m(SCI_ADDREFDOCUMENT, 0, a.loader_idle.get() as i64);
    a.status.set(chrome::create_status(hwnd));
    a.infobar.set(chrome::create_infobar(hwnd));
    findbar::create(hwnd);
    view(0).show(true);
}

pub fn register_scratch_hotkey() -> bool {
    let hk = settings().scratchpad_hotkey.clone();
    unsafe {
        let hwnd = app().hwnd.get();
        UnregisterHotKey(hwnd, HOTKEY_SCRATCH);
        if let Some(c) = Chord::parse(&hk) {
            return RegisterHotKey(hwnd, HOTKEY_SCRATCH, c.hot_mods(), c.vk as u32) != 0;
        }
    }
    false
}

unsafe fn message_loop() {
    let mut msg: MSG = zeroed();
    while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
        if palette::pre_translate(&msg) || findbar::pre_translate(&msg) || scratchpad::pre_translate(&msg) || commands::pre_translate(&msg) {
            continue;
        }
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}

// ------------------------------------------------------------------ theme

pub fn apply_theme_all() {
    let a = app();
    let t = a.theme.get();
    darkmode::set_app_dark(t.dark);
    let hwnd = a.hwnd.get();
    gfx::style_frame(hwnd, &t, false);
    unsafe {
        DrawMenuBar(hwnd);
    }
    for g in 0..2 {
        let v = view(g);
        darkmode::set_window_dark(v.hwnd, t.dark);
        restyle_view(g);
        preview::apply_theme(g);
    }
    findbar::apply_theme();
    palette::apply_theme();
    scratchpad::apply_theme();
    unsafe {
        RedrawWindow(hwnd, null(), null_mut(), RDW_INVALIDATE | RDW_ALLCHILDREN | RDW_FRAME | RDW_ERASE);
    }
    invalidate(R_ALL);
}

/// Re-applies colours and settings to view `g` for the document it shows.
pub fn restyle_view(g: usize) {
    let a = app();
    let (lang, large, md) = {
        let st = a.st.borrow();
        match st.doc(st.groups[g].active) {
            Some(d) => (d.lang, d.large, d.is_markdown()),
            None => (0, false, false),
        }
    };
    let s = a.settings.borrow().clone();
    editor::style_view(&view(g), &a.theme.get(), &s, lang, large);
    a.vflags[g].markdown.set(md);
    a.vflags[g].auto_indent.set(s.auto_indent);
}

pub fn reload_theme() {
    let a = app();
    let name = a.settings.borrow().theme.clone();
    a.theme.set(Theme::named(&name));
    apply_theme_all();
}

pub fn set_topmost(on: bool) {
    unsafe {
        SetWindowPos(app().hwnd.get(), if on { HWND_TOPMOST } else { HWND_NOTOPMOST }, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
}

/// Saves settings.json and applies whatever changed.
pub fn apply_settings(new: Settings) {
    let a = app();
    let old = a.settings.replace(new.clone());
    let _ = new.save(&a.paths.settings());
    a.settings_stamp.set(crate::fileio::stamp(&a.paths.settings()));
    settings_changed(&old, &new);
}

pub fn settings_changed(old: &Settings, new: &Settings) {
    let a = app();
    if old.theme != new.theme {
        reload_theme();
    } else {
        for g in 0..2 {
            restyle_view(g);
        }
        scratchpad::apply_theme();
    }
    if old.always_on_top != new.always_on_top {
        set_topmost(new.always_on_top);
    }
    if old.tray_icon != new.tray_icon {
        if new.tray_icon {
            tray::add(a.hwnd.get());
        } else {
            tray::remove(a.hwnd.get());
        }
    }
    if old.scratchpad_hotkey != new.scratchpad_hotkey && !register_scratch_hotkey() && !new.scratchpad_hotkey.is_empty() {
        set_status(&format!("{} is already used by another program", new.scratchpad_hotkey));
    }
    if old.keys != new.keys {
        commands::rebuild_keymap();
    }
    if old.notes_dir != new.notes_dir || old.search_roots != new.search_roots {
        crate::notes::start_index();
    }
    invalidate(R_ALL);
}

/// Picks up hand edits to settings.json (saved from Nib or elsewhere).
pub fn check_settings_file() {
    let a = app();
    let path = a.paths.settings();
    let now = crate::fileio::stamp(&path);
    if now == a.settings_stamp.get() {
        return;
    }
    a.settings_stamp.set(now);
    let new = Settings::load(&path);
    let old = a.settings.replace(new.clone());
    if old != new {
        settings_changed(&old, &new);
        set_status("Settings reloaded");
    }
}

pub fn save_persist() {
    let a = app();
    let json = serde_json::to_vec_pretty(&a.st.borrow().persist).unwrap_or_default();
    let _ = crate::fileio::save_atomic(&a.paths.state(), &json);
}

// ------------------------------------------------------------------ layout

pub struct Layout {
    pub info: Option<RECT>,
    pub strips: [Option<RECT>; 2],
    pub editors: [Option<RECT>; 2],
    pub previews: [Option<RECT>; 2],
    pub splitter: Option<RECT>,
    pub find: Option<RECT>,
    pub status: RECT,
}

pub fn compute_layout() -> Layout {
    let a = app();
    let mut rc: RECT = unsafe { zeroed() };
    unsafe { GetClientRect(a.hwnd.get(), &mut rc) };
    let st = a.st.borrow();
    let status_h = px(24);
    let tab_h = px(34);
    let status = RECT { left: 0, top: rc.bottom - status_h, right: rc.right, bottom: rc.bottom };
    let mut bottom = status.top;
    let find = findbar::height().map(|h| {
        let r = RECT { left: 0, top: bottom - h, right: rc.right, bottom };
        bottom -= h;
        r
    });
    let mut top = 0;
    let info = st.info.as_ref().filter(|i| i.doc == 0 || i.doc == st.active_id() || !st.shown_in(i.doc).is_empty()).map(|_| {
        let r = RECT { left: 0, top, right: rc.right, bottom: top + px(36) };
        top += px(36);
        r
    });
    let mut l = Layout { info, strips: [None, None], editors: [None, None], previews: [None, None], splitter: None, find, status };
    let cols: Vec<(usize, i32, i32)> = if st.split {
        let sw = px(5);
        let mid = ((rc.right as f32) * st.split_ratio.clamp(0.15, 0.85)) as i32;
        l.splitter = Some(RECT { left: mid - sw / 2, top, right: mid - sw / 2 + sw, bottom });
        vec![(0, 0, mid - sw / 2), (1, mid - sw / 2 + sw, rc.right)]
    } else {
        vec![(0, 0, rc.right)]
    };
    for (g, x0, x1) in cols {
        l.strips[g] = Some(RECT { left: x0, top, right: x1, bottom: top + tab_h });
        let ed = RECT { left: x0, top: top + tab_h, right: x1, bottom };
        let show_preview = st.doc(st.groups[g].active).is_some_and(|d| d.preview);
        if show_preview {
            let mid = (ed.left + ed.right) / 2;
            l.editors[g] = Some(RECT { right: mid, ..ed });
            l.previews[g] = Some(RECT { left: mid + 1, ..ed });
        } else {
            l.editors[g] = Some(ed);
        }
    }
    l
}

pub fn layout() {
    let a = app();
    let l = compute_layout();
    unsafe {
        let place = |h: HWND, r: Option<RECT>| match r {
            Some(r) => {
                SetWindowPos(h, null_mut(), r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW);
            }
            None => {
                ShowWindow(h, SW_HIDE);
            }
        };
        for g in 0..2 {
            place(a.strips[g].get(), l.strips[g]);
            place(view(g).hwnd, l.editors[g]);
            place(a.previews[g].get(), l.previews[g]);
        }
        place(a.infobar.get(), l.info);
        place(a.status.get(), Some(l.status));
        place(findbar::hwnd(), l.find);
        InvalidateRect(a.hwnd.get(), null(), 1);
    }
}

// ------------------------------------------------------------------ notifications

fn group_of_view(hwnd: HWND) -> Option<usize> {
    (0..2).find(|&g| view(g).hwnd == hwnd)
}

unsafe fn on_sci_notify(n: &Notification) {
    let Some(g) = group_of_view(n.hwnd_from) else { return };
    let a = app();
    let f = &a.vflags[g];
    match n.code as i64 {
        SCN_MODIFIED => {
            if n.modification_type as i64 & (SC_MOD_INSERTTEXT | SC_MOD_DELETETEXT) != 0 {
                f.modified.set(true);
                f.last_edit.set(now_ms());
                f.rev.set(f.rev.get() + 1);
                SetTimer(a.hwnd.get(), TIMER_EDIT, 250, None);
                invalidate(R_STATUS);
            }
        }
        SCN_SAVEPOINTLEFT | SCN_SAVEPOINTREACHED => invalidate(R_DIRTY | R_TABS | R_TITLE),
        SCN_UPDATEUI => {
            let v = view(g);
            if n.updated as i64 & (SC_UPDATE_SELECTION | SC_UPDATE_CONTENT) != 0 {
                editor::match_braces(&v);
                editor::highlight_occurrences(&v);
                invalidate(R_STATUS);
            }
            if n.updated as i64 & SC_UPDATE_V_SCROLL != 0 {
                compare::on_scroll(g);
                preview::sync_scroll(g);
            }
        }
        SCN_CHARADDED => {
            let v = view(g);
            let eol_cr = v.m(SCI_GETEOLMODE, 0, 0) == SC_EOL_CR;
            if (n.ch == '\n' as i32 || (n.ch == '\r' as i32 && eol_cr)) && f.auto_indent.get() {
                editor::on_newline(&v, f.markdown.get());
            }
        }
        SCN_MARGINCLICK => {
            if n.margin == 1 {
                let v = view(g);
                let line = v.line_from_pos(n.position as usize);
                v.goto_line(line);
                editor::toggle_bookmark(&v);
            }
        }
        SCN_FOCUSIN => {
            if let Ok(mut st) = a.st.try_borrow_mut() {
                if st.active_group != g {
                    st.active_group = g;
                    drop(st);
                    invalidate(R_TABS | R_STATUS | R_TITLE);
                }
            }
        }
        SCN_ZOOM => {
            let z = view(g).m(SCI_GETZOOM, 0, 0) as i32;
            if let Ok(mut s) = a.settings.try_borrow_mut() {
                s.zoom = z;
            }
            let other = view(1 - g);
            if other.m(SCI_GETZOOM, 0, 0) as i32 != z {
                other.m(SCI_SETZOOM, z as i64, 0);
            }
            editor::update_margins(&view(g), &settings());
            invalidate(R_STATUS);
        }
        SCN_URIDROPPED
            if !n.text.is_null() => {
                let s = std::ffi::CStr::from_ptr(n.text as *const i8).to_string_lossy().into_owned();
                let files: Vec<PathBuf> = s.lines().map(|l| l.trim()).filter(|l| !l.is_empty()).map(|l| PathBuf::from(l.trim_start_matches("file:///").replace("%20", " "))).collect();
                post_open(files);
            }
        _ => {}
    }
}

/// Opens files from a context where the state may be borrowed (notifications, IPC).
pub fn post_open(files: Vec<PathBuf>) {
    if files.is_empty() {
        return;
    }
    let b = Box::new(files);
    unsafe { PostMessageW(app().hwnd.get(), WM_APP_OPEN, 0, Box::into_raw(b) as LPARAM) };
}

fn process_refresh() {
    let a = app();
    let bits = a.refresh.replace(0);
    if bits & R_DIRTY != 0 {
        docs::sync_dirty_flags();
    }
    if bits & R_LAYOUT != 0 {
        layout();
    }
    if bits & (R_TABS | R_DIRTY) != 0 {
        for g in 0..2 {
            unsafe { InvalidateRect(a.strips[g].get(), null(), 0) };
        }
    }
    if bits & (R_TITLE | R_DIRTY) != 0 {
        update_title();
    }
    if bits & (R_STATUS | R_DIRTY) != 0 {
        unsafe { InvalidateRect(a.status.get(), null(), 0) };
    }
    unsafe { InvalidateRect(a.infobar.get(), null(), 0) };
}

pub fn update_title() {
    let a = app();
    let title = {
        let st = a.st.borrow();
        match st.doc(st.active_id()) {
            Some(d) => {
                let dir = d.dir_hint();
                let dot = if d.dirty { "● " } else { "" };
                if dir.is_empty() {
                    format!("{dot}{} - Nib", d.name())
                } else {
                    format!("{dot}{} - {dir} - Nib", d.name())
                }
            }
            None => "Nib".into(),
        }
    };
    let w = wide(&title);
    unsafe { SetWindowTextW(a.hwnd.get(), w.as_ptr()) };
}

// ------------------------------------------------------------------ timers

fn on_tick() {
    docs::snapshot(false);
    docs::autosave_tick();
    let expire = {
        let st = app().st.borrow();
        st.status_msg.as_ref().is_some_and(|(_, t)| now_ms() - t > 4000)
    };
    if expire {
        app().st.borrow_mut().status_msg = None;
        invalidate(R_STATUS);
    }
}

fn on_edit_settled() {
    unsafe { KillTimer(app().hwnd.get(), TIMER_EDIT) };
    for g in 0..2 {
        editor::update_margins(&view(g), &settings());
    }
    docs::update_counts();
    preview::refresh_all();
    compare::on_edit();
    findbar::on_doc_changed();
    invalidate(R_STATUS);
}

// ------------------------------------------------------------------ window procedure

pub fn quit() {
    let a = app();
    if a.quitting.get() {
        return;
    }
    if !docs::confirm_quit() {
        return;
    }
    a.quitting.set(true);
    save_window_placement();
    docs::snapshot(true);
    docs::write_manifest_now();
    a.store.end();
    scratchpad::save_now();
    save_persist();
    unsafe { DestroyWindow(a.hwnd.get()) };
}

fn save_window_placement() {
    let a = app();
    unsafe {
        let mut wp: WINDOWPLACEMENT = zeroed();
        wp.length = size_of::<WINDOWPLACEMENT>() as u32;
        if GetWindowPlacement(a.hwnd.get(), &mut wp) != 0 {
            let r = wp.rcNormalPosition;
            let mut st = a.st.borrow_mut();
            st.persist.window = Some([r.left, r.top, r.right - r.left, r.bottom - r.top]);
            st.persist.maximized = wp.showCmd == SW_SHOWMAXIMIZED as u32;
        }
    }
}

pub fn show_main() {
    let hwnd = app().hwnd.get();
    unsafe {
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        } else if IsWindowVisible(hwnd) == 0 {
            ShowWindow(hwnd, SW_SHOW);
        }
        SetForegroundWindow(hwnd);
        SetFocus(active_view().hwnd);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let a = app();
    if let Some(r) = darkmode::handle_menu_bar(hwnd, msg, wp, lp, &a.theme.get()) {
        return r;
    }
    match msg {
        WM_NCPAINT | WM_NCACTIVATE => {
            let r = DefWindowProcW(hwnd, msg, wp, lp);
            darkmode::paint_menu_bar_line(hwnd, &a.theme.get());
            r
        }
        WM_ERASEBKGND => {
            let dc = wp as HDC;
            let mut rc: RECT = zeroed();
            GetClientRect(hwnd, &mut rc);
            let b = CreateSolidBrush(a.theme.get().chrome.colorref());
            FillRect(dc, &rc, b);
            DeleteObject(b);
            1
        }
        WM_SIZE => {
            layout();
            0
        }
        WM_GETMINMAXINFO => {
            let mmi = &mut *(lp as *mut MINMAXINFO);
            mmi.ptMinTrackSize = POINT { x: px(360), y: px(240) };
            0
        }
        WM_NOTIFY => {
            let hdr = &*(lp as *const NMHDR);
            if hdr.idFrom == ID_VIEW[0] || hdr.idFrom == ID_VIEW[1] {
                on_sci_notify(&*(lp as *const Notification));
            } else if let Some(r) = preview::on_notify(lp) {
                return r;
            }
            0
        }
        WM_COMMAND => {
            let id = wp & 0xffff;
            let code = ((wp >> 16) & 0xffff) as u32;
            if lp != 0 && findbar::on_command(lp as HWND, code) {
                return 0;
            }
            if code == 0 || code == 1 {
                commands::on_menu(id);
            }
            0
        }
        WM_INITMENUPOPUP => {
            commands::init_menu_popup(wp as HMENU);
            0
        }
        WM_TIMER => {
            match wp {
                TIMER_TICK => on_tick(),
                TIMER_EDIT => on_edit_settled(),
                TIMER_DISK => {
                    KillTimer(hwnd, TIMER_DISK);
                    docs::check_disk_changes();
                    check_settings_file();
                    crate::snippets::check_reload();
                }
                _ => {}
            }
            0
        }
        WM_APP_REFRESH => {
            process_refresh();
            0
        }
        WM_APP_OPEN => {
            let files = Box::from_raw(lp as *mut Vec<PathBuf>);
            show_main();
            for f in files.iter() {
                docs::open_path(f, None);
            }
            0
        }
        WM_APP_NOTES => {
            crate::notes::on_results(wp, lp);
            0
        }
        WM_APP_INDEX => {
            crate::notes::on_index(lp);
            0
        }
        WM_APP_AI => {
            crate::ai::on_result(lp);
            0
        }
        WM_ACTIVATEAPP => {
            if wp != 0 {
                // Let the activation finish before touching the disk.
                SetTimer(hwnd, TIMER_DISK, 150, None);
            } else {
                docs::on_focus_lost();
            }
            0
        }
        WM_SETFOCUS => {
            SetFocus(active_view().hwnd);
            0
        }
        WM_HOTKEY => {
            if wp as i32 == HOTKEY_SCRATCH {
                scratchpad::toggle();
            }
            0
        }
        WM_DROPFILES => {
            let drop = wp as HDROP;
            let n = DragQueryFileW(drop, u32::MAX, null_mut(), 0);
            let mut files = Vec::new();
            for i in 0..n {
                let len = DragQueryFileW(drop, i, null_mut(), 0);
                let mut buf = vec![0u16; len as usize + 1];
                DragQueryFileW(drop, i, buf.as_mut_ptr(), len + 1);
                files.push(PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])));
            }
            DragFinish(drop);
            for f in files {
                docs::open_path(&f, None);
            }
            0
        }
        WM_COPYDATA => {
            let cds = &*(lp as *const COPYDATASTRUCT);
            crate::ipc::on_copydata(cds);
            1
        }
        WM_SETCURSOR => {
            if (lp & 0xffff) as u32 == HTCLIENT {
                let mut pt: POINT = zeroed();
                GetCursorPos(&mut pt);
                ScreenToClient(hwnd, &mut pt);
                if compute_layout().splitter.is_some_and(|r| gfx::contains(&r, pt.x, pt.y)) {
                    SetCursor(LoadCursorW(null_mut(), IDC_SIZEWE));
                    return 1;
                }
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_LBUTTONDOWN => {
            let (x, y) = (util::lo(lp), util::hi(lp));
            if compute_layout().splitter.is_some_and(|r| gfx::contains(&r, x, y)) {
                a.splitter_drag.set(true);
                SetCapture(hwnd);
            }
            0
        }
        WM_MOUSEMOVE => {
            if a.splitter_drag.get() {
                let mut rc: RECT = zeroed();
                GetClientRect(hwnd, &mut rc);
                let x = util::lo(lp);
                a.st.borrow_mut().split_ratio = (x as f32 / rc.right.max(1) as f32).clamp(0.15, 0.85);
                layout();
            }
            0
        }
        WM_LBUTTONUP => {
            if a.splitter_drag.replace(false) {
                ReleaseCapture();
                a.manifest_dirty.set(true);
            }
            0
        }
        WM_SETTINGCHANGE => {
            if lp != 0 && util::from_wide(lp as *const u16) == "ImmersiveColorSet" && a.settings.borrow().theme == "system" {
                reload_theme();
            }
            0
        }
        WM_DPICHANGED => {
            *a.fonts.borrow_mut() = Fonts::new(((wp >> 16) & 0xffff) as i32, 10);
            let r = &*(lp as *const RECT);
            SetWindowPos(hwnd, null_mut(), r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            findbar::on_dpi();
            layout();
            0
        }
        tray::WM_TRAY => {
            tray::on_message(hwnd, lp);
            0
        }
        WM_QUERYENDSESSION => {
            docs::snapshot(true);
            docs::write_manifest_now();
            a.store.flush();
            1
        }
        WM_ENDSESSION => {
            if wp != 0 {
                a.quitting.set(true);
                docs::snapshot(true);
                docs::write_manifest_now();
                a.store.end();
                scratchpad::save_now();
                save_persist();
            }
            0
        }
        WM_CLOSE => {
            if a.settings.borrow().close_to_tray && a.settings.borrow().tray_icon && !a.quitting.get() {
                docs::snapshot(true);
                docs::write_manifest_now();
                ShowWindow(hwnd, SW_HIDE);
            } else {
                quit();
            }
            0
        }
        WM_DESTROY => {
            tray::remove(hwnd);
            UnregisterHotKey(hwnd, HOTKEY_SCRATCH);
            PostQuitMessage(0);
            0
        }
        m if m == tray::taskbar_created() => {
            if a.settings.borrow().tray_icon {
                tray::add(hwnd);
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

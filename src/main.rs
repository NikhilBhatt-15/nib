//! Nib: a fast, private, keyboard-first text editor for Windows.
//!
//! Native Win32 with Scintilla as the editing component. See README.md for
//! the feature list and docs/design.md for how the pieces fit together.

#![windows_subsystem = "windows"]

#[allow(dead_code, clippy::all)]
mod sci_consts {
    include!(concat!(env!("OUT_DIR"), "/sci_consts.rs"));
}

mod ai;
mod app;
mod chrome;
mod commands;
mod compare;
mod darkmode;
mod doc;
mod docs;
mod editor;
mod fileio;
mod findbar;
mod fuzzy;
mod gfx;
mod history;
mod http;
mod ipc;
mod keys;
mod languages;
mod notes;
mod palette;
mod paths;
mod preview;
mod scratchpad;
mod sci;
mod session;
mod settings;
mod shell;
mod snippets;
mod tabs;
mod textenc;
mod textops;
mod theme;
mod tray;
mod util;

/// Writes panics to crash.log in the data folder and tells the user. The
/// session on disk stays as of the last snapshot (at most a second old).
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let msg = format!("{info}\n\n{}", std::backtrace::Backtrace::force_capture());
        let dir = paths::Paths::resolve().data;
        let _ = std::fs::write(dir.join("crash.log"), &msg);
        let text = util::wide(&format!(
            "Nib hit an internal error and has to close.\n\nYour unsaved work up to the last second is kept and will be restored next time.\n\nDetails were written to {}",
            dir.join("crash.log").display()
        ));
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(std::ptr::null_mut(), text.as_ptr(), windows_sys::w!("Nib"), windows_sys::Win32::UI::WindowsAndMessaging::MB_OK | windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONERROR);
        }
    }));
}

/// Native faults (access violations in Win32 or Scintilla) bypass the panic
/// hook; log them too so crashes can be diagnosed from crash.log.
unsafe extern "system" fn on_exception(info: *mut windows_sys::Win32::System::Diagnostics::Debug::EXCEPTION_POINTERS) -> i32 {
    let rec = &*(*info).ExceptionRecord;
    let code = rec.ExceptionCode as u32;
    if code == 0xC000_0005 || code == 0xC000_0374 || code == 0xC000_0409 {
        let msg = format!("Native exception 0x{code:08X} at {:?}\n\n{}", rec.ExceptionAddress, std::backtrace::Backtrace::force_capture());
        let _ = std::fs::write(paths::Paths::resolve().data.join("crash.log"), msg);
    }
    0 // EXCEPTION_CONTINUE_SEARCH
}

fn main() {
    install_panic_hook();
    unsafe { windows_sys::Win32::System::Diagnostics::Debug::AddVectoredExceptionHandler(1, Some(on_exception)) };
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cwd = std::env::current_dir().unwrap_or_default();
    if ipc::try_forward(&args, &cwd) {
        return;
    }
    unsafe {
        windows_sys::Win32::System::Ole::OleInitialize(std::ptr::null_mut());
    }
    gfx::startup();
    sci::register_classes();
    app::run(app::parse_args(&args, &cwd));
}

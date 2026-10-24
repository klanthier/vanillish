//! **Vanillish for Windows** — the client iNilla runs on iOS and Vanillish on Android, as a
//! Windows PC game. This crate is the exe's entry point and nothing else.
//!
//! [`main`] picks Vanillish's folder (`%LOCALAPPDATA%\Vanillish`, or the exe's own folder for a
//! portable copy: `inilla_platform::paths::choose_root`), then it is the same client the phones
//! run: [`benilla_app::inilla_boot`] lays the folder out (the state folder, the game data, the
//! session log), and [`benilla_app::run`] builds the Bevy app, whose winit runner owns the main
//! thread until the game quits.
//!
//! A player build has no console window (`windows_subsystem = "windows"`): the session log is in
//! `benilla-config\Diagnostics`, and `Vanillish.exe --console` run from a terminal prints it there
//! too. The `dev` build keeps the console subsystem.

#![cfg_attr(all(windows, not(feature = "dev")), windows_subsystem = "windows")]

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    use benilla_app::BuildId;

    // Held to the end of `main`: the terminal gets its code page back when the game ends.
    let _console = std::env::args_os()
        .skip(1)
        .any(|a| a == "--console")
        .then(attach_console)
        .flatten();
    if let Some(root) = inilla_platform::paths::default_root() {
        inilla_platform::paths::set_root(root);
    }
    benilla_app::inilla_boot();
    let build = BuildId {
        version: env!("CARGO_PKG_VERSION"),
        describe: env!("BENILLA_GIT_DESCRIBE"),
        sha: env!("BENILLA_GIT_SHA"),
        short: env!("BENILLA_GIT_SHORT"),
        date: env!("BENILLA_GIT_DATE"),
        profile: env!("BENILLA_PROFILE"),
        // A PC carries no source folder: its state lives in Vanillish's own folder.
        project_dir: "",
        ..Default::default()
    };
    match benilla_app::run(build) {
        benilla_app::AppExit::Success => std::process::ExitCode::SUCCESS,
        benilla_app::AppExit::Error(code) => std::process::ExitCode::from(code.get()),
    }
}

/// `--console`: print into the terminal that started the exe (a GUI-subsystem exe has none of its
/// own). It runs before [`benilla_app::inilla_boot`], whose log capture then passes every line on
/// to this console.
///
/// A GUI exe started from a terminal has no standard handles; once attached they are pointed at
/// the console's own output, `CONOUT$`. A handle that is already valid is a redirect
/// (`Vanillish.exe --console > log.txt`) and stays. The console's code page becomes UTF-8, for
/// the bytes written to it as they are (Rust's own console writes are UTF-16); the console is the
/// terminal's, so the returned guard puts its old code page back.
#[cfg(windows)]
fn attach_console() -> Option<ConsoleCodePage> {
    use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Console::{
        AttachConsole, GetConsoleOutputCP, GetStdHandle, SetConsoleOutputCP, SetStdHandle,
        ATTACH_PARENT_PROCESS, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
    };

    let conout: Vec<u16> = "CONOUT$\0".encode_utf16().collect();
    // SAFETY: Win32 calls on this process's own standard handles; `conout` is a NUL-terminated
    // UTF-16 string that outlives the call reading it.
    unsafe {
        // Started from Explorer, or the parent has no console: nothing to attach to.
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            return None;
        }
        let mut console = INVALID_HANDLE_VALUE;
        for std in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            let handle = GetStdHandle(std);
            if !handle.is_null() && handle != INVALID_HANDLE_VALUE {
                continue;
            }
            if console == INVALID_HANDLE_VALUE {
                console = CreateFileW(
                    conout.as_ptr(),
                    GENERIC_READ | GENERIC_WRITE,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    std::ptr::null(),
                    OPEN_EXISTING,
                    0,
                    std::ptr::null_mut(),
                );
            }
            if console != INVALID_HANDLE_VALUE {
                SetStdHandle(std, console);
            }
        }
        let old = GetConsoleOutputCP();
        (old != 0 && SetConsoleOutputCP(65001) != 0).then_some(ConsoleCodePage(old))
    }
}

/// The attached console's code page before [`attach_console`], restored on drop.
#[cfg(windows)]
struct ConsoleCodePage(u32);

#[cfg(windows)]
impl Drop for ConsoleCodePage {
    fn drop(&mut self) {
        // SAFETY: a plain Win32 call on the console this process is attached to.
        unsafe { windows_sys::Win32::System::Console::SetConsoleOutputCP(self.0) };
    }
}

#[cfg(not(windows))]
fn main() -> std::process::ExitCode {
    eprintln!(
        "vanillish-windows builds Vanillish.exe, for Windows: \
         cargo build --target x86_64-pc-windows-msvc -p vanillish-windows \
         (on this OS, `cargo play` runs the client)"
    );
    std::process::ExitCode::FAILURE
}

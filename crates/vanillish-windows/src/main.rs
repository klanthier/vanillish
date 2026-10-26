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

    if std::env::args_os().skip(1).any(|a| a == "--console") {
        attach_console();
    }
    if let Some(root) = inilla_platform::paths::default_root() {
        inilla_platform::paths::set_root(root);
    }
    benilla_app::inilla_boot();
    let build = BuildId {
        // A release's own version (`vanillish-vX.Y.Z`, as the exe's version resource says it).
        version: match option_env!("VANILLISH_VERSION_NAME") {
            Some(v) => v,
            None => env!("CARGO_PKG_VERSION"),
        },
        describe: env!("BENILLA_GIT_DESCRIBE"),
        sha: env!("BENILLA_GIT_SHA"),
        short: env!("BENILLA_GIT_SHORT"),
        date: env!("BENILLA_GIT_DATE"),
        profile: env!("BENILLA_PROFILE"),
        // A PC carries no source folder: its state lives in Vanillish's own folder.
        project_dir: "",
        ..Default::default()
    };
    // A GUI exe that panics just vanishes: say so, and where the crash report went (benilla's
    // hook wrote it before the unwind reached here).
    match std::panic::catch_unwind(|| benilla_app::run(build)) {
        Ok(benilla_app::AppExit::Success) => std::process::ExitCode::SUCCESS,
        // An error exit (Bevy ends the app this way when its render systems panic, a lost GPU
        // device among them) would leave the player with a window that just vanished.
        Ok(benilla_app::AppExit::Error(code)) => {
            stopped(&format!("It ended with error code {}.", code.get()));
            std::process::ExitCode::from(code.get())
        }
        Err(payload) => {
            let why = payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_default();
            stopped(&why);
            std::process::ExitCode::FAILURE
        }
    }
}

/// The message box a player sees when the game stopped on a panic or an error exit.
#[cfg(windows)]
fn stopped(why: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

    // A scripted run (CI's smoke run, an unattended one) has nobody to click the box away.
    if std::env::var_os("VANILLISH_NO_DIALOG").is_some()
        || std::env::var_os("WOW_UNATTENDED").is_some()
    {
        return;
    }

    let first = why
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(300)
        .collect::<String>();
    let folder = inilla_platform::paths::config_dir()
        .map(|c| c.join("Diagnostics").display().to_string())
        .unwrap_or_else(|| "benilla-config\\Diagnostics".into());
    let text = format!(
        "Vanillish stopped unexpectedly.\n\n{first}\n\nA crash report and the session log are in \
         {folder}."
    );
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (text, title) = (wide(&text), wide("Vanillish"));
    // SAFETY: two NUL-terminated UTF-16 strings that outlive the call; no owner window.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        )
    };
}

/// `--console`: print into the terminal that started the exe (a GUI-subsystem exe has none of its
/// own). It runs before [`benilla_app::inilla_boot`], whose log capture then passes every line on
/// to this console, in UTF-16 (the console's code page is left as the terminal set it).
///
/// A GUI exe started from a terminal has no standard handles; once attached they are pointed at
/// the console's own output, `CONOUT$`. A handle that is already valid is a redirect
/// (`Vanillish.exe --console > log.txt`) and stays.
#[cfg(windows)]
fn attach_console() {
    use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Console::{
        AttachConsole, GetStdHandle, SetStdHandle, ATTACH_PARENT_PROCESS, STD_ERROR_HANDLE,
        STD_OUTPUT_HANDLE,
    };

    let conout: Vec<u16> = "CONOUT$\0".encode_utf16().collect();
    // SAFETY: Win32 calls on this process's own standard handles; `conout` is a NUL-terminated
    // UTF-16 string that outlives the call reading it.
    unsafe {
        // Started from Explorer, or the parent has no console: nothing to attach to.
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            return;
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

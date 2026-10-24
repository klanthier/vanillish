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
/// own).
#[cfg(windows)]
fn attach_console() {}

#[cfg(not(windows))]
fn main() -> std::process::ExitCode {
    eprintln!(
        "vanillish-windows builds Vanillish.exe, for Windows: \
         cargo build --target x86_64-pc-windows-msvc -p vanillish-windows \
         (on this OS, `cargo play` runs the client)"
    );
    std::process::ExitCode::FAILURE
}

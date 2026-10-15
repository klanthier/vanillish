//! **Vanillish** — benilla on Android, as iNilla is benilla on iOS. This crate is the app's entry
//! point and nothing else.
//!
//! GameActivity (`android/app`, `VanillishActivity`) loads `libvanillish.so` and android-activity
//! starts [`android_main`] on its own thread. From there it is the same client the desktop runs:
//! [`benilla_app::inilla_boot`] lays out the app's folders (the state folder and the game data in
//! the app's corner of shared storage, the session log, a `Data` folder copied in over USB
//! imported in place), then [`benilla_app::run`] builds the Bevy app, whose winit runner owns the
//! thread until the game quits.
//!
//! The Java side's `native` methods land in [`jni_exports`].

use benilla_app::BuildId;

#[cfg(target_os = "android")]
mod jni_exports;

/// The build stamp, as the iOS entry and the `benilla` shim pass it.
pub fn build_id() -> BuildId {
    BuildId {
        version: env!("CARGO_PKG_VERSION"),
        describe: env!("BENILLA_GIT_DESCRIBE"),
        sha: env!("BENILLA_GIT_SHA"),
        short: env!("BENILLA_GIT_SHORT"),
        date: env!("BENILLA_GIT_DATE"),
        profile: env!("BENILLA_PROFILE"),
        // The phone carries no source folder: its state lives in the app's storage.
        project_dir: "",
        ..Default::default()
    }
}

/// Boot and run the client; the process's exit code.
pub fn run() -> i32 {
    benilla_app::inilla_boot();
    match benilla_app::run(build_id()) {
        benilla_app::AppExit::Success => 0,
        benilla_app::AppExit::Error(code) => i32::from(code.get()),
    }
}

/// The Android entry point, on the thread android-activity starts for it.
///
/// A Bevy app cannot be built twice in one process (winit's event loop is once per process), and
/// Android keeps a finished app's process around to start it faster. So when the game quits, the
/// process ends with it — through the Java side (`Bridge.quit`: close the task, kill the process),
/// not `std::process::exit`, whose `atexit` handlers and C++ static destructors would run while
/// the UI thread may still be inside GameActivity. The Java side also relaunches into a fresh
/// process if Android ever creates the Activity again in this one (`VanillishActivity.onCreate`).
#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(app: bevy::android::android_activity::AndroidApp) {
    let _ = bevy::android::ANDROID_APP.set(app);
    let code = run();
    println!("vanillish: the game ended ({code}); closing");
    inilla_platform::device::quit();
    // The UI thread ends the process; if it cannot (no Activity any more), end it from here.
    std::thread::sleep(std::time::Duration::from_secs(3));
    // SAFETY: `_exit` ends the process without running atexit handlers or destructors.
    unsafe { libc_exit(code) };
}

#[cfg(target_os = "android")]
extern "C" {
    #[link_name = "_exit"]
    fn libc_exit(code: i32) -> !;
}

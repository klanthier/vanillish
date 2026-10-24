//! `IFileOpenDialog` for [`crate::picker::pick`]: the system's folder dialog, on a thread of its
//! own in a single-threaded COM apartment — the dialog's own — and never on the main thread,
//! where winit's `OleInitialize` already holds one. It is owned by the game's window, so it comes
//! up in front of a full-screen game and modal to it. The answer goes through the desktop's
//! delivery path, as the `$INILLA_PICK` rehearsal's does.

use std::sync::atomic::{AtomicIsize, Ordering};

use windows::core::{w, Error, PWSTR};
use windows::Win32::Foundation::{ERROR_CANCELLED, HWND};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
};
use windows::Win32::UI::Shell::{
    FileOpenDialog, IFileOpenDialog, FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST, FOS_PICKFOLDERS,
    SIGDN_FILESYSPATH,
};
use windows_sys::Win32::UI::WindowsAndMessaging::IsWindow;

use crate::picker::Picked;

/// The game's `HWND` ([`crate::picker::set_owner_window`]); 0 for none.
static OWNER: AtomicIsize = AtomicIsize::new(0);

pub(crate) fn set_owner_window(hwnd: isize) {
    OWNER.store(hwnd, Ordering::Relaxed);
}

/// Open the dialog on its own thread. `false` when the thread could not start.
pub(crate) fn pick() -> bool {
    let owner = OWNER.load(Ordering::Relaxed);
    std::thread::Builder::new()
        .name("inilla-picker".into())
        .spawn(move || match choose(owner) {
            Ok(Some(path)) => crate::picker::deliver_folder(path),
            Ok(None) => crate::picker::android::deliver(Picked::Cancelled),
            Err(e) => crate::picker::android::deliver(Picked::Failed(e)),
        })
        .is_ok()
}

/// The folder the player chose, `None` when they backed out.
fn choose(owner: isize) -> Result<Option<String>, String> {
    // SAFETY: COM for this thread alone, balanced below once it took.
    let init = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
    if init.is_err() {
        return Err(format!(
            "the folder dialog could not start: {}",
            Error::from(init)
        ));
    }
    // The dialog's objects are released inside, before COM goes.
    let chosen = dialog(owner).map_err(|e| format!("the folder dialog failed: {e}"));
    // SAFETY: balances the `CoInitializeEx` above, on the same thread.
    unsafe { CoUninitialize() };
    chosen
}

fn dialog(owner: isize) -> windows::core::Result<Option<String>> {
    // SAFETY: COM calls on this thread's apartment, each on the live dialog it created.
    unsafe {
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
        let options = dialog.GetOptions()?;
        dialog.SetOptions(options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST)?;
        dialog.SetTitle(w!("Choose your World of Warcraft 1.12.1 folder"))?;
        // A window that is gone (or none) owns nothing: the dialog opens on its own.
        let owner = (owner != 0 && IsWindow(owner as _) != 0).then_some(HWND(owner as _));
        match dialog.Show(owner) {
            Ok(()) => {}
            Err(e) if e.code() == ERROR_CANCELLED.to_hresult() => return Ok(None),
            Err(e) => return Err(e),
        }
        let name: PWSTR = dialog.GetResult()?.GetDisplayName(SIGDN_FILESYSPATH)?;
        let path = String::from_utf16_lossy(name.as_wide());
        CoTaskMemFree(Some(name.0 as _));
        Ok(Some(path))
    }
}

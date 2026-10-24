//! Vanillish: **the Windows host** — the Win32 and WinRT calls behind `inilla-platform`'s plain
//! functions, one module per concern. Everything here is `cfg(windows)`: the Linux gates see it
//! only through the cross check (`windows/toolchain/linux-env.sh`). No call here panics: a failed
//! one answers as the desktop's no-op does.

pub(crate) mod device;
pub(crate) mod fs;
pub(crate) mod keychain;
pub(crate) mod net;
pub(crate) mod pasteboard;
pub(crate) mod picker;
pub(crate) mod stdio;

/// `s` as a NUL-terminated UTF-16 string, what every `…W` call takes.
pub(crate) fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    s.as_ref().encode_wide().chain(Some(0)).collect()
}

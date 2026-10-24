//! `GetDiskFreeSpaceExW` for [`crate::fs::free_space`].

use std::path::Path;

use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

/// The caller's free bytes (`lpFreeBytesAvailableToCaller`: after disk quotas), on the volume of
/// `dir` or its nearest existing parent.
pub(crate) fn free_space(dir: &Path) -> Option<u64> {
    let dir = crate::fs::existing_folder(dir)?;
    let mut path = super::wide(dir);
    path.pop();
    // A share's root (`\\server\share\`) is only accepted with its trailing backslash; any
    // other folder takes one too.
    if !matches!(path.last(), Some(&c) if c == u16::from(b'\\') || c == u16::from(b'/')) {
        path.push(u16::from(b'\\'));
    }
    path.push(0);
    let mut free = 0u64;
    // SAFETY: `path` is NUL-terminated and outlives the call; the two totals are optional.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            path.as_ptr(),
            &mut free,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then_some(free)
}

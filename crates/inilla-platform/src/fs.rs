//! Vanillish: **the file system's own questions**, where `std` has no answer.

use std::path::Path;

/// The bytes an unprivileged writer may still put on the volume that holds `dir` (or, when `dir`
/// does not exist yet, its nearest existing parent). `None` where the platform does not say.
pub fn free_space(dir: &Path) -> Option<u64> {
    #[cfg(windows)]
    {
        crate::windows::fs::free_space(dir)
    }
    #[cfg(not(windows))]
    {
        let _ = dir;
        None
    }
}

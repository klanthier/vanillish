//! `GetDiskFreeSpaceExW` for [`crate::fs::free_space`].

use std::path::Path;

pub(crate) fn free_space(dir: &Path) -> Option<u64> {
    let _ = dir;
    None
}

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

/// `dir`, or its nearest parent that is a folder: a download's target may not exist yet, but the
/// volume it will land on does.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn existing_folder(dir: &Path) -> Option<&Path> {
    dir.ancestors().find(|p| p.is_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_folder_asks_its_nearest_existing_parent() {
        let tmp = std::env::temp_dir();
        assert_eq!(existing_folder(&tmp), Some(tmp.as_path()));
        let deep = tmp
            .join(format!("inilla-fs-{}", std::process::id()))
            .join("Data");
        assert_eq!(existing_folder(&deep), Some(tmp.as_path()));
        assert_eq!(existing_folder(Path::new("no-such-relative-folder")), None);
    }
}

//! Vanillish: **files in and out of the app through the system** — a folder the player picks
//! outside the app, read file by file, and a file handed to another app (the share sheet).
//!
//! - **Android**: the Storage Access Framework. [`pick`] opens the system's folder picker
//!   (`ACTION_OPEN_DOCUMENT_TREE`, starting in Download); the Java side (`FolderImport.java`)
//!   takes a persistable grant on the folder and delivers it through `libvanillish`'s JNI shims
//!   into [`android::deliver`]. [`list`] and [`open`] then run on the import's own thread: the
//!   folder's archives as JSON, and one archive as a detached file descriptor this side owns.
//!   The grant outlives the process, so a copy cut off by Pause, a crash or Android killing the
//!   app in the background resumes ([`pending`]) until [`forget`] gives it back.
//! - **Desktop**: `$INILLA_PICK` names a real folder, which [`pick`] "picks" at once — the whole
//!   import, from the button to the progress bar, rehearses on Linux.
//! - **iOS**: not available (yet); a folder `UIDocumentPickerViewController` would fit here.
//!
//! The copy itself is `inilla_core::import` (resumable, checked, tested without a device).

use std::sync::Mutex;

pub use inilla_core::import::{Listed, Listing};

/// What the picker answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Picked {
    /// A folder: `tree` is what [`list`] and [`open`] take (a `content://` tree URI on Android, a
    /// path on the desktop), `label` its name for the player.
    Folder { tree: String, label: String },
    /// The player backed out.
    Cancelled,
    /// The picker could not open (no app handles it, the Activity was gone).
    Failed(String),
}

#[derive(Default)]
struct State {
    waiting: bool,
    delivered: Option<Picked>,
    /// Desktop: the picked folder until [`forget`] (Android keeps it in the app's preferences).
    pending: Option<(String, String)>,
}

static STATE: Mutex<State> = Mutex::new(State {
    waiting: false,
    delivered: None,
    pending: None,
});

/// The store, whatever happened to a thread that held it: a JNI shim must never panic.
fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// The folder `$INILLA_PICK` names (the desktop rehearsal).
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn rehearsal() -> Option<String> {
    std::env::var("INILLA_PICK").ok().filter(|v| !v.is_empty())
}

/// Whether this platform can pick a folder: Android yes; the desktop when `$INILLA_PICK` is set.
pub fn available() -> bool {
    #[cfg(target_os = "android")]
    {
        true
    }
    #[cfg(target_os = "ios")]
    {
        false
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        rehearsal().is_some()
    }
}

/// Open the folder picker. `false` when it could not be asked for; otherwise the answer arrives
/// through [`poll`] (after the player is done, which may be after a restart of the app).
pub fn pick() -> bool {
    #[cfg(target_os = "android")]
    {
        state().waiting = true;
        let asked = crate::android::picker::pick();
        if !asked {
            state().waiting = false;
        }
        asked
    }
    #[cfg(target_os = "ios")]
    {
        false
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        let Some(path) = rehearsal() else {
            return false;
        };
        let label = std::path::Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());
        state().pending = Some((path.clone(), label.clone()));
        android::deliver(Picked::Folder { tree: path, label });
        true
    }
}

/// The picker is open: [`pick`] was answered with nothing yet.
pub fn waiting() -> bool {
    state().waiting
}

/// The picker's answer, once (no JNI: the Java side delivered it into a Mutex).
pub fn poll() -> Option<Picked> {
    state().delivered.take()
}

/// A picked folder whose copy never finished, `(tree, label)` — still readable, since the grant
/// was persisted. Asks the Java side on Android (a JNI call: not every frame).
pub fn pending() -> Option<(String, String)> {
    #[cfg(target_os = "android")]
    {
        crate::android::picker::pending()
            .as_deref()
            .and_then(inilla_core::import::parse_pending)
    }
    #[cfg(not(target_os = "android"))]
    {
        state().pending.clone()
    }
}

/// The archives in the picked folder `tree`, down to `max_depth` folders. Slow on a big folder or
/// a USB stick: call it from a worker thread.
pub fn list(tree: &str, max_depth: u32) -> Result<Listing, String> {
    #[cfg(target_os = "android")]
    {
        let json = crate::android::picker::list(tree, max_depth)?;
        Listing::parse(&json)
    }
    #[cfg(target_os = "ios")]
    {
        let _ = (tree, max_depth);
        Err("there is no folder picker on this platform".into())
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        let root = std::path::Path::new(tree);
        if !root.is_dir() {
            return Err(format!("{tree} is not a folder"));
        }
        Ok(inilla_core::import::list_folder(root, max_depth))
    }
}

/// One listed file of `tree`, by its [`Listed::id`], opened for reading. On Android a detached
/// file descriptor from the provider, now owned (and closed) by the returned `File`.
pub fn open(tree: &str, id: &str) -> Result<std::fs::File, String> {
    #[cfg(target_os = "android")]
    {
        crate::android::picker::open(tree, id)
    }
    #[cfg(target_os = "ios")]
    {
        let _ = (tree, id);
        Err("there is no folder picker on this platform".into())
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        // Ids are paths relative to the picked folder; nothing outside it is opened.
        let rel = std::path::Path::new(id);
        if rel.is_absolute()
            || rel
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            return Err(format!("refusing {id:?}: not inside the picked folder"));
        }
        let path = std::path::Path::new(tree).join(rel);
        std::fs::File::open(&path).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// Done with `tree`: give the grant back and stop offering to resume it.
pub fn forget(tree: &str) {
    #[cfg(target_os = "android")]
    crate::android::picker::forget(tree);
    let mut s = state();
    if s.pending.as_ref().is_some_and(|(t, _)| t == tree) {
        s.pending = None;
    }
}

/// Whether [`share_file`] can hand a file to another app here (Android's share sheet).
pub fn can_share() -> bool {
    cfg!(target_os = "android")
}

/// Offer the file at `path` to another app (mail, chat, a cloud drive) through the system's
/// share sheet; `mime` is its type (`text/plain` for a log). `false` when it could not.
pub fn share_file(path: &std::path::Path, mime: &str) -> bool {
    #[cfg(target_os = "android")]
    {
        crate::android::picker::share_file(path, mime)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (path, mime);
        false
    }
}

/// The delivery end for the Java side (`libvanillish`'s JNI shims call it on Android's UI thread).
/// Plain Rust: it never panics, and the desktop rehearsal delivers through it too.
pub mod android {
    use super::{state, Picked};

    /// The picker answered: keep the answer for [`super::poll`].
    pub fn deliver(p: Picked) {
        let mut s = state();
        s.waiting = false;
        s.delivered = Some(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_is_delivered_once() {
        state().waiting = true;
        assert!(waiting());
        android::deliver(Picked::Folder {
            tree: "content://t".into(),
            label: "WoW".into(),
        });
        assert!(!waiting());
        assert_eq!(
            poll(),
            Some(Picked::Folder {
                tree: "content://t".into(),
                label: "WoW".into(),
            })
        );
        assert_eq!(poll(), None);
        android::deliver(Picked::Cancelled);
        assert_eq!(poll(), Some(Picked::Cancelled));
    }

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    #[test]
    fn the_desktop_opens_only_inside_the_folder() {
        let root = std::env::temp_dir().join(format!("inilla-pick-{}", std::process::id()));
        std::fs::create_dir_all(root.join("Data")).unwrap();
        std::fs::write(root.join("Data/dbc.MPQ"), b"x").unwrap();
        let tree = root.display().to_string();
        assert!(open(&tree, "Data/dbc.MPQ").is_ok());
        assert!(open(&tree, "../etc/passwd").is_err());
        assert!(open(&tree, "/etc/passwd").is_err());
        let listing = list(&tree, 3).unwrap();
        assert_eq!(listing.files.len(), 1);
        assert_eq!(listing.files[0].id, "Data/dbc.MPQ");
        assert!(list(&root.join("missing").display().to_string(), 3).is_err());
    }
}

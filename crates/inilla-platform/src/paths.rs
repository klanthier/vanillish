//! Where things live inside the iOS app container.
//!
//! - `Documents/` is what the Files app and Finder/iTunes file sharing show the player
//!   (`UIFileSharingEnabled`). benilla's state folder (`benilla-config/`: settings, key bindings,
//!   macros, saved variables, **AddOns**, screenshots, crash reports and session logs) goes here,
//!   so every one of those is reachable without a Mac — and it is also the drop box a player
//!   copies their `Data` folder into to import it.
//! - `Library/Application Support/iNilla/Data/` holds the game data once imported or downloaded:
//!   out of the player's way (a 5 GB folder in Files is one accidental delete from a re-download)
//!   and excluded from iCloud backup.
//!
//! Resolved from `$HOME`, which iOS points at the container root for the app (and LiveContainer
//! points at the guest app's container). Off iOS every function answers `None` unless
//! `$INILLA_CONTAINER` names a folder to play the container: the desktop build otherwise keeps
//! benilla's own resolution (`$BENILLA_HOME`, `$WOW_DATA`, beside the binary).
//!
//! **Vanillish (Android).** The app's corner of shared storage plays `Documents/`:
//! `Android/data/<package>/files/` (`getExternalFilesDir`), which a PC sees over USB (File
//! transfer). benilla's state folder lives there, and it is the drop box for a copied `Data`
//! folder. The game data sits beside it in `.vanillish/Data` — the same mount, so the import is a
//! rename (instant) and not a 5 GB copy, while the dot keeps it out of the way. Caches go to the
//! private cache dir. Shared storage is asked for again for a few seconds when it does not answer
//! at launch; if it still does not, [`storage_unavailable`] says so, and the app asks the player
//! to open it again rather than set up (or download 5 GB) in a private stand-in that the next
//! launch would not use. The stand-in (`files/Documents`) only keeps the app running meanwhile.

use std::path::PathBuf;

/// Vanillish: the Android roots, `(documents, app_support, caches)`. Without shared storage, a
/// private stand-in, flagged by [`storage_unavailable`].
#[cfg(target_os = "android")]
fn android_roots() -> Option<(PathBuf, PathBuf, PathBuf)> {
    let dirs = crate::android::paths::dirs();
    let docs = dirs
        .external
        .clone()
        .or_else(|| dirs.internal.as_ref().map(|d| d.join("Documents")))?;
    let support = docs.join(".vanillish");
    let caches = dirs
        .cache
        .as_ref()
        .map(|c| c.join("vanillish"))
        .unwrap_or_else(|| support.join("Caches"));
    Some((docs, support, caches))
}

#[cfg_attr(target_os = "android", allow(dead_code))]
fn home() -> Option<PathBuf> {
    if cfg!(target_os = "ios") {
        return std::env::var_os("HOME").map(PathBuf::from);
    }
    // Off iOS, `$INILLA_CONTAINER` stands in for the app container, so the desktop build can
    // rehearse the iOS layout (the Documents import, the data download) against a scratch folder.
    std::env::var_os("INILLA_CONTAINER")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// Vanillish: Android's shared storage did not answer at launch (asked again for a few seconds),
/// so the folders are a private stand-in this time: the settings, the saved variables and the game
/// data are not where they were, and nothing written there would be found next time. The setup
/// screen says so and starts no download or import. Always false elsewhere.
pub fn storage_unavailable() -> bool {
    #[cfg(target_os = "android")]
    {
        let dirs = crate::android::paths::dirs();
        dirs.external.is_none() && dirs.internal.is_some()
    }
    #[cfg(not(target_os = "android"))]
    {
        false
    }
}

/// Whether paths resolve into an (emulated or real) app container.
pub fn in_container() -> bool {
    #[cfg(target_os = "android")]
    {
        android_roots().is_some()
    }
    #[cfg(not(target_os = "android"))]
    {
        home().is_some()
    }
}

/// `Documents/` — visible in the Files app (Android: over USB from a PC).
pub fn documents() -> Option<PathBuf> {
    #[cfg(target_os = "android")]
    {
        android_roots().map(|r| r.0)
    }
    #[cfg(not(target_os = "android"))]
    {
        home().map(|h| h.join("Documents"))
    }
}

/// `Library/Application Support/iNilla/` (Android: `<documents>/.vanillish/`).
pub fn app_support() -> Option<PathBuf> {
    #[cfg(target_os = "android")]
    {
        android_roots().map(|r| r.1)
    }
    #[cfg(not(target_os = "android"))]
    {
        home().map(|h| h.join("Library").join("Application Support").join("iNilla"))
    }
}

/// `Library/Caches/iNilla/` — may be purged by the OS under storage pressure.
pub fn caches() -> Option<PathBuf> {
    #[cfg(target_os = "android")]
    {
        android_roots().map(|r| r.2)
    }
    #[cfg(not(target_os = "android"))]
    {
        home().map(|h| h.join("Library").join("Caches").join("iNilla"))
    }
}

/// The game data folder (`WOW_DATA`).
pub fn data_dir() -> Option<PathBuf> {
    app_support().map(|d| d.join("Data"))
}

/// benilla's state folder (`BENILLA_HOME`).
pub fn config_dir() -> Option<PathBuf> {
    documents().map(|d| d.join("benilla-config"))
}

/// What runs iNilla: installed as its own app, or as a guest inside LiveContainer, which gives
/// each guest app a home folder inside its own container and its own `Info.plist`, entitlements
/// and Files-app entry in place of the guest's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Host {
    App,
    LiveContainer,
    Desktop,
    /// Vanillish: the Android app.
    Android,
}

pub fn host() -> Host {
    if cfg!(target_os = "android") {
        return Host::Android;
    }
    if !cfg!(target_os = "ios") {
        return Host::Desktop;
    }
    let home = std::env::var("HOME").unwrap_or_default();
    classify_home(&home, std::env::var_os("LC_HOME_PATH").is_some())
}

/// LiveContainer points a guest's `$HOME` at `<its Documents>/Data/Application/<folder>` (or
/// the same under `LiveContainer/` in its app group) and keeps its own home in `$LC_HOME_PATH`;
/// an installed app's home is `…/Containers/Data/Application/<UUID>`, never under a Documents.
pub fn classify_home(home: &str, lc_env: bool) -> Host {
    if lc_env
        || home.contains("/Documents/Data/Application/")
        || home.contains("/LiveContainer/Data/Application/")
    {
        Host::LiveContainer
    } else {
        Host::App
    }
}

/// Where the Files app shows `Documents/`, as the taps to get there.
pub fn files_location() -> String {
    if host() == Host::Android {
        let docs = documents()
            .map(|d| d.display().to_string())
            .unwrap_or_default();
        return files_location_for(Host::Android, &docs);
    }
    let home = std::env::var("HOME").unwrap_or_default();
    files_location_for(host(), &home)
}

/// Vanillish: the package name out of an Android app-specific path
/// (`…/Android/data/<package>/files`).
pub fn android_package(path: &str) -> Option<&str> {
    let rest = path.split("/Android/data/").nth(1)?;
    rest.split('/').next().filter(|p| !p.is_empty())
}

pub fn files_location_for(host: Host, home: &str) -> String {
    match host {
        Host::LiveContainer if home.contains("/Documents/Data/Application/") => {
            let folder = home.trim_end_matches('/').rsplit('/').next().unwrap_or("");
            format!("On My iPhone → LiveContainer → Data → Application → {folder} → Documents")
        }
        // A shared (app group) guest's folder is not one the Files app lists.
        Host::LiveContainer => {
            "not shown in the Files app for a shared LiveContainer app — download the data \
             instead, or make iNilla a private app in LiveContainer"
                .to_string()
        }
        Host::App => "On My iPhone → iNilla".to_string(),
        // `home` is the documents folder here.
        Host::Android => match android_package(home) {
            Some(pkg) => format!(
                "from a PC over USB (File transfer): Internal storage → Android → data → {pkg} → files"
            ),
            None => home.to_string(),
        },
        Host::Desktop => match documents() {
            Some(d) => d.display().to_string(),
            None => "benilla-config beside the binary".to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_android_drop_box_is_named_by_its_package() {
        let docs = "/storage/emulated/0/Android/data/io.github.klanthier.vanillish/files";
        assert_eq!(android_package(docs), Some("io.github.klanthier.vanillish"));
        assert_eq!(
            files_location_for(Host::Android, docs),
            "from a PC over USB (File transfer): Internal storage → Android → data → \
             io.github.klanthier.vanillish → files"
        );
        assert_eq!(android_package("/data/user/0/x/files"), None);
    }

    #[test]
    fn an_installed_app_is_not_a_guest() {
        let home =
            "/private/var/mobile/Containers/Data/Application/0B5D6C1E-7A0F-4B9E-9C84-1F2E3D4C5B6A";
        assert_eq!(classify_home(home, false), Host::App);
        assert_eq!(files_location_for(Host::App, home), "On My iPhone → iNilla");
    }

    #[test]
    fn a_livecontainer_guest_is_found_by_its_home() {
        let home = "/private/var/mobile/Containers/Data/Application/0B5D6C1E/Documents/Data/Application/iNilla-2F1A";
        assert_eq!(classify_home(home, false), Host::LiveContainer);
        assert_eq!(
            files_location_for(Host::LiveContainer, home),
            "On My iPhone → LiveContainer → Data → Application → iNilla-2F1A → Documents"
        );
        let shared = "/private/var/mobile/Containers/Shared/AppGroup/9C1D/LiveContainer/Data/Application/77AA";
        assert_eq!(classify_home(shared, false), Host::LiveContainer);
        assert!(files_location_for(Host::LiveContainer, shared).contains("download"));
        assert_eq!(classify_home("/anything", true), Host::LiveContainer);
    }
}

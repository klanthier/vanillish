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
//!
//! **Vanillish (Windows).** The Windows entry ([`set_root`], before the boot) names one folder
//! that plays the whole container: `%LOCALAPPDATA%\Vanillish`, or the exe's own folder when a
//! `portable.txt` (or an earlier `benilla-config`) sits beside it ([`choose_root`]). Inside it,
//! one level each: `benilla-config\` (the state folder, as on every platform), `Data\` (the game
//! data the app downloads or copies in) and `Cache\`. A player who already has 1.12.1 on this PC
//! points Vanillish at it instead ([`set_wow_folder`]): the game data is then that folder, read in
//! place and never written ([`data_in_place`]). Without a root (benilla's own `cargo play` on
//! Windows) nothing here changes.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Vanillish (Windows): the folder that plays the app container, when the entry set one.
static ROOT: OnceLock<PathBuf> = OnceLock::new();

/// Vanillish (Windows): play the app container in `root` from now on. Call once, before the boot
/// (`benilla_app::inilla_boot`); a second call is ignored.
pub fn set_root(root: PathBuf) {
    let _ = ROOT.set(root);
}

/// Vanillish (Windows): the root [`set_root`] named, if any.
pub fn root() -> Option<&'static Path> {
    ROOT.get().map(PathBuf::as_path)
}

/// Vanillish (Windows): the root for an exe in `exe_dir`. Portable — the exe's own folder — when
/// the player asked for it with a `portable.txt` beside the exe or a portable run already left
/// its `benilla-config` there, but never inside a WoW install (the install is read-only:
/// benilla's state is never written into it); otherwise `<local app data>\Vanillish`, and the
/// exe's folder only when Windows names no local app data.
pub fn choose_root(exe_dir: Option<&Path>, local_app_data: Option<&Path>) -> Option<PathBuf> {
    let portable = exe_dir.filter(|d| {
        (d.join("portable.txt").is_file() || d.join("benilla-config").is_dir())
            && !is_wow_install(d)
    });
    if let Some(d) = portable {
        return Some(d.to_path_buf());
    }
    local_app_data.map(|l| l.join("Vanillish")).or_else(|| {
        exe_dir
            .filter(|d| !is_wow_install(d))
            .map(Path::to_path_buf)
    })
}

/// Vanillish (Windows): [`choose_root`] for this process — the exe's folder and
/// `%LOCALAPPDATA%`.
pub fn default_root() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok();
    let exe_dir = exe.as_deref().and_then(Path::parent);
    let local = std::env::var_os("LOCALAPPDATA")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    choose_root(exe_dir, local.as_deref())
}

/// A 1.12.1 client's own folder: its `WoW.exe` (any case) beside a `Data` folder.
fn is_wow_install(dir: &Path) -> bool {
    let has_exe = std::fs::read_dir(dir).is_ok_and(|rd| {
        rd.flatten().any(|e| {
            e.file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case("wow.exe")
        })
    });
    has_exe && dir.join("Data").is_dir()
}

/// Vanillish (Windows): the `Data` folder of the 1.12.1 client the player pointed at, which may
/// be the client's folder (with `WoW.exe` and `Data\` in it) or its `Data` folder itself.
pub fn wow_data_in(picked: &Path) -> PathBuf {
    let data = picked.join("Data");
    if data.is_dir() {
        data
    } else {
        picked.to_path_buf()
    }
}

/// Where the WoW-folder setting is kept: `benilla-config\inilla\install.toml`.
fn install_file() -> Option<PathBuf> {
    root().map(|r| r.join("benilla-config").join("inilla").join("install.toml"))
}

/// Vanillish (Windows): the `Data` folder of the player's own 1.12.1 client, read in place, when
/// they pointed Vanillish at one ([`set_wow_folder`]); `None` without a root.
pub fn wow_folder() -> Option<PathBuf> {
    #[cfg(not(target_os = "ios"))]
    {
        let text = std::fs::read_to_string(install_file()?).ok()?;
        let table: toml::Table = toml::from_str(&text).ok()?;
        table
            .get("wow_data")
            .and_then(toml::Value::as_str)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
    }
    #[cfg(target_os = "ios")]
    {
        None
    }
}

/// Vanillish (Windows): read the game data from the player's own client in `data` (its `Data`
/// folder, see [`wow_data_in`]) from the next launch on, or, with `None`, from Vanillish's own
/// `Data` folder again. The archives are opened at launch, so this takes a restart.
pub fn set_wow_folder(data: Option<&Path>) -> std::io::Result<()> {
    let Some(file) = install_file() else {
        return Err(std::io::Error::other(
            "no Vanillish folder on this platform",
        ));
    };
    match data {
        None => match std::fs::remove_file(&file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        },
        #[cfg(not(target_os = "ios"))]
        Some(dir) => {
            if let Some(parent) = file.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut table = toml::Table::new();
            table.insert(
                "wow_data".into(),
                toml::Value::String(dir.to_string_lossy().into_owned()),
            );
            let text = format!(
                "# Vanillish reads the game data from this folder, in place, and never writes \
                 into it.\n{}",
                toml::to_string(&table).map_err(std::io::Error::other)?
            );
            let tmp = file.with_extension("toml.tmp");
            std::fs::write(&tmp, text)?;
            std::fs::rename(&tmp, &file)
        }
        #[cfg(target_os = "ios")]
        Some(_) => Err(std::io::Error::other(
            "no Vanillish folder on this platform",
        )),
    }
}

/// Vanillish (Windows): the game data is the player's own client, read in place: nothing may be
/// downloaded, copied or moved into [`data_dir`].
pub fn data_in_place() -> bool {
    root().is_some() && wow_folder().is_some()
}

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
        root().is_some() || home().is_some()
    }
}

/// `Documents/` — visible in the Files app (Android: over USB from a PC; Windows: the root).
pub fn documents() -> Option<PathBuf> {
    #[cfg(target_os = "android")]
    {
        android_roots().map(|r| r.0)
    }
    #[cfg(not(target_os = "android"))]
    {
        root()
            .map(Path::to_path_buf)
            .or_else(|| home().map(|h| h.join("Documents")))
    }
}

/// `Library/Application Support/iNilla/` (Android: `<documents>/.vanillish/`; Windows: the root).
pub fn app_support() -> Option<PathBuf> {
    #[cfg(target_os = "android")]
    {
        android_roots().map(|r| r.1)
    }
    #[cfg(not(target_os = "android"))]
    {
        root().map(Path::to_path_buf).or_else(|| {
            home().map(|h| h.join("Library").join("Application Support").join("iNilla"))
        })
    }
}

/// `Library/Caches/iNilla/` — may be purged by the OS under storage pressure (Windows:
/// `<root>\Cache`).
pub fn caches() -> Option<PathBuf> {
    #[cfg(target_os = "android")]
    {
        android_roots().map(|r| r.2)
    }
    #[cfg(not(target_os = "android"))]
    {
        root()
            .map(|r| r.join("Cache"))
            .or_else(|| home().map(|h| h.join("Library").join("Caches").join("iNilla")))
    }
}

/// The game data folder (`WOW_DATA`). Vanillish (Windows): the player's own client's, when they
/// pointed Vanillish at one ([`wow_folder`]).
pub fn data_dir() -> Option<PathBuf> {
    if root().is_some() {
        if let Some(own) = wow_folder() {
            return Some(own);
        }
    }
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
    /// Vanillish: the Windows app, in its own folder ([`set_root`]).
    Windows,
}

pub fn host() -> Host {
    if cfg!(target_os = "android") {
        return Host::Android;
    }
    if root().is_some() {
        return Host::Windows;
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
        // The root, as File Explorer's address bar shows it.
        Host::Windows => documents()
            .map(|d| d.display().to_string())
            .unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh scratch folder under the system's temp dir.
    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("inilla-paths-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_windows_root_is_local_app_data_unless_portable() {
        let exe = scratch("exe");
        let local = scratch("local");
        assert_eq!(
            choose_root(Some(&exe), Some(&local)),
            Some(local.join("Vanillish"))
        );
        // No local app data: the exe's own folder.
        assert_eq!(choose_root(Some(&exe), None), Some(exe.clone()));
        assert_eq!(choose_root(None, None), None);
        // `portable.txt`, or a portable run's state folder, keeps everything beside the exe.
        std::fs::write(exe.join("portable.txt"), "").unwrap();
        assert_eq!(choose_root(Some(&exe), Some(&local)), Some(exe.clone()));
        std::fs::remove_file(exe.join("portable.txt")).unwrap();
        std::fs::create_dir(exe.join("benilla-config")).unwrap();
        assert_eq!(choose_root(Some(&exe), Some(&local)), Some(exe.clone()));
        let _ = std::fs::remove_dir_all(&exe);
        let _ = std::fs::remove_dir_all(&local);
    }

    #[test]
    fn a_wow_install_is_never_the_root() {
        let wow = scratch("wow");
        std::fs::write(wow.join("WoW.exe"), "").unwrap();
        std::fs::create_dir(wow.join("Data")).unwrap();
        std::fs::write(wow.join("portable.txt"), "").unwrap();
        let local = scratch("local2");
        assert_eq!(
            choose_root(Some(&wow), Some(&local)),
            Some(local.join("Vanillish"))
        );
        assert_eq!(choose_root(Some(&wow), None), None);
        let _ = std::fs::remove_dir_all(&wow);
        let _ = std::fs::remove_dir_all(&local);
    }

    #[test]
    fn a_picked_client_folder_means_its_data() {
        let wow = scratch("picked");
        assert_eq!(wow_data_in(&wow), wow);
        std::fs::create_dir(wow.join("Data")).unwrap();
        assert_eq!(wow_data_in(&wow), wow.join("Data"));
        assert_eq!(wow_data_in(&wow.join("Data")), wow.join("Data"));
        let _ = std::fs::remove_dir_all(&wow);
    }

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

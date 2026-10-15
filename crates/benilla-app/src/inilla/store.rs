//! iNilla's files: four TOML documents in `benilla-config/inilla/`, beside benilla's own state
//! (so on iOS they sit in Documents, visible and editable in the Files app, and survive updates
//! and SideStore re-signs with everything else).
//!
//! | file | holds |
//! |---|---|
//! | `settings.toml` | [`InillaSettings`] — quality, UI scale, audio session, debug switches |
//! | `servers.toml` | [`Launcher`] — server profiles and accounts (never passwords) |
//! | `touch.toml` | [`TouchLayout`] — the HUD as the player arranged it |
//! | `touch-tablet.toml` | Vanillish: the HUD on a foldable's inner screen ([`ScreenClass`]) |
//! | `touch-tabletop.toml` | Vanillish: the HUD below the hinge, the inner screen half folded |
//! | `controller.toml` | [`PadProfile`] — the controller mapping |
//!
//! A file that does not parse is kept aside as `<name>.broken` and the defaults are used, so a
//! hand edit gone wrong costs the edit, not the app.

use std::path::PathBuf;

use serde::de::DeserializeOwned;
use serde::Serialize;

use inilla_core::fold::ScreenClass;
use inilla_core::pad::PadProfile;
use inilla_core::profiles::Launcher;
use inilla_core::settings::InillaSettings;
use inilla_core::touch::TouchLayout;

pub(crate) fn dir() -> Option<PathBuf> {
    crate::local_state::home().map(|h| h.join("inilla"))
}

fn load<T: DeserializeOwned + Default>(name: &str) -> T {
    load_or(name, T::default)
}

/// `name`, or `fallback()` when it is missing or broken.
fn load_or<T: DeserializeOwned>(name: &str, fallback: impl FnOnce() -> T) -> T {
    let Some(path) = dir().map(|d| d.join(name)) else {
        return fallback();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return fallback();
    };
    match toml::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            bevy::log::warn!(
                "inilla: {} does not parse ({e}); using defaults",
                path.display()
            );
            let _ = std::fs::rename(&path, path.with_extension("toml.broken"));
            fallback()
        }
    }
}

fn save<T: Serialize>(name: &str, value: &T) {
    let Some(dir) = dir() else {
        return;
    };
    let text = match toml::to_string_pretty(value) {
        Ok(t) => t,
        Err(e) => {
            bevy::log::warn!("inilla: could not serialize {name}: {e}");
            return;
        }
    };
    if let Err(e) = crate::local_state::write_atomic(&dir.join(name), &text) {
        bevy::log::warn!("inilla: could not save {name}: {e}");
    }
}

pub(crate) fn load_settings() -> InillaSettings {
    load("settings.toml")
}

pub(crate) fn save_settings(v: &InillaSettings) {
    save("settings.toml", v);
}

pub(crate) fn load_launcher() -> Launcher {
    load("servers.toml")
}

pub(crate) fn save_launcher(v: &Launcher) {
    save("servers.toml", v);
}

/// The phone's layout (`touch.toml`): the one in use at launch, until the screen says otherwise.
pub(crate) fn load_layout() -> TouchLayout {
    load_layout_for(ScreenClass::Phone)
}

/// Vanillish: the layout of a class of screen, from its own file — or that class's shipped
/// layout when there is none yet, never the phone's.
pub(crate) fn load_layout_for(class: ScreenClass) -> TouchLayout {
    let mut l: TouchLayout = load_or(class.file_name(), || class.defaults());
    l.class = class;
    l.upgrade();
    l
}

/// The layout into its own class's file (the phone's is `touch.toml`, as in iNilla).
pub(crate) fn save_layout(v: &TouchLayout) {
    save(v.class.file_name(), v);
}

/// Vanillish: a class's layout file gone, so the next look at that screen reads its shipped
/// layout ("Reset all"). Nothing to do when there is none.
pub(crate) fn remove_layout(class: ScreenClass) {
    let Some(path) = dir().map(|d| d.join(class.file_name())) else {
        return;
    };
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => bevy::log::warn!("inilla: could not remove {}: {e}", path.display()),
    }
}

pub(crate) fn load_pad() -> PadProfile {
    load("controller.toml")
}

pub(crate) fn save_pad(v: &PadProfile) {
    save("controller.toml", v);
}

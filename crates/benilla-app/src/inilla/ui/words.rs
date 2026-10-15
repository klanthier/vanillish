//! **Every sentence that names the platform or the app**, in one place: what iNilla's screens say
//! on iOS (and the desktop), verbatim, and what Vanillish says on Android. A screen asks here
//! rather than spelling "iPhone", "Keychain" or "Finder" itself, so the Android build differs from
//! iOS in this file's text alone.
//!
//! Only glyphs the font test (`super::tests`) has proven: the arrow, the bullet, the middle dot,
//! the em dash and the ellipsis.

use inilla_core::settings::Quality;
use inilla_platform::paths::{self, Host};

/// Vanillish: built for Android — the sentences below pick their Android form.
pub(crate) const ANDROID: bool = cfg!(target_os = "android");

#[cfg(target_os = "android")]
macro_rules! app {
    () => {
        "Vanillish"
    };
}
#[cfg(not(target_os = "android"))]
macro_rules! app {
    () => {
        "iNilla"
    };
}

/// The app's name: the screens' heading, the HUD button, the console page, the User-Agent.
pub(crate) const APP: &str = app!();

// ── ui/mod.rs ──

/// How the app comes back after `restart()`.
pub(crate) fn relaunch_hint() -> &'static str {
    match paths::host() {
        Host::LiveContainer => {
            "Then open LiveContainer and run iNilla again (or its home-screen shortcut)."
        }
        Host::Android => {
            "Vanillish starts again by itself; if it does not, open it from the apps screen."
        }
        _ => "Then tap the iNilla icon on the home screen.",
    }
}

// ── setup.rs ──

pub(crate) const NEEDS_RESTART: &str = if ANDROID {
    "Vanillish opens the archives when it starts: restart it to play."
} else {
    "iNilla opens the archives when it starts. Close it and open it again to play."
};

pub(crate) const RESTART_NOW: &str = if ANDROID {
    "Restart Vanillish now"
} else {
    "Close iNilla now"
};

pub(crate) const NEEDS_DATA: &str = concat!(
    app!(),
    " needs the game data from your own World of Warcraft 1.12.1 (build 5875) client."
);

pub(crate) const KEEP_OPEN: &str = if ANDROID {
    "Keep Vanillish open and the screen on; a paused or interrupted download or copy resumes \
     where it stopped."
} else {
    "Keep iNilla open and the screen on; a paused or interrupted download resumes where it stopped."
};

pub(crate) const COPY_TITLE: &str = concat!("1 · Copy your Data folder into ", app!());

/// Where a computer puts the `Data` folder.
pub(crate) fn from_computer() -> String {
    if ANDROID {
        format!(
            "• From a PC: connect the phone with a USB cable and choose File transfer on the \
             phone, then open {} and drag the client's Data folder (or the whole World of \
             Warcraft folder) in. A Mac needs an MTP app such as OpenMTP.",
            drop_box()
        )
    } else {
        "• From a Mac or PC: connect the iPhone, open Finder (Windows: Apple Devices / iTunes) → \
         your iPhone → Files → iNilla (in LiveContainer: LiveContainer, then the folder \
         below), and drag the client's Data folder in."
            .to_string()
    }
}

/// How the device itself brings the `Data` folder in.
pub(crate) fn on_device() -> String {
    if ANDROID {
        "• On the phone: Import from folder… copies it from this phone's storage or a USB-C \
         stick or drive (about 5 GB; the original stays). The picker cannot use the Download \
         folder itself: put the game in a folder of its own, e.g. Download/WoW, and pick that."
            .to_string()
    } else {
        format!(
            "• On the iPhone: Files app → {} — paste the Data folder from iCloud Drive, a USB-C \
             drive or a server.",
            paths::files_location()
        )
    }
}

/// Vanillish: the drop box as a PC's file manager shows it:
/// `Internal storage → Android → data → <package> → files` (`.debug` builds have their own).
pub(crate) fn drop_box() -> String {
    let docs = paths::documents()
        .map(|d| d.display().to_string())
        .unwrap_or_default();
    match paths::android_package(&docs) {
        Some(pkg) => format!("Internal storage → Android → data → {pkg} → files"),
        None => docs,
    }
}

pub(crate) const IMPORT_FOLDER: &str = "Import from folder…";

pub(crate) const PICKER_WAITING: &str =
    "Choose the Data folder (or the World of Warcraft folder) in the picker…";

pub(crate) fn resume_from(label: &str) -> String {
    format!("Resume copying from {label}")
}

// ── data.rs ──

/// "Check for copied data" found nothing.
pub(crate) fn nothing_to_import() -> String {
    if ANDROID {
        let pkg = paths::documents()
            .and_then(|d| paths::android_package(&d.display().to_string()).map(str::to_string))
            .unwrap_or_else(|| "the app".into());
        format!(
            "No Data folder found in Vanillish's folder yet. Copy it in from a PC (Android → data \
             → {pkg} → files) or use {IMPORT_FOLDER}, then try again."
        )
    } else {
        "No Data folder found in iNilla's Documents yet. Copy it in and try again.".to_string()
    }
}

// ── launcher.rs ──

pub(crate) const REMEMBER_PASSWORD: &str = if ANDROID {
    "Remember the password (sealed on this phone with an Android Keystore key)"
} else {
    "Remember the password (kept in the iOS Keychain)"
};

pub(crate) const AUTO_LOGIN: &str = concat!("Log in automatically when ", app!(), " starts");

pub(crate) const NO_SIGN_UP: &str = concat!(
    app!(),
    " cannot create accounts: 1.12 has no in-game sign-up. Register on your server's website or \
     with `account create` on its console, then add the account here."
);

pub(crate) const NOT_SAVED: &str = if ANDROID {
    "Password not saved: the Android Keystore refused it"
} else {
    "Password not saved: the Keychain refused it"
};

// ── settings.rs ──

/// The graphics preset's radio label; on Android with its frame rate (`max_hz` is the panel's
/// highest, when the display says).
pub(crate) fn quality_label(q: Quality, max_hz: Option<f32>) -> String {
    if !ANDROID {
        return q.label().to_string();
    }
    match (q, max_hz) {
        (Quality::Saver | Quality::Balanced, _) => format!("{} {} fps", q.label(), q.fps_cap()),
        (Quality::High, Some(hz)) => format!("High {hz:.0} fps"),
        (Quality::High, None) => "High (full rate)".to_string(),
        (Quality::Custom, _) => q.label().to_string(),
    }
}

pub(crate) fn graphics_hint(max_hz: Option<f32>) -> String {
    if !ANDROID {
        return "Battery saver caps at 30 fps with a short view; High runs at the display's full \
                rate. Custom leaves the stock video options alone."
            .to_string();
    }
    let full = match max_hz {
        Some(hz) => format!("the panel's full {hz:.0} Hz"),
        None => "the display's full rate".to_string(),
    };
    format!(
        "Battery saver: 30 fps, short view. Balanced: 60 fps. High: {full}, best on a charger. \
         Custom leaves the stock video options alone."
    )
}

pub(crate) const RENDER_HINT: &str = if ANDROID {
    "The 3D world renders at this fraction of the screen; the interface stays sharp. 80 % is a \
     good default."
} else {
    "The 3D world renders at this fraction of the screen; the interface stays sharp. 80 % is a good phone default."
};

pub(crate) const UI_SCALE_HINT: &str = if ANDROID {
    "Automatic keeps a stock action button about a fingertip wide: 1.25 on the cover screen, \
     smaller on the inner screen, which has room for more."
} else {
    "Automatic is 1.25 on a phone: action buttons near fingertip size, and quest and vendor windows still clear of the action bars."
};

pub(crate) const ISLAND_ROW: &str = if ANDROID {
    "Keep windows clear of the camera cutout"
} else {
    "Keep windows clear of the Dynamic Island"
};

pub(crate) const ISLAND_HINT: &str = if ANDROID {
    "Game windows stop where the front camera's cutout begins, on whichever side it is; the \
     world still fills the screen, and the menu buttons sit beside it."
} else {
    "Game windows stop where the Dynamic Island (or the notch) begins, on whichever side it is; the world still fills the screen, and the menu buttons sit beside the island."
};

/// Vanillish: the inner screen's orientation row (`inner_any_orientation`).
pub(crate) const INNER_PORTRAIT_ROW: &str = "Inner screen: follow the device into portrait";
pub(crate) const INNER_PORTRAIT_HINT: &str =
    "Unfolded, the game turns with the phone. The cover screen always stays landscape: the \
     touch controls and these screens need its width.";

/// Vanillish: staying connected in the background (`stay_connected`).
pub(crate) const STAY_CONNECTED_ROW: &str = "Stay connected in the background";
pub(crate) const STAY_CONNECTED_HINT: &str =
    "A quick look at another app, or folding the phone shut, does not drop you from the server; \
     a notification shows while the game waits in the background.";

pub(crate) const NO_CONTROLLER: &str = if ANDROID {
    "No controller connected. Pair one in the phone's Settings → Connections → Bluetooth, or plug \
     in a USB-C controller (Backbone One, Razer Kishi, GameSir)."
} else {
    "No controller connected. Pair one in iOS Settings → Bluetooth, or attach a Backbone."
};

/// Vanillish: in place of the silent-switch row, which Android has no switch for.
pub(crate) const NO_SILENT_SWITCH: &str =
    "Android has no silent switch: the volume keys set the game's volume, and Mute or Vibrate \
     mode does not silence it.";

/// The remote console's hint under its addresses.
pub(crate) fn console_hint(port: u16) -> String {
    let base = "Open that address in a browser on a computer on the same Wi-Fi";
    let rest = "the live log, Lua, game commands, screenshots and the benilla-config files. \
                Anyone with the address can drive the game — turn it off when you are done.";
    if ANDROID {
        format!(
            "{base} or the phone's hotspot: {rest} Over USB with USB debugging: adb forward \
             tcp:{port} tcp:{port}, then http://localhost:{port}/ with the same token."
        )
    } else {
        format!("{base}: {rest}")
    }
}

pub(crate) const SESSION_LOG_ROW: &str = if ANDROID {
    "Write a session log (benilla-config → Diagnostics in Vanillish's folder, from a PC over USB; \
     also in logcat as \"vanillish\")"
} else {
    "Write a session log (Files → iNilla → benilla-config → Diagnostics)"
};

/// Vanillish: the share sheet for the session log.
pub(crate) const SHARE_LOG: &str = "Share session log…";
pub(crate) const NO_LOG_YET: &str = "No session log to share yet";
pub(crate) const SHARE_FAILED: &str = "The share sheet did not open";

pub(crate) const ABOUT_TITLE: &str = if ANDROID {
    "Vanillish — benilla for Android"
} else {
    "iNilla — benilla for iPhone"
};

pub(crate) const CREDIT: &str = if ANDROID {
    "benilla is a from-scratch World of Warcraft 1.12.1 client in Rust \
     (github.com/samwhosung/benilla); iNilla (github.com/Martze96/iNilla) is its iOS port, and \
     Vanillish is iNilla built for Android. None of them ships Blizzard content, nor is \
     affiliated with Blizzard Entertainment."
} else {
    "benilla is a from-scratch World of Warcraft 1.12.1 client in Rust (github.com/samwhosung/benilla); iNilla is its iOS port. Neither ships Blizzard content, nor is affiliated with Blizzard Entertainment."
};

pub(crate) const RESET_ALL: &str = concat!("Reset all ", app!(), " settings");

// ── logs.rs ──

pub(crate) const LOG_COPIED: &str = "Log copied";

/// Vanillish: the copy was capped (Android's clipboard goes over binder).
pub(crate) fn log_copied_last(lines: usize) -> String {
    format!("Copied the last {lines} lines — Share for the whole log")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The iOS build says exactly what iNilla said before these words moved here.
    #[cfg(not(target_os = "android"))]
    #[test]
    fn ios_keeps_its_words() {
        assert_eq!(APP, "iNilla");
        assert_eq!(
            NEEDS_DATA,
            "iNilla needs the game data from your own World of Warcraft 1.12.1 (build 5875) client."
        );
        assert_eq!(COPY_TITLE, "1 · Copy your Data folder into iNilla");
        assert_eq!(AUTO_LOGIN, "Log in automatically when iNilla starts");
        assert_eq!(RESET_ALL, "Reset all iNilla settings");
        assert!(NO_SIGN_UP.starts_with("iNilla cannot create accounts: 1.12 has no in-game"));
        assert_eq!(quality_label(Quality::High, Some(120.0)), "High");
        assert!(from_computer().contains("Finder (Windows: Apple Devices / iTunes) → your iPhone"));
        assert_eq!(
            console_hint(7878),
            "Open that address in a browser on a computer on the same Wi-Fi: the live log, Lua, \
             game commands, screenshots and the benilla-config files. Anyone with the address \
             can drive the game — turn it off when you are done."
        );
    }
}

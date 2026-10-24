//! **Every sentence that names the platform or the app**, in one place: what iNilla's screens say
//! on iOS (and the desktop), verbatim, and what Vanillish says on Android. A screen asks here
//! rather than spelling "iPhone", "Keychain" or "Finder" itself, so the Android build differs from
//! iOS in this file's text alone.
//!
//! Vanillish: and what Vanillish says on Windows ([`WINDOWS`]), where its sentences differ.
//!
//! Only glyphs the font test (`super::tests`) has proven: the arrow, the bullet, the middle dot,
//! the em dash and the ellipsis.

use inilla_core::connect::RegisterError;
use inilla_core::settings::Quality;
use inilla_platform::paths::{self, Host};

/// Vanillish: built for Android — the sentences below pick their Android form.
pub(crate) const ANDROID: bool = cfg!(target_os = "android");

/// Vanillish: built as Vanillish for Windows — benilla-app's `vanillish` feature, which only the
/// Windows entry (`vanillish-windows`) turns on; upstream's desktop run never has it.
pub(crate) const WINDOWS: bool = cfg!(feature = "vanillish");

/// Vanillish: either Vanillish, Android's or Windows'.
pub(crate) const VANILLISH: bool = ANDROID || WINDOWS;

#[cfg(any(target_os = "android", feature = "vanillish"))]
macro_rules! app {
    () => {
        "Vanillish"
    };
}
#[cfg(not(any(target_os = "android", feature = "vanillish")))]
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
        // Vanillish: Windows starts the exe again (`device::relaunch`).
        h if h == Host::Windows || WINDOWS => {
            "Vanillish starts again by itself; if it does not, open Vanillish.exe again."
        }
        _ => "Then tap the iNilla icon on the home screen.",
    }
}

// ── setup.rs ──

pub(crate) const NEEDS_RESTART: &str = if VANILLISH {
    "Vanillish opens the archives when it starts: restart it to play."
} else {
    "iNilla opens the archives when it starts. Close it and open it again to play."
};

pub(crate) const RESTART_NOW: &str = if VANILLISH {
    "Restart Vanillish now"
} else {
    "Close iNilla now"
};

/// Vanillish: shared storage did not answer at launch (`paths::storage_unavailable`; Android only).
pub(crate) const STORAGE_UNAVAILABLE: &str =
    "Shared storage is not available right now. Close Vanillish and open it again.";

pub(crate) const NEEDS_DATA: &str = concat!(
    app!(),
    " needs the game data from your own World of Warcraft 1.12.1 (build 5875) client."
);

pub(crate) const KEEP_OPEN: &str = if ANDROID {
    "Keep Vanillish open and the screen on; a paused or interrupted download or copy resumes \
     where it stopped."
} else if WINDOWS {
    "Keep Vanillish open and the PC awake; a paused or interrupted download or copy resumes where \
     it stopped."
} else {
    "Keep iNilla open and the screen on; a paused or interrupted download resumes where it stopped."
};

/// Vanillish: on Windows the client usually sits on this PC already, and is played in place.
pub(crate) const COPY_TITLE: &str = if WINDOWS {
    "1 · Your own 1.12.1 client"
} else {
    concat!("1 · Copy your Data folder into ", app!())
};

/// Where a computer puts the `Data` folder. Vanillish (Windows): the computer is this PC, whose
/// client is read where it is ([`USE_WOW_FOLDER`]).
pub(crate) fn from_computer() -> String {
    if WINDOWS {
        format!(
            "• Already have 1.12.1 on this PC? {USE_WOW_FOLDER} plays it where it is: nothing is \
             copied, and nothing is written into it."
        )
    } else if ANDROID {
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

/// How the device itself brings the `Data` folder in. Vanillish (Windows): a copy into the
/// Vanillish folder.
pub(crate) fn on_device() -> String {
    if WINDOWS {
        let root = drop_box();
        format!(
            "• Or copy it in (about 5 GB): the client's Data folder into {root}\\Data, or the whole \
             World of Warcraft folder into {root}, then Check for copied data. {IMPORT_FOLDER} \
             makes the copy for you."
        )
    } else if ANDROID {
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
/// `Internal storage → Android → data → <package> → files` (`.debug` builds have their own). On
/// Windows the Vanillish folder, as File Explorer's address bar shows it.
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

/// Vanillish (Windows): the player's own 1.12.1 client as the game data, read in place
/// (`paths::set_wow_folder`).
pub(crate) const USE_WOW_FOLDER: &str = "Use my WoW folder…";
pub(crate) const STOP_WOW_FOLDER: &str = "Stop using my WoW folder";

/// Vanillish (Windows): the setup screen's line while the game data is the player's own client.
pub(crate) fn wow_folder_in_use(data: &std::path::Path) -> String {
    format!(
        "Playing from your own client, in place: {}. Vanillish only reads it.",
        data.display()
    )
}

/// Vanillish (Windows): the picked folder is not a whole 1.12.1 client.
pub(crate) fn wow_folder_incomplete(label: &str, missing: &[&str]) -> String {
    format!(
        "{label} is not a whole 1.12.1 client: {} missing. Pick the World of Warcraft folder (the \
         one with WoW.exe) or its Data folder.",
        missing.join(", ")
    )
}

/// Vanillish (Windows): the picked folder is Vanillish's own.
pub(crate) const WOW_FOLDER_IS_OURS: &str =
    "That is Vanillish's own folder, where its own copy of \
     the game files goes: choose the folder of your World of Warcraft 1.12.1 client instead.";

/// Vanillish (Windows): what the next start reads, once the player picked their client or let it
/// go.
pub(crate) const WOW_FOLDER_CHOSEN: &str = "Vanillish plays from this folder from its next start: \
     nothing is copied, and nothing is written into it.";
pub(crate) const WOW_FOLDER_STOPPED: &str =
    "Vanillish uses its own Data folder again from its next start.";

pub(crate) fn wow_folder_not_saved(why: &str) -> String {
    format!("Vanillish could not remember the folder: {why}.")
}

/// Vanillish (Windows): a download or a copy while the game data is the player's own client, which
/// is never written into.
pub(crate) const IN_PLACE_READ_ONLY: &str = "The game data is your own client, read in place: \
     nothing is downloaded or copied into it. Stop using my WoW folder first (Settings → Debug → \
     Game data…).";

// ── data.rs ──

/// "Check for copied data" found nothing.
pub(crate) fn nothing_to_import() -> String {
    if WINDOWS {
        format!(
            "No Data folder found in {} yet. Copy it in and try again, or play your client where \
             it is: {USE_WOW_FOLDER}",
            drop_box()
        )
    } else if ANDROID {
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
} else if WINDOWS {
    "Remember the password (sealed on this PC with your Windows account, DPAPI)"
} else {
    "Remember the password (kept in the iOS Keychain)"
};

pub(crate) const AUTO_LOGIN: &str = concat!("Log in automatically when ", app!(), " starts");

pub(crate) const NO_SIGN_UP: &str = if VANILLISH {
    "1.12 has no in-game sign-up. On a server that takes sign-ups, Connect creates the account; \
     otherwise register on the server's website or with `account create` on its console, then add \
     the account here."
} else {
    concat!(
        app!(),
        " cannot create accounts: 1.12 has no in-game sign-up. Register on your server's website \
         or with `account create` on its console, then add the account here."
    )
};

pub(crate) const NOT_SAVED: &str = if ANDROID {
    "Password not saved: the Android Keystore refused it"
} else if WINDOWS {
    "Password not saved: Windows did not seal it with your account (DPAPI)"
} else {
    "Password not saved: the Keychain refused it"
};

/// Under the server form's world-server host (moved here from the launcher). Vanillish: a PC on
/// Windows.
pub(crate) const REALM_OVERRIDE_HINT: &str = if WINDOWS {
    "Use when the realm list hands out an address this PC cannot reach (a LAN IP while you are \
     outside, or a public IP inside a network without hairpin NAT). host or host:port."
} else {
    "Use when the realm list hands out an address this phone cannot reach (a LAN IP while you are \
     outside, or a public IP inside a network without hairpin NAT). host or host:port."
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

pub(crate) const RENDER_HINT: &str = if VANILLISH {
    "The 3D world renders at this fraction of the screen; the interface stays sharp. 80 % is a \
     good default."
} else {
    "The 3D world renders at this fraction of the screen; the interface stays sharp. 80 % is a good phone default."
};

/// Vanillish: the modern lighting switch (`modern_lighting`), read when the app starts.
pub(crate) const MODERN_LIGHTING_ROW: &str = "Modern lighting (restart)";
macro_rules! modern_lighting_hint {
    ($cost:literal) => {
        concat!(
            "Sunlight and torchlight per pixel, relief and soft highlights drawn from the game's \
             own textures, and water that catches the sky. The art stays the same. Applies the \
             next time ",
            app!(),
            " starts, and costs some ",
            $cost,
            "."
        )
    };
}
/// Vanillish: a PC pays for it in GPU time, not battery.
pub(crate) const MODERN_LIGHTING_HINT: &str = if WINDOWS {
    modern_lighting_hint!("GPU time")
} else {
    modern_lighting_hint!("battery")
};

pub(crate) const UI_SCALE_HINT: &str = if ANDROID {
    "Automatic keeps a stock action button about a fingertip wide: 1.25 on the cover screen, \
     smaller on the inner screen, which has room for more."
} else if WINDOWS {
    "Automatic is 1.25 on a touch screen: action buttons near fingertip size, and quest and \
     vendor windows still clear of the action bars."
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
} else if WINDOWS {
    "No controller connected. Plug in an Xbox controller, or pair it in Windows Settings → \
     Bluetooth & devices (or the Xbox Wireless Adapter)."
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
    if WINDOWS {
        format!(
            "Open that address in a browser on this PC, or on a phone or another computer on the \
             same network: {rest} The first time, Windows Firewall asks whether Vanillish may \
             accept connections: allow it on Private networks."
        )
    } else if ANDROID {
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

/// The session log's row. Vanillish (Windows): with the folder the logs go to, as File Explorer
/// shows it.
pub(crate) fn session_log_row() -> String {
    match paths::config_dir().filter(|_| WINDOWS) {
        Some(config) => format!(
            "Write a session log ({})",
            config.join("Diagnostics").display()
        ),
        None => SESSION_LOG_ROW.to_string(),
    }
}

/// Vanillish (Windows): the Vanillish folder in File Explorer, where a phone has Share.
pub(crate) const OPEN_FOLDER: &str = "Open the Vanillish folder";
pub(crate) const OPEN_FOLDER_FAILED: &str = "File Explorer did not open";

/// Vanillish: the share sheet for the session log.
pub(crate) const SHARE_LOG: &str = "Share session log…";
pub(crate) const NO_LOG_YET: &str = "No session log to share yet";
pub(crate) const SHARE_FAILED: &str = "The share sheet did not open";

pub(crate) const ABOUT_TITLE: &str = if ANDROID {
    "Vanillish — benilla for Android"
} else if WINDOWS {
    "Vanillish — benilla for Windows"
} else {
    "iNilla — benilla for iPhone"
};

pub(crate) const CREDIT: &str = if ANDROID {
    "benilla is a from-scratch World of Warcraft 1.12.1 client in Rust \
     (github.com/samwhosung/benilla); iNilla (github.com/Martze96/iNilla) is its iOS port, and \
     Vanillish is iNilla built for Android. None of them ships Blizzard content, nor is \
     affiliated with Blizzard Entertainment."
} else if WINDOWS {
    "benilla is a from-scratch World of Warcraft 1.12.1 client in Rust \
     (github.com/samwhosung/benilla); iNilla (github.com/Martze96/iNilla) is its iOS port, and \
     Vanillish is iNilla built for Android and for Windows. None of them ships Blizzard content, \
     nor is affiliated with Blizzard Entertainment."
} else {
    "benilla is a from-scratch World of Warcraft 1.12.1 client in Rust (github.com/samwhosung/benilla); iNilla is its iOS port. Neither ships Blizzard content, nor is affiliated with Blizzard Entertainment."
};

pub(crate) const RESET_ALL: &str = concat!("Reset all ", app!(), " settings");

// ── connect.rs (Vanillish) ──

/// Vanillish: the first screen is Connect — one field, the server's address — on Android and
/// Windows, and on a desktop rehearsing it with `$INILLA_QUICK_CONNECT=1`. Never on iOS, which
/// keeps iNilla's flow.
pub(crate) fn quick_connect() -> bool {
    VANILLISH
        || (!cfg!(target_os = "ios") && std::env::var("INILLA_QUICK_CONNECT").as_deref() == Ok("1"))
}

pub(crate) const CONNECT_TITLE: &str = "Connect";
pub(crate) const CONNECT_TO_SERVER: &str = "Connect to a server…";
pub(crate) const CONNECT_INTRO: &str = concat!(
    "Type the address of the computer that runs your server. ",
    app!(),
    " sets up the rest: the server, the game files and your account."
);
pub(crate) const ADDRESS_ROW: &str = "Server address";
pub(crate) const ADDRESS_EXAMPLE: &str = "192.168.1.20";
pub(crate) const ADDRESS_HINT: &str = "The IP the server printed when it started, or its name \
     (my-mac.local). host:port and http:// links work too.";
pub(crate) const CONNECT_BUTTON: &str = "Connect";
pub(crate) const ASKING: &str = "Asking the server…";
pub(crate) const MANUAL: &str = "Enter the details myself";
pub(crate) const OTHER_WAYS: &str = "Other ways to get the game files…";

/// Nothing answered at `host`: what to check.
pub(crate) fn unreachable(host: &str) -> String {
    // Vanillish: a PC is on a network rather than mobile data, and may run the server itself.
    if WINDOWS {
        return format!(
            "Nothing answers at {host}. Is the server running, and is this PC on the same network \
             as it (not a guest network)? On the server's computer the firewall must let in ports \
             3724, 8085 and 8099. A server on this same PC: type localhost."
        );
    }
    let device = if ANDROID { "phone" } else { "device" };
    let mut s = format!(
        "Nothing answers at {host}. Is the server running, and is this {device} on the same Wi-Fi \
         as it (not a guest network, not mobile data)? On the server's computer the firewall \
         must let in ports 3724, 8085 and 8099."
    );
    if cfg!(target_os = "ios") {
        s.push_str(
            " And iNilla must be allowed on the local network (Settings → Privacy & Security → \
             Local Network).",
        );
    }
    s
}

/// A plain 1.12.1 realm answered, with nothing that describes it.
pub(crate) fn plain_realm(host: &str) -> String {
    format!(
        "A 1.12.1 realm answers at {host}, but it does not describe itself: no game files to \
         download, no sign-up. Check the details and save."
    )
}

/// The server Connect set up before, asked again: its realm answers, its data host does not.
pub(crate) fn only_the_realm(host: &str) -> String {
    format!(
        "Only the realm answers at {host}: the server's data host (port 8099) does not, so the \
         game files and sign-up wait until it is running again."
    )
}

/// The server described itself in a way this app cannot use.
pub(crate) fn unusable(why: &str) -> String {
    format!(concat!(app!(), " cannot use this server: {}."), why)
}

/// `✔ Vanillish · vmangos · realm Vanillish (PvE) · LAN`
pub(crate) fn summary(
    name: &str,
    core: &str,
    realm: Option<(&str, Option<&str>)>,
    lan: bool,
) -> String {
    let mut s = format!("✔ {name} · {core}");
    match realm {
        Some((realm, Some(kind))) => s.push_str(&format!(" · realm {realm} ({kind})")),
        Some((realm, None)) => s.push_str(&format!(" · realm {realm}")),
        None => {}
    }
    s.push_str(if lan { " · LAN" } else { " · Internet" });
    s
}

/// A data host from before `server.json`: its data, and nothing else known.
pub(crate) const DATA_ONLY: &str =
    "This server shares its game files, but says nothing about its realm: it is reached on the \
     usual port 3724.";

pub(crate) const FILES_TITLE: &str = "The game files";

pub(crate) fn files_ready(archives: usize, gb: f64) -> String {
    format!("✔ The game files are here — {archives} archives, {gb:.1} GB.")
}

pub(crate) fn download_game(gb: f64) -> String {
    format!("Download the game ({gb:.1} GB)")
}

pub(crate) const DOWNLOAD_RESUME: &str = "Resume the download";
pub(crate) const ASKING_FILES: &str = "Asking for the list of game files…";
pub(crate) const TRY_AGAIN: &str = "Try again";

pub(crate) fn not_enough_space(need_gb: f64, free_gb: f64) -> String {
    format!("Not enough space: {need_gb:.1} GB needed, {free_gb:.1} GB free.")
}

pub(crate) const METERED: &str = if WINDOWS {
    "This PC is on a metered connection (Windows Settings → Network & internet): the download \
     counts against your plan."
} else {
    "This phone is on a metered connection (mobile data or a hotspot): the download counts \
     against your plan."
};
pub(crate) const PICK_LANGUAGE: &str = "The server has the game in several languages. Pick one: \
     quests, items and NPCs follow it.";
pub(crate) const TOKEN_ROW: &str = "Access code";
pub(crate) const TOKEN_HINT: &str = if ANDROID {
    "The server's owner protected the game files with an access code (INILLA_DATA_TOKEN in its \
     .env). It is saved on this phone."
} else if WINDOWS {
    "The server's owner protected the game files with an access code (INILLA_DATA_TOKEN in its \
     .env). It is saved on this PC, sealed with your Windows account."
} else {
    "The server's owner protected the game files with an access code (INILLA_DATA_TOKEN in its \
     .env). It is saved on this device."
};
pub(crate) const TOKEN_REFUSED_HERE: &str = "The server did not accept that access code.";
pub(crate) const CONTINUE: &str = "Continue";
pub(crate) const BRING_YOUR_OWN: &str =
    "This server does not share the game files. Bring your own:";

pub(crate) const ACCOUNT_TITLE: &str = "Your account";
pub(crate) const CREATE_TAB: &str = "Create an account";
pub(crate) const HAVE_TAB: &str = "I have an account";
pub(crate) const NAME_ROW: &str = "Account name";
pub(crate) const PASSWORD_ROW: &str = "Password";

pub(crate) fn name_hint(min: usize, max: usize) -> String {
    format!("{min} to {max} letters or digits")
}

pub(crate) fn password_hint(min: usize, max: usize) -> String {
    format!(
        "{min} to {max} characters, no spaces or quotes. Capitals do not matter: 1.12 sends \
         passwords in capitals."
    )
}

pub(crate) const SHOW_PASSWORD: &str = "Show";
pub(crate) const HIDE_PASSWORD: &str = "Hide";
pub(crate) const CREATE_ACCOUNT: &str = "Create account";
pub(crate) const SAVE_ACCOUNT: &str = "Save";
pub(crate) const CREATING: &str = "Creating the account…";
pub(crate) const ANOTHER_ACCOUNT: &str = "Use another account";
pub(crate) const NAME_NEEDED: &str = "Enter the account name.";
pub(crate) const PASSWORD_NEEDED: &str = "Enter the password, or let it be asked at each login.";

pub(crate) fn created(name: &str) -> String {
    format!("Account {name} created.")
}

pub(crate) fn account_saved(name: &str) -> String {
    format!("✔ Account {name}")
}

/// The account is saved; the game files are not all here yet.
pub(crate) fn logs_in_later(name: &str) -> String {
    format!(
        concat!(app!(), " logs in as {} once the game files are in."),
        name
    )
}

/// The account is saved; the game files arrived, and a restart opens them.
pub(crate) fn logs_in_after_restart(name: &str) -> String {
    format!(
        concat!(app!(), " logs in as {} when it starts again."),
        name
    )
}

pub(crate) const NO_SIGN_UP_HERE: &str = "This server does not take sign-ups from the app: ask \
     its owner for an account, then enter it here.";

/// Why a sign-up did not happen, for the player.
pub(crate) fn register_error(e: &RegisterError) -> String {
    match e {
        RegisterError::BadName(m) | RegisterError::BadPassword(m) => m.clone(),
        RegisterError::NameTaken => {
            "That account name is taken. If it is yours, choose \"I have an account\".".into()
        }
        RegisterError::Closed => "This server does not take sign-ups from here.".into(),
        RegisterError::TokenRequired => "The server wants its access code for sign-ups.".into(),
        RegisterError::RateLimited {
            retry_after: Some(secs),
        } => match secs.div_ceil(60) {
            0 | 1 => "Too many new accounts from here: try again in a minute.".into(),
            mins => format!("Too many new accounts from here: try again in {mins} minutes."),
        },
        RegisterError::RateLimited { retry_after: None } => {
            "Too many new accounts from here: try again later.".into()
        }
        RegisterError::Unavailable => "The server is still starting. Try again in a minute.".into(),
        RegisterError::Unreachable(why) => format!(
            "No answer from the server ({why}). Try again: the same request goes out, so the \
             account is never made twice."
        ),
        RegisterError::Failed(m) => format!(
            "Sign-up failed: {}. Ask the server's owner.",
            m.trim().trim_end_matches('.')
        ),
    }
}

pub(crate) fn play_as(name: &str) -> String {
    format!("▶ Play as {name}")
}

// ── admin.rs (Vanillish) ──

/// Vanillish: under the admin code's field — where it is kept, and that it is never sent.
pub(crate) const ADMIN_CODE_KEPT: &str = if ANDROID {
    "It is saved on this phone, sealed with an Android Keystore key; only a signature made with it \
     is sent."
} else if WINDOWS {
    "It is saved on this PC, sealed with your Windows account; only a signature made with it is \
     sent."
} else {
    "It is saved on this device; only a signature made with it is sent."
};

/// Vanillish: the code was refused by the secret store, so it lasts until the app closes.
pub(crate) const ADMIN_CODE_NOT_SAVED: &str = if ANDROID {
    "The admin code was not saved: the Android Keystore refused it. Vanillish keeps it until it \
     closes."
} else if WINDOWS {
    "The admin code was not saved: Windows did not seal it with your account. Vanillish keeps it \
     until it closes."
} else {
    concat!(
        "The admin code was not saved. ",
        app!(),
        " keeps it until it closes."
    )
};

/// Vanillish: the second press that confirms Server defaults (moved here from the admin screen); a
/// PC clicks.
pub(crate) const DEFAULTS_AGAIN: &str = if WINDOWS {
    "Click again to go back to the server's settings"
} else {
    "Tap again to go back to the server's settings"
};

/// Vanillish: the second press that confirms a gift of gold (moved here from the admin screen).
pub(crate) fn send_again(amount: &str, to: &str) -> String {
    let press = if WINDOWS { "Click" } else { "Tap" };
    format!("Send {amount} to {to}? {press} again")
}

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
    #[cfg(not(any(target_os = "android", feature = "vanillish")))]
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
        assert_eq!(
            REALM_OVERRIDE_HINT,
            "Use when the realm list hands out an address this phone cannot reach (a LAN IP \
             while you are outside, or a public IP inside a network without hairpin NAT). host \
             or host:port."
        );
        assert_eq!(send_again("5g", "Thrall"), "Send 5g to Thrall? Tap again");
    }

    /// Vanillish: the Windows build is Vanillish, starts at Connect, and says PC, not phone.
    #[cfg(feature = "vanillish")]
    #[test]
    fn windows_says_vanillish() {
        assert_eq!(APP, "Vanillish");
        assert!(quick_connect());
        assert_eq!(RESTART_NOW, "Restart Vanillish now");
        assert_eq!(AUTO_LOGIN, "Log in automatically when Vanillish starts");
        assert!(relaunch_hint().contains("Vanillish.exe"));
        assert!(MODERN_LIGHTING_HINT.ends_with("costs some GPU time."));
        assert!(NO_CONTROLLER.contains("Xbox Wireless Adapter"));
        assert!(
            unreachable("10.0.0.5").contains("this PC") && unreachable("x").contains("localhost")
        );
        assert!(console_hint(7878).contains("Windows Firewall"));
        assert!(REMEMBER_PASSWORD.contains("Windows account"));
        assert!(from_computer().contains(USE_WOW_FOLDER));
        assert_eq!(send_again("5g", "Thrall"), "Send 5g to Thrall? Click again");
        assert!(DEFAULTS_AGAIN.starts_with("Click again"));
        assert!(REALM_OVERRIDE_HINT.contains("this PC cannot reach"));
        for s in [
            NEEDS_RESTART,
            KEEP_OPEN,
            NO_SIGN_UP,
            METERED,
            TOKEN_HINT,
            ADMIN_CODE_KEPT,
            ABOUT_TITLE,
            CREDIT,
        ] {
            assert!(!s.contains("iNilla cannot") && !s.contains("phone"), "{s}");
        }
    }

    /// Vanillish: what Connect says about the server and about a refused sign-up.
    #[test]
    fn connect_says_what_happened() {
        assert_eq!(
            summary("Vanillish", "vmangos", Some(("Azeroth", Some("PvE"))), true),
            "✔ Vanillish · vmangos · realm Azeroth (PvE) · LAN"
        );
        assert_eq!(
            summary("Files", "other 1.12.1", None, false),
            "✔ Files · other 1.12.1 · Internet"
        );
        let limited = |s| register_error(&RegisterError::RateLimited { retry_after: s });
        assert_eq!(
            limited(Some(540)),
            "Too many new accounts from here: try again in 9 minutes."
        );
        assert_eq!(
            limited(Some(30)),
            "Too many new accounts from here: try again in a minute."
        );
        assert!(limited(None).ends_with("try again later."));
        assert!(register_error(&RegisterError::NameTaken).contains("I have an account"));
        assert_eq!(
            register_error(&RegisterError::BadName("Too short.".into())),
            "Too short."
        );
        // The server's sentence ends with its own full stop: one, not two.
        assert_eq!(
            register_error(&RegisterError::Failed(
                "The salt is not 64 hex digits.".into()
            )),
            "Sign-up failed: The salt is not 64 hex digits. Ask the server's owner."
        );
        assert_eq!(
            register_error(&RegisterError::Failed("the server answered 404".into())),
            "Sign-up failed: the server answered 404. Ask the server's owner."
        );
        assert!(unreachable("10.0.0.5").contains("3724, 8085 and 8099"));
        assert!(unusable("it is for client build 6005").ends_with("build 6005."));
    }

    /// Vanillish: where the admin menu says the code is kept.
    #[test]
    fn admin_says_where_the_code_is() {
        if ANDROID {
            assert_eq!(
                ADMIN_CODE_KEPT,
                "It is saved on this phone, sealed with an Android Keystore key; only a \
                 signature made with it is sent."
            );
            assert_eq!(
                ADMIN_CODE_NOT_SAVED,
                "The admin code was not saved: the Android Keystore refused it. Vanillish keeps \
                 it until it closes."
            );
        } else if WINDOWS {
            assert_eq!(
                ADMIN_CODE_KEPT,
                "It is saved on this PC, sealed with your Windows account; only a signature made \
                 with it is sent."
            );
            assert_eq!(
                ADMIN_CODE_NOT_SAVED,
                "The admin code was not saved: Windows did not seal it with your account. \
                 Vanillish keeps it until it closes."
            );
        } else {
            assert_eq!(
                ADMIN_CODE_KEPT,
                "It is saved on this device; only a signature made with it is sent."
            );
            assert_eq!(
                ADMIN_CODE_NOT_SAVED,
                "The admin code was not saved. iNilla keeps it until it closes."
            );
        }
    }
}

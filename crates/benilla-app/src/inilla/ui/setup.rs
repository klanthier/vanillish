//! **First run: the game data.** Two ways in, both ending at the same folder:
//!
//! - **Copy it in** — the player's own 1.12.1 `Data` folder, copied into iNilla's Documents by
//!   Finder / the Apple Devices app (File Sharing), the Files app (iCloud Drive, a USB-C stick,
//!   a network share) or AirDrop. Checked and moved into place without a second copy.
//! - **Download it** — from the data host of an iNilla server, the player's own or their operator's:
//!   a manifest of the archives with their hashes, then each archive, resumable, verified.
//!
//! The app itself ships no Blizzard files; whichever way they arrive, they are the player's own.
//!
//! Vanillish: on Android "copy it in" is a PC over USB (File transfer) into the app's folder, or
//! **Import from folder…** — the system's folder picker, then a resumable copy with the same
//! progress bar and Pause as the download (`DataState::import_folder`).

use bevy_egui::egui;

use super::{hint, ime, restart, row, words, Overlay, Panel};
use crate::inilla::data::DataState;
use crate::inilla::hud::GOLD;
use crate::inilla::platform::ImePurpose;
use crate::inilla::Inilla;

/// Roughly what a complete 1.12.1 `Data` folder takes, with room to spare: below this free, the
/// setup screen warns before a copy or download starts.
const TYPICAL_NEED: u64 = 5_400_000_000;

pub(super) fn show(
    ui: &mut egui::Ui,
    overlay: &mut Overlay,
    inilla: &mut Inilla,
    data: &mut DataState,
) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        // Vanillish: shared storage did not answer at launch. What is in the folders is not there
        // this time, and nothing set up now would be found the next (no download, no import).
        if inilla_platform::paths::storage_unavailable() {
            ui.label(
                egui::RichText::new(words::STORAGE_UNAVAILABLE)
                    .color(egui::Color32::LIGHT_RED)
                    .size(18.0),
            );
            hint(
                ui,
                "Your game data, settings and saved interface live in the app's folder on this \
                 phone's shared storage, which Android did not open this time (it happens briefly \
                 after a system update). Nothing has been lost.",
            );
            return;
        }
        let st = &data.status;
        if st.complete() {
            ui.label(
                egui::RichText::new(format!(
                    "✔ Game data ready — {} archives, {:.1} GB.",
                    st.present.len(),
                    st.total_bytes() as f64 / 1e9
                ))
                .color(egui::Color32::LIGHT_GREEN)
                .size(19.0),
            );
            if !st.missing_expected.is_empty() {
                hint(
                    ui,
                    &format!(
                        "Also usually present, but not required: {}.",
                        st.missing_expected.join(", ")
                    ),
                );
            }
            if data.needs_restart() {
                ui.label(words::NEEDS_RESTART);
                if ui
                    .add(
                        egui::Button::new(egui::RichText::new(words::RESTART_NOW).color(GOLD))
                            .min_size(egui::vec2(200.0, 44.0)),
                    )
                    .clicked()
                {
                    inilla.settings.setup_done = true;
                    restart(inilla);
                }
                hint(ui, super::relaunch_hint());
            } else if ui.button("Continue").clicked() {
                inilla.settings.setup_done = true;
                overlay.panel = Some(Panel::Launcher);
            }
            ui.separator();
        } else {
            ui.label(egui::RichText::new(words::NEEDS_DATA).size(18.0));
            hint(
                ui,
                "About 5 GB: the .MPQ archives in the client's Data folder. The app contains no \
                 Blizzard files — they are yours, read at runtime. English (enUS) is the \
                 tested language; German (deDE) data works the same way.",
            );
            if !st.present.is_empty() {
                ui.label(format!(
                    "Found {} of the archives so far; missing: {}.",
                    st.present.len(),
                    st.missing_required.join(", ")
                ));
            }
        }

        if progress(ui, data) {
            return;
        }

        if let Some((ok, msg)) = &data.message {
            ui.colored_label(
                if *ok {
                    egui::Color32::LIGHT_GREEN
                } else {
                    egui::Color32::LIGHT_RED
                },
                msg,
            );
        }

        // ── 1 · copy it in ──
        ui.add_space(6.0);
        ui.label(egui::RichText::new(words::COPY_TITLE).strong().color(GOLD));
        ui.label(words::from_computer());
        ui.label(words::on_device());
        // Vanillish: a folder outside the app, through the system's picker (Android; `$INILLA_PICK`
        // on the desktop).
        if inilla_platform::picker::available() {
            folder_import(ui, data);
        }
        ui.horizontal(|ui| {
            if ui
                .add(egui::Button::new("Check for copied data").min_size(egui::vec2(220.0, 44.0)))
                .clicked()
            {
                data.scan_import();
            }
            if ui
                .button("Check again after restart")
                .on_hover_text("A folder copied in before launch is imported automatically")
                .clicked()
            {
                restart(inilla);
            }
        });

        // ── 2 · download it ──
        ui.add_space(10.0);
        ui.label(
            egui::RichText::new("2 · Download it from your server")
                .strong()
                .color(GOLD),
        );
        if overlay.data_url.is_empty() {
            // The server's own data URL, or where an iNilla server puts it: port 8099 beside the
            // realm the player already added.
            if let Some(url) = inilla.launcher.current_server().map(|s| match &s.data {
                Some(d) if !d.url.is_empty() => d.url.clone(),
                _ => format!("http://{}:8099/", s.host.trim()),
            }) {
                overlay.data_url = url;
            }
        }
        row(ui, "Data URL", |ui| {
            let r = ui.add(
                egui::TextEdit::singleline(&mut overlay.data_url)
                    .hint_text("http://192.168.1.10:8099/")
                    .desired_width(360.0),
            );
            ime(&r, ImePurpose::Url);
        });
        row(ui, "Access token", |ui| {
            let r = ui.add(
                egui::TextEdit::singleline(&mut overlay.data_token)
                    .password(true)
                    .hint_text("if the host needs one")
                    .desired_width(360.0),
            );
            ime(&r, ImePurpose::Password);
        });
        hint(
            ui,
            "An iNilla server serves this on port 8099 (github.com/Martze96/iNilla-server). The \
             token is only needed if the server set one. Only serve the game to yourself and \
             people who own it.",
        );
        let url = overlay.data_url.trim().to_string();
        let token = Some(overlay.data_token.clone());
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !url.is_empty(),
                    egui::Button::new("Check server").min_size(egui::vec2(160.0, 44.0)),
                )
                .clicked()
            {
                data.fetch_manifest(url.clone(), token.clone(), data.locale.clone());
            }
            if let Some(m) = &data.manifest {
                let free = data.free_bytes();
                let need = m.total_bytes().saturating_sub(data.status.total_bytes());
                let enough = free.is_none_or(|f| f > need + 200_000_000);
                let label = if data.status.present.is_empty() {
                    "Download"
                } else {
                    "Download / resume"
                };
                if ui
                    .add_enabled(
                        enough,
                        egui::Button::new(egui::RichText::new(label).color(GOLD))
                            .min_size(egui::vec2(180.0, 44.0)),
                    )
                    .clicked()
                {
                    data.download(token.clone());
                }
                if let Some(f) = free {
                    ui.label(format!("{:.1} GB free", f as f64 / 1e9));
                    if !enough {
                        ui.colored_label(egui::Color32::LIGHT_RED, "not enough space");
                    }
                }
            }
        });

        // ── the language, when the server has game data in more than one ──
        if let Some(locales) = data.locales.clone() {
            let current = data
                .locale
                .clone()
                .unwrap_or_else(|| locales.default.clone());
            row(ui, "Language", |ui| {
                for l in &locales.locales {
                    let name = inilla_core::addons::locale_name(&l.locale);
                    if ui.selectable_label(l.locale == current, name).clicked()
                        && l.locale != current
                    {
                        data.fetch_manifest(url.clone(), token.clone(), Some(l.locale.clone()));
                    }
                }
            });
            hint(
                ui,
                "The game data is one language at a time: switching downloads that language's \
                 archives (about 5 GB) in place of the current ones. Quests, items and NPCs follow \
                 it too.",
            );
        }

        // ── addons ──
        if let Some(addons) = data.addons.clone().filter(|a| !a.addons.is_empty()) {
            ui.add_space(10.0);
            ui.label(
                egui::RichText::new("3 · Addons from your server")
                    .strong()
                    .color(GOLD),
            );
            let installed = crate::inilla::data::addons_dir()
                .map(|d| inilla_core::addons::InstalledAddons::load(&d))
                .unwrap_or_default();
            for a in &addons.addons {
                let version = a.version.clone().unwrap_or_default();
                let state = match installed.addons.get(&a.name) {
                    Some(v) if *v == version => "installed",
                    Some(_) => "update available",
                    None => "not installed",
                };
                ui.label(format!(
                    "• {} {} — {:.1} MB, {state}",
                    a.name,
                    version,
                    a.bytes() as f64 / 1e6
                ));
            }
            hint(
                ui,
                "Installed into benilla-config/AddOns and kept current from here. Addons you copy \
                 in yourself are never touched.",
            );
            if ui
                .add(egui::Button::new("Install / update addons").min_size(egui::vec2(220.0, 44.0)))
                .clicked()
            {
                data.sync_addons(token.clone());
            }
        }
    });
}

/// The running job, if any: what it is doing, the bar, the bytes, Pause. `true` while one runs.
/// (Vanillish: the Connect screen shows its download with it too.)
pub(super) fn progress(ui: &mut egui::Ui, data: &mut DataState) -> bool {
    let Some((label, p)) = data.busy() else {
        return false;
    };
    ui.separator();
    let frac = if p.bytes_total > 0 {
        p.bytes_done as f32 / p.bytes_total as f32
    } else if p.file_count > 0 {
        p.file_index as f32 / p.file_count as f32
    } else {
        0.0
    };
    ui.label(format!("{label}: {} {}", p.state, p.file));
    ui.add(
        egui::ProgressBar::new(frac)
            .show_percentage()
            .desired_width(ui.available_width().min(560.0)),
    );
    if p.bytes_total > 0 {
        ui.label(format!(
            "{:.2} of {:.2} GB",
            p.bytes_done as f64 / 1e9,
            p.bytes_total as f64 / 1e9
        ));
    }
    // Vanillish: the folder copy pauses (and resumes) as the download does.
    if matches!(label, "download" | "folder import") && ui.button("Pause").clicked() {
        data.pause();
    }
    hint(ui, words::KEEP_OPEN);
    true
}

/// Vanillish: **Import from folder…** — the picker, a copy left unfinished (Pause, a crash, the
/// app killed in the background: its grant and its `.copy` files survive), and the free space the
/// copy will need.
fn folder_import(ui: &mut egui::Ui, data: &mut DataState) {
    // The picker covers this screen while it is up, so the buttons stay live: a picker whose
    // answer never came back (the system lost it) must not lock them.
    ui.horizontal_wrapped(|ui| {
        let import = egui::Button::new(egui::RichText::new(words::IMPORT_FOLDER).color(GOLD))
            .min_size(egui::vec2(220.0, 44.0));
        if ui.add(import).clicked() && !inilla_platform::picker::pick() {
            data.message = Some((false, "The folder picker could not open.".into()));
        }
        if let Some((tree, label)) = data.pending_folder.clone() {
            let resume =
                egui::Button::new(words::resume_from(&label)).min_size(egui::vec2(220.0, 44.0));
            if ui.add(resume).clicked() {
                data.import_folder(tree, label);
            }
        }
        if let Some(free) = data.free_bytes() {
            let text = format!("{:.1} GB free", free as f64 / 1e9);
            let need = TYPICAL_NEED.saturating_sub(data.status.total_bytes());
            if free < need {
                ui.colored_label(
                    egui::Color32::LIGHT_RED,
                    format!("{text} — about {:.1} GB is needed", need as f64 / 1e9),
                );
            } else {
                ui.label(text);
            }
        }
    });
    if inilla_platform::picker::waiting() {
        hint(ui, words::PICKER_WAITING);
    }
}

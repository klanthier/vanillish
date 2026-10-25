//! **Settings**: iNilla's own switches, grouped for a thumb. Game settings (graphics details,
//! sound volumes, key bindings, the interface options) stay in the stock options windows — the
//! Game menu is one tap away on the radial.
//!
//! Vanillish: the platform's words come from [`words`]; Android adds the frame rates to the
//! graphics presets, the inner screen's orientation and the background connection, hides the
//! silent switch it has none of, and shares the session log. Off iOS, the Server tab is the server
//! admin menu (`super::admin`).

use std::sync::Mutex;
use std::time::{Duration, Instant};

use bevy_egui::egui;

use benilla_world::build_id::BuildId;
use inilla_core::fold::ScreenClass;
use inilla_core::pad::{Layer, PadBinding, PadButton, PadProfile, Special};
use inilla_core::settings::Quality;

use super::{hint, row, words, Overlay, Panel, SettingsTab};
use crate::inilla::admin::Admin;
use crate::inilla::data::DataState;
use crate::inilla::hud::GOLD;
use crate::inilla::pad::PadHud;
use crate::inilla::{store, Inilla};

pub(super) fn show(
    ui: &mut egui::Ui,
    overlay: &mut Overlay,
    inilla: &mut Inilla,
    data: &DataState,
    pad: &PadHud,
    build: &BuildId,
    admin: &mut Admin,
) {
    ui.horizontal_wrapped(|ui| {
        for (tab, label) in [
            (SettingsTab::General, "General"),
            (SettingsTab::Touch, "Touch"),
            (SettingsTab::Controller, "Controller"),
            (SettingsTab::Audio, "Audio"),
            (SettingsTab::Debug, "Debug"),
            (SettingsTab::About, "About"),
            // Vanillish: the server admin menu (iOS keeps iNilla's tabs).
            (SettingsTab::Server, "Server"),
        ]
        .into_iter()
        .filter(|(tab, _)| *tab != SettingsTab::Server || !cfg!(target_os = "ios"))
        {
            if ui
                .add(egui::Button::selectable(
                    overlay.tab == tab,
                    egui::RichText::new(label).size(18.0),
                ))
                .clicked()
            {
                overlay.tab = tab;
            }
        }
    });
    ui.separator();
    egui::ScrollArea::vertical().show(ui, |ui| match overlay.tab {
        SettingsTab::General => general(ui, inilla),
        SettingsTab::Touch => touch(ui, overlay, inilla),
        SettingsTab::Controller => controller(ui, inilla, pad),
        SettingsTab::Audio => audio(ui, inilla),
        SettingsTab::Debug => debug(ui, overlay, inilla),
        SettingsTab::About => about(ui, inilla, data, build),
        // Vanillish: the server admin menu.
        SettingsTab::Server => super::admin::show(ui, admin, inilla),
    });
}

/// Vanillish: the panel's highest refresh rate, asked of the display at most every two seconds
/// (a JNI call; the cover and inner panels may differ). `None` where the OS does not say.
fn panel_max_hz() -> Option<f32> {
    static CACHE: Mutex<Option<(Instant, Option<f32>)>> = Mutex::new(None);
    if !words::ANDROID {
        return None;
    }
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    match *cache {
        Some((at, hz)) if at.elapsed() < Duration::from_secs(2) => hz,
        _ => {
            let hz = inilla_platform::device::refresh_rate().map(|(_, max)| max.round());
            *cache = Some((Instant::now(), hz));
            hz
        }
    }
}

fn general(ui: &mut egui::Ui, k: &mut Inilla) {
    let s = &mut k.settings;
    let max_hz = panel_max_hz();
    row(ui, "Graphics", |ui| {
        for q in Quality::ALL {
            ui.radio_value(&mut s.quality, q, words::quality_label(q, max_hz));
        }
    });
    hint(ui, &words::graphics_hint(max_hz));
    row(ui, "Render resolution", |ui| {
        ui.add(
            egui::Slider::new(&mut s.render_scale, 0.5..=1.0)
                .step_by(0.05)
                .custom_formatter(|v, _| format!("{:.0} %", v * 100.0)),
        );
    });
    hint(ui, words::RENDER_HINT);
    // Vanillish: the modern lighting look, read at launch (iOS keeps iNilla's rows).
    if !cfg!(target_os = "ios") {
        ui.checkbox(&mut s.modern_lighting, words::MODERN_LIGHTING_ROW);
        hint(ui, words::MODERN_LIGHTING_HINT);
    }
    let mut auto = s.ui_scale == 0.0;
    row(ui, "Interface size", |ui| {
        if ui.checkbox(&mut auto, "Automatic").changed() {
            s.ui_scale = if auto { 0.0 } else { 1.25 };
        }
        if !auto {
            ui.add(egui::Slider::new(&mut s.ui_scale, 0.64..=1.5).step_by(0.01));
        }
    });
    hint(ui, words::UI_SCALE_HINT);
    ui.checkbox(&mut s.keep_screen_on, "Keep the screen on while playing");
    ui.checkbox(
        &mut s.auto_keyboard,
        "Show the keyboard when a text box is focused",
    );
    ui.checkbox(&mut s.haptics, "Haptic feedback");
    ui.checkbox(
        &mut s.compact_stock_ui,
        "Compact stock interface (hide the action bar's gryphons)",
    );
    ui.checkbox(&mut s.island_margin, words::ISLAND_ROW);
    hint(ui, words::ISLAND_HINT);
    if words::ANDROID {
        ui.checkbox(&mut s.inner_any_orientation, words::INNER_PORTRAIT_ROW);
        hint(ui, words::INNER_PORTRAIT_HINT);
        ui.checkbox(&mut s.stay_connected, words::STAY_CONNECTED_ROW);
        hint(ui, words::STAY_CONNECTED_HINT);
        ui.checkbox(
            &mut s.hide_touch_with_mouse,
            "Hide the touch controls while a mouse or keyboard is in use",
        );
        hint(ui, "Samsung DeX, or a Bluetooth mouse or keyboard: the touch controls step aside while you play with them, and the next touch brings them back.");
    }
}

fn touch(ui: &mut egui::Ui, overlay: &mut Overlay, k: &mut Inilla) {
    let l = &mut k.layout;
    ui.checkbox(&mut l.enabled, "Show the touch controls");
    ui.checkbox(
        &mut k.settings.hide_touch_with_controller,
        "Hide them while a controller is in use",
    );
    row(ui, "Size", |ui| {
        ui.add(egui::Slider::new(&mut l.hud_scale, 0.6..=1.6).step_by(0.05));
    });
    row(ui, "Opacity", |ui| {
        ui.add(egui::Slider::new(&mut l.opacity, 0.15..=1.0).step_by(0.05));
    });
    row(ui, "View", |ui| {
        let views = l.views.clone();
        for v in views {
            ui.radio_value(&mut l.active_view, v.clone(), v);
        }
    });
    hint(ui, "Views swap the buttons around the jump button: Combat shows action slots, Explore travel verbs, Social chat verbs, Minimal only the essentials. The view button (its icon shows the view) cycles them in play.");
    ui.horizontal(|ui| {
        if ui
            .add(
                egui::Button::new(egui::RichText::new("Edit layout…").color(GOLD))
                    .min_size(egui::vec2(160.0, 44.0)),
            )
            .clicked()
        {
            overlay.panel = Some(Panel::Editor);
        }
        if ui.button("Reset layout").clicked() {
            let enabled = l.enabled;
            // Vanillish: this screen's shipped layout.
            *l = l.class.defaults();
            l.enabled = enabled;
        }
    });
    // Vanillish: a foldable's screens each have their own layout (iOS keeps iNilla's one).
    if !cfg!(target_os = "ios") {
        ui.separator();
        let class = l.class;
        row(ui, "This screen", |ui| {
            ui.label(egui::RichText::new(class.label()).color(GOLD));
        });
        hint(ui, "Each screen has its own layout: the cover screen, the inner screen, and the inner screen half folded on a table. The right one comes up as you fold and unfold, and edits change this screen's only.");
        if class != ScreenClass::Phone && ui.button("Copy the phone layout here").clicked() {
            let mut phone = k
                .other_layouts
                .get(&ScreenClass::Phone)
                .cloned()
                .unwrap_or_else(|| store::load_layout_for(ScreenClass::Phone));
            phone.class = class;
            phone.enabled = l.enabled;
            *l = phone;
        }
        ui.checkbox(
            &mut k.settings.tabletop_split,
            "Tabletop: the game above the hinge, the controls below it",
        );
        hint(ui, "Half fold the inner screen and stand it on a table: the world and the game's windows move above the hinge, and the touch controls get the half below it to themselves. Off: the inner screen stays one screen.");
    }
    ui.separator();
    ui.label(egui::RichText::new("Feel").strong());
    let f = &mut l.feel;
    row(ui, "Look sensitivity", |ui| {
        ui.add(egui::Slider::new(&mut f.look_sensitivity, 0.3..=4.0).step_by(0.1));
    });
    ui.checkbox(&mut f.invert_y, "Invert look up/down");
    ui.checkbox(
        &mut f.drag_turns_character,
        "Dragging turns the character with the camera",
    );
    row(ui, "Long press (right click)", |ui| {
        ui.add(egui::Slider::new(&mut f.long_press_ms, 250..=900).suffix(" ms"));
    });
    row(ui, "Double tap window", |ui| {
        ui.add(egui::Slider::new(&mut f.double_tap_ms, 150..=500).suffix(" ms"));
    });
    row(ui, "Stick size", |ui| {
        ui.add(egui::Slider::new(&mut f.stick_radius, 36.0..=90.0).suffix(" pt"));
    });
    row(ui, "Left stick sideways", |ui| {
        ui.radio_value(&mut f.stick_steers, false, "Strafe");
        ui.radio_value(&mut f.stick_steers, true, "Steer");
    });
    hint(ui, "Steer turns the character and the camera with it, like the A and D keys; strafe side-steps, like Q and E. Forward and back walk either way.");
    hint(ui, "Hold jump for the hub: slide to a view on the inner arc or a window on the outer one and let go. Hold any other button to see what it does; letting go then does nothing.");
    hint(ui, "On the world: tap = select, double tap or long press = interact/attack, drag = look, pinch = zoom. On a window: tap = click, long press = right click, drag = move items, two fingers = scroll.");
}

/// Every binding a controller button can take, for the picker.
fn choices() -> Vec<PadBinding> {
    let mut v = vec![
        PadBinding::None,
        PadBinding::Special(Special::Interact),
        PadBinding::Special(Special::RadialMenu),
        PadBinding::Special(Special::VirtualCursor),
        PadBinding::Special(Special::InillaMenu),
        PadBinding::Special(Special::ToggleTouchHud),
    ];
    v.extend(crate::inilla::synth::command_names().map(|(n, _)| PadBinding::cmd(n)));
    v
}

fn controller(ui: &mut egui::Ui, k: &mut Inilla, pad: &PadHud) {
    match &pad.name {
        Some(n) => ui.label(
            egui::RichText::new(format!("Connected: {n}")).color(egui::Color32::LIGHT_GREEN),
        ),
        None => ui.label(words::NO_CONTROLLER),
    };
    let p = &mut k.pad;
    row(ui, "Stick deadzone", |ui| {
        ui.add(egui::Slider::new(&mut p.stick_deadzone, 0.05..=0.4).step_by(0.01));
    });
    row(ui, "Trigger threshold", |ui| {
        ui.add(egui::Slider::new(&mut p.trigger_threshold, 0.1..=0.9).step_by(0.05));
    });
    row(ui, "Turn speed", |ui| {
        ui.add(egui::Slider::new(&mut p.look_yaw_speed, 60.0..=480.0).suffix(" °/s"));
    });
    row(ui, "Look up/down speed", |ui| {
        ui.add(egui::Slider::new(&mut p.look_pitch_speed, 30.0..=300.0).suffix(" °/s"));
    });
    row(ui, "Response curve", |ui| {
        ui.add(egui::Slider::new(&mut p.look_curve, 1.0..=3.0).step_by(0.1));
    });
    ui.checkbox(&mut p.invert_y, "Invert look up/down");
    ui.checkbox(
        &mut k.settings.controller_hotkeys,
        "Show controller buttons on the action bars",
    );
    ui.checkbox(
        &mut k.settings.controller_bar,
        "Show the controller bar at the bottom of the screen",
    );
    hint(
        ui,
        "Each action button shows what presses it (X, LT A, RT ↑) while the controller is in use, \
         and holding a trigger lights up the buttons it switches to. LT uses the bottom-left bar, \
         RT the bottom-right, LT+RT the right bar.",
    );
    let p = &mut k.pad;
    row(ui, "Cursor speed", |ui| {
        ui.add(egui::Slider::new(&mut p.cursor_speed, 300.0..=2000.0).suffix(" pt/s"));
    });
    ui.checkbox(
        &mut p.swap_confirm,
        "Confirm with B / cancel with A (Nintendo layout)",
    );
    ui.checkbox(
        &mut p.auto_cursor,
        "Cursor mode opens with loot, vendor and quest windows",
    );
    ui.separator();
    ui.label(egui::RichText::new("Mapping").strong());
    hint(ui, "Hold LT, RT or both to switch the face buttons and d-pad to another layer — 32 bindings without leaving the sticks. By default LT is the bottom-left action bar, RT the bottom-right, LT+RT the right bar: drag spells onto those bars as on desktop.");
    let all = choices();
    let glyphs = pad.glyphs;
    egui::Grid::new("pad-map")
        .striped(true)
        .min_col_width(120.0)
        .show(ui, |ui| {
            ui.label("");
            for l in Layer::ALL {
                ui.label(egui::RichText::new(l.label()).strong().color(GOLD));
            }
            ui.end_row();
            let buttons = [
                PadButton::A,
                PadButton::B,
                PadButton::X,
                PadButton::Y,
                PadButton::Up,
                PadButton::Right,
                PadButton::Down,
                PadButton::Left,
                PadButton::LB,
                PadButton::RB,
                PadButton::L3,
                PadButton::R3,
                PadButton::Menu,
                PadButton::Options,
            ];
            for b in buttons {
                ui.label(egui::RichText::new(b.glyph(glyphs)).size(18.0));
                for l in Layer::ALL {
                    let current = p.binding(l, b).clone();
                    let mut pick = current.clone();
                    egui::ComboBox::from_id_salt((
                        "padmap",
                        l.label(),
                        b.glyph(Default::default()),
                    ))
                    .selected_text(current.label())
                    .width(140.0)
                    .height(360.0)
                    .show_ui(ui, |ui| {
                        for c in &all {
                            ui.selectable_value(&mut pick, c.clone(), c.label());
                        }
                    });
                    if pick != current {
                        p.set_binding(l, b, pick);
                    }
                }
                ui.end_row();
            }
        });
    if ui.button("Reset controller settings").clicked() {
        *p = PadProfile::default();
    }
}

fn audio(ui: &mut egui::Ui, k: &mut Inilla) {
    let s = &mut k.settings;
    ui.checkbox(
        &mut s.mix_with_other_audio,
        "Let music and podcasts from other apps keep playing",
    );
    // Vanillish: Android's media stream ignores Mute and Vibrate: no switch, so no row (the setting
    // stays in the file for iOS).
    if words::ANDROID {
        hint(ui, words::NO_SILENT_SWITCH);
    } else {
        ui.checkbox(
            &mut s.ignore_silent_switch,
            "Play sound even when the silent switch is on",
        );
    }
    hint(
        ui,
        "Volumes, music and ambience are in the stock Sound Options (Game menu → Sound & Voice).",
    );
}

fn debug(ui: &mut egui::Ui, overlay: &mut Overlay, k: &mut Inilla) {
    let d = &mut k.settings.debug;
    let was = d.remote_console;
    ui.checkbox(&mut d.remote_console, "Remote console on the local network");
    if d.remote_console && !was && d.console_token.is_empty() {
        d.console_token = crate::inilla::console::new_token();
    }
    if d.remote_console {
        let port = d.console_port;
        for ip in crate::inilla::console::local_addresses() {
            ui.label(
                egui::RichText::new(format!("http://{ip}:{port}/?token={}", d.console_token))
                    .monospace()
                    .color(GOLD),
            );
        }
        hint(ui, &words::console_hint(port));
        ui.horizontal(|ui| {
            if ui.button("Copy address").clicked() {
                if let Some(ip) = crate::inilla::console::local_addresses().first() {
                    inilla_platform::pasteboard::set(&format!(
                        "http://{ip}:{port}/?token={}",
                        d.console_token
                    ));
                    overlay.toast("Console address copied");
                }
            }
            if ui.button("New token").clicked() {
                d.console_token = crate::inilla::console::new_token();
            }
        });
    }
    ui.checkbox(&mut d.log_to_file, words::SESSION_LOG_ROW);
    ui.checkbox(&mut d.perf_overlay, "Show frame rate, memory and heat");
    ui.checkbox(&mut d.show_touch_zones, "Show touch zones");
    ui.horizontal(|ui| {
        if ui
            .add(egui::Button::new("View session log").min_size(egui::vec2(180.0, 44.0)))
            .clicked()
        {
            overlay.panel = Some(Panel::Logs);
        }
        if ui.button("Game data…").clicked() {
            overlay.panel = Some(Panel::Setup);
        }
        // Vanillish: the one way to get a log off the phone without a computer.
        if inilla_platform::picker::can_share() && ui.button(words::SHARE_LOG).clicked() {
            super::logs::share_session_log(overlay);
        }
    });
}

fn about(ui: &mut egui::Ui, k: &mut Inilla, data: &DataState, build: &BuildId) {
    ui.label(egui::RichText::new(words::ABOUT_TITLE).strong().color(GOLD));
    ui.label(format!(
        "Build {} ({}, {})",
        build.short, build.date, build.profile
    ));
    ui.label(inilla_platform::device::describe());
    ui.label(format!(
        "Game data: {} archives, {:.1} GB{}",
        data.status.present.len(),
        data.status.total_bytes() as f64 / 1e9,
        data.dir
            .as_ref()
            .map(|d| format!(" in {}", d.display()))
            .unwrap_or_default()
    ));
    hint(ui, words::CREDIT);
    ui.add_space(12.0);
    if ui
        .button(egui::RichText::new(words::RESET_ALL).color(egui::Color32::LIGHT_RED))
        .clicked()
    {
        // Vanillish: the system's game mode last seen is kept — not a setting, and a reset is no
        // change of the mode for the preset to follow.
        let last_game_mode = k.settings.last_game_mode;
        k.settings = Default::default();
        k.settings.setup_done = true;
        k.settings.last_game_mode = last_game_mode;
        // Vanillish: every screen's layout (the cover's, the inner screen's, the tabletop's), the
        // one in use staying this screen's.
        crate::inilla::screen::reset_layouts(k);
        k.pad = PadProfile::default();
    }
}

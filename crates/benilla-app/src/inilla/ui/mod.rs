//! **iNilla's own screens**, in egui over the game: first-run setup, the launcher (servers and
//! accounts), settings, the HUD layout editor and the log viewer. Styled dark and gold to sit
//! beside the stock interface, and sized for a thumb (44-point rows, 17-point text).
//!
//! Vanillish: every sentence that names the platform comes from [`words`]; on Android the screens
//! also take the system Back first ([`back_to_screens`]) and tell the soft keyboard what the
//! focused field holds ([`ime`]). On Android the first screen is [`connect`]'s — the server's
//! address, then its game files and an account — rather than the game-data setup.

/// Vanillish: the server admin menu, Settings → Server (`SettingsTab::Server`).
mod admin;
/// Vanillish: quick connect's screen (`Panel::Connect`).
mod connect;
mod editor;
mod launcher;
mod logs;
mod settings;
mod setup;
/// Vanillish: the platform's and the app's words (iNilla's on iOS, Vanillish's on Android).
pub(crate) mod words;

use bevy::ecs::message::MessageCursor;
use bevy::input::keyboard::KeyboardInput;
use bevy::input::ButtonState;
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPreUpdateSet, EguiPrimaryContextPass};

use super::data::DataState;
use super::hud::GOLD;
use super::platform::{BackPresses, ImePurpose};
use super::session::LoginQueue;
use super::Inilla;
use crate::char_select::ClientState;

pub(crate) use editor::EditorState;
pub(crate) use launcher::{AccountForm, ServerForm};

/// Which screen is up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Panel {
    Setup,
    Launcher,
    Settings,
    Editor,
    Logs,
    /// Vanillish: quick connect.
    Connect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum SettingsTab {
    #[default]
    General,
    Touch,
    Controller,
    Audio,
    Debug,
    About,
    /// Vanillish: the server admin menu (off iOS).
    Server,
}

#[derive(Resource, Default)]
pub(crate) struct Overlay {
    pub(crate) panel: Option<Panel>,
    /// A one-line message and the real time it expires.
    pub(crate) toast: Option<(String, f64)>,
    pub(crate) tab: SettingsTab,
    pub(crate) server_form: Option<ServerForm>,
    pub(crate) account_form: Option<AccountForm>,
    /// The launcher's selected server (by id).
    pub(crate) selected_server: Option<String>,
    /// A password typed for an account with none saved.
    pub(crate) password_prompt: Option<(String, String)>,
    pub(crate) editor: EditorState,
    pub(crate) log_filter: String,
    pub(crate) log_follow: bool,
    /// The setup screen's download form.
    pub(crate) data_url: String,
    pub(crate) data_token: String,
    /// Vanillish: the Connect screen's fields.
    pub(crate) connect: connect::ConnectForm,
    /// Vanillish: Connect, not the setup screen, is the screen without the game data (it was the
    /// first one): the others lead back to it.
    pub(crate) connect_home: bool,
    /// The one automatic login per launch has been spent.
    auto_login_done: bool,
    /// The clock the toasts are measured against (real time, updated every frame).
    now: f64,
}

impl Overlay {
    /// iNilla's screen owns the input: nothing reaches the game underneath.
    pub(crate) fn blocks_game(&self) -> bool {
        self.panel.is_some()
    }

    pub(crate) fn open_launcher(&mut self) {
        self.panel = Some(Panel::Launcher);
    }

    /// The iNilla button / menu binding: the launcher at the glue screens, settings in the world.
    pub(crate) fn toggle_menu(&mut self) {
        if self.panel.is_some() {
            self.panel = None;
        } else {
            self.panel = Some(Panel::Settings);
        }
    }

    pub(crate) fn toast(&mut self, text: impl Into<String>) {
        self.toast = Some((text.into(), self.now + 3.0));
    }
}

pub(crate) struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Overlay>()
            .add_systems(
                Update,
                (
                    tick_clock,
                    first_screen,
                    desktop_hotkey,
                    back_to_screens,
                    plain_keyboard_when_closed,
                ),
            )
            .add_systems(OnEnter(ClientState::Login), back_at_login)
            .add_systems(
                EguiPrimaryContextPass,
                (style_once, draw_panels, track_pointer)
                    .chain()
                    .run_if(super::lane_up),
            );
        // Vanillish: a field typed in keeps its keys from the game (iOS unchanged).
        if !cfg!(target_os = "ios") {
            app.add_systems(
                PreUpdate,
                keyboard_to_the_screens.after(EguiPreUpdateSet::ProcessInput),
            );
        }
    }
}

/// Vanillish: while a field on these screens has the keyboard, its key presses are the field's
/// alone — taken from the game's queue once egui has read them, so typing in the world moves
/// nobody and its Enter opens no chat. Releases stay, so a key held into a screen still lets go.
fn keyboard_to_the_screens(
    overlay: Res<Overlay>,
    lane: Res<super::EguiWanted>,
    mut egui: Query<&mut bevy_egui::EguiContext, With<bevy_egui::PrimaryEguiContext>>,
    mut keys: ResMut<Messages<KeyboardInput>>,
    mut cursor: Local<MessageCursor<KeyboardInput>>,
) {
    // Since the last run: what no reader after this one has had yet.
    let fresh: Vec<KeyboardInput> = cursor.read(&keys).cloned().collect();
    // A read: `get_mut` is the only getter bevy_egui offers without its `immutable_ctx` feature.
    let typing = overlay.panel.is_some()
        && lane.0
        && egui.iter_mut().any(|mut ctx| {
            ctx.bypass_change_detection()
                .get_mut()
                .wants_keyboard_input()
        });
    if !typing || !fresh.iter().any(|k| k.state == ButtonState::Pressed) {
        return;
    }
    // Messages cannot be removed one by one: drop the buffers (every reader has had the older
    // ones) and write this frame's releases back.
    keys.drain().for_each(drop);
    for k in fresh {
        if k.state == ButtonState::Released {
            keys.write(k);
        }
    }
    cursor.clear(&keys);
}

fn tick_clock(mut overlay: ResMut<Overlay>, time: Res<Time<Real>>) {
    overlay.now = time.elapsed_secs_f64();
}

/// What the player sees first: setup until the data is complete, then the launcher — unless the
/// current account logs in on its own.
///
/// Vanillish: with quick connect (Android) Connect stands in for setup — unless the current
/// server was entered by hand without a data source, whose player brings their own files — and
/// opens over the login when there is no server yet, or the current one is a server it set up
/// that has no account yet (the account was skipped before the relaunch).
fn first_screen(
    mut overlay: ResMut<Overlay>,
    data: Res<DataState>,
    inilla: Res<Inilla>,
    mut queue: ResMut<LoginQueue>,
    client: Res<State<ClientState>>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    *done = true;
    let current = inilla.launcher.current_server();
    if !data.ready_at_boot {
        let quick = words::quick_connect() && current.is_none_or(|s| s.data.is_some());
        overlay.connect_home = quick;
        overlay.panel = Some(if quick { Panel::Connect } else { Panel::Setup });
        return;
    }
    if *client.get() != ClientState::Login {
        return;
    }
    if let Some(account) = inilla.launcher.current_account() {
        if account.auto_login && !overlay.auto_login_done {
            if let Some(pass) = super::session::saved_password(account) {
                overlay.auto_login_done = true;
                queue.request(&account.username, &pass);
                return;
            }
        }
    }
    // No server yet (the files came another way), or one Connect set up with no account yet.
    let unfinished = current
        .is_none_or(|s| s.info.is_some() && inilla.launcher.accounts_for(&s.id).next().is_none());
    if words::quick_connect() && unfinished {
        overlay.panel = Some(Panel::Connect);
        return;
    }
    // On a desktop the launcher waits for F10: benilla's own login screen (and `$WOW_USER`) is
    // the desktop workflow.
    if launcher_at_login() {
        overlay.panel = Some(Panel::Launcher);
    }
}

/// Vanillish: the launcher opens by itself at the login screen on a touch device, and in
/// Vanillish for Windows, a player's app rather than the desktop workflow — whose touch HUD stays
/// off.
fn launcher_at_login() -> bool {
    super::touch_device() || words::WINDOWS
}

/// F10 opens iNilla's screens on a desktop, where there is no HUD button for them.
fn desktop_hotkey(
    keys: Res<ButtonInput<KeyCode>>,
    mut overlay: ResMut<Overlay>,
    client: Res<State<ClientState>>,
) {
    if !keys.just_pressed(KeyCode::F10) {
        return;
    }
    overlay.panel = match overlay.panel {
        Some(_) => None,
        None if *client.get() == ClientState::InWorld => Some(Panel::Settings),
        None => Some(Panel::Launcher),
    };
}

/// Logging out lands at the login screen: offer the account picker again.
fn back_at_login(mut overlay: ResMut<Overlay>, data: Res<DataState>, mut seen: Local<bool>) {
    // The first entry is the boot's, which `first_screen` already decided.
    if !std::mem::replace(&mut *seen, true) {
        return;
    }
    if data.ready_at_boot && launcher_at_login() && overlay.panel.is_none() {
        overlay.panel = Some(Panel::Launcher);
    }
}

/// egui's own fonts, with Hack — which egui already ships for monospace — behind the proportional
/// face too: Ubuntu Light has no arrows, `□`, `△` or `◐`, and a missing glyph draws as a box.
pub(crate) fn fonts() -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
        family.push("Hack".to_owned());
    }
    fonts
}

/// Dark, gold-accented, and big enough for a fingertip.
fn style_once(mut contexts: EguiContexts, mut done: Local<bool>) -> Result {
    if *done {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;
    ctx.set_fonts(fonts());
    let mut style = (*ctx.style()).clone();
    let touch = super::touch_device();
    style.visuals = egui::Visuals::dark();
    style.visuals.window_fill = egui::Color32::from_rgba_unmultiplied(14, 14, 20, 240);
    style.visuals.panel_fill = egui::Color32::from_rgba_unmultiplied(10, 10, 14, 235);
    style.visuals.selection.bg_fill = egui::Color32::from_rgb(120, 96, 24);
    style.visuals.hyperlink_color = GOLD;
    style.visuals.widgets.active.bg_stroke = egui::Stroke::new(1.5_f32, GOLD);
    style.visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0_f32, GOLD);
    style.visuals.window_stroke = egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(120, 96, 24));
    if touch {
        style.spacing.interact_size = egui::vec2(44.0, 40.0);
        style.spacing.item_spacing = egui::vec2(10.0, 10.0);
        style.spacing.button_padding = egui::vec2(14.0, 8.0);
        style.spacing.slider_width = 220.0;
        for (text_style, font) in style.text_styles.iter_mut() {
            font.size = match text_style {
                egui::TextStyle::Heading => 24.0,
                egui::TextStyle::Small => 13.0,
                egui::TextStyle::Monospace => 13.0,
                _ => 17.0,
            };
        }
    }
    ctx.set_style(style);
    *done = true;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn draw_panels(
    mut contexts: EguiContexts,
    mut overlay: ResMut<Overlay>,
    mut inilla: ResMut<Inilla>,
    mut data: ResMut<DataState>,
    mut queue: ResMut<LoginQueue>,
    client: Res<State<ClientState>>,
    safe: Res<super::platform::SafeArea>,
    hud: Res<super::screen::HudScreen>,
    pad: Res<super::pad::PadHud>,
    build: Res<benilla_world::build_id::BuildId>,
    mut ime_purpose: ResMut<ImePurpose>,
    mut quick: ResMut<super::connect::Connect>,
    // Vanillish: the Server tab's state and requests.
    mut admin: ResMut<super::admin::Admin>,
) -> Result {
    let Some(panel) = overlay.panel else {
        ime_purpose.set_if_neq(ImePurpose::Text);
        return Ok(());
    };
    let ctx = contexts.ctx_mut()?.clone();
    // Vanillish: the fields drawn below say what they hold while focused (`ime`).
    ctx.data_mut(|d| d.remove::<ImePurpose>(ime_id()));
    let insets = safe.0;
    let frame = egui::Frame::new()
        .fill(ctx.style().visuals.panel_fill)
        .inner_margin(egui::Margin {
            left: (insets[1] + 16.0) as i8,
            right: (insets[3] + 16.0) as i8,
            top: (insets[0] + 12.0) as i8,
            bottom: (insets[2] + 12.0) as i8,
        });
    let in_world = *client.get() == ClientState::InWorld;
    match panel {
        // Vanillish: where the layout will be placed (on a tabletop the deck, even at the glue).
        Panel::Editor => editor::show(&ctx, &mut overlay, &mut inilla, &hud.layout_screen),
        _ => {
            egui::CentralPanel::default().frame(frame).show(&ctx, |ui| {
                header(ui, &mut overlay, panel, in_world, data.ready_at_boot);
                ui.separator();
                match panel {
                    Panel::Setup => setup::show(ui, &mut overlay, &mut inilla, &mut data),
                    Panel::Launcher => {
                        launcher::show(ui, &mut overlay, &mut inilla, &mut queue, &data)
                    }
                    Panel::Settings => settings::show(
                        ui,
                        &mut overlay,
                        &mut inilla,
                        &data,
                        &pad,
                        &build,
                        &mut admin,
                    ),
                    Panel::Logs => logs::show(ui, &mut overlay),
                    Panel::Connect => connect::show(
                        ui,
                        &mut overlay,
                        &mut inilla,
                        &mut data,
                        &mut quick,
                        &mut queue,
                    ),
                    Panel::Editor => {}
                }
            });
        }
    }
    let purpose = ctx
        .data(|d| d.get_temp::<ImePurpose>(ime_id()))
        .unwrap_or_default();
    ime_purpose.set_if_neq(purpose);
    Ok(())
}

/// Vanillish: where a focused field leaves its [`ImePurpose`] for [`draw_panels`] (egui's
/// per-frame scratch memory, so no form needs a new parameter).
fn ime_id() -> egui::Id {
    egui::Id::new("vanillish-ime-purpose")
}

/// Vanillish: with no screen up there is no field of iNilla's to type into (the lane may sleep
/// before `draw_panels` runs once more to say so).
fn plain_keyboard_when_closed(overlay: Res<Overlay>, mut ime_purpose: ResMut<ImePurpose>) {
    if overlay.panel.is_none() {
        ime_purpose.set_if_neq(ImePurpose::Text);
    }
}

/// Vanillish: while `field` has the focus, the soft keyboard opens for `purpose` — a password is
/// never suggested or learned, an address gets the URL layout. Call it on the `TextEdit`'s
/// response.
pub(super) fn ime(field: &egui::Response, purpose: ImePurpose) {
    if field.has_focus() {
        field.ctx.data_mut(|d| d.insert_temp(ime_id(), purpose));
    }
}

/// The strip across the top of every screen: the title and the ways out.
fn header(
    ui: &mut egui::Ui,
    overlay: &mut Overlay,
    panel: Panel,
    in_world: bool,
    data_ready: bool,
) {
    ui.horizontal(|ui| {
        ui.heading(egui::RichText::new(words::APP).color(GOLD).strong());
        let title = match panel {
            Panel::Setup => "Game data",
            Panel::Launcher => "Servers & accounts",
            Panel::Settings => "Settings",
            Panel::Logs => "Session log",
            Panel::Editor => "Edit HUD",
            Panel::Connect => words::CONNECT_TITLE,
        };
        ui.label(egui::RichText::new(title).size(20.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Without the game data there is nothing behind the first screen to close it to.
            let can_close = !matches!(panel, Panel::Setup | Panel::Connect) || data_ready;
            if can_close && ui.button("× Close").clicked() {
                // Vanillish: without the data and without the touch HUD's glue button, nothing
                // leads back from a bare window: close to the first screen, as Back does.
                overlay.panel = if !data_ready && !super::touch_device() {
                    Some(if overlay.connect_home {
                        Panel::Connect
                    } else {
                        Panel::Setup
                    })
                } else {
                    None
                };
            }
            if panel != Panel::Settings && ui.button("⚙ Settings").clicked() {
                overlay.panel = Some(Panel::Settings);
            }
            // Vanillish: back to Connect from the other ways to get the game files.
            if panel == Panel::Setup
                && !in_world
                && words::quick_connect()
                && ui.button(words::CONNECT_TITLE).clicked()
            {
                overlay.panel = Some(Panel::Connect);
            }
            if !in_world && panel != Panel::Launcher && data_ready && ui.button("Servers").clicked()
            {
                overlay.panel = Some(Panel::Launcher);
            }
        });
    });
}

/// Tell the game when a finger or the pointer is on one of iNilla's screens.
fn track_pointer(
    mut contexts: EguiContexts,
    mut over: ResMut<crate::ui_script::EguiPointerOver>,
    overlay: Res<Overlay>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    let now = overlay.panel.is_some() || ctx.is_pointer_over_area() || ctx.wants_pointer_input();
    if over.0 != now {
        over.0 = now;
    }
    Ok(())
}

/// A labelled row: the label on the left at a fixed width, the widget after it.
pub(super) fn row(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.add_sized([200.0, 30.0], egui::Label::new(label));
        add(ui);
    });
}

/// A short grey explanation under a setting.
pub(super) fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).small().color(egui::Color32::GRAY));
}

/// How to start iNilla again after [`restart`] (Vanillish: the `Host::Android` arm says it comes
/// back by itself).
pub(super) fn relaunch_hint() -> &'static str {
    words::relaunch_hint()
}

/// Leave the app so the next launch opens the archives (iOS has no relaunch API; the player taps
/// the icon again). Settings are written first.
///
/// Vanillish: Android can start the app again — in a fresh process, since a Bevy app is built
/// once per process — so `device::relaunch()` comes first; the Java side ends this process. The
/// exit stays as the fallback (no Activity to relaunch from).
pub(super) fn restart(inilla: &Inilla) {
    super::store::save_settings(&inilla.settings);
    super::store::save_launcher(&inilla.launcher);
    super::store::save_layout(&inilla.layout);
    super::store::save_pad(&inilla.pad);
    info!("inilla: exiting for a relaunch");
    if inilla_platform::device::relaunch() {
        info!("inilla: relaunching");
    }
    std::process::exit(0);
}

/// Vanillish: what one system Back press does on iNilla's screens, most local first — iOS has no
/// Back, and on Android it must reach these screens before the game, since a panel does not
/// block the keyboard and an Escape would close the game's window underneath. `false` when no
/// screen is open: that press is the game's Escape.
fn take_back(overlay: &mut Overlay, data_ready: bool) -> bool {
    let Some(panel) = overlay.panel else {
        return false;
    };
    let forms = overlay.server_form.is_some() || overlay.account_form.is_some();
    match panel {
        // The inline password prompt, then the server or account form: Cancel.
        Panel::Launcher if !forms && overlay.password_prompt.is_some() => {
            overlay.password_prompt = None;
        }
        Panel::Launcher if forms => {
            overlay.server_form = None;
            overlay.account_form = None;
        }
        // The editor's Done.
        Panel::Editor => {
            overlay.panel = None;
            overlay.editor = EditorState::default();
        }
        Panel::Logs => overlay.panel = Some(Panel::Settings),
        // Without the game data there is nothing behind the first screen (Vanillish: Connect, when
        // it was the first one), and the others lead back to it.
        Panel::Connect if !data_ready => {}
        Panel::Setup if !data_ready && !overlay.connect_home => {}
        _ if !data_ready => {
            overlay.panel = Some(if overlay.connect_home {
                Panel::Connect
            } else {
                Panel::Setup
            });
        }
        _ => overlay.panel = None,
    }
    true
}

/// Vanillish: the system Back presses this frame go to iNilla's screens first — an open popup (a
/// combo box) closes, then [`take_back`]; what no screen takes stays in [`BackPresses`] for the
/// game's Escape.
fn back_to_screens(
    mut back: ResMut<BackPresses>,
    mut overlay: ResMut<Overlay>,
    data: Res<DataState>,
    mut contexts: EguiContexts,
) {
    if back.0 == 0 {
        return;
    }
    let ctx = contexts.ctx_mut().ok().cloned();
    while back.0 > 0 {
        // Only a screen's popup: with no screen up, a popup egui still remembers is a stale one.
        let popup = ctx
            .as_ref()
            .filter(|c| overlay.panel.is_some() && egui::Popup::is_any_open(c));
        if let Some(c) = popup {
            egui::Popup::close_all(c);
        } else if !take_back(&mut overlay, data.ready_at_boot) {
            break;
        }
        back.0 -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::{take_back, Overlay, Panel};

    /// Vanillish: Back undoes the most local thing first, and only a bare game gets the Escape.
    #[test]
    fn back_closes_the_innermost_screen_first() {
        let mut o = Overlay {
            panel: Some(Panel::Launcher),
            password_prompt: Some(("a".into(), String::new())),
            ..Default::default()
        };
        assert!(take_back(&mut o, true));
        assert!(o.password_prompt.is_none());
        assert_eq!(o.panel, Some(Panel::Launcher));
        assert!(take_back(&mut o, true));
        assert_eq!(o.panel, None);
        assert!(!take_back(&mut o, true), "no screen: the game's Escape");

        o.panel = Some(Panel::Logs);
        assert!(take_back(&mut o, true));
        assert_eq!(o.panel, Some(Panel::Settings));

        // Without the game data the setup screen stays, and the others lead back to it.
        o.panel = Some(Panel::Settings);
        assert!(take_back(&mut o, false));
        assert_eq!(o.panel, Some(Panel::Setup));
        assert!(take_back(&mut o, false));
        assert_eq!(o.panel, Some(Panel::Setup));

        // Vanillish: Connect first, it stays; the others — the setup screen too — lead back to it.
        o.connect_home = true;
        o.panel = Some(Panel::Connect);
        assert!(take_back(&mut o, false));
        assert_eq!(o.panel, Some(Panel::Connect));
        for from in [Panel::Setup, Panel::Settings, Panel::Launcher] {
            o.panel = Some(from);
            assert!(take_back(&mut o, false));
            assert_eq!(o.panel, Some(Panel::Connect), "from {from:?}");
        }
        // The manual form Connect opened closes first.
        o.panel = Some(Panel::Launcher);
        o.server_form = Some(super::ServerForm::new());
        assert!(take_back(&mut o, false));
        assert!(o.server_form.is_none());
        assert_eq!(o.panel, Some(Panel::Launcher));
        // With the game data, Connect closes like any screen.
        o.panel = Some(Panel::Connect);
        assert!(take_back(&mut o, true));
        assert_eq!(o.panel, None);
    }

    /// Every symbol iNilla's HUD and screens print is in the font egui draws it with: a glyph the
    /// font lacks draws as a box on the phone. Non-ASCII outside comments counts as printed.
    #[test]
    fn every_symbol_the_ui_prints_is_in_its_font() {
        let sources = [
            include_str!("../hud.rs"),
            include_str!("../mod.rs"),
            include_str!("../session.rs"),
            include_str!("../data.rs"),
            include_str!("../boot.rs"),
            include_str!("mod.rs"),
            include_str!("editor.rs"),
            include_str!("launcher.rs"),
            include_str!("logs.rs"),
            include_str!("settings.rs"),
            include_str!("setup.rs"),
            include_str!("words.rs"),
            include_str!("connect.rs"),
            include_str!("../connect.rs"),
            include_str!("../../../../inilla-core/src/connect.rs"),
            include_str!("admin.rs"),
            include_str!("../admin.rs"),
            include_str!("../../../../inilla-core/src/admin.rs"),
            include_str!("../console.rs"),
            include_str!("../windows.rs"),
            include_str!("../../../../inilla-core/src/import.rs"),
            include_str!("../../../../inilla-platform/src/picker.rs"),
            include_str!("../../../../inilla-platform/src/paths.rs"),
            include_str!("../../../../inilla-core/src/pad.rs"),
            include_str!("../../../../inilla-core/src/touch.rs"),
            include_str!("../../../../inilla-core/src/fold.rs"),
            include_str!("../../../../inilla-core/src/addons.rs"),
            include_str!("../../../../inilla-core/src/data.rs"),
            include_str!("../../../../inilla-core/src/profiles.rs"),
        ];
        let ctx = bevy_egui::egui::Context::default();
        ctx.set_fonts(super::fonts());
        let _ = ctx.run(Default::default(), |_| {});
        let font = bevy_egui::egui::FontId::proportional(16.0);
        let mut missing: Vec<char> = Vec::new();
        for line in sources.iter().flat_map(|s| s.lines()) {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            let code = code.split(" // ").next().unwrap_or(code);
            for c in code.chars().filter(|c| !c.is_ascii()) {
                if !missing.contains(&c) && !ctx.fonts_mut(|f| f.has_glyph(&font, c)) {
                    missing.push(c);
                }
            }
        }
        assert!(missing.is_empty(), "no glyph for {missing:?}");
    }
}

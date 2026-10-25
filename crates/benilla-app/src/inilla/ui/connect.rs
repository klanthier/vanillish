//! Vanillish: **Connect** — Android's first screen. One field, the server's address; then, side by
//! side, the game files (left) and the account (right), so every field sits in the top of the
//! cover screen, above the soft keyboard. What the server is, the download chain and the sign-up
//! run in `inilla::connect`; this draws them and takes the taps.
//!
//! The happy path is the address and three taps: Connect (the files start downloading by
//! themselves on Wi-Fi), Create account (a name and a password come generated), Restart — after
//! which the account logs in by itself and the realm is picked by name.

use bevy_egui::egui;
use rand::Rng;

use inilla_core::addons::locale_name;
use inilla_core::connect::{self as kc, DataStep, RegistrationInfo};
use inilla_core::profiles::{Account, ServerProfile};

use super::{hint, ime, restart, words, Overlay, Panel, ServerForm};
use crate::inilla::connect::{
    files_step, make_verifier, room_for, save_account, still_needed, Busy, Chain, Connect, Found,
    Kept,
};
use crate::inilla::data::{DataState, SPARE};
use crate::inilla::hud::GOLD;
use crate::inilla::platform::ImePurpose;
use crate::inilla::session::{self, LoginQueue};
use crate::inilla::Inilla;

/// The account card's two ways in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Tab {
    #[default]
    Create,
    Have,
}

/// What the player has typed on the Connect screen (the screen's state lives in
/// `inilla::connect::Connect`).
#[derive(Clone, Debug, Default)]
pub(crate) struct ConnectForm {
    pub(crate) address: String,
    token: String,
    name: String,
    password: String,
    /// The password is the generated one: shown in clear until the player types their own.
    generated: bool,
    show: bool,
    pub(crate) tab: Tab,
    /// The two checkboxes, stored as their opposite so both start on.
    forget: bool,
    no_auto: bool,
    /// Another account on a server that has one already.
    pub(crate) another: bool,
    /// The fields hold their first values (the generated name and password).
    primed: bool,
    /// What is wrong with what the player typed.
    error: Option<String>,
}

impl ConnectForm {
    /// The launcher's "Create account" on a server Connect set up.
    pub(crate) fn create_another(&mut self) {
        self.another = true;
        self.tab = Tab::Create;
        self.error = None;
    }

    /// An account was saved: the fields are spent, and the next one starts afresh.
    pub(crate) fn account_saved(&mut self) {
        self.another = false;
        self.primed = false;
        self.password.clear();
        self.error = None;
    }

    fn remember(&self) -> bool {
        !self.forget
    }

    fn auto_login(&self) -> bool {
        !self.forget && !self.no_auto
    }
}

/// The address as the player would have typed it, from a base `server.json` answered at: the
/// host alone when the base is that host's default one.
pub(super) fn as_typed(base: &str) -> String {
    match kc::parse_address(base) {
        Ok(a) if kc::parse_address(&a.host).is_ok_and(|b| b.base == a.base) => a.host,
        _ => base.to_string(),
    }
}

/// The first values: the current server's address, a generated name and password.
fn prime(f: &mut ConnectForm, current: Option<&ServerProfile>) {
    if f.primed {
        return;
    }
    f.primed = true;
    if f.address.trim().is_empty() {
        if let Some(s) = current {
            f.address = s.info.as_deref().map_or_else(|| s.host.clone(), as_typed);
        }
    }
    let mut rng = rand::thread_rng();
    let mut bytes = [0u8; 12];
    rng.fill(&mut bytes);
    f.name = kc::suggest_name(rng.gen());
    f.password = kc::suggest_password(&bytes);
    f.generated = true;
    f.show = true;
}

pub(super) fn show(
    ui: &mut egui::Ui,
    overlay: &mut Overlay,
    inilla: &mut Inilla,
    data: &mut DataState,
    connect: &mut Connect,
    queue: &mut LoginQueue,
) {
    // Shared storage did not answer at launch: nothing set up now would be found the next time.
    if inilla_platform::paths::storage_unavailable() {
        ui.label(
            egui::RichText::new(words::STORAGE_UNAVAILABLE)
                .color(egui::Color32::LIGHT_RED)
                .size(18.0),
        );
        return;
    }
    let current = inilla.launcher.current_server().cloned();
    prime(&mut overlay.connect, current.as_ref());
    let here = connect.here(current.as_ref()).cloned();
    // The files ask for the access code: the account card does not ask a second time.
    let files_ask = match (&here, &current) {
        (Some(found), Some(profile)) => {
            let token = connect.token_for(profile);
            let step = files_step(
                found,
                profile,
                data.status.complete(),
                token.is_some(),
                connect.token_refused,
            );
            step == DataStep::AskToken && data.busy().is_none()
        }
        _ => false,
    };
    // Reopened for a server Connect set up before (after a relaunch, from the launcher): ask it
    // again for its sign-up rules and its files, once.
    connect.ask_again(current.as_ref());
    ui.columns(2, |cols| {
        egui::ScrollArea::vertical()
            .id_salt("connect-server")
            .show(&mut cols[0], |ui| {
                server(ui, overlay, connect, here.as_ref(), current.as_ref());
                if let (Some(found), Some(profile)) = (&here, &current) {
                    ui.add_space(6.0);
                    files(ui, overlay, inilla, data, connect, found, profile);
                }
                ui.add_space(10.0);
                other_ways(ui, overlay, connect, current.as_ref());
            });
        egui::ScrollArea::vertical()
            .id_salt("connect-account")
            .show(&mut cols[1], |ui| match (&here, &current) {
                (Some(found), Some(profile)) => {
                    let card = Card {
                        found,
                        profile,
                        files_ask,
                    };
                    account(ui, overlay, inilla, data, connect, queue, &card)
                }
                _ => {
                    ui.label(egui::RichText::new(words::CONNECT_INTRO).size(18.0));
                    hint(ui, words::ADDRESS_HINT);
                }
            });
    });
}

/// The address, Connect, what the server turned out to be.
fn server(
    ui: &mut egui::Ui,
    overlay: &mut Overlay,
    connect: &mut Connect,
    here: Option<&Found>,
    current: Option<&ServerProfile>,
) {
    ui.label(egui::RichText::new(words::ADDRESS_ROW).strong().color(GOLD));
    let probing = connect.busy() == Some(Busy::Probing);
    let mut go = false;
    ui.horizontal(|ui| {
        let width = (ui.available_width() - 130.0).max(160.0);
        let r = ui.add(
            egui::TextEdit::singleline(&mut overlay.connect.address)
                .hint_text(words::ADDRESS_EXAMPLE)
                .desired_width(width),
        );
        ime(&r, ImePurpose::Url);
        go |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let button = egui::Button::new(egui::RichText::new(words::CONNECT_BUTTON).color(GOLD))
            .min_size(egui::vec2(110.0, 40.0));
        go |= ui.add_enabled(!probing, button).clicked();
    });
    if go && connect.busy().is_none() {
        match kc::parse_address(&overlay.connect.address) {
            Ok(addr) => connect.start_probe(addr),
            Err(e) => {
                connect.status = Some((false, e));
                connect.detail = None;
            }
        }
    }
    if probing {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(words::ASKING);
        });
    } else if let Some((ok, text)) = &connect.status {
        ui.colored_label(colour(*ok), text);
        if let Some(why) = &connect.detail {
            hint(ui, why);
        }
    }
    let (Some(found), Some(profile)) = (here, current) else {
        return;
    };
    let realm = found.info.as_ref().and_then(|i| match i.realms.as_slice() {
        [r] if !r.name.trim().is_empty() => Some((r.name.as_str(), r.kind_label())),
        _ => None,
    });
    ui.label(
        egui::RichText::new(words::summary(
            &profile.name,
            profile.core.label(),
            realm,
            profile.is_local(),
        ))
        .color(egui::Color32::LIGHT_GREEN),
    );
    if found.info.is_none() {
        hint(ui, words::DATA_ONLY);
    }
    if !profile.notes.is_empty() {
        hint(ui, &profile.notes);
    }
}

/// The game files: what they need next, the download's progress, the restart that opens them.
fn files(
    ui: &mut egui::Ui,
    overlay: &mut Overlay,
    inilla: &mut Inilla,
    data: &mut DataState,
    connect: &mut Connect,
    found: &Found,
    profile: &ServerProfile,
) {
    ui.label(egui::RichText::new(words::FILES_TITLE).strong().color(GOLD));
    if data.busy().is_some_and(|(label, _)| label == "manifest") {
        asking_files(ui);
        return;
    }
    // A download or a folder import: its progress and Pause, as on the setup screen.
    if super::setup::progress(ui, data) {
        return;
    }
    let token = connect.token_for(profile);
    let step = files_step(
        found,
        profile,
        data.status.complete(),
        token.is_some(),
        connect.token_refused,
    );
    match step {
        DataStep::Have => {
            let st = &data.status;
            ui.label(
                egui::RichText::new(words::files_ready(
                    st.present.len(),
                    st.total_bytes() as f64 / 1e9,
                ))
                .color(egui::Color32::LIGHT_GREEN),
            );
            if data.needs_restart() {
                data_message(ui, data);
                ui.label(words::NEEDS_RESTART);
                let button = egui::Button::new(egui::RichText::new(words::RESTART_NOW).color(GOLD))
                    .min_size(egui::vec2(220.0, 44.0));
                if ui.add(button).clicked() {
                    inilla.settings.setup_done = true;
                    restart(inilla);
                }
                hint(ui, super::relaunch_hint());
            }
        }
        DataStep::BringYourOwn => {
            ui.label(words::BRING_YOUR_OWN);
            if inilla_platform::picker::available() {
                let button =
                    egui::Button::new(egui::RichText::new(words::IMPORT_FOLDER).color(GOLD))
                        .min_size(egui::vec2(220.0, 44.0));
                if ui.add(button).clicked() && !inilla_platform::picker::pick() {
                    data.message = Some((false, "The folder picker could not open.".into()));
                }
            }
            data_message(ui, data);
        }
        DataStep::PickLanguage(locales) => {
            ui.label(words::PICK_LANGUAGE);
            ui.horizontal_wrapped(|ui| {
                for l in &locales.locales {
                    let name = locale_name(&l.locale);
                    let text = if l.bytes > 0 {
                        format!("{name} · {:.1} GB", l.bytes as f64 / 1e9)
                    } else {
                        name.to_string()
                    };
                    if ui
                        .add(egui::Button::new(text).min_size(egui::vec2(140.0, 40.0)))
                        .clicked()
                    {
                        if let Some(d) = inilla
                            .launcher
                            .server_mut(&found.id)
                            .and_then(|s| s.data.as_mut())
                        {
                            d.locale = Some(l.locale.clone());
                        }
                    }
                }
            });
        }
        DataStep::AskToken => {
            if connect.token_refused {
                ui.colored_label(egui::Color32::LIGHT_RED, words::TOKEN_REFUSED_HERE);
            }
            token_field(ui, overlay, connect, &found.id);
            hint(ui, words::TOKEN_HINT);
        }
        DataStep::Fetch { .. } => download(ui, data, connect),
    }
}

fn asking_files(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.spinner();
        ui.label(words::ASKING_FILES);
    });
}

/// The download chain's state, and the tap it waits for.
fn download(ui: &mut egui::Ui, data: &mut DataState, connect: &mut Connect) {
    let Some(p) = connect.pending.as_mut() else {
        asking_files(ui);
        return;
    };
    match p.state {
        Chain::Ask | Chain::Asking | Chain::Downloading => asking_files(ui),
        Chain::Offer => {
            let need = still_needed(data).unwrap_or(0);
            let room = room_for(data, need);
            if p.metered == Some(true) {
                ui.colored_label(egui::Color32::YELLOW, words::METERED);
            }
            let button = egui::Button::new(
                egui::RichText::new(words::download_game(need as f64 / 1e9)).color(GOLD),
            )
            .min_size(egui::vec2(220.0, 44.0));
            if ui.add_enabled(room, button).clicked() {
                data.download(p.token.clone());
                p.state = Chain::Downloading;
            }
            if let (false, Some(free)) = (room, data.free_bytes()) {
                ui.colored_label(
                    egui::Color32::LIGHT_RED,
                    words::not_enough_space((need + SPARE) as f64 / 1e9, free as f64 / 1e9),
                );
            }
            hint(ui, words::KEEP_OPEN);
        }
        Chain::Stopped => {
            data_message(ui, data);
            let button = egui::Button::new(egui::RichText::new(words::DOWNLOAD_RESUME).color(GOLD))
                .min_size(egui::vec2(220.0, 44.0));
            if ui.add(button).clicked() {
                data.download(p.token.clone());
                p.state = Chain::Downloading;
            }
        }
        Chain::Failed => {
            data_message(ui, data);
            if ui
                .add(egui::Button::new(words::TRY_AGAIN).min_size(egui::vec2(160.0, 44.0)))
                .clicked()
            {
                p.state = Chain::Ask;
            }
        }
    }
}

/// The last data job's sentence.
fn data_message(ui: &mut egui::Ui, data: &DataState) {
    if let Some((ok, msg)) = &data.message {
        ui.colored_label(colour(*ok), msg);
    }
}

fn colour(ok: bool) -> egui::Color32 {
    if ok {
        egui::Color32::LIGHT_GREEN
    } else {
        egui::Color32::LIGHT_RED
    }
}

/// The access code: typed once, sealed in the Keystore for this server.
fn token_field(ui: &mut egui::Ui, overlay: &mut Overlay, connect: &mut Connect, server: &str) {
    ui.label(words::TOKEN_ROW);
    ui.horizontal(|ui| {
        let width = (ui.available_width() - 130.0).max(140.0);
        let r = ui.add(
            egui::TextEdit::singleline(&mut overlay.connect.token)
                .password(true)
                .desired_width(width),
        );
        ime(&r, ImePurpose::Password);
        let typed = !overlay.connect.token.trim().is_empty();
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let button = egui::Button::new(words::CONTINUE).min_size(egui::vec2(110.0, 40.0));
        if (ui.add_enabled(typed, button).clicked() || enter) && typed {
            connect.set_token(server, &overlay.connect.token);
            overlay.connect.token.clear();
        }
    });
}

/// Enter the details by hand, or bring the files another way.
fn other_ways(
    ui: &mut egui::Ui,
    overlay: &mut Overlay,
    connect: &mut Connect,
    current: Option<&ServerProfile>,
) {
    ui.horizontal_wrapped(|ui| {
        if ui.button(words::MANUAL).clicked() {
            overlay.server_form = Some(ServerForm::new());
            overlay.panel = Some(Panel::Launcher);
        }
        if ui.button(words::OTHER_WAYS).clicked() {
            // The setup screen's download form starts at this server's files, with its code.
            if let Some(s) = current {
                if let Some(d) = &s.data {
                    overlay.data_url = d.url.clone();
                }
                if let Some(t) = connect.token_for(s) {
                    overlay.data_token = t;
                }
            }
            overlay.panel = Some(Panel::Setup);
        }
    });
}

/// What the account card is about.
struct Card<'a> {
    found: &'a Found,
    profile: &'a ServerProfile,
    /// The files column is asking for the access code already.
    files_ask: bool,
}

/// The account card: the saved account and Play, or Create an account / I have an account.
fn account(
    ui: &mut egui::Ui,
    overlay: &mut Overlay,
    inilla: &mut Inilla,
    data: &DataState,
    connect: &mut Connect,
    queue: &mut LoginQueue,
    card: &Card,
) {
    let (found, profile) = (card.found, card.profile);
    ui.label(
        egui::RichText::new(format!("{} on {}", words::ACCOUNT_TITLE, profile.name))
            .strong()
            .color(GOLD),
    );
    let saved = inilla
        .launcher
        .current_account()
        .filter(|a| a.profile == found.id)
        .cloned();
    if let (Some(a), false) = (&saved, overlay.connect.another) {
        if let Some((ok, text)) = &connect.account_status {
            ui.colored_label(colour(*ok), text);
        }
        ui.label(
            egui::RichText::new(words::account_saved(&a.username))
                .color(egui::Color32::LIGHT_GREEN)
                .size(18.0),
        );
        play(ui, overlay, data, connect, queue, a);
        ui.add_space(8.0);
        if ui.button(words::ANOTHER_ACCOUNT).clicked() {
            overlay.connect.another = true;
            connect.account_status = None;
        }
        return;
    }
    let rules = found.info.as_ref().and_then(|i| i.sign_up_rules()).cloned();
    let url = found
        .info
        .as_ref()
        .and_then(|i| i.register_url(&found.base));
    let f = &mut overlay.connect;
    match (&rules, &url) {
        (Some(_), Some(_)) => {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut f.tab, Tab::Create, words::CREATE_TAB);
                ui.selectable_value(&mut f.tab, Tab::Have, words::HAVE_TAB);
            });
        }
        _ => f.tab = Tab::Have,
    }
    match (f.tab, rules, url) {
        (Tab::Create, Some(rules), Some(url)) => create(ui, overlay, connect, card, &rules, url),
        _ => have(ui, overlay, inilla, connect, found),
    }
    if let Some((ok, text)) = &connect.account_status {
        ui.colored_label(colour(*ok), text);
    }
    let registration = found.info.as_ref().and_then(|i| i.registration.as_ref());
    if !registration.is_some_and(RegistrationInfo::usable) {
        hint(ui, words::NO_SIGN_UP_HERE);
    }
    if let Some(note) = registration
        .and_then(|r| r.note.as_deref())
        .filter(|n| !n.trim().is_empty())
    {
        hint(ui, note);
    }
}

/// Log in now when the archives are open; otherwise say when it happens by itself.
fn play(
    ui: &mut egui::Ui,
    overlay: &mut Overlay,
    data: &DataState,
    connect: &Connect,
    queue: &mut LoginQueue,
    a: &Account,
) {
    if data.ready_at_boot && !data.needs_restart() {
        let button =
            egui::Button::new(egui::RichText::new(words::play_as(&a.username)).color(GOLD))
                .min_size(egui::vec2(220.0, 44.0));
        if ui.add(button).clicked() {
            // The password this flow saved, else the Keystore's (asked on the tap only).
            let pass = connect
                .password
                .as_ref()
                .filter(|(n, _)| n.eq_ignore_ascii_case(&a.username))
                .map(|(_, p)| p.clone())
                .or_else(|| session::saved_password(a));
            match pass {
                Some(pass) => {
                    queue.request(&a.username, &pass);
                    overlay.panel = None;
                }
                None => {
                    overlay.password_prompt = Some((a.id.clone(), String::new()));
                    overlay.panel = Some(Panel::Launcher);
                }
            }
        }
    } else if a.auto_login && data.needs_restart() {
        hint(ui, &words::logs_in_after_restart(&a.username));
    } else if a.auto_login {
        hint(ui, &words::logs_in_later(&a.username));
    }
}

/// A new account: the generated name and password (both editable), checked as typed, signed up
/// with the verifier made here.
fn create(
    ui: &mut egui::Ui,
    overlay: &mut Overlay,
    connect: &mut Connect,
    card: &Card,
    rules: &RegistrationInfo,
    url: String,
) {
    let profile = card.profile;
    let (name_min, name_max) = rules.name_range();
    let (pass_min, pass_max) = rules.password_range();
    let width = ui.available_width().min(320.0);
    let f = &mut overlay.connect;
    if f.name.is_empty() && f.password.is_empty() {
        // Back from "I have an account", which emptied them: generated afresh next frame.
        f.primed = false;
    }
    ui.label(words::NAME_ROW);
    let r = ui.add(
        egui::TextEdit::singleline(&mut f.name)
            .char_limit(name_max)
            .desired_width(width),
    );
    ime(&r, ImePurpose::Password);
    hint(ui, &words::name_hint(name_min, name_max));
    password_row(ui, f, pass_max, width);
    hint(ui, &words::password_hint(pass_min, pass_max));
    let token = connect.token_for(profile);
    if rules.token && (token.is_none() || connect.token_refused) && !card.files_ask {
        token_field(ui, overlay, connect, &profile.id);
    }
    let f = &mut overlay.connect;
    checkboxes(ui, f);
    if let Some(e) = &f.error {
        ui.colored_label(egui::Color32::LIGHT_RED, e);
    }
    if connect.busy() == Some(Busy::SigningUp) {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(words::CREATING);
        });
        return;
    }
    let button = egui::Button::new(egui::RichText::new(words::CREATE_ACCOUNT).color(GOLD))
        .min_size(egui::vec2(200.0, 44.0));
    if !ui.add_enabled(connect.busy().is_none(), button).clicked() {
        return;
    }
    // The same name and password as the sign-up sent before: sent again unchanged, so a lost
    // answer is a replay rather than "taken".
    let name = f.name.trim().to_string();
    let kept = match connect
        .kept
        .clone()
        .filter(|k| k.name == name && k.password == f.password)
    {
        Some(k) => Ok(k),
        None => kc::sign_up(rules, &name, &f.password, &make_verifier).map(|sign_up| Kept {
            name: name.clone(),
            password: f.password.clone(),
            sign_up,
            remember: true,
            auto_login: true,
        }),
    };
    match kept {
        Ok(mut k) => {
            k.remember = f.remember();
            k.auto_login = f.auto_login();
            f.error = None;
            let token = connect.token_for(profile);
            connect.start_sign_up(url, token, k);
        }
        Err(e) => f.error = Some(words::register_error(&e)),
    }
}

/// An account the player has already: saved as it is, nothing sent (a wrong password shows at the
/// login, where realmd says so — checking it here would cost a login attempt).
fn have(
    ui: &mut egui::Ui,
    overlay: &mut Overlay,
    inilla: &mut Inilla,
    connect: &mut Connect,
    found: &Found,
) {
    let width = ui.available_width().min(320.0);
    let f = &mut overlay.connect;
    if f.generated {
        // Not this account's: the generated ones were for a new one.
        f.name.clear();
        f.password.clear();
        f.generated = false;
        f.show = false;
    }
    ui.label(words::NAME_ROW);
    let r = ui.add(
        egui::TextEdit::singleline(&mut f.name)
            .char_limit(kc::MAX_ACCOUNT_LEN)
            .desired_width(width),
    );
    ime(&r, ImePurpose::Password);
    password_row(ui, f, kc::MAX_ACCOUNT_LEN, width);
    checkboxes(ui, f);
    if let Some(e) = &f.error {
        ui.colored_label(egui::Color32::LIGHT_RED, e);
    }
    let button = egui::Button::new(egui::RichText::new(words::SAVE_ACCOUNT).color(GOLD))
        .min_size(egui::vec2(140.0, 44.0));
    if !ui.add(button).clicked() {
        return;
    }
    let name = f.name.trim().to_string();
    if name.is_empty() {
        f.error = Some(words::NAME_NEEDED.into());
        return;
    }
    if f.remember() && f.password.is_empty() {
        f.error = Some(words::PASSWORD_NEEDED.into());
        return;
    }
    let password = std::mem::take(&mut f.password);
    if !save_account(
        inilla,
        &found.id,
        &name,
        &password,
        f.remember(),
        f.auto_login(),
    ) {
        overlay.toast(words::NOT_SAVED);
    }
    connect.password = Some((name, password));
    connect.account_status = None;
    overlay.connect.account_saved();
}

fn password_row(ui: &mut egui::Ui, f: &mut ConnectForm, max: usize, width: f32) {
    ui.label(words::PASSWORD_ROW);
    ui.horizontal(|ui| {
        let r = ui.add(
            egui::TextEdit::singleline(&mut f.password)
                .password(!f.show)
                .char_limit(max)
                .desired_width((width - 90.0).max(120.0)),
        );
        ime(&r, ImePurpose::Password);
        if r.changed() && f.generated {
            // The player's own now: hidden like any password.
            f.generated = false;
            f.show = false;
        }
        let label = if f.show {
            words::HIDE_PASSWORD
        } else {
            words::SHOW_PASSWORD
        };
        if ui.button(label).clicked() {
            f.show = !f.show;
        }
    });
}

fn checkboxes(ui: &mut egui::Ui, f: &mut ConnectForm) {
    let mut remember = f.remember();
    ui.checkbox(&mut remember, words::REMEMBER_PASSWORD);
    f.forget = !remember;
    let mut auto = f.auto_login();
    ui.add_enabled(remember, egui::Checkbox::new(&mut auto, words::AUTO_LOGIN));
    if remember {
        f.no_auto = !auto;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A base `server.json` answered at reads back as what the player typed.
    #[test]
    fn the_address_field_shows_what_was_typed() {
        assert_eq!(as_typed("http://192.168.1.20:8099/"), "192.168.1.20");
        assert_eq!(as_typed("http://[fd00::2]:8099/"), "fd00::2");
        assert_eq!(as_typed("http://nas.local:9000/"), "http://nas.local:9000/");
        assert_eq!(
            as_typed("https://wow.example.com/v/"),
            "https://wow.example.com/v/"
        );
    }

    /// The first draw fills the form with what passes the default rules, and both boxes on.
    #[test]
    fn a_primed_form_can_create_at_once() {
        let mut f = ConnectForm::default();
        let mut p = ServerProfile::new("home", "10.0.0.5");
        p.info = Some("http://10.0.0.5:8099/".into());
        prime(&mut f, Some(&p));
        assert_eq!(f.address, "10.0.0.5");
        let rules = RegistrationInfo::default();
        assert!(rules.check_name(&f.name).is_ok(), "{}", f.name);
        assert!(rules.check_password(&f.password).is_ok(), "{}", f.password);
        assert!(f.show && f.remember() && f.auto_login());
        // Primed once: what the player typed stays.
        f.address = "typed".into();
        prime(&mut f, Some(&p));
        assert_eq!(f.address, "typed");
        f.account_saved();
        assert!(f.password.is_empty() && !f.primed);
    }
}

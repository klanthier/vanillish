//! Vanillish: **Settings → Server**, the server admin menu (off iOS) — the realm's experience,
//! loot, gold and speed rates for everyone, live and kept across restarts, and gold for players
//! by mail. The requests run in `inilla::admin`; this draws what it holds and takes the taps.
//!
//! Every slider is logarithmic, its range widened to take in the live value and clamped on edits
//! only, so drawing never moves a value the server holds; a group's slider says "mixed" while its
//! knobs differ. Nothing is sent until Apply, and Server defaults and Send take two taps.

use std::ops::RangeInclusive;

use bevy_egui::egui;

use inilla_core::admin::{
    self as ka, AdminError, GoldReply, GoldTo, Group, Knob, Rate, Section, Status,
};
use inilla_core::connect as kc;

use super::connect::as_typed;
use super::{hint, ime, words};
use crate::inilla::admin::{Admin, Busy, GoldForm, Menu, Note};
use crate::inilla::hud::GOLD;
use crate::inilla::platform::ImePurpose;
use crate::inilla::Inilla;

const ASKING: &str = "Asking…";
const NO_SERVER: &str = "No server yet: add one in Servers first.";
const NO_MENU: &str =
    "This server has no admin menu: update it (run its setup again), and set ADMIN_CODE.";
const NOT_HERE: &str = "The admin menu answers only on the server's own network.";
const OFF: &str = "The admin menu is off on this server.";
const STARTING: &str = "The world server is starting…";
const WRONG_CODE: &str = "That is not this server's admin code.";
/// A 429 answers a wrong code only (a right one always passes): no wait would help.
const TOO_MANY: &str = "Too many wrong codes from here lately: check the admin code (ADMIN_CODE \
     in the server's .env).";
/// A 429 not the data host's own: a limit in front of it (a proxy's).
const THROTTLED: &str = "Too many requests from here lately: wait a moment and try again.";
const NO_ANSWER: &str =
    "No answer in time: it may still have been done. Refresh before trying again.";
const GOLD_NO_ANSWER: &str =
    "No answer in time: the gold may still arrive. Check a mailbox before sending again.";
const OLDER: &str = "This server is older than the admin menu: update it (run its setup again).";
const NEWER: &str = "This server is newer than this app: update the app to use its admin menu.";
const UNSIGNED: &str = "The server did not see this request's signature: a proxy in front of it \
     may drop the Authorization header.";
const STALE_NONCE: &str = "The server took the request for an old one, twice: try again.";
const CODE_INTRO: &str =
    "The admin code is in the server's .env (ADMIN_CODE), and at the end of its setup.";
const CODE_EXAMPLE: &str = "ABCD-EFGH-JKLM-NPQR";
const FORGET: &str = "Forget the code";
const APPLY_HINT: &str =
    "Applying also resets the message of the day and the player limit to the server's settings.";
const LOOT_HINT: &str = "From the next kill. Boss and dungeon loot that drops one item from a \
     list does not change.";
const NO_SPEED: &str =
    "This server's vmangos has no speed setting: run its setup again to update it.";
const TELL: &str = "Tell the players in chat";
const DEFAULTS: &str = "Server defaults";
const CHANGE: &str = "Change";
const ADDRESS: &str = "Server address";
const USE: &str = "Use";
const CANCEL: &str = "Cancel";
const ADDRESS_HINT: &str = "Where the admin menu is asked, as you would type it in Connect: an IP \
     (192.168.1.20) or a name (my-mac.local), port 8099 unless you add another. Connect asks there \
     too; the realm's own address is in Servers → Edit.";
const NOBODY: &str = "Nobody is online: a name below still gets it, by mail.";

pub(super) fn show(ui: &mut egui::Ui, admin: &mut Admin, inilla: &mut Inilla) {
    if admin.server_id.is_none() {
        ui.label(NO_SERVER);
        return;
    }
    header(ui, admin);
    address_field(ui, admin, inilla);
    status_line(ui, admin);
    if !matches!(admin.menu, Some(Menu::At(_))) {
        return;
    }
    if !admin.has_code() {
        code_field(ui, admin);
        return;
    }
    if let Some(status) = admin.status.clone() {
        ui.add_space(6.0);
        rates(ui, admin, &status);
        ui.separator();
        gold(ui, admin, &status);
    }
    ui.add_space(12.0);
    // Not while a request signed with it runs: its 401 would be about the code.
    let forget = egui::Button::new(FORGET).small();
    if ui.add_enabled(admin.busy().is_none(), forget).clicked() {
        admin.forget_code();
    }
}

/// The server's name and where it is asked (a tap on it, or Change, edits it), and Refresh.
fn header(ui: &mut egui::Ui, admin: &mut Admin) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(&admin.name).strong().color(GOLD));
        let at = as_typed(&admin.base);
        let tapped = ui
            .add(
                egui::Label::new(egui::RichText::new(&at).small().color(egui::Color32::GRAY))
                    .sense(egui::Sense::click()),
            )
            .clicked();
        let change = ui.add_enabled(admin.can_move(), egui::Button::new(CHANGE).small());
        if admin.address.is_none() && admin.can_move() && (tapped || change.clicked()) {
            admin.edit_address(at);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_enabled(admin.busy().is_none(), egui::Button::new("Refresh"))
                .clicked()
            {
                admin.want = true;
            }
        });
    });
}

/// The address being changed: the field takes the focus as it opens, so the keyboard comes up;
/// Use (or Enter) makes it the server's, Cancel leaves the old one.
fn address_field(ui: &mut egui::Ui, admin: &mut Admin, inilla: &mut Inilla) {
    let can = admin.can_move();
    let focus = std::mem::take(&mut admin.address_focus);
    let Some(typed) = admin.address.as_mut() else {
        return;
    };
    let mut done = None;
    ui.horizontal(|ui| {
        ui.label(ADDRESS);
        let width = (ui.available_width() - 190.0).clamp(160.0, 320.0);
        let r = ui.add(
            egui::TextEdit::singleline(typed)
                .hint_text("192.168.1.20")
                .desired_width(width),
        );
        // The focus first, then the purpose: the keyboard opens on the address layout at once.
        if focus {
            r.request_focus();
        }
        ime(&r, ImePurpose::Url);
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let button = egui::Button::new(egui::RichText::new(USE).color(GOLD))
            .min_size(egui::vec2(80.0, 40.0));
        if (ui.add_enabled(can, button).clicked() || enter) && can {
            done = Some(true);
        }
        if ui.button(CANCEL).clicked() {
            done = Some(false);
        }
    });
    match done {
        Some(true) => admin.use_address(&mut inilla.launcher),
        Some(false) => {
            admin.address = None;
            admin.address_error = None;
        }
        None => {}
    }
    if let Some(e) = &admin.address_error {
        ui.colored_label(egui::Color32::LIGHT_RED, e);
    }
    if admin.address.is_some() {
        hint(ui, ADDRESS_HINT);
    }
}

/// One line for where things stand, the network's or the server's own reason under it, and the
/// last change's outcome.
fn status_line(ui: &mut egui::Ui, admin: &Admin) {
    if admin.busy().is_some() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(ASKING);
        });
    } else if let Some((ok, text, why)) = line(admin) {
        ui.colored_label(colour(ok), text);
        if let Some(why) = why {
            hint(ui, &why);
        }
    }
    let note = match &admin.note {
        Some(Note::Applied(changed)) if changed.is_empty() => Some((
            true,
            "Nothing to change: the server has these values already.",
        )),
        Some(Note::Applied(_)) => Some((
            true,
            "Applied: live now, and kept when the server restarts.",
        )),
        Some(Note::Reset(_)) => Some((true, "Back to the server's own settings.")),
        Some(Note::NotSaved) => Some((false, words::ADMIN_CODE_NOT_SAVED)),
        None => None,
    };
    if let Some((ok, text)) = note {
        ui.colored_label(colour(ok), text);
    }
}

fn line(admin: &Admin) -> Option<(bool, String, Option<String>)> {
    if let Some(e) = &admin.error {
        return Some(error_line(e, &admin.base, false));
    }
    match admin.menu.as_ref()? {
        Menu::Absent => Some((false, NO_MENU.into(), None)),
        Menu::NotHere => Some((false, NOT_HERE.into(), Some("INILLA_ADMIN=lan".into()))),
        Menu::At(_) => {
            let s = admin.status.as_ref()?;
            if !s.world_up() {
                return Some((false, STARTING.into(), None));
            }
            let up = match s.players.len() {
                _ if !s.players_ok => {
                    "The world server is up (its player list did not come).".into()
                }
                0 => "The world server is up: nobody is online.".into(),
                1 => "The world server is up: 1 player online.".into(),
                n => format!("The world server is up: {n} players online."),
            };
            Some((true, up, None))
        }
    }
}

/// What a refusal says: the sentence, and the raw reason as a hint. `gold`: a send's, which a
/// refresh says nothing about.
fn error_line(e: &AdminError, base: &str, gold: bool) -> (bool, String, Option<String>) {
    let text = |s: &str| s.to_string();
    match e {
        AdminError::Unreachable(why) => {
            let host = kc::parse_address(base).map_or_else(|_| base.to_string(), |a| a.host);
            (false, words::unreachable(&host), Some(why.clone()))
        }
        AdminError::NoAnswer => (false, text(NO_ANSWER), None),
        AdminError::Off => (false, text(OFF), Some("INILLA_ADMIN=off".into())),
        AdminError::Closed => (false, text(NOT_HERE), Some("INILLA_ADMIN=lan".into())),
        AdminError::WrongCode => (false, text(WRONG_CODE), None),
        AdminError::Nonce => (false, text(STALE_NONCE), None),
        AdminError::Unsigned => (false, text(UNSIGNED), None),
        AdminError::TooMany { .. } => (false, text(TOO_MANY), None),
        AdminError::Throttled => (false, text(THROTTLED), Some("429".into())),
        AdminError::WorldDown(why) => (false, text(STARTING), Some(why.clone())),
        AdminError::Refused { code, message } => {
            let message = message.trim().trim_end_matches('.');
            // The console's command may still run: what it did shows on the next status — unless
            // the server's sentence says what to do, or that nothing was done.
            let said = message.contains("Refresh") || message.contains("nothing was sent");
            let text = match code.as_str() {
                "console_timeout" | "internal" if !gold && !said => {
                    format!("{message}. Refresh to see what took.")
                }
                _ => format!("{message}."),
            };
            (false, text, Some(code.clone()))
        }
        AdminError::Older(status) => (
            false,
            text(OLDER),
            Some(format!(
                "it answered {status} where the admin menu should be"
            )),
        ),
        AdminError::Newer(format) => (
            false,
            text(NEWER),
            Some(format!(
                "server.json is in format {format}; this app reads {}",
                kc::SERVER_INFO_FORMAT
            )),
        ),
    }
}

fn colour(ok: bool) -> egui::Color32 {
    if ok {
        egui::Color32::LIGHT_GREEN
    } else {
        egui::Color32::LIGHT_RED
    }
}

/// The admin code: typed once (dashes optional), checked here, sealed in the Keystore for this
/// server. Nothing is sent before Continue.
fn code_field(ui: &mut egui::Ui, admin: &mut Admin) {
    ui.label(CODE_INTRO);
    ui.horizontal(|ui| {
        let width = (ui.available_width() - 130.0).clamp(160.0, 320.0);
        let r = ui.add(
            egui::TextEdit::singleline(&mut admin.typed)
                .password(true)
                .hint_text(CODE_EXAMPLE)
                .desired_width(width),
        );
        ime(&r, ImePurpose::Password);
        // Not while a request runs: its answer would be about the code before.
        let can = !admin.typed.trim().is_empty() && admin.busy().is_none();
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let button = egui::Button::new(egui::RichText::new(words::CONTINUE).color(GOLD))
            .min_size(egui::vec2(110.0, 40.0));
        if (ui.add_enabled(can, button).clicked() || enter) && can {
            admin.set_code();
        }
    });
    if let Some(e) = &admin.code_error {
        ui.colored_label(egui::Color32::LIGHT_RED, e);
    }
    hint(ui, words::ADMIN_CODE_KEPT);
}

/// The three groups of sliders, then Tell the players, Apply / Undo and Server defaults.
fn rates(ui: &mut egui::Ui, admin: &mut Admin, status: &Status) {
    // A refresh leaves the sliders be; a change on its way, or a world not up, does not.
    let live = admin.world_up() && !matches!(admin.busy(), Some(Busy::Applying | Busy::Resetting));
    for (section, title) in [
        (Section::Experience, "Experience"),
        (Section::Loot, "Loot"),
        (Section::Movement, "Movement"),
    ] {
        ui.label(egui::RichText::new(title).strong().color(GOLD));
        let mut any = false;
        for g in ka::GROUPS.iter().filter(|g| g.section == section) {
            any |= group(ui, admin, g, status, live);
        }
        match section {
            Section::Loot => hint(ui, LOOT_HINT),
            Section::Movement if !any => hint(ui, NO_SPEED),
            _ => {}
        }
    }
    ui.add_space(6.0);
    let mut tell = !admin.quiet;
    ui.checkbox(&mut tell, TELL);
    admin.quiet = !tell;
    let dirty = ka::dirty(&admin.draft, status);
    let can = admin.can_change();
    ui.horizontal_wrapped(|ui| {
        if dirty {
            let apply = egui::Button::new(egui::RichText::new("Apply").color(GOLD))
                .min_size(egui::vec2(140.0, 44.0));
            if ui.add_enabled(can, apply).clicked() {
                admin.apply();
            }
            let undo = egui::Button::new("Undo").min_size(egui::vec2(90.0, 44.0));
            if ui.add_enabled(admin.busy().is_none(), undo).clicked() {
                admin.undo();
            }
        }
        if status.overrides > 0 {
            let label = if admin.confirm_reset {
                egui::RichText::new(words::DEFAULTS_AGAIN).color(egui::Color32::LIGHT_RED)
            } else {
                egui::RichText::new(DEFAULTS)
            };
            let button = egui::Button::new(label).min_size(egui::vec2(44.0, 44.0));
            if ui.add_enabled(can, button).clicked() {
                if admin.confirm_reset {
                    admin.reset();
                } else {
                    admin.confirm_reset = true;
                }
            }
        }
    });
    if dirty {
        hint(ui, APPLY_HINT);
    }
}

/// One group's slider — one knob under its own label — and its knobs one by one; `false` when the
/// server has none of them.
fn group(ui: &mut egui::Ui, admin: &mut Admin, g: &Group, status: &Status, live: bool) -> bool {
    let (Some((value, mixed)), Some(range)) = (
        ka::group_value(g, &admin.draft),
        ka::group_range(g, &admin.draft, status),
    ) else {
        return false;
    };
    let knobs: Vec<&'static Knob> = g
        .keys
        .iter()
        .filter(|k| admin.draft.contains_key(**k) && status.rates.contains_key(**k))
        .filter_map(|k| ka::knob(k))
        .collect();
    let Some(first) = knobs.first() else {
        return false;
    };
    let label = if g.keys.len() == 1 {
        first.label
    } else {
        g.label
    };
    let mut v = value;
    labelled(ui, label, |ui| {
        if ui
            .add_enabled(live, slider(&mut v, range, first.step, mixed))
            .changed()
        {
            let v = snap(v, first.step);
            for k in &knobs {
                admin.draft.insert(k.key.to_string(), v);
            }
        }
    });
    server_hint(ui, knobs.iter().filter_map(|k| status.rates.get(k.key)));
    if g.keys.len() > 1 {
        egui::CollapsingHeader::new("Separately")
            .id_salt(("admin-separately", g.label))
            .default_open(mixed)
            .show(ui, |ui| {
                for k in &knobs {
                    knob_row(ui, admin, k, status, live);
                }
            });
    }
    true
}

fn knob_row(ui: &mut egui::Ui, admin: &mut Admin, k: &Knob, status: &Status, live: bool) {
    let (Some(rate), Some(v)) = (status.rates.get(k.key), admin.draft.get_mut(k.key)) else {
        return;
    };
    let range = ka::slider_range(k, *v, rate);
    labelled(ui, k.label, |ui| {
        if ui
            .add_enabled(live, slider(v, range, k.step, false))
            .changed()
        {
            *v = snap(*v, k.step);
        }
    });
    server_hint(ui, std::iter::once(rate));
}

/// `server ×1` under a value the live conf overrides.
fn server_hint<'a>(ui: &mut egui::Ui, rates: impl Iterator<Item = &'a Rate>) {
    let rates: Vec<&Rate> = rates.collect();
    if !rates.iter().any(|r| r.overridden()) {
        return;
    }
    let server = rates[0].server;
    if rates
        .iter()
        .all(|r| (r.server - server).abs() <= ka::TOLERANCE)
    {
        hint(ui, &format!("server {}", times(server)));
    } else {
        hint(ui, "the server's own values differ");
    }
}

/// A row whose label wraps within the settings' 200-point label column.
fn labelled(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_width(200.0);
            ui.add(egui::Label::new(label).wrap());
        });
        add(ui);
    });
}

/// A rate's slider: log scale, never clamping a value it did not set, shown as `×1.5`.
fn slider(value: &mut f64, range: RangeInclusive<f64>, step: f64, mixed: bool) -> egui::Slider<'_> {
    egui::Slider::new(value, range)
        .logarithmic(true)
        .smallest_positive(step)
        .clamping(egui::SliderClamping::Edits)
        .custom_formatter(move |v, _| if mixed { "mixed".into() } else { times(v) })
        .custom_parser(|s| {
            s.trim()
                .trim_start_matches(['×', 'x', 'X'])
                .trim()
                .parse()
                .ok()
        })
}

/// A moved value on its knob's step (0.05, the speed's 0.01), counted from zero.
fn snap(v: f64, step: f64) -> f64 {
    ((v / step).round() * step * 100.0).round() / 100.0
}

/// `×1.5`, `×2`, `×0.75`.
fn times(v: f64) -> String {
    let s = format!("{v:.2}");
    format!("×{}", s.trim_end_matches('0').trim_end_matches('.'))
}

/// Give gold: online players, another name, an amount in whole gold; Send takes two taps.
fn gold(ui: &mut egui::Ui, admin: &mut Admin, status: &Status) {
    ui.label(egui::RichText::new("Give gold").strong().color(GOLD));
    let can = admin.can_change();
    let base = admin.base.clone();
    let f = &mut admin.gold;
    ui.checkbox(&mut f.everyone, "Everyone online");
    if !status.players.is_empty() {
        ui.horizontal_wrapped(|ui| {
            for p in &status.players {
                let on = f.everyone || f.picked.contains(&p.name);
                let button = egui::Button::selectable(on, format!("{} · {}", p.name, p.level));
                if ui.add_enabled(!f.everyone, button).clicked() && !f.picked.remove(&p.name) {
                    f.picked.insert(p.name.clone());
                }
            }
        });
    } else if status.players_ok {
        hint(ui, NOBODY);
    }
    labelled(ui, "Other name", |ui| {
        let r = ui.add_enabled(
            !f.everyone,
            egui::TextEdit::singleline(&mut f.other)
                .char_limit(12)
                .desired_width(200.0),
        );
        ime(&r, ImePurpose::Password);
    });
    let other = match f.other.trim() {
        "" => Ok(None),
        _ if f.everyone => Ok(None),
        typed => ka::check_name(typed).map(Some),
    };
    if let Err(e) = &other {
        ui.colored_label(egui::Color32::LIGHT_RED, e);
    }
    ui.horizontal_wrapped(|ui| {
        for g in ka::GOLD_STEPS {
            let button = egui::Button::selectable(f.amount == g, ka::format_gold(ka::gold(g)))
                .min_size(egui::vec2(70.0, 40.0));
            if ui.add(button).clicked() {
                f.amount = g;
            }
        }
    });
    labelled(ui, "Amount", |ui| {
        ui.add(
            egui::Slider::new(&mut f.amount, ka::GOLD_RANGE)
                .logarithmic(true)
                .custom_formatter(|v, _| ka::format_gold(ka::gold(v.round() as u32))),
        );
    });
    // The choices of a send kept for a replay (no answer, or a refusal that did not rule it
    // out): that send again, to its names.
    let chosen = other.ok().map(|o| {
        let choice = f.choice(o.clone());
        let to = match f.replay(&choice) {
            Some(to) => Ok(Some(to.clone())),
            None => recipients(f, status, o),
        };
        (choice, to)
    });
    let to = match chosen {
        Some((choice, Ok(Some(to)))) => Some((choice, to)),
        Some((_, Err(why))) => {
            hint(ui, &why);
            None
        }
        _ => None,
    };
    let copper = ka::gold(f.amount);
    let amount = ka::format_gold(copper);
    let asked = to.as_ref().map(|(_, t)| (t.clone(), copper));
    let confirmed = asked.is_some() && f.confirm == asked;
    let label = match &to {
        Some((_, t)) if confirmed => {
            egui::RichText::new(words::send_again(&amount, &who(t))).color(egui::Color32::LIGHT_RED)
        }
        _ => egui::RichText::new(format!("Send {amount}")).color(GOLD),
    };
    let button = egui::Button::new(label).min_size(egui::vec2(160.0, 44.0));
    let mut send = None;
    if ui.add_enabled(can && to.is_some(), button).clicked() {
        if confirmed {
            send = to;
        } else {
            f.confirm = asked;
        }
    }
    if let Some(outcome) = &f.outcome {
        for (ok, text) in gold_lines(outcome, &base) {
            ui.colored_label(colour(ok), text);
        }
    }
    // Only the names the server ran out of time for, all of them (sent at most the agent's limit
    // at a time); the unsure ones are a mailbox's to say.
    let resend = match f.not_sent() {
        0 => false,
        n => {
            let button = egui::Button::new(format!("Send to the {n} not sent"))
                .min_size(egui::vec2(160.0, 44.0));
            ui.add_enabled(can, button).clicked()
        }
    };
    if let Some((choice, to)) = send {
        admin.send_gold(choice, to);
    } else if resend {
        admin.resend_gold();
    }
}

/// Who the form sends to: everyone online, or the online players picked and the other name, at
/// most [`ka::MAX_GOLD_NAMES`] (the agent refuses a send of more) — past it, everyone online when
/// every one of them is picked and no other name. `Ok(None)`: nobody; `Err`: why not.
fn recipients(
    f: &GoldForm,
    status: &Status,
    other: Option<String>,
) -> Result<Option<GoldTo>, String> {
    if f.everyone {
        return Ok((!status.players_ok || !status.players.is_empty()).then_some(GoldTo::Online));
    }
    let mut names: Vec<String> = status
        .players
        .iter()
        .filter(|p| f.picked.contains(&p.name))
        .map(|p| p.name.clone())
        .collect();
    let all = !names.is_empty() && names.len() == status.players.len();
    if let Some(o) = other {
        if !names.iter().any(|n| n.to_lowercase() == o.to_lowercase()) {
            names.push(o);
        }
    }
    if names.len() > ka::MAX_GOLD_NAMES {
        if all && names.len() == status.players.len() {
            return Ok(Some(GoldTo::Online));
        }
        return Err(format!(
            "Pick up to {} players, or Everyone online.",
            ka::MAX_GOLD_NAMES
        ));
    }
    Ok((!names.is_empty()).then_some(GoldTo::Names(names)))
}

fn who(to: &GoldTo) -> String {
    match to {
        GoldTo::Online => "everyone online".into(),
        GoldTo::Names(names) if names.len() == 1 => names[0].clone(),
        GoldTo::Names(names) => format!("{} players", names.len()),
    }
}

/// What a send came to, line by line.
fn gold_lines(outcome: &Result<GoldReply, AdminError>, base: &str) -> Vec<(bool, String)> {
    let r = match outcome {
        Ok(r) => r,
        Err(AdminError::NoAnswer) => return vec![(false, GOLD_NO_ANSWER.into())],
        Err(e) => {
            let (ok, text, _) = error_line(e, base, true);
            return vec![(ok, text)];
        }
    };
    let mut out = Vec::new();
    if !r.sent.is_empty() {
        out.push((
            true,
            format!(
                "Sent by mail to {}: they collect it at any mailbox.",
                r.sent.join(", ")
            ),
        ));
    }
    for name in &r.missing {
        out.push((false, format!("No character named {name}.")));
    }
    if !r.unsure.is_empty() {
        out.push((
            false,
            format!(
                "Maybe sent to {}: the server did not say. Check a mailbox before sending to \
                 them again.",
                r.unsure.join(", ")
            ),
        ));
    }
    if !r.not_sent.is_empty() {
        out.push((
            false,
            format!(
                "Not sent to {}: the server ran out of time.",
                r.not_sent.join(", ")
            ),
        ));
    }
    if out.is_empty() {
        out.push((false, "Nobody was online to send it to.".into()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_read_as_multipliers() {
        assert_eq!(times(1.0), "×1");
        assert_eq!(times(1.5), "×1.5");
        assert_eq!(times(0.75), "×0.75");
        assert_eq!(times(10.0), "×10");
        assert_eq!(times(0.0), "×0");
        assert_eq!(snap(1.4999, 0.05), 1.5);
        assert_eq!(snap(1.02, 0.05), 1.0);
        assert_eq!(snap(1.234, 0.01), 1.23);
        assert_eq!(snap(0.02, 0.05), 0.0);
    }

    /// A send's reply says who gets the mail, who does not exist, and what is not known.
    #[test]
    fn a_gold_reply_says_who_gets_it() {
        let reply = GoldReply {
            sent: vec!["Thrall".into(), "Jaina".into()],
            missing: vec!["Nobody".into()],
            unsure: vec!["Rexxar".into()],
            not_sent: vec![],
        };
        let lines = gold_lines(&Ok(reply), "http://10.0.0.5:8099/");
        assert_eq!(
            lines[0],
            (
                true,
                "Sent by mail to Thrall, Jaina: they collect it at any mailbox.".into()
            )
        );
        assert_eq!(lines[1], (false, "No character named Nobody.".into()));
        assert!(lines[2].1.starts_with("Maybe sent to Rexxar"));
        assert_eq!(lines.len(), 3);
        // Names the server ran out of time for: said so, the resend is its own button.
        let late = GoldReply {
            not_sent: vec!["Uther".into(), "Varian".into()],
            ..GoldReply::default()
        };
        assert_eq!(
            gold_lines(&Ok(late), ""),
            vec![(
                false,
                "Not sent to Uther, Varian: the server ran out of time.".into()
            )]
        );
        assert_eq!(
            gold_lines(&Err(AdminError::NoAnswer), "")[0].1,
            GOLD_NO_ANSWER
        );
        assert_eq!(
            gold_lines(&Ok(GoldReply::default()), "")[0].1,
            "Nobody was online to send it to."
        );
        // Who the form sends to.
        let mut f = GoldForm::default();
        let status: Status = ka::read_reply(
            200,
            r#"{"ok":true,"world":"up","players":[{"name":"Thrall"},{"name":"Jaina"}],"players_ok":true}"#,
        )
        .unwrap();
        assert_eq!(recipients(&f, &status, None), Ok(None));
        f.picked.insert("Jaina".into());
        assert_eq!(
            recipients(&f, &status, Some("Thrall".into())),
            Ok(Some(GoldTo::Names(vec!["Jaina".into(), "Thrall".into()])))
        );
        assert_eq!(
            recipients(&f, &status, Some("JAINA".into())),
            Ok(Some(GoldTo::Names(vec!["Jaina".into()])))
        );
        // Every one online picked: still those names, under the agent's limit.
        f.picked.insert("Thrall".into());
        assert_eq!(
            recipients(&f, &status, None),
            Ok(Some(GoldTo::Names(vec!["Thrall".into(), "Jaina".into()])))
        );
        f.everyone = true;
        assert_eq!(recipients(&f, &status, None), Ok(Some(GoldTo::Online)));
        assert_eq!(
            recipients(&f, &Status::default(), None),
            Ok(Some(GoldTo::Online))
        );
        let nobody = Status {
            players_ok: true,
            ..Status::default()
        };
        assert_eq!(recipients(&f, &nobody, None), Ok(None));
        // Past the agent's limit: everyone online when all of them are picked, else refused here.
        let crowd = Status {
            world: "up".into(),
            players: (0..ka::MAX_GOLD_NAMES + 1)
                .map(|i| ka::Player {
                    name: format!("P{i}"),
                    ..ka::Player::default()
                })
                .collect(),
            players_ok: true,
            ..Status::default()
        };
        let mut f = GoldForm::default();
        f.picked = crowd.players.iter().map(|p| p.name.clone()).collect();
        assert_eq!(recipients(&f, &crowd, None), Ok(Some(GoldTo::Online)));
        let too_many = recipients(&f, &crowd, Some("Rexxar".into()));
        assert_eq!(
            too_many,
            Err("Pick up to 50 players, or Everyone online.".into())
        );
        f.picked.pop_first();
        assert!(matches!(
            recipients(&f, &crowd, None),
            Ok(Some(GoldTo::Names(n))) if n.len() == ka::MAX_GOLD_NAMES
        ));
        assert!(recipients(&f, &crowd, Some("Rexxar".into())).is_err());
        assert_eq!(
            who(&GoldTo::Names(vec!["A".into(), "B".into()])),
            "2 players"
        );
    }

    /// A server sentence that says what to do, or that nothing was done, is shown as it is; the
    /// refresh hint only follows one that says neither, and never a gold send's.
    #[test]
    fn refusals_say_what_to_do() {
        let (ok, text, why) = error_line(
            &AdminError::Refused {
                code: "admin_misconfigured".into(),
                message: "The data host and the world server disagree on ADMIN_CODE.".into(),
            },
            "",
            false,
        );
        assert!(!ok);
        assert_eq!(
            text,
            "The data host and the world server disagree on ADMIN_CODE."
        );
        assert_eq!(why.as_deref(), Some("admin_misconfigured"));
        let refused = |code: &str, message: &str, gold| {
            let e = AdminError::Refused {
                code: code.into(),
                message: message.into(),
            };
            error_line(&e, "", gold).1
        };
        // The agent's own sentences (`vanillish-admin`).
        let timeout = "The world server did not answer in time: the rates are saved and apply \
             once it does. Refresh before trying again.";
        assert_eq!(refused("console_timeout", timeout, false), timeout);
        let no_db = "The characters database did not answer: nothing was sent.";
        assert_eq!(refused("internal", no_db, true), no_db);
        assert_eq!(refused("internal", no_db, false), no_db);
        let failed = "The admin agent failed: see docker compose logs mangosd.";
        assert_eq!(
            refused("internal", failed, false),
            "The admin agent failed: see docker compose logs mangosd. Refresh to see what took."
        );
        assert_eq!(refused("internal", failed, true), failed);
        for text in [timeout, no_db, failed] {
            assert!(refused("internal", text, false).matches("Refresh").count() <= 1);
        }
        let (_, text, _) = error_line(
            &AdminError::Unreachable("refused".into()),
            "http://10.0.0.5:8099/",
            false,
        );
        assert!(text.contains("10.0.0.5"), "{text}");
        assert_eq!(
            error_line(&AdminError::WorldDown("x".into()), "", false).1,
            STARTING
        );
        assert_eq!(error_line(&AdminError::Newer(2), "", false).1, NEWER);
        // The wrong-code limit holds no right code back: no wait is promised.
        let too_many = AdminError::TooMany {
            retry_after: Some(540),
        };
        assert_eq!(error_line(&too_many, "", false).1, TOO_MANY);
        assert!(!TOO_MANY.contains("minute") && !TOO_MANY.contains("try again"));
        // Said on the tab's line, which is drawn above the code field.
        let mut admin = Admin::default();
        admin.server_id = Some("home".into());
        admin.menu = Some(Menu::At("http://10.0.0.5:8099/admin/".into()));
        admin.error = Some(too_many);
        assert_eq!(line(&admin).map(|l| l.1).as_deref(), Some(TOO_MANY));
        assert_eq!(error_line(&AdminError::Throttled, "", false).1, THROTTLED);
    }
}

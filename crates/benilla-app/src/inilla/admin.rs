//! Vanillish: the **server admin menu**, the app half — what Settings → Server (`ui::admin`)
//! draws, and its requests to the launcher's current server, on one worker thread at a time,
//! polled each frame ([`poll_admin`]):
//!
//! - `GET <base>server.json` (open): is there an `admin` section, and does it answer here;
//! - with a code saved for this server, `GET <admin>nonce`, then the signed route
//!   (`inilla_core::admin::auth_header`); a spent nonce gets one fresh one and one retry.
//!
//! Nothing but `server.json` is asked before a code is saved. The code is read from the Keystore
//! once per server and never sent; a 401 `admin_code` or a 429 `too_many` (which answers a wrong
//! code only) for the code still saved deletes it. A gold send that got no answer, or a refusal
//! that does not rule it out, keeps its request id and its names while the form stays as it was,
//! so sending again is answered as a replay, never paid twice.

use std::collections::{BTreeSet, VecDeque};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use bevy::prelude::*;
use serde::de::DeserializeOwned;

use inilla_core::admin::{self as ka, AdminError, Draft, GoldReply, GoldTo, Route, Status};
use inilla_core::connect::{self as kc, InfoError};
use inilla_core::profiles::ServerProfile;

use super::ui::{words, Overlay, Panel, SettingsTab};
use super::Inilla;

/// While the tab shows, a status older than this (from when the last answer came, a change's too)
/// is asked for again.
const STALE: Duration = Duration::from_secs(15);

/// What `server.json` says about the admin menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Menu {
    /// Where its routes are (a trailing slash).
    At(String),
    /// No `admin` section: a data host from before the menu, or no usable `ADMIN_CODE`.
    Absent,
    /// It answers only on the server's own network (`open: false`).
    NotHere,
}

/// What the one worker thread is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Busy {
    Asking,
    Applying,
    Resetting,
    Sending,
}

/// The last change's outcome, worth a line until the next one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Note {
    /// Rates applied: the keys the server changed (none: it had them already).
    Applied(Vec<String>),
    /// Back to the server's own settings.
    Reset(Vec<String>),
    /// The Keystore refused the code: kept in memory only.
    NotSaved,
}

/// The owner's choices in the Give gold form, the other name as checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GoldChoice {
    everyone: bool,
    picked: BTreeSet<String>,
    other: Option<String>,
    copper: u64,
}

/// One gold send: its request id, who and how much, and the choices it came from (`None`: the
/// names a send left not sent, sent again).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GoldSend {
    id: String,
    pub(crate) to: GoldTo,
    pub(crate) copper: u64,
    form: Option<GoldChoice>,
}

/// The Give gold form.
#[derive(Clone, Debug)]
pub(crate) struct GoldForm {
    /// Online characters picked, by name.
    pub(crate) picked: BTreeSet<String>,
    pub(crate) everyone: bool,
    /// A character not online (the mail waits for them).
    pub(crate) other: String,
    /// Whole gold.
    pub(crate) amount: u32,
    /// The send the first tap was for (the second sends it).
    pub(crate) confirm: Option<(GoldTo, u64)>,
    /// A send from the form that got no answer, or a refusal that does not rule it out: sent
    /// again, under its id and to its names (who has logged off since too), while the choices stay
    /// as they were.
    pending: Option<GoldSend>,
    sending: Option<GoldSend>,
    /// The names sends left not sent, in sends of at most [`ka::MAX_GOLD_NAMES`] (the agent's
    /// limit), each under an id of its own: the front one goes next, and leaves only once its own
    /// answer comes (a retry until then is a replay).
    resend: VecDeque<GoldSend>,
    /// What the last send came to.
    pub(crate) outcome: Option<Result<GoldReply, AdminError>>,
}

impl GoldForm {
    /// The choices as they stand (`other`: the other name as checked).
    pub(crate) fn choice(&self, other: Option<String>) -> GoldChoice {
        let copper = ka::gold(self.amount);
        if self.everyone {
            return GoldChoice {
                everyone: true,
                picked: BTreeSet::new(),
                other: None,
                copper,
            };
        }
        GoldChoice {
            everyone: false,
            picked: self.picked.clone(),
            other,
            copper,
        }
    }

    /// How many names the sends of the names not sent hold, all together.
    pub(crate) fn not_sent(&self) -> usize {
        self.resend
            .iter()
            .map(|r| match &r.to {
                GoldTo::Names(names) => names.len(),
                GoldTo::Online => 0,
            })
            .sum()
    }

    /// Who the send kept for a replay went to, while `choice` is what it came from.
    pub(crate) fn replay(&self, choice: &GoldChoice) -> Option<&GoldTo> {
        self.pending
            .as_ref()
            .filter(|p| p.form.as_ref() == Some(choice))
            .map(|p| &p.to)
    }
}

impl Default for GoldForm {
    fn default() -> Self {
        Self {
            picked: BTreeSet::new(),
            everyone: false,
            other: String::new(),
            amount: 10,
            confirm: None,
            pending: None,
            sending: None,
            resend: VecDeque::new(),
            outcome: None,
        }
    }
}

enum Task {
    /// `server.json`, then — with a code — the status.
    Refresh { base: String, code: Option<String> },
    /// A signed `POST` at the menu's URL.
    Post {
        url: String,
        code: String,
        route: Route,
        body: String,
    },
}

enum Reply {
    /// `server.json` alone (no menu, or no code yet).
    Nothing,
    Status(Status),
    Gold(GoldReply),
}

struct Done {
    /// What `server.json` said, when this job asked it.
    menu: Option<Menu>,
    reply: Result<Reply, AdminError>,
}

struct Job {
    /// The server it asks: a reply for another (the player switched) is dropped.
    server_id: String,
    /// The code it signs with: a reply once another is saved (or none) is dropped too.
    signed_with: Option<String>,
    busy: Busy,
    handle: JoinHandle<Done>,
}

#[derive(Resource, Default)]
pub(crate) struct Admin {
    /// The launcher's current server (`None`: there is none).
    pub(crate) server_id: Option<String>,
    pub(crate) name: String,
    /// Where `server.json` is asked: the profile's `info`, else the host's data host.
    pub(crate) base: String,
    pub(crate) menu: Option<Menu>,
    /// The saved code, read from the Keystore once per server (inside, `None`: none saved).
    code: Option<Option<String>>,
    pub(crate) status: Option<Status>,
    pub(crate) draft: Draft,
    /// Why the last request did not do what it asked.
    pub(crate) error: Option<AdminError>,
    pub(crate) note: Option<Note>,
    /// The code field, and why what was typed is refused.
    pub(crate) typed: String,
    pub(crate) code_error: Option<String>,
    /// "Tell the players in chat", stored as its opposite so it starts on.
    pub(crate) quiet: bool,
    /// Server defaults was tapped once.
    pub(crate) confirm_reset: bool,
    pub(crate) gold: GoldForm,
    /// Ask again as soon as the worker is free (the tab came up, Refresh, a reply to check).
    pub(crate) want: bool,
    job: Option<Job>,
    asked_at: Option<Instant>,
    shown: bool,
}

impl Admin {
    pub(crate) fn busy(&self) -> Option<Busy> {
        self.job.as_ref().map(|j| j.busy)
    }

    /// A code is saved for this server (the Keystore read once).
    pub(crate) fn has_code(&mut self) -> bool {
        self.code().is_some()
    }

    fn code(&mut self) -> Option<String> {
        let id = self.server_id.as_deref()?;
        if self.code.is_none() {
            let saved = inilla_platform::keychain::get(&ka::code_key(id))
                .and_then(|c| ka::normalize_code(&c).ok());
            self.code = Some(saved);
        }
        self.code.clone().flatten()
    }

    /// The world server is up, as the last status said, and nothing has said otherwise since.
    pub(crate) fn world_up(&self) -> bool {
        !matches!(self.error, Some(AdminError::WorldDown(_)))
            && self.status.as_ref().is_some_and(Status::world_up)
    }

    /// A change can go out: the world server is up, and nothing runs.
    pub(crate) fn can_change(&self) -> bool {
        self.job.is_none() && self.world_up()
    }

    /// Save the code typed (normalised; refused before anything is sent), then ask with it.
    pub(crate) fn set_code(&mut self) {
        let code = match ka::normalize_code(&self.typed) {
            Ok(code) => code,
            Err(e) => {
                self.code_error = Some(e.to_string());
                return;
            }
        };
        let Some(id) = self.server_id.clone() else {
            return;
        };
        self.typed.clear();
        self.code_error = None;
        self.note = None;
        if let Err(e) = inilla_platform::keychain::set(&ka::code_key(&id), &code) {
            warn!("inilla: admin: the admin code was not saved: {e}");
            self.note = Some(Note::NotSaved);
        }
        self.code = Some(Some(code));
        self.error = None;
        self.want = true;
    }

    /// Forget the saved code (the player's tap, or the server's 401 `admin_code` or 429
    /// `too_many`).
    pub(crate) fn forget_code(&mut self) {
        if let Some(id) = &self.server_id {
            inilla_platform::keychain::delete(&ka::code_key(id));
        }
        self.code = Some(None);
        self.status = None;
        self.draft.clear();
        self.confirm_reset = false;
        self.gold.confirm = None;
    }

    /// The sliders back to what the server has.
    pub(crate) fn undo(&mut self) {
        if let Some(s) = &self.status {
            self.draft = ka::draft(s);
        }
    }

    /// Send the sliders' changes.
    pub(crate) fn apply(&mut self) {
        let Some(status) = &self.status else {
            return;
        };
        let changes = ka::changes(&self.draft, status);
        if changes.is_empty() {
            return;
        }
        let body = ka::rates_body(&new_id(), &changes, !self.quiet);
        self.post(Busy::Applying, Route::Rates, body);
    }

    /// Every knob back to the server's own `mangosd.conf`.
    pub(crate) fn reset(&mut self) {
        self.confirm_reset = false;
        let body = ka::reset_body(&new_id(), !self.quiet);
        self.post(Busy::Resetting, Route::Reset, body);
    }

    /// Mail what the form chose to `to` — or, the choices those of the send kept for a replay,
    /// that send again: its id, its names.
    pub(crate) fn send_gold(&mut self, choice: GoldChoice, to: GoldTo) {
        let send = match &self.gold.pending {
            Some(p) if p.form.as_ref() == Some(&choice) => p.clone(),
            _ => GoldSend {
                id: new_id(),
                to,
                copper: choice.copper,
                form: Some(choice),
            },
        };
        self.gold.resend.clear();
        self.post_gold(send);
    }

    /// Mail the next of the sends of names left not sent, and no one else.
    pub(crate) fn resend_gold(&mut self) {
        if let Some(send) = self.gold.resend.front().cloned() {
            self.post_gold(send);
        }
    }

    fn post_gold(&mut self, send: GoldSend) {
        let body = ka::gold_body(&send.id, &send.to, send.copper);
        self.gold.confirm = None;
        self.gold.outcome = None;
        self.gold.sending = Some(send);
        self.post(Busy::Sending, Route::Gold, body);
    }

    fn post(&mut self, busy: Busy, route: Route, body: String) {
        let (Some(Menu::At(url)), Some(code)) = (self.menu.clone(), self.code()) else {
            return;
        };
        if busy != Busy::Sending {
            self.note = None;
        }
        info!("inilla: admin: {} {}", route.method(), route.path());
        self.start(
            busy,
            Task::Post {
                url,
                code,
                route,
                body,
            },
        );
    }

    fn refresh(&mut self) {
        let code = self.code();
        self.want = false;
        self.asked_at = Some(Instant::now());
        info!("inilla: admin: asking {}", self.base);
        let base = self.base.clone();
        self.start(Busy::Asking, Task::Refresh { base, code });
    }

    fn start(&mut self, busy: Busy, task: Task) {
        let Some(server_id) = self.server_id.clone() else {
            return;
        };
        if self.job.is_some() {
            return;
        }
        let signed_with = match &task {
            Task::Refresh { code, .. } => code.clone(),
            Task::Post { code, .. } => Some(code.clone()),
        };
        let handle = std::thread::Builder::new()
            .name("vanillish-admin".into())
            .spawn(move || run(task));
        match handle {
            Ok(handle) => {
                self.job = Some(Job {
                    server_id,
                    signed_with,
                    busy,
                    handle,
                })
            }
            Err(e) => self.error = Some(AdminError::Unreachable(format!("could not start: {e}"))),
        }
    }

    /// Follow the launcher's current server: another one (or the same at another address) starts
    /// everything afresh; a job still running for the old one finishes, and is dropped.
    fn follow(&mut self, current: Option<&ServerProfile>, shown: bool) {
        let same = current.map(|s| s.id.as_str()) == self.server_id.as_deref();
        let base = match current {
            Some(s) if shown || !same => base_for(s),
            Some(_) => self.base.clone(),
            None => String::new(),
        };
        if same && base == self.base {
            return;
        }
        *self = Admin {
            server_id: current.map(|s| s.id.clone()),
            name: current.map(|s| s.name.clone()).unwrap_or_default(),
            base,
            job: self.job.take(),
            shown: self.shown,
            ..Admin::default()
        };
    }

    /// A finished job, applied — when it was for the server this is about.
    fn take_finished(&mut self) {
        if !self.job.as_ref().is_some_and(|j| j.handle.is_finished()) {
            return;
        }
        let Some(job) = self.job.take() else {
            return;
        };
        let done = job.handle.join();
        if self.server_id.as_deref() != Some(job.server_id.as_str()) {
            return;
        }
        // The stale clock runs from the last answer, a change's too: an ask that took long is not
        // due again as it ends, and a change's refusal stays on screen until the clock asks again.
        self.asked_at = Some(Instant::now());
        // The code was forgotten or replaced meanwhile: what came back, a 401 too, is about the
        // old one (a new code's own ask is wanted already).
        if self.code.clone().flatten() != job.signed_with {
            self.gold.sending = None;
            return;
        }
        match done {
            Ok(done) => self.finished(job.busy, done),
            Err(_) => {
                self.error = Some(AdminError::Refused {
                    code: "crashed".into(),
                    message: "The request crashed.".into(),
                })
            }
        }
    }

    fn finished(&mut self, busy: Busy, done: Done) {
        if let Some(menu) = done.menu {
            if !matches!(menu, Menu::At(_)) {
                self.status = None;
                self.draft.clear();
            }
            self.menu = Some(menu);
        }
        let sending = self.gold.sending.take();
        match done.reply {
            Ok(Reply::Nothing) => self.error = None,
            Ok(Reply::Status(s)) => {
                self.error = None;
                let changed = s.changed.clone();
                self.take_status(s, busy != Busy::Asking);
                match busy {
                    Busy::Applying => self.note = Some(Note::Applied(changed)),
                    Busy::Resetting => self.note = Some(Note::Reset(changed)),
                    Busy::Asking | Busy::Sending => {}
                }
            }
            Ok(Reply::Gold(g)) => {
                info!(
                    "inilla: admin: gold sent to {}, missing {}, unsure {}, not sent {}",
                    g.sent.len(),
                    g.missing.len(),
                    g.unsure.len(),
                    g.not_sent.len()
                );
                self.gold.pending = None;
                if let Some(s) = &sending {
                    // A resend's own answer: that one is done.
                    if self.gold.resend.front().is_some_and(|r| r.id == s.id) {
                        self.gold.resend.pop_front();
                    }
                    // The names the agent's deadline left were not sent: they alone, next, in sends
                    // the agent takes, each under a new id.
                    for names in g.not_sent.chunks(ka::MAX_GOLD_NAMES).rev() {
                        self.gold.resend.push_front(GoldSend {
                            id: new_id(),
                            to: GoldTo::Names(names.to_vec()),
                            copper: s.copper,
                            form: None,
                        });
                    }
                }
                self.gold.outcome = Some(Ok(g));
                self.want = true;
            }
            Err(e) => {
                info!("inilla: admin: refused: {e:?}");
                // A 401 `admin_code`, or the data host's 429 `too_many`, which answers a wrong code
                // only: the field comes back, under the sentence.
                let wrong_code = matches!(e, AdminError::WrongCode | AdminError::TooMany { .. });
                if wrong_code {
                    self.forget_code();
                }
                if busy == Busy::Sending {
                    // Unless the code, the menu, the signature or a server without the menu ruled
                    // the request out, the same send again goes under the same id: one the agent
                    // never ran just runs, one it ran answers as it did (a resend keeps its own,
                    // in `resend`).
                    let ruled_out = wrong_code
                        || matches!(
                            e,
                            AdminError::Off | AdminError::Closed | AdminError::Unsigned
                        )
                        || matches!(e, AdminError::Older(s) if (400..500).contains(&s));
                    if !ruled_out && sending.as_ref().is_some_and(|s| s.form.is_some()) {
                        self.gold.pending = sending;
                    }
                    if e == AdminError::NoAnswer {
                        self.want = true;
                    }
                    if wrong_code {
                        self.error = Some(e.clone());
                    }
                    self.gold.outcome = Some(Err(e));
                } else {
                    // A change that got no answer: the status says what took. A status that got
                    // none did nothing: the stale clock asks again.
                    if e == AdminError::NoAnswer && busy != Busy::Asking {
                        self.want = true;
                    }
                    self.error = Some(e);
                }
            }
        }
    }

    /// A status in: the draft starts from it — but the sliders the player moved keep their values
    /// ([`ka::rebase`]). A change's own reply always replaces it.
    fn take_status(&mut self, s: Status, after_change: bool) {
        self.draft = match &self.status {
            Some(old) if !after_change => ka::rebase(&self.draft, old, &s),
            _ => ka::draft(&s),
        };
        if s.overrides == 0 {
            self.confirm_reset = false;
        }
        self.status = Some(s);
    }

    /// Ask now: the tab just came up or Refresh was tapped, nothing was asked for this server yet,
    /// or the status shown — or the "starting" or the no answer said instead of one — is stale.
    fn due(&self) -> bool {
        self.want
            || self.asked_at.is_none_or(|at| {
                at.elapsed() > STALE
                    && (self.status.is_some()
                        || matches!(
                            self.error,
                            Some(AdminError::WorldDown(_) | AdminError::NoAnswer)
                        ))
            })
    }
}

/// Where `profile`'s `server.json` is: where Connect found it, else the host's data host (a server
/// entered by hand).
fn base_for(profile: &ServerProfile) -> String {
    profile
        .info
        .clone()
        .filter(|b| !b.trim().is_empty())
        .or_else(|| kc::parse_address(&profile.host).ok().map(|a| a.base))
        .unwrap_or_default()
}

/// A fresh request id.
fn new_id() -> String {
    ka::request_id(rand::random())
}

pub(crate) struct AdminPlugin;

impl Plugin for AdminPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Admin>().add_systems(Update, poll_admin);
    }
}

/// The current server followed, a finished job applied, and a refresh when the Server tab first
/// shows, or its status is older than [`STALE`] while it shows.
fn poll_admin(mut admin: ResMut<Admin>, inilla: Res<Inilla>, overlay: Res<Overlay>) {
    let shown = overlay.panel == Some(Panel::Settings) && overlay.tab == SettingsTab::Server;
    let admin = &mut *admin;
    admin.follow(inilla.launcher.current_server(), shown);
    admin.take_finished();
    if shown && !admin.shown {
        admin.want = true;
    }
    admin.shown = shown;
    if shown && admin.job.is_none() && admin.server_id.is_some() && admin.due() {
        admin.refresh();
    }
}

/// The worker's HTTP: 4 s to connect, 30 s to answer (past the data host's own 25 s), no
/// redirects (a signed request goes where `server.json` said, or nowhere).
struct Client(ureq::Agent);

/// No answer: `sent` when the request may have reached the server.
struct Lost {
    sent: bool,
    why: String,
}

impl Client {
    fn new() -> Self {
        Self(
            ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(4))
                .timeout_read(Duration::from_secs(30))
                .redirects(0)
                .build(),
        )
    }

    fn send(
        &self,
        url: &str,
        auth: Option<&str>,
        body: Option<&str>,
    ) -> Result<(u16, String), Lost> {
        let req = match body {
            Some(_) => self.0.post(url),
            None => self.0.get(url),
        }
        .set("User-Agent", words::APP);
        let req = match auth {
            Some(a) => req.set("Authorization", a),
            None => req,
        };
        let result = match body {
            Some(b) => req.set("Content-Type", "application/json").send_string(b),
            None => req.call(),
        };
        match result {
            Ok(resp) | Err(ureq::Error::Status(_, resp)) => {
                let status = resp.status();
                resp.into_string().map(|b| (status, b)).map_err(|e| Lost {
                    sent: true,
                    why: e.to_string(),
                })
            }
            Err(ureq::Error::Transport(t)) => Err(Lost {
                sent: !matches!(
                    t.kind(),
                    ureq::ErrorKind::Dns
                        | ureq::ErrorKind::ConnectionFailed
                        | ureq::ErrorKind::InvalidUrl
                        | ureq::ErrorKind::UnknownScheme
                ),
                why: t.to_string(),
            }),
        }
    }
}

fn run(task: Task) -> Done {
    let http = Client::new();
    match task {
        Task::Refresh { base, code } => {
            let url = kc::resolve(&base, "server.json");
            let (status, body) = match http.send(&url, None, None) {
                Ok(answer) => answer,
                Err(lost) => {
                    return Done {
                        menu: None,
                        reply: Err(AdminError::Unreachable(lost.why)),
                    }
                }
            };
            let menu = match read_menu(&base, status, &body) {
                Ok(menu) => menu,
                Err(e) => {
                    return Done {
                        menu: None,
                        reply: Err(e),
                    }
                }
            };
            let reply = match (&menu, code) {
                (Menu::At(url), Some(code)) => {
                    signed(&http, url, &code, Route::Status, None).map(Reply::Status)
                }
                _ => Ok(Reply::Nothing),
            };
            Done {
                menu: Some(menu),
                reply,
            }
        }
        Task::Post {
            url,
            code,
            route,
            body,
        } => {
            let reply = match route {
                Route::Gold => signed(&http, &url, &code, route, Some(&body)).map(Reply::Gold),
                _ => signed(&http, &url, &code, route, Some(&body)).map(Reply::Status),
            };
            Done { menu: None, reply }
        }
    }
}

/// What `server.json` says of the menu. No menu only when it said so: a 200 without an `admin`
/// section, or nothing at that address that is a data host (a 404, a page that is not the
/// document). A gateway's 502, another refusal or a newer format leave the menu as it was.
fn read_menu(base: &str, status: u16, body: &str) -> Result<Menu, AdminError> {
    let info = match (status, kc::parse_server_info(body)) {
        (200, Ok(info)) => info,
        (200, Err(InfoError::Format(n))) => return Err(AdminError::Newer(n)),
        (200, Err(InfoError::Malformed(_))) | (404, _) => return Ok(Menu::Absent),
        (status, _) => {
            return Err(AdminError::Unreachable(format!(
                "server.json answered {status}"
            )))
        }
    };
    Ok(match info.admin {
        None => Menu::Absent,
        Some(a) if !a.open => Menu::NotHere,
        Some(a) => Menu::At(ka::admin_url(base, &a)),
    })
}

/// `GET <url>nonce`, then `route` signed with it; a nonce the data host no longer knows gets one
/// fresh one (nothing was forwarded for it). A gateway's own page with a 5xx (a proxy's 502 as the
/// data host restarts) for the nonce or a `GET` is the data host not answering, nothing sent; for
/// a `POST` itself it is no answer: the change may have gone through.
fn signed<T: DeserializeOwned>(
    http: &Client,
    url: &str,
    code: &str,
    route: Route,
    body: Option<&str>,
) -> Result<T, AdminError> {
    let gone = |status: u16| AdminError::Unreachable(format!("the admin menu answered {status}"));
    let mut tries = 0;
    loop {
        tries += 1;
        let (status, text) = http
            .send(&format!("{url}{}", ka::NONCE), None, None)
            .map_err(|l| AdminError::Unreachable(l.why))?;
        let nonce = match ka::read_nonce(status, &text) {
            Err(AdminError::Older(_) | AdminError::NoAnswer) if status >= 500 => {
                return Err(gone(status))
            }
            nonce => nonce?,
        };
        let auth = ka::auth_header(
            code,
            &nonce,
            route.method(),
            route.path(),
            body.unwrap_or_default().as_bytes(),
        );
        let target = format!("{url}{}", route.relative());
        let (status, text) = http.send(&target, Some(&auth), body).map_err(|l| match l {
            Lost { sent: true, .. } if body.is_some() => AdminError::NoAnswer,
            Lost { why, .. } => AdminError::Unreachable(why),
        })?;
        match ka::read_reply(status, &text) {
            Err(AdminError::Nonce) if tries < 2 => continue,
            Err(AdminError::Older(s)) if s >= 500 => {
                return Err(match body {
                    Some(_) => AdminError::NoAnswer,
                    None => gone(s),
                })
            }
            reply => return reply,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use inilla_core::http::{self, Request};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};

    const CODE: &str = "ABCD-EFGH-JKLM-NPQR";

    /// A stand-in data host on loopback.
    struct StandIn {
        base: String,
        /// The requests it saw (`GET /admin/nonce`).
        seen: Arc<Mutex<Vec<String>>>,
        /// The path a gateway answers for with its own page, the data host away (none: `""`).
        down: Arc<Mutex<String>>,
    }

    fn stand_in() -> StandIn {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen: Arc<Mutex<Vec<String>>> = Arc::default();
        let down: Arc<Mutex<String>> = Arc::default();
        let (log, away) = (seen.clone(), down.clone());
        let nonces = Arc::new(Mutex::new(0u32));
        std::thread::spawn(move || {
            http::serve(listener, move |req: Request, out: &mut TcpStream| {
                log.lock()
                    .unwrap()
                    .push(format!("{} {}", req.method, req.path));
                let _ = match (req.method.as_str(), req.path.as_str()) {
                    (_, path) if *away.lock().unwrap() == path => {
                        http::respond(out, 502, "text/html", b"<html>Bad Gateway</html>")
                    }
                    ("GET", "/server.json") => http::respond_json(
                        out,
                        200,
                        r#"{"format":1,"name":"Loop","admin":{"url":"admin/","open":true}}"#,
                    ),
                    ("GET", "/admin/nonce") => {
                        let mut n = nonces.lock().unwrap();
                        *n += 1;
                        http::respond_json(
                            out,
                            200,
                            &format!(r#"{{"ok":true,"nonce":"{:032x}","expires_in":60}}"#, *n),
                        )
                    }
                    (method, path) => {
                        let body = String::from_utf8_lossy(&req.body).into_owned();
                        let auth = req.header("authorization").unwrap_or_default().to_string();
                        let nonce = auth
                            .strip_prefix("Vanillish-Admin ")
                            .and_then(|a| a.split(':').next())
                            .unwrap_or_default()
                            .to_string();
                        let right = ka::auth_header(CODE, &nonce, method, path, &req.body);
                        let wrong =
                            ka::auth_header("WXYZ-WXYZ-WXYZ-WXYZ", &nonce, method, path, &req.body);
                        if nonce == format!("{:032x}", 1) {
                            http::respond_json(
                                out,
                                401,
                                r#"{"ok":false,"error":"admin_nonce","message":"Spent."}"#,
                            )
                        } else if auth == wrong {
                            http::respond_json(
                                out,
                                401,
                                r#"{"ok":false,"error":"admin_code","message":"Wrong."}"#,
                            )
                        } else if auth != right {
                            http::respond_json(
                                out,
                                401,
                                r#"{"ok":false,"error":"admin_auth","message":"?"}"#,
                            )
                        } else if path == "/admin/gold" && body.contains(r#""id":"502""#) {
                            // A proxy's own page: the data host went away with the send passed on.
                            http::respond(out, 502, "text/html", b"<html>Bad Gateway</html>")
                        } else if path == "/admin/gold" && body.contains(r#""id":"404""#) {
                            http::respond(out, 404, "text/html", b"<html>Not Found</html>")
                        } else if path == "/admin/gold" && body.contains("Upstream") {
                            // A gateway's own JSON, no error code: the send may have reached the
                            // agent.
                            http::respond_json(
                                out,
                                502,
                                r#"{"message":"An invalid response was received from the upstream server"}"#,
                            )
                        } else if path == "/admin/gold" {
                            http::respond_json(
                                out,
                                200,
                                r#"{"ok":true,"sent":["Thrall"],"missing":[],"unsure":[],"not_sent":[]}"#,
                            )
                        } else {
                            http::respond_json(
                                out,
                                200,
                                r#"{"ok":true,"world":"up","overrides":0,"rates":{"Rate.XP.Kill":{"value":1,"server":1,"min":0,"max":20}},"players":[],"players_ok":true}"#,
                            )
                        }
                    }
                };
            });
        });
        StandIn {
            base: format!("http://127.0.0.1:{port}/"),
            seen,
            down,
        }
    }

    /// The worker against the stand-in: `server.json`'s section read, the nonce asked, the MAC the
    /// data host's rule checks, a spent nonce retried once, a wrong code told apart, gold's body
    /// sent as signed, a gateway's page told apart from no answer.
    #[test]
    fn the_worker_signs_what_the_data_host_checks() {
        let StandIn { base, seen, down } = stand_in();
        let code = ka::normalize_code(CODE).unwrap();
        // No code: server.json alone.
        let done = run(Task::Refresh {
            base: base.clone(),
            code: None,
        });
        assert_eq!(done.menu, Some(Menu::At(format!("{base}admin/"))));
        assert!(matches!(done.reply, Ok(Reply::Nothing)));
        assert_eq!(seen.lock().unwrap().as_slice(), ["GET /server.json"]);
        // With the code: the first nonce is refused as spent, a second one is asked, once.
        let done = run(Task::Refresh {
            base: base.clone(),
            code: Some(code.clone()),
        });
        let Ok(Reply::Status(s)) = done.reply else {
            panic!("no status");
        };
        assert!(s.world_up() && s.rates.contains_key(ka::XP_KILL));
        assert_eq!(
            seen.lock().unwrap()[1..],
            [
                "GET /server.json",
                "GET /admin/nonce",
                "GET /admin/status",
                "GET /admin/nonce",
                "GET /admin/status"
            ]
        );
        // A signed POST: the body is what is signed.
        let url = format!("{base}admin/");
        let body = ka::gold_body("7", &GoldTo::Names(vec!["Thrall".into()]), ka::gold(10));
        let done = run(Task::Post {
            url: url.clone(),
            code: code.clone(),
            route: Route::Gold,
            body,
        });
        let Ok(Reply::Gold(g)) = done.reply else {
            panic!("no gold reply");
        };
        assert_eq!(g.sent, vec!["Thrall".to_string()]);
        // A gateway's page with a 5xx for it: it may have been done (its id kept for a replay);
        // with a 404, a server without the menu.
        for (id, reply) in [
            ("502", AdminError::NoAnswer),
            ("404", AdminError::Older(404)),
        ] {
            let body = ka::gold_body(id, &GoldTo::Names(vec!["Thrall".into()]), ka::gold(10));
            let done = run(Task::Post {
                url: url.clone(),
                code: code.clone(),
                route: Route::Gold,
                body,
            });
            assert_eq!(done.reply.err(), Some(reply), "{id}");
        }
        // A gateway's page for the nonce: the data host away, nothing sent — a change neither.
        let away = AdminError::Unreachable("the admin menu answered 502".into());
        *down.lock().unwrap() = "/admin/nonce".into();
        let before = seen.lock().unwrap().len();
        let xp = std::collections::BTreeMap::from([(ka::XP_KILL.to_string(), 2.0)]);
        for (route, body) in [
            (
                Route::Gold,
                ka::gold_body("8", &GoldTo::Names(vec!["Thrall".into()]), ka::gold(10)),
            ),
            (Route::Rates, ka::rates_body("9", &xp, true)),
        ] {
            let done = run(Task::Post {
                url: url.clone(),
                code: code.clone(),
                route,
                body,
            });
            assert_eq!(done.reply.err(), Some(away.clone()), "{route:?}");
        }
        assert_eq!(
            seen.lock().unwrap()[before..],
            ["GET /admin/nonce", "GET /admin/nonce"],
            "nothing posted"
        );
        // And for the status: not a server older than the menu.
        *down.lock().unwrap() = "/admin/status".into();
        let done = run(Task::Refresh {
            base: base.clone(),
            code: Some(code.clone()),
        });
        assert_eq!(done.menu, Some(Menu::At(url.clone())));
        assert_eq!(done.reply.err(), Some(away));
        down.lock().unwrap().clear();
        // Another code: refused as the wrong one.
        let done = run(Task::Post {
            url,
            code: "WXYZWXYZWXYZWXYZ".into(),
            route: Route::Rates,
            body: "{}".into(),
        });
        assert!(matches!(done.reply, Err(AdminError::WrongCode)));
        // Nothing listens: unreachable, nothing done.
        let closed = TcpListener::bind("127.0.0.1:0").unwrap();
        let gone = closed.local_addr().unwrap().port();
        drop(closed);
        let done = run(Task::Refresh {
            base: format!("http://127.0.0.1:{gone}/"),
            code: Some(code),
        });
        assert!(matches!(done.reply, Err(AdminError::Unreachable(_))));
        assert_eq!(done.menu, None);
    }

    fn status(value: f64) -> Status {
        ka::read_reply(
            200,
            &format!(
                r#"{{"ok":true,"world":"up","overrides":1,"rates":{{"Rate.XP.Kill":{{"value":{value},"server":1,"min":0,"max":20}}}},"players":[],"players_ok":true}}"#
            ),
        )
        .unwrap()
    }

    /// A refresh never overwrites sliders the player moved; a change's own reply does; another
    /// server starts afresh and drops the old one's job.
    #[test]
    fn a_moved_slider_survives_a_refresh() {
        let mut a = Admin {
            server_id: Some("home".into()),
            ..Admin::default()
        };
        a.take_status(status(2.0), false);
        assert_eq!(a.draft[ka::XP_KILL], 2.0);
        a.draft.insert(ka::XP_KILL.into(), 3.0);
        a.take_status(status(2.5), false);
        assert_eq!(a.draft[ka::XP_KILL], 3.0, "moved: kept");
        a.take_status(status(3.0), true);
        assert_eq!(a.draft[ka::XP_KILL], 3.0);
        a.draft.insert(ka::XP_KILL.into(), 4.0);
        a.finished(
            Busy::Applying,
            Done {
                menu: None,
                reply: Ok(Reply::Status(status(1.0))),
            },
        );
        assert_eq!(a.draft[ka::XP_KILL], 1.0, "a change's reply replaces it");
        assert_eq!(a.note, Some(Note::Applied(vec![])));
        // A send that got no answer keeps its id for the same send.
        let send = GoldSend {
            id: "abc".into(),
            to: GoldTo::Online,
            copper: 100,
            form: Some(a.gold.choice(None)),
        };
        a.gold.sending = Some(send.clone());
        a.finished(
            Busy::Sending,
            Done {
                menu: None,
                reply: Err(AdminError::NoAnswer),
            },
        );
        assert_eq!(a.gold.pending, Some(send));
        assert!(a.want && a.error.is_none());
        // No admin section any more: nothing to show.
        a.finished(
            Busy::Asking,
            Done {
                menu: Some(Menu::Absent),
                reply: Ok(Reply::Nothing),
            },
        );
        assert!(a.status.is_none() && a.draft.is_empty());
        // Another server: everything afresh.
        a.status = Some(status(2.0));
        let other = ServerProfile::new("Away", "10.0.0.8");
        a.follow(Some(&other), false);
        assert_eq!(a.server_id.as_deref(), Some(other.id.as_str()));
        assert_eq!(a.base, "http://10.0.0.8:8099/");
        assert!(a.status.is_none() && a.menu.is_none() && a.gold.pending.is_none());
        assert!(a.due());
    }

    fn two(xp: f64, rare: f64) -> Status {
        ka::read_reply(
            200,
            &format!(
                r#"{{"ok":true,"world":"up","overrides":0,"rates":{{"Rate.XP.Kill":{{"value":{xp},"server":1,"min":0,"max":20}},"Rate.Drop.Item.Rare":{{"value":{rare},"server":1,"min":0,"max":20}}}},"players":[],"players_ok":true}}"#
            ),
        )
        .unwrap()
    }

    /// A refresh under a moved slider takes every other knob from the server, so Apply sends the
    /// move alone and never puts back what another device changed.
    #[test]
    fn a_refresh_takes_the_knobs_not_moved() {
        let mut a = Admin::default();
        a.take_status(two(1.0, 1.0), false);
        a.draft.insert(ka::XP_KILL.into(), 3.0);
        a.take_status(two(1.0, 2.0), false);
        assert_eq!((a.draft[ka::XP_KILL], a.draft[ka::DROP_RARE]), (3.0, 2.0));
        let status = a.status.as_ref().unwrap();
        assert_eq!(
            ka::changes(&a.draft, status),
            std::collections::BTreeMap::from([(ka::XP_KILL.to_string(), 3.0)])
        );
    }

    fn job(signed_with: &str, reply: Result<Reply, AdminError>) -> Job {
        let handle = std::thread::spawn(move || Done { menu: None, reply });
        while !handle.is_finished() {
            std::thread::sleep(Duration::from_millis(1));
        }
        Job {
            server_id: "home".into(),
            signed_with: Some(signed_with.into()),
            busy: Busy::Asking,
            handle,
        }
    }

    /// A 401 for a code since replaced says nothing of the one saved now: dropped, the new code
    /// kept. For the code still saved, it forgets it.
    #[test]
    fn a_reply_for_another_code_is_dropped() {
        const OLD: &str = "WXYZWXYZWXYZWXYZ";
        const NEW: &str = "ABCDEFGHJKLMNPQR";
        let mut a = Admin {
            server_id: Some("home".into()),
            code: Some(Some(NEW.into())),
            ..Admin::default()
        };
        a.job = Some(job(OLD, Err(AdminError::WrongCode)));
        a.take_finished();
        assert!(a.job.is_none() && a.error.is_none());
        assert_eq!(a.code, Some(Some(NEW.into())));
        a.job = Some(job(NEW, Err(AdminError::WrongCode)));
        a.take_finished();
        assert_eq!(a.code, Some(None));
        assert_eq!(a.error, Some(AdminError::WrongCode));
        // The data host's 429 `too_many` answers a wrong code only: forgotten too, its sentence on
        // the tab's line above the field that comes back — a gold send's as well.
        let too_many = || {
            ka::read_reply::<Status>(
                429,
                r#"{"ok":false,"error":"too_many","message":"Wrong.","retry_after":540}"#,
            )
            .map(Reply::Status)
        };
        for busy in [Busy::Asking, Busy::Sending] {
            a.code = Some(Some(NEW.into()));
            a.error = None;
            a.gold.sending = (busy == Busy::Sending).then(|| GoldSend {
                id: "abc".into(),
                to: GoldTo::Online,
                copper: 100,
                form: None,
            });
            a.job = Some(Job {
                busy,
                ..job(NEW, too_many())
            });
            a.take_finished();
            assert_eq!(a.code, Some(None), "{busy:?}");
            assert!(
                matches!(a.error, Some(AdminError::TooMany { .. })),
                "{busy:?}"
            );
        }
        assert!(matches!(
            a.gold.outcome,
            Some(Err(AdminError::TooMany { .. }))
        ));
        // A 429 from in front of the data host (a proxy's own limit) says nothing of the code.
        for body in [
            "<html>429 Too Many Requests</html>",
            r#"{"ok":false,"error":"rate_limited"}"#,
        ] {
            a.code = Some(Some(NEW.into()));
            a.job = Some(job(
                NEW,
                ka::read_reply::<Status>(429, body).map(Reply::Status),
            ));
            a.take_finished();
            assert_eq!(a.code, Some(Some(NEW.into())), "{body}");
            assert_eq!(a.error, Some(AdminError::Throttled), "{body}");
        }
    }

    /// The stale clock runs from the last answer, a change's too: an ask that took long is not due
    /// again the moment it lands, a change's refusal is not asked away at once, and an ask that got
    /// no answer asks again on the clock, not at once — so the frames between show what came, with
    /// the buttons free.
    #[test]
    fn the_stale_clock_runs_from_the_answer() {
        const SAVED: &str = "ABCDEFGHJKLMNPQR";
        let mut a = Admin {
            server_id: Some("home".into()),
            code: Some(Some(SAVED.into())),
            ..Admin::default()
        };
        a.take_status(status(1.0), false);
        // Asked longer than STALE ago, answered now.
        a.asked_at = Instant::now().checked_sub(STALE * 2);
        a.job = Some(job(SAVED, Ok(Reply::Status(status(2.0)))));
        a.take_finished();
        assert_eq!(
            a.status.as_ref().map(|s| s.rates[ka::XP_KILL].value),
            Some(2.0)
        );
        assert!(!a.due(), "not due as its answer lands");
        // A change the server refused (its console busy), a status shown: the sentence stays.
        a.asked_at = Instant::now().checked_sub(STALE * 2);
        let refused = AdminError::Refused {
            code: "console_busy".into(),
            message: "The world server's console is busy: try again in a moment.".into(),
        };
        a.job = Some(Job {
            busy: Busy::Applying,
            ..job(SAVED, Err(refused.clone()))
        });
        a.take_finished();
        assert_eq!(a.error, Some(refused));
        assert!(!a.want && !a.due(), "not asked away at once");
        // A status ask that got no answer (the agent wedged: the data host's 504).
        a.asked_at = Instant::now().checked_sub(STALE * 2);
        a.job = Some(job(SAVED, Err(AdminError::NoAnswer)));
        a.take_finished();
        assert_eq!(a.error, Some(AdminError::NoAnswer));
        assert!(!a.want && !a.due(), "not asked again at once");
        a.asked_at = Instant::now().checked_sub(STALE * 2);
        assert!(a.due(), "the clock asks again");
        a.status = None;
        assert!(a.due(), "with no status shown too");
        // A change that got no answer: the status that says what took is asked for at once.
        a.asked_at = Some(Instant::now());
        for busy in [Busy::Applying, Busy::Sending] {
            a.want = false;
            a.finished(
                busy,
                Done {
                    menu: None,
                    reply: Err(AdminError::NoAnswer),
                },
            );
            assert!(a.want && a.due(), "{busy:?}");
        }
    }

    /// `server.json` says "no menu" only when it does; a gateway's error or a newer format is an
    /// error that keeps the menu, the status, the moved sliders and the timer.
    #[test]
    fn only_server_json_itself_says_there_is_no_menu() {
        let base = "http://10.0.0.5:8099/";
        let at = Menu::At(format!("{base}admin/"));
        let with = r#"{"format":1,"name":"Home","admin":{"url":"admin/","open":true}}"#;
        assert_eq!(read_menu(base, 200, with), Ok(at.clone()));
        let closed = r#"{"format":1,"name":"Home","admin":{"url":"admin/","open":false}}"#;
        assert_eq!(read_menu(base, 200, closed), Ok(Menu::NotHere));
        assert_eq!(
            read_menu(base, 200, r#"{"format":1,"name":"Home"}"#),
            Ok(Menu::Absent)
        );
        assert_eq!(read_menu(base, 404, "Not Found"), Ok(Menu::Absent));
        assert_eq!(read_menu(base, 200, "<html></html>"), Ok(Menu::Absent));
        assert_eq!(
            read_menu(base, 200, r#"{"format":2,"name":"Home"}"#),
            Err(AdminError::Newer(2))
        );
        for status in [500, 502, 503, 504, 403, 301] {
            assert_eq!(
                read_menu(base, status, "<html>Bad Gateway</html>"),
                Err(AdminError::Unreachable(format!(
                    "server.json answered {status}"
                ))),
            );
        }
        let mut a = Admin {
            server_id: Some("home".into()),
            menu: Some(at.clone()),
            ..Admin::default()
        };
        a.take_status(status(2.0), false);
        a.draft.insert(ka::XP_KILL.into(), 3.0);
        a.finished(
            Busy::Asking,
            Done {
                menu: None,
                reply: Err(read_menu(base, 502, "").unwrap_err()),
            },
        );
        assert_eq!(a.menu, Some(at));
        assert!(a.status.is_some() && a.draft[ka::XP_KILL] == 3.0);
        assert!(matches!(a.error, Some(AdminError::Unreachable(_))));
        a.asked_at = Instant::now().checked_sub(STALE * 2);
        assert!(a.due(), "the timer still asks");
    }

    /// "The world server is starting…" with no status yet asks again on the timer too.
    #[test]
    fn starting_asks_again() {
        let mut a = Admin {
            server_id: Some("home".into()),
            asked_at: Instant::now().checked_sub(STALE * 2),
            ..Admin::default()
        };
        assert!(a.asked_at.is_some());
        assert!(!a.due(), "no status, nothing said: the player taps Refresh");
        a.error = Some(AdminError::WorldDown("starting".into()));
        assert!(a.due());
        a.asked_at = Some(Instant::now());
        assert!(!a.due(), "not before it is stale");
    }

    /// Names the agent ran out of time for get a resend of their own: those names, the same
    /// amount, a new id, kept for a retry; the unsure ones never.
    #[test]
    fn the_names_not_sent_are_sent_alone() {
        let mut a = Admin {
            server_id: Some("home".into()),
            ..Admin::default()
        };
        a.send_gold(a.gold.choice(None), GoldTo::Online);
        let first = a.gold.sending.clone().unwrap();
        let reply = GoldReply {
            sent: vec!["Thrall".into()],
            unsure: vec!["Rexxar".into()],
            not_sent: vec!["Uther".into(), "Varian".into()],
            ..GoldReply::default()
        };
        a.finished(
            Busy::Sending,
            Done {
                menu: None,
                reply: Ok(Reply::Gold(reply)),
            },
        );
        assert_eq!(a.gold.resend.len(), 1);
        let resend = a.gold.resend[0].clone();
        assert_eq!(
            resend.to,
            GoldTo::Names(vec!["Uther".into(), "Varian".into()])
        );
        assert_eq!((resend.copper, resend.form.as_ref()), (first.copper, None));
        assert_ne!(resend.id, first.id);
        a.resend_gold();
        assert_eq!(a.gold.sending, Some(resend.clone()));
        a.finished(
            Busy::Sending,
            Done {
                menu: None,
                reply: Err(AdminError::NoAnswer),
            },
        );
        assert_eq!(
            a.gold.resend,
            VecDeque::from([resend]),
            "the retry is a replay"
        );
        assert_eq!(a.gold.pending, None, "the form's Send is not this resend");
        a.resend_gold();
        let unsure = GoldReply {
            unsure: vec!["Uther".into()],
            ..GoldReply::default()
        };
        a.finished(
            Busy::Sending,
            Done {
                menu: None,
                reply: Ok(Reply::Gold(unsure)),
            },
        );
        assert!(a.gold.resend.is_empty() && a.gold.not_sent() == 0);
    }

    /// More names left not sent than one send takes: sends of at most the agent's limit, every
    /// name in one of them once, each under its own id. One leaves only once its own answer comes
    /// (no answer: the same again), and the names it left go next.
    #[test]
    fn the_names_not_sent_go_in_sends_the_agent_takes() {
        let mut a = Admin {
            server_id: Some("home".into()),
            ..Admin::default()
        };
        a.send_gold(a.gold.choice(None), GoldTo::Online);
        let names: Vec<String> = (0..2 * ka::MAX_GOLD_NAMES + 7)
            .map(|i| format!("P{i}"))
            .collect();
        let gold = |reply| Done { menu: None, reply };
        a.finished(
            Busy::Sending,
            gold(Ok(Reply::Gold(GoldReply {
                not_sent: names.clone(),
                ..GoldReply::default()
            }))),
        );
        let chunks: Vec<Vec<String>> = a
            .gold
            .resend
            .iter()
            .map(|r| match &r.to {
                GoldTo::Names(n) => n.clone(),
                GoldTo::Online => panic!("a resend to everyone online"),
            })
            .collect();
        assert_eq!(chunks.len(), 3);
        assert!(chunks
            .iter()
            .all(|c| !c.is_empty() && c.len() <= ka::MAX_GOLD_NAMES));
        assert_eq!(chunks.concat(), names, "every name once");
        assert_eq!(a.gold.not_sent(), names.len());
        let ids: BTreeSet<String> = a.gold.resend.iter().map(|r| r.id.clone()).collect();
        assert_eq!(ids.len(), chunks.len());
        // The first goes; no answer: it stays first, and goes again under its id.
        let first = a.gold.resend[0].clone();
        let second = a.gold.resend[1].clone();
        a.resend_gold();
        assert_eq!(a.gold.sending.as_ref(), Some(&first));
        a.finished(Busy::Sending, gold(Err(AdminError::NoAnswer)));
        assert_eq!(a.gold.resend.front(), Some(&first));
        assert_eq!(a.gold.not_sent(), names.len());
        a.resend_gold();
        assert_eq!(a.gold.sending.as_ref(), Some(&first));
        // Its answer: it leaves, and the two it left go next, under a new id.
        let left = vec![names[3].clone(), names[9].clone()];
        a.finished(
            Busy::Sending,
            gold(Ok(Reply::Gold(GoldReply {
                sent: names[..3].to_vec(),
                not_sent: left.clone(),
                ..GoldReply::default()
            }))),
        );
        assert_eq!(a.gold.resend.len(), 3);
        assert_eq!(a.gold.resend[0].to, GoldTo::Names(left));
        assert!(!ids.contains(&a.gold.resend[0].id));
        assert_eq!(a.gold.resend[1], second);
        assert_eq!(a.gold.not_sent(), names.len() - ka::MAX_GOLD_NAMES + 2);
        // A send from the form: those are dropped.
        a.send_gold(a.gold.choice(None), GoldTo::Online);
        assert!(a.gold.resend.is_empty());
    }

    /// A send that got no answer goes again under its id and to its names while the choices stay
    /// the same, though a picked player has logged off since; other choices, another send.
    #[test]
    fn a_send_without_answer_goes_again_to_its_names() {
        let mut a = Admin {
            server_id: Some("home".into()),
            ..Admin::default()
        };
        a.gold.picked = BTreeSet::from(["Jaina".to_string(), "Thrall".to_string()]);
        let both = GoldTo::Names(vec!["Thrall".into(), "Jaina".into()]);
        a.send_gold(a.gold.choice(None), both.clone());
        let first = a.gold.sending.clone().unwrap();
        a.finished(
            Busy::Sending,
            Done {
                menu: None,
                reply: Err(AdminError::NoAnswer),
            },
        );
        assert_eq!(a.gold.pending, Some(first.clone()));
        // The refresh that follows: Jaina is gone from the players online.
        a.take_status(status(1.0), false);
        assert_eq!(a.gold.replay(&a.gold.choice(None)), Some(&both));
        a.send_gold(a.gold.choice(None), GoldTo::Names(vec!["Thrall".into()]));
        assert_eq!(a.gold.sending, Some(first.clone()));
        // Another amount, or another name: a new send.
        a.gold.amount = 100;
        assert_eq!(a.gold.replay(&a.gold.choice(None)), None);
        a.send_gold(a.gold.choice(None), GoldTo::Names(vec!["Thrall".into()]));
        assert_ne!(a.gold.sending.as_ref().map(|s| &s.id), Some(&first.id));
        a.gold.amount = 10;
        assert_eq!(a.gold.replay(&a.gold.choice(None)), Some(&both));
        assert_eq!(a.gold.replay(&a.gold.choice(Some("Rexxar".into()))), None);
    }

    /// A gateway's own JSON 502 for a send, no error code in it, may come after the agent ran it:
    /// the id is kept, and the next Send is the same request. So for any refusal but one that
    /// rules the request out (the code, the menu, the signature, a server without the menu).
    #[test]
    fn a_send_refused_on_its_way_keeps_its_id() {
        let StandIn { base, seen, .. } = stand_in();
        let mut a = Admin {
            server_id: Some("home".into()),
            menu: Some(Menu::At(format!("{base}admin/"))),
            code: Some(Some(ka::normalize_code(CODE).unwrap())),
            ..Admin::default()
        };
        let wait = |a: &mut Admin| {
            assert!(a.job.is_some(), "a job runs");
            while !a.job.as_ref().is_some_and(|j| j.handle.is_finished()) {
                std::thread::sleep(Duration::from_millis(1));
            }
            a.take_finished();
        };
        let to = GoldTo::Names(vec!["Upstream".into()]);
        let choice = a.gold.choice(Some("Upstream".into()));
        a.send_gold(choice.clone(), to.clone());
        let first = a.gold.sending.clone().unwrap();
        wait(&mut a);
        assert_eq!(
            a.gold.outcome,
            Some(Err(AdminError::Refused {
                code: "502".into(),
                message: "An invalid response was received from the upstream server".into()
            }))
        );
        assert_eq!(a.gold.pending, Some(first.clone()));
        assert_eq!(a.gold.replay(&choice), Some(&to));
        // The next Send: the same id, and again kept.
        a.send_gold(choice.clone(), to.clone());
        assert_eq!(a.gold.sending.as_ref().map(|s| &s.id), Some(&first.id));
        wait(&mut a);
        assert_eq!(a.gold.pending, Some(first));
        // Both reached it (the first twice: the stand-in's first nonce is a spent one).
        assert_eq!(
            seen.lock()
                .unwrap()
                .iter()
                .filter(|r| *r == "POST /admin/gold")
                .count(),
            3
        );
        // What else keeps it, and what does not.
        for (e, kept) in [
            (AdminError::Unreachable("refused".into()), true),
            (AdminError::Nonce, true),
            (AdminError::Throttled, true),
            (AdminError::WorldDown("starting".into()), true),
            (
                AdminError::Refused {
                    code: "console_busy".into(),
                    message: "Busy.".into(),
                },
                true,
            ),
            (AdminError::WrongCode, false),
            (AdminError::TooMany { retry_after: None }, false),
            (AdminError::Off, false),
            (AdminError::Closed, false),
            (AdminError::Unsigned, false),
            (AdminError::Older(404), false),
        ] {
            let mut b = Admin::default();
            b.send_gold(choice.clone(), to.clone());
            let send = b.gold.sending.clone();
            b.finished(
                Busy::Sending,
                Done {
                    menu: None,
                    reply: Err(e.clone()),
                },
            );
            assert_eq!(b.gold.pending, send.filter(|_| kept), "{e:?}");
            assert_eq!(b.gold.outcome, Some(Err(e)));
        }
    }
}

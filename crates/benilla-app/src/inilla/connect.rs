//! Vanillish: **quick connect**, the app half — the player types the server's address, and the
//! app asks it what it is (`inilla_core::connect`), makes the server profile, starts the game
//! files' download and creates the account, all on worker threads, while the Connect screen
//! (`ui::connect`) draws what this holds.
//!
//! - **The probe**: `GET <base>server.json` (4 s to connect, 10 s to answer), then an older data
//!   host's `/`, then a 2 s TCP connect to the realm's 3724. What it finds is applied here
//!   ([`poll_connect`]): the profile is adopted and made current (so `session` points benilla's
//!   realmlist, realm name and world override at it), or the manual server form opens prefilled.
//! - **The game files**: the same manifest and download jobs the setup screen runs
//!   (`data::DataState`), chained — the manifest, then the download ([`chain_download`]). The
//!   download starts by itself on an unmetered network, after one tap on a metered one.
//! - **The account**: the sign-up's SRP6 salt and verifier are made here
//!   ([`make_verifier`], benilla's own SRP6 — the code that will log in), and only they are posted.
//!   The password goes to the Keystore; the account logs in by itself from then on.
//!
//! Only the last description, a pending download and a pending sign-up are held in memory: the
//! profile and the account are in `servers.toml`, the password and the access code in the
//! Keystore, so a relaunch (which the first download needs) carries the whole flow.

use std::thread::JoinHandle;
use std::time::Duration;

use bevy::prelude::*;

use inilla_core::connect::{
    self as kc, data_step, Address, DataStep, Probe, RegisterError, ServerInfo, SignUp,
};
use inilla_core::profiles::{Account, DataSource, ServerProfile};

use super::data::{DataState, Http, SPARE, TOKEN_REFUSED};
use super::ui::{words, Overlay, Panel, ServerForm};
use super::Inilla;

/// The server Connect is about: the profile it made, and what the server said.
#[derive(Clone, Debug)]
pub(crate) struct Found {
    /// The profile's id in the launcher.
    pub(crate) id: String,
    /// Where `server.json` answered (or an older data host's base).
    pub(crate) base: String,
    /// `None`: a data host from before `server.json` — its data alone is known.
    pub(crate) info: Option<ServerInfo>,
}

/// Where the manifest-then-download chain stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Chain {
    /// The manifest is to be asked for, once the data jobs are idle.
    Ask,
    /// The manifest job runs.
    Asking,
    /// The manifest is in; the download waits for the player's tap (a metered network, or too
    /// little space for it to start by itself).
    Offer,
    /// The download job runs.
    Downloading,
    /// The download stopped short (paused, the connection dropped): Resume.
    Stopped,
    /// The manifest did not come: `DataState::message` says why; Try again.
    Failed,
}

/// The game files' download, chained: the manifest, then the archives.
#[derive(Clone, Debug)]
pub(crate) struct PendingDownload {
    pub(crate) url: String,
    pub(crate) locale: Option<String>,
    pub(crate) token: Option<String>,
    /// The network costs by the byte, as it was when the chain started (`None`: not known).
    pub(crate) metered: Option<bool>,
    /// Start the download without a tap once the manifest is in: an unmetered network only.
    auto: bool,
    pub(crate) state: Chain,
}

/// A sign-up sent: the name and password it was made from and the body, sent again unchanged on a
/// retry (same salt, so a request whose answer was lost is answered as a replay). The checkboxes'
/// choice rides along to the save.
#[derive(Clone, Debug)]
pub(crate) struct Kept {
    pub(crate) name: String,
    pub(crate) password: String,
    pub(crate) sign_up: SignUp,
    pub(crate) remember: bool,
    pub(crate) auto_login: bool,
}

enum Done {
    /// What the address led to; `true` when it was the current server asked again on its own.
    Probe(Address, Probe, bool),
    SignUp(Result<String, RegisterError>),
}

/// What the one worker thread is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Busy {
    Probing,
    SigningUp,
}

#[derive(Resource, Default)]
pub(crate) struct Connect {
    pub(crate) found: Option<Found>,
    /// What the last probe found, when it is worth a line: `(good, sentence)`.
    pub(crate) status: Option<(bool, String)>,
    /// A smaller line under a failed probe: the network's own reason.
    pub(crate) detail: Option<String>,
    /// What the last sign-up came to.
    pub(crate) account_status: Option<(bool, String)>,
    pub(crate) pending: Option<PendingDownload>,
    /// The saved access code was refused (a 401 on the manifest, `token_required` on a sign-up):
    /// ask for it again.
    pub(crate) token_refused: bool,
    pub(crate) kept: Option<Kept>,
    /// The password of the account this flow saved, for Play without a Keystore round trip.
    pub(crate) password: Option<(String, String)>,
    /// The access code per server id, read from the Keystore once.
    token: Option<(String, Option<String>)>,
    /// The base last asked on its own (the screen reopened for a known server): once
    /// ([`Connect::reask`]).
    pub(crate) asked: Option<String>,
    job: Option<(Busy, JoinHandle<Done>)>,
}

impl Connect {
    pub(crate) fn busy(&self) -> Option<Busy> {
        self.job.as_ref().map(|(b, _)| *b)
    }

    /// The server Connect found, while it is still the launcher's current one.
    pub(crate) fn here(&self, current: Option<&ServerProfile>) -> Option<&Found> {
        self.found
            .as_ref()
            .filter(|f| current.is_some_and(|c| c.id == f.id))
    }

    /// The base to ask again on its own, once: the current server's, when Connect reopens for a
    /// server it set up before (after a relaunch, from the launcher) and holds nothing on it. Only
    /// these asks are counted, so a Connect the player typed that found nothing keeps its sentence
    /// on screen instead of being followed at once by the old server's.
    pub(crate) fn reask(&self, current: Option<&ServerProfile>) -> Option<String> {
        if self.job.is_some() || self.here(current).is_some() {
            return None;
        }
        let base = current?.info.clone()?;
        (self.asked.as_deref() != Some(base.as_str())).then_some(base)
    }

    /// Ask the current server again on its own, when [`Self::reask`] says to.
    pub(crate) fn ask_again(&mut self, current: Option<&ServerProfile>) {
        let Some(base) = self.reask(current) else {
            return;
        };
        self.asked = Some(base.clone());
        if let Ok(addr) = kc::parse_address(&base) {
            self.probe(addr, true);
        }
    }

    /// Ask `addr` — the address the player typed — what it is, on a worker thread.
    pub(crate) fn start_probe(&mut self, addr: Address) {
        self.probe(addr, false);
    }

    /// `again`: not typed now, but the current server asked again on its own — what it finds is
    /// shown here, and never leads away from the screen.
    fn probe(&mut self, addr: Address, again: bool) {
        if self.job.is_some() {
            return;
        }
        info!("inilla: connect: asking {}", addr.base);
        self.status = None;
        self.detail = None;
        let handle = std::thread::Builder::new()
            .name("inilla-connect".into())
            .spawn(move || {
                let found = kc::probe(&Http::quick(None), &addr, &tcp_listens);
                Done::Probe(addr, found, again)
            });
        match handle {
            Ok(h) => self.job = Some((Busy::Probing, h)),
            Err(e) => self.status = Some((false, format!("could not start: {e}"))),
        }
    }

    /// Post `kept`'s sign-up to `url` on a worker thread, with the access code when the server
    /// wants it.
    pub(crate) fn start_sign_up(&mut self, url: String, token: Option<String>, kept: Kept) {
        if self.job.is_some() {
            return;
        }
        info!(
            "inilla: connect: creating the account {}",
            kept.sign_up.username
        );
        self.account_status = None;
        let sign_up = kept.sign_up.clone();
        self.kept = Some(kept);
        let handle = std::thread::Builder::new()
            .name("inilla-sign-up".into())
            .spawn(move || Done::SignUp(kc::register(&Http::quick(token), &url, &sign_up)));
        match handle {
            Ok(h) => self.job = Some((Busy::SigningUp, h)),
            Err(e) => self.account_status = Some((false, format!("could not start: {e}"))),
        }
    }

    /// The access code for `server`'s data and sign-up: one the player saved (the Keystore, read
    /// once per server), or one written into the launcher file by hand.
    pub(crate) fn token_for(&mut self, server: &ServerProfile) -> Option<String> {
        if self.token.as_ref().is_none_or(|(id, _)| *id != server.id) {
            let saved = inilla_platform::keychain::get(&DataSource::token_key(&server.id))
                .or_else(|| server.data.as_ref().and_then(|d| d.token.clone()))
                .filter(|t| !t.trim().is_empty());
            self.token = Some((server.id.clone(), saved));
        }
        self.token.as_ref().and_then(|(_, t)| t.clone())
    }

    /// Save the access code the player typed for `server_id` (kept in memory too, should the
    /// Keystore refuse it).
    pub(crate) fn set_token(&mut self, server_id: &str, token: &str) {
        let token = token.trim().to_string();
        if let Err(e) = inilla_platform::keychain::set(&DataSource::token_key(server_id), &token) {
            warn!("inilla: connect: the access code was not saved: {e}");
        }
        self.token = Some((server_id.to_string(), Some(token)));
        self.token_refused = false;
        // A chain that stopped on the old code starts over with the new one.
        self.pending = None;
    }

    /// Forget a refused access code, so the screen asks again.
    fn refuse_token(&mut self, server_id: &str) {
        inilla_platform::keychain::delete(&DataSource::token_key(server_id));
        self.token = Some((server_id.to_string(), None));
        self.token_refused = true;
        self.pending = None;
    }
}

/// What the game files need next for the server Connect found (`complete`: the archives are all
/// here): [`kc::data_step`] for a server that described itself; for an older data host, its
/// manifest. Either way the access code once the data host `refused` it — even where the
/// description says no code is needed (one set since it was read, a proxy in front): asking the
/// host again unchanged would only be refused again, and again.
pub(crate) fn files_step(
    found: &Found,
    profile: &ServerProfile,
    complete: bool,
    have_token: bool,
    refused: bool,
) -> DataStep {
    let chosen = profile.data.as_ref().and_then(|d| d.locale.as_deref());
    match &found.info {
        Some(info) => {
            match data_step(info, &found.base, complete, chosen, have_token && !refused) {
                DataStep::Fetch { .. } if refused => DataStep::AskToken,
                step => step,
            }
        }
        None if complete => DataStep::Have,
        None if refused => DataStep::AskToken,
        None => DataStep::Fetch {
            url: profile
                .data
                .as_ref()
                .map_or_else(|| found.base.clone(), |d| d.url.clone()),
            locale: chosen.map(str::to_string),
        },
    }
}

/// Whether there is room for `need` more bytes on the data volume (`None` free: not known, so
/// yes).
pub(crate) fn room_for(data: &DataState, need: u64) -> bool {
    data.free_bytes().is_none_or(|free| free > need + SPARE)
}

/// What the manifest asks of the disk beyond what is already there.
pub(crate) fn still_needed(data: &DataState) -> Option<u64> {
    data.manifest
        .as_ref()
        .map(|m| m.total_bytes().saturating_sub(data.status.total_bytes()))
}

/// A short TCP connect to `host:port`, for a realm that has no HTTP to ask.
fn tcp_listens(host: &str, port: u16) -> bool {
    use std::net::{TcpStream, ToSocketAddrs};
    let Ok(addrs) = (host, port).to_socket_addrs() else {
        return false;
    };
    addrs
        .take(4)
        .any(|a| TcpStream::connect_timeout(&a, Duration::from_secs(2)).is_ok())
}

/// The account's SRP6 salt and verifier, little endian, as vmangos' own sign-up makes them
/// (`benilla_srp::generate_account`: a fresh salt with its top bit set) — from the name and
/// password benilla's logon will hash.
pub(crate) fn make_verifier(user: &str, pass: &str) -> Option<([u8; 32], [u8; 32])> {
    let user = benilla_srp::NormalizedString::new(user).ok()?;
    let pass = benilla_srp::NormalizedString::new(pass).ok()?;
    Some(benilla_srp::generate_account(&user, &pass))
}

/// Save the account `name` on `server`, with its password in the Keystore when `remember`, and
/// make it the one that logs in by itself when `auto_login` (only one does). `false` when the
/// Keystore refused the password: the launcher then asks for it at login.
pub(crate) fn save_account(
    inilla: &mut Inilla,
    server: &str,
    name: &str,
    password: &str,
    remember: bool,
    auto_login: bool,
) -> bool {
    let auto_login = remember && auto_login;
    let id = inilla.launcher.add_account(Account::new(server, name));
    if let Some(a) = inilla.launcher.accounts.iter_mut().find(|a| a.id == id) {
        a.remember_password = remember;
        a.auto_login = auto_login;
    }
    if auto_login {
        for a in inilla.launcher.accounts.iter_mut().filter(|a| a.id != id) {
            a.auto_login = false;
        }
    }
    inilla.launcher.last_server = Some(server.to_string());
    inilla.launcher.last_account = Some(id.clone());
    let key = format!("account:{id}");
    if !remember || password.is_empty() {
        if !remember {
            inilla_platform::keychain::delete(&key);
        }
        return true;
    }
    match inilla_platform::keychain::set(&key, password) {
        Ok(()) => true,
        Err(e) => {
            warn!("inilla: connect: keychain: {e}");
            false
        }
    }
}

pub(crate) struct ConnectPlugin;

impl Plugin for ConnectPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Connect>()
            .add_systems(Update, (poll_connect, chain_download).chain());
    }
}

/// A finished probe or sign-up, applied: the profile adopted, the form prefilled, the account
/// saved — or the sentence saying why not.
fn poll_connect(
    mut connect: ResMut<Connect>,
    mut inilla: ResMut<Inilla>,
    mut overlay: ResMut<Overlay>,
) {
    let finished = connect.job.as_ref().is_some_and(|(_, h)| h.is_finished());
    if !finished {
        return;
    }
    let Some((_, handle)) = connect.job.take() else {
        return;
    };
    let Ok(done) = handle.join() else {
        connect.status = Some((false, "the request crashed".into()));
        return;
    };
    match done {
        Done::Probe(addr, found, again) => {
            probed(&mut connect, &mut inilla, &mut overlay, addr, found, again)
        }
        Done::SignUp(result) => signed_up(&mut connect, &mut inilla, &mut overlay, result),
    }
}

fn probed(
    connect: &mut Connect,
    inilla: &mut Inilla,
    overlay: &mut Overlay,
    addr: Address,
    found: Probe,
    again: bool,
) {
    connect.pending = None;
    connect.token_refused = false;
    connect.kept = None;
    connect.account_status = None;
    match found {
        Probe::Described { info, base } => {
            let id = kc::adopt(&mut inilla.launcher, info.profile(&addr));
            info!(
                "inilla: connect {base} → {} ({}, {})",
                info.name,
                if info.software.is_empty() {
                    "unnamed"
                } else {
                    &info.software
                },
                info.core
            );
            connect.found = Some(Found {
                id,
                base,
                info: Some(*info),
            });
        }
        Probe::DataOnly { base } => {
            info!("inilla: connect {base} → an older data host, data only");
            let id = kc::adopt_data_host(&mut inilla.launcher, &addr, &base);
            connect.found = Some(Found {
                id,
                base,
                info: None,
            });
        }
        // The current server asked again on its own (its data host is down, its realm up): said
        // here — the player opened this screen, often to add another server, and stays on it.
        Probe::RealmOnly if again => {
            info!("inilla: connect {} → only the realm answers", addr.base);
            connect.found = None;
            connect.status = Some((false, words::only_the_realm(&addr.host)));
        }
        Probe::RealmOnly => {
            info!("inilla: connect {} → a plain realm on 3724", addr.host);
            connect.found = None;
            let p = kc::manual_profile(&addr, None);
            let note = words::plain_realm(&addr.host);
            // A server the launcher has for this logon already is edited, not added a second time.
            let known = kc::same_logon(&inilla.launcher, &p.host, p.port).cloned();
            overlay.server_form = Some(match known {
                Some(s) => {
                    if inilla.launcher.last_server.as_deref() != Some(s.id.as_str()) {
                        inilla.launcher.last_server = Some(s.id.clone());
                        inilla.launcher.last_account = None;
                    }
                    ServerForm::edit_noted(&s, note)
                }
                None => ServerForm::prefilled(&p, note),
            });
            overlay.panel = Some(Panel::Launcher);
        }
        Probe::Unusable { why } => {
            info!("inilla: connect {} → unusable: {why}", addr.base);
            connect.found = None;
            connect.status = Some((false, words::unusable(&why)));
        }
        Probe::Nothing { why } => {
            info!("inilla: connect {} → nothing: {why}", addr.base);
            connect.found = None;
            connect.status = Some((false, words::unreachable(&addr.host)));
            connect.detail = Some(why);
        }
    }
}

fn signed_up(
    connect: &mut Connect,
    inilla: &mut Inilla,
    overlay: &mut Overlay,
    result: Result<String, RegisterError>,
) {
    let Some(server) = connect.found.as_ref().map(|f| f.id.clone()) else {
        return;
    };
    match result {
        Ok(name) => {
            info!("inilla: connect: account {name} created");
            let Some(kept) = connect.kept.take() else {
                return;
            };
            if !save_account(
                inilla,
                &server,
                &name,
                &kept.password,
                kept.remember,
                kept.auto_login,
            ) {
                overlay.toast(words::NOT_SAVED);
            }
            connect.password = Some((name.clone(), kept.password));
            connect.account_status = Some((true, words::created(&name)));
            overlay.connect.account_saved();
        }
        Err(e) => {
            info!("inilla: connect: sign-up refused: {e:?}");
            if e == RegisterError::TokenRequired {
                connect.refuse_token(&server);
            }
            connect.account_status = Some((false, words::register_error(&e)));
        }
    }
}

/// The game files' chain: once the data jobs are idle, the manifest, then the download — by
/// itself on an unmetered network with room for it, else after the screen's tap. A 401 on the
/// manifest asks for the access code again.
fn chain_download(mut connect: ResMut<Connect>, inilla: Res<Inilla>, mut data: ResMut<DataState>) {
    let Some(found) = connect.found.clone() else {
        return;
    };
    let Some(profile) = inilla.launcher.server(&found.id).cloned() else {
        // The player deleted the server.
        connect.found = None;
        connect.pending = None;
        return;
    };
    // Only for the server the launcher is on (the player may have picked another), and only
    // between the data jobs.
    let current = inilla.launcher.current_server().map(|s| s.id.as_str());
    if current != Some(found.id.as_str()) || data.busy().is_some() {
        return;
    }
    let token = connect.token_for(&profile);
    let step = files_step(
        &found,
        &profile,
        data.status.complete(),
        token.is_some(),
        connect.token_refused,
    );
    let DataStep::Fetch { url, locale } = step else {
        connect.pending = None;
        return;
    };
    // Vanillish (Windows): nothing is downloaded into the player's own client, read in place.
    if data.reads_in_place() {
        connect.pending = None;
        return;
    }
    // A new chain for a new source or language (the player picked another one).
    let stale = connect
        .pending
        .as_ref()
        .is_none_or(|p| p.url != url || p.locale != locale);
    if stale {
        let metered = inilla_platform::device::network_metered();
        connect.pending = Some(PendingDownload {
            url,
            locale,
            token: token.clone(),
            metered,
            auto: metered == Some(false),
            state: Chain::Ask,
        });
    }
    let mut refused = false;
    if let Some(p) = connect.pending.as_mut() {
        match p.state {
            Chain::Ask => {
                // The offer shown must be this source's, not one the setup screen checked.
                data.manifest = None;
                data.fetch_manifest(p.url.clone(), p.token.clone(), p.locale.clone());
                p.state = Chain::Asking;
            }
            Chain::Asking => match &data.manifest {
                Some(_) => {
                    p.state = Chain::Offer;
                    let room = still_needed(&data).is_some_and(|need| room_for(&data, need));
                    if p.auto && room {
                        info!("inilla: connect: downloading the game files (unmetered network)");
                        data.download(p.token.clone());
                        p.state = Chain::Downloading;
                    }
                }
                None if data
                    .message
                    .as_ref()
                    .is_some_and(|(_, m)| m == TOKEN_REFUSED) =>
                {
                    refused = true;
                }
                None => p.state = Chain::Failed,
            },
            Chain::Downloading => {
                // The job is over without the whole set: paused, cut off or refused.
                if !data.status.complete() {
                    p.state = Chain::Stopped;
                }
            }
            Chain::Offer | Chain::Stopped | Chain::Failed => {}
        }
    }
    if refused {
        connect.refuse_token(&found.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use inilla_core::connect::{parse_address, register, sign_up, Post};
    use inilla_core::http::{self, Request};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};

    /// One sign-up as the stand-in host received it: its content type, its token, its body.
    type Received = (Option<String>, Option<String>, String);

    /// The app's own HTTP client (ureq) against a stand-in data host on loopback: `server.json`
    /// read, a sign-up posted as JSON with the access code, every refusal's JSON read from a 4xx
    /// — and the salt and verifier benilla's SRP6 made are ones the logon can prove.
    #[test]
    fn the_app_client_signs_up_with_a_verifier_the_logon_can_prove() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let posted: Arc<Mutex<Vec<Received>>> = Arc::default();
        let seen = posted.clone();
        std::thread::spawn(move || {
            http::serve(listener, move |req: Request, out: &mut TcpStream| {
                let _ = match (req.method.as_str(), req.path.as_str()) {
                    ("GET", "/server.json") => http::respond_json(
                        out,
                        200,
                        r#"{"format":1,"name":"Loop","core":"vmangos","build":5875,"auth":{"port":3724},
                            "registration":{"open":true,"token":true,"credentials":"srp6"}}"#,
                    ),
                    ("POST", "/accounts") => {
                        let body = String::from_utf8_lossy(&req.body).into_owned();
                        seen.lock().unwrap().push((
                            req.header("content-type").map(str::to_string),
                            req.token().map(str::to_string),
                            body.clone(),
                        ));
                        let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                        match v["username"].as_str() {
                            _ if req.token() != Some("code") => http::respond_json(
                                out,
                                401,
                                r#"{"ok":false,"error":"token_required","message":"The access code is needed."}"#,
                            ),
                            Some("TAKEN") => http::respond_json(
                                out,
                                409,
                                r#"{"ok":false,"error":"name_taken","message":"Taken."}"#,
                            ),
                            Some("BUSY") => http::respond_json(
                                out,
                                429,
                                r#"{"ok":false,"error":"rate_limited","message":"Later.","retry_after":540}"#,
                            ),
                            Some(name) => http::respond_json(
                                out,
                                201,
                                &format!(
                                    r#"{{"ok":true,"username":"{name}","message":"Created."}}"#
                                ),
                            ),
                            None => http::respond_json(
                                out,
                                400,
                                r#"{"ok":false,"error":"bad_request","message":"No name."}"#,
                            ),
                        }
                    }
                    _ => http::respond_text(out, 404, "not found"),
                };
            });
        });

        let addr = parse_address(&format!("127.0.0.1:{port}")).unwrap();
        let Probe::Described { info, base } = kc::probe(&Http::quick(None), &addr, &|_, _| false)
        else {
            panic!("the loopback host did not describe itself");
        };
        let rules = info.sign_up_rules().unwrap().clone();
        let url = info.register_url(&base).unwrap();
        let up = sign_up(&rules, "alice", "s3cret!", &make_verifier).unwrap();

        assert_eq!(
            register(&Http::quick(None), &url, &up),
            Err(RegisterError::TokenRequired)
        );
        let with_code = Http::quick(Some("code".into()));
        assert_eq!(register(&with_code, &url, &up), Ok("ALICE".into()));
        let taken = sign_up(&rules, "taken", "s3cret!", &make_verifier).unwrap();
        assert_eq!(
            register(&with_code, &url, &taken),
            Err(RegisterError::NameTaken)
        );
        let busy = sign_up(&rules, "busy", "s3cret!", &make_verifier).unwrap();
        assert_eq!(
            register(&with_code, &url, &busy),
            Err(RegisterError::RateLimited {
                retry_after: Some(540)
            })
        );
        assert_eq!(
            with_code.post_json(&format!("http://127.0.0.1:{port}/nowhere"), "{}"),
            Ok((404, "not found".into()))
        );

        // JSON, with the code; the password itself nowhere in the body.
        let posted = posted.lock().unwrap();
        let (kind, token, body) = &posted[1];
        assert_eq!(kind.as_deref(), Some("application/json"));
        assert_eq!(token.as_deref(), Some("code"));
        assert!(!body.to_ascii_uppercase().contains("S3CRET"), "{body}");

        // The pair is the logon's: the verifier is g^x for this salt, name and password, and the
        // salt's top bit is set (vmangos' `SetRand(256)`), so the account logs in.
        let v: serde_json::Value = serde_json::from_str(body).unwrap();
        let le = |hex: &str| -> Vec<u8> {
            let mut b: Vec<u8> = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                .collect();
            b.reverse();
            b
        };
        let salt: [u8; 32] = le(v["salt"].as_str().unwrap()).try_into().unwrap();
        let verifier = le(v["verifier"].as_str().unwrap());
        assert!(salt[31] & 0x80 != 0);
        let user = benilla_srp::NormalizedString::new("ALICE").unwrap();
        let pass = benilla_srp::NormalizedString::new("S3CRET!").unwrap();
        assert_eq!(
            benilla_srp::password_verifier(&user, &pass, &salt).to_vec(),
            verifier
        );
    }

    /// The whole flow against a real data host (`server/datahost`, `INILLA_SIGNUP=lan`, a token,
    /// a realm in a container), as the phone runs it: the app's own client, its SRP6, its chain's
    /// reading of a refused token. `VANILLISH_LIVE=<host:port> VANILLISH_LIVE_TOKEN=<token> cargo
    /// test -p benilla-app --lib a_real_data_host -- --ignored --nocapture`; it prints the account
    /// it made, whose `s`/`v` rows the database must hold as posted.
    #[test]
    #[ignore = "needs a running data host: $VANILLISH_LIVE"]
    fn a_real_data_host_takes_the_whole_flow() {
        use inilla_core::data as kd;
        use inilla_core::profiles::Launcher;
        let typed = std::env::var("VANILLISH_LIVE").expect("$VANILLISH_LIVE");
        let token = std::env::var("VANILLISH_LIVE_TOKEN").ok();
        let addr = parse_address(&typed).unwrap();
        let Probe::Described { info, base } = kc::probe(&Http::quick(None), &addr, &tcp_listens)
        else {
            panic!("{typed} did not describe itself");
        };
        println!("described: {info:?}");
        let mut launcher = Launcher::default();
        let id = kc::adopt(&mut launcher, info.profile(&addr));
        let p = launcher.server(&id).unwrap().clone();
        println!(
            "profile: {} {}:{} realm={:?} override={:?} data={:?}",
            p.name, p.host, p.port, p.realm_name, p.realm_override, p.data
        );
        assert_eq!(p.host, addr.host);

        // The data: a refused token reads as the one sentence the chain asks the code again on.
        let need_token = info.data.as_ref().is_some_and(|d| d.token);
        let step = data_step(&info, &base, false, None, token.is_some());
        let DataStep::Fetch { url, .. } = step else {
            panic!("{step:?}");
        };
        if need_token {
            assert_eq!(
                kd::fetch_manifest(&Http::new(None), &url).err().as_deref(),
                Some(TOKEN_REFUSED)
            );
        }
        let http = Http::new(token.clone());
        let m = kd::fetch_manifest(&http, &url).unwrap();
        let dir = std::env::temp_dir().join(format!("vanillish-live-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        kd::download(&http, &url, &m, &dir, |_| {}, &|| true).unwrap();
        assert!(kd::verify(&m, &dir, |_, _, _| {}).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);

        // The account: refused without the code when it needs one, made with it, the lost answer
        // replayed, another salt for the same name taken.
        let rules = info.sign_up_rules().expect("sign-up open").clone();
        let register_url = info.register_url(&base).unwrap();
        let name = kc::suggest_name(rand::random());
        let up = sign_up(&rules, &name, "s3cret!", &make_verifier).unwrap();
        if rules.token {
            assert_eq!(
                register(&Http::quick(None), &register_url, &up),
                Err(RegisterError::TokenRequired)
            );
        }
        let with = Http::quick(token);
        assert_eq!(register(&with, &register_url, &up), Ok(name.clone()));
        assert_eq!(register(&with, &register_url, &up), Ok(name.clone()));
        let again = sign_up(&rules, &name, "s3cret!", &make_verifier).unwrap();
        assert_eq!(
            register(&with, &register_url, &again),
            Err(RegisterError::NameTaken)
        );
        println!("created {name}: {}", up.json);
    }

    fn inilla() -> Inilla {
        Inilla {
            settings: Default::default(),
            launcher: Default::default(),
            layout: Default::default(),
            other_layouts: Default::default(),
            pad: Default::default(),
        }
    }

    /// What the full stack describes: one realm in a container, the data behind no token.
    fn stack() -> ServerInfo {
        kc::parse_server_info(
            r#"{"format":1,"name":"Home","core":"vmangos","build":5875,"auth":{"port":3724},
                "realms":[{"name":"Home","world":{"address":"172.18.0.3","port":8085,"same_host":true}}],
                "data":{"url":"","token":false,"default":"enUS",
                        "locales":[{"locale":"enUS","path":"enUS/","bytes":5400000000}]},
                "registration":{"open":true,"credentials":"srp6"}}"#,
        )
        .unwrap()
    }

    /// A refused access code is asked for again — even where the description says none is needed
    /// (one set since it was read): the chain must not ask the host again and again.
    #[test]
    fn a_refused_code_is_asked_for_whatever_the_description_says() {
        let addr = parse_address("192.168.1.20").unwrap();
        let info = stack();
        let profile = info.profile(&addr);
        let found = Found {
            id: profile.id.clone(),
            base: addr.base.clone(),
            info: Some(info),
        };
        let fetch = DataStep::Fetch {
            url: addr.base.clone(),
            locale: None,
        };
        assert_eq!(files_step(&found, &profile, false, false, false), fetch);
        assert_eq!(
            files_step(&found, &profile, false, false, true),
            DataStep::AskToken
        );
        assert_eq!(
            files_step(&found, &profile, false, true, true),
            DataStep::AskToken
        );
        // The archives are all here: nothing to ask.
        assert_eq!(
            files_step(&found, &profile, true, false, true),
            DataStep::Have
        );
        // An older data host: its manifest, until it refuses the code.
        let older = Found {
            info: None,
            ..found
        };
        let mut by_hand = kc::manual_profile(&addr, Some(&addr.base));
        by_hand.id = older.id.clone();
        assert_eq!(files_step(&older, &by_hand, false, false, false), fetch);
        assert_eq!(
            files_step(&older, &by_hand, false, false, true),
            DataStep::AskToken
        );
    }

    /// The current server is asked again on its own once; a Connect the player typed that found
    /// nothing is not followed by the old server's answer.
    #[test]
    fn the_current_server_is_asked_again_once_and_never_over_a_typed_answer() {
        let mut c = Connect::default();
        let mut known = ServerProfile::new("Home", "192.168.1.20");
        known.info = Some("http://192.168.1.20:8099/".into());
        assert_eq!(
            c.reask(Some(&known)).as_deref(),
            Some("http://192.168.1.20:8099/")
        );
        assert_eq!(c.reask(None), None);
        assert_eq!(c.reask(Some(&ServerProfile::new("x", "10.0.0.1"))), None);
        c.asked = known.info.clone();
        assert_eq!(c.reask(Some(&known)), None, "once");
        // The player typed another address, and nothing answered there.
        let (mut i, mut o) = (inilla(), Overlay::default());
        i.launcher.last_server = Some(i.launcher.add_server(known.clone()));
        let closed = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = closed.local_addr().unwrap().port();
        drop(closed);
        c.start_probe(parse_address(&format!("127.0.0.1:{port}")).unwrap());
        assert_eq!(c.busy(), Some(Busy::Probing));
        assert_eq!(
            c.reask(i.launcher.current_server()),
            None,
            "one job at a time"
        );
        let (_, job) = c.job.take().unwrap();
        let Done::Probe(addr, found, again) = job.join().unwrap() else {
            panic!("not a probe");
        };
        assert!(
            !again && matches!(found, Probe::Nothing { .. }),
            "{found:?}"
        );
        probed(&mut c, &mut i, &mut o, addr, found, again);
        assert!(c
            .status
            .as_ref()
            .is_some_and(|(ok, s)| !ok && s.contains("127.0.0.1")));
        assert_eq!(c.asked, known.info, "a typed address is not counted");
        assert_eq!(c.reask(i.launcher.current_server()), None);
        // Another server Connect set up, picked in the launcher: asked once.
        let mut other = ServerProfile::new("Away", "10.0.0.8");
        other.info = Some("http://10.0.0.8:8099/".into());
        assert!(c.reask(Some(&other)).is_some());
    }

    /// A plain realm found again is the launcher's server edited, never a second one; asked again
    /// on its own it is only said, and the screen stays.
    #[test]
    fn a_plain_realm_found_again_is_edited_not_added() {
        let (mut c, mut i, mut o) = (Connect::default(), inilla(), Overlay::default());
        let mut mine = ServerProfile::new("Mine", "192.168.1.20");
        mine.realm_override = Some("192.168.1.20".into());
        let id = i.launcher.add_server(mine);
        i.launcher.last_server = Some(
            i.launcher
                .add_server(ServerProfile::new("Other", "10.0.0.8")),
        );
        o.panel = Some(Panel::Connect);
        let addr = parse_address("192.168.1.20:3724").unwrap();
        probed(&mut c, &mut i, &mut o, addr.clone(), Probe::RealmOnly, true);
        assert_eq!(o.panel, Some(Panel::Connect));
        assert!(o.server_form.is_none());
        assert!(c.status.as_ref().is_some_and(|(ok, _)| !ok));
        probed(&mut c, &mut i, &mut o, addr, Probe::RealmOnly, false);
        assert_eq!(o.panel, Some(Panel::Launcher));
        assert_eq!(
            o.server_form.as_ref().and_then(ServerForm::editing),
            Some(id.as_str())
        );
        assert_eq!(i.launcher.last_server.as_deref(), Some(id.as_str()));
        assert_eq!(i.launcher.servers.len(), 2);
        // A realm the launcher does not have: a new server's form.
        let new = parse_address("10.0.0.99").unwrap();
        probed(&mut c, &mut i, &mut o, new, Probe::RealmOnly, false);
        assert_eq!(o.server_form.as_ref().and_then(ServerForm::editing), None);
    }

    /// A server that described itself, answered once by its data host alone, keeps all it said.
    #[test]
    fn an_older_answer_keeps_the_profile_whole() {
        let (mut c, mut i, mut o) = (Connect::default(), inilla(), Overlay::default());
        let addr = parse_address("mac-mini.local").unwrap();
        let info = stack();
        probed(
            &mut c,
            &mut i,
            &mut o,
            addr.clone(),
            Probe::Described {
                info: Box::new(info),
                base: addr.base.clone(),
            },
            false,
        );
        let before = i.launcher.current_server().cloned().unwrap();
        assert_eq!(before.realm_override.as_deref(), Some("mac-mini.local"));
        probed(
            &mut c,
            &mut i,
            &mut o,
            addr.clone(),
            Probe::DataOnly {
                base: addr.base.clone(),
            },
            true,
        );
        assert_eq!(i.launcher.current_server(), Some(&before));
        assert!(c
            .found
            .as_ref()
            .is_some_and(|f| f.info.is_none() && f.id == before.id));
    }

    /// Nothing listens on the port: the probe says so with the network's reason, quickly.
    #[test]
    fn a_silent_address_is_nothing() {
        let closed = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = closed.local_addr().unwrap().port();
        drop(closed);
        let addr = parse_address(&format!("127.0.0.1:{port}")).unwrap();
        let found = kc::probe(&Http::quick(None), &addr, &|_, _| false);
        assert!(matches!(found, Probe::Nothing { .. }), "{found:?}");
        assert!(!tcp_listens("127.0.0.1", port));
    }
}

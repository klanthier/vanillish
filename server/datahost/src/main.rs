//! **inilla-datahost** (Vanillish's data host) — hand your own World of Warcraft 1.12.1 `Data`
//! folder to your own phone, once, with the addons that go with it; tell the app what this server
//! is; create the player's account; and pass the owner's admin menu on to the world server. The
//! app's Connect screen (Vanillish) or setup screen (iNilla) downloads the data from here
//! (resumable, SHA-256 verified).
//!
//! ```text
//! inilla-datahost serve --data "/games/WoW 1.12.1/Data"
//!     → http://<this machine>:8099/   (type this machine's address in the app)
//! inilla-datahost serve --data enUS=/games/enUS/Data --data deDE=/games/deDE/Data \
//!                       --addons /addons/all --addons deDE=/addons/deDE --token s3cret
//!     → the same, in two languages, with addons, behind a token
//! inilla-datahost serve --realm-name Vanillish --world-same-host --signup lan
//!     → no game data (yet): /server.json and sign-up only
//! INILLA_ADMIN_CODE="$ADMIN_CODE" inilla-datahost serve --admin-agent mangosd:8091
//!     → and the admin menu, from the LAN (the code from the environment only, never a flag)
//! inilla-datahost manifest --data "/games/WoW 1.12.1/Data"
//!     → prints the manifest (and caches it) without serving
//! ```
//!
//! It serves exactly what the client mounts — the MPQs at the root of each `Data/` — and the
//! addons it was given, nothing else:
//!
//! | request | answer |
//! |---|---|
//! | `GET\|HEAD /server.json` | what this server is: logon, realm, data, sign-up ([`info`]); never needs the token |
//! | `POST /accounts` | a player account, from the salt and verifier the phone computed ([`accounts`]) |
//! | `GET /admin/nonce`, `GET /admin/status`, `POST /admin/{rates,rates/reset,gold}` | the admin menu, signed with the admin code, passed on to the world server's agent ([`admin`]) |
//! | `GET /locales.json` | the languages this host has game data for, the first one the default |
//! | `GET [/<locale>]/manifest.json` | the archives: names, sizes, SHA-256 |
//! | `GET\|HEAD [/<locale>]/files/<name>` | one archive, with `Range`, so a download resumes |
//! | `GET [/<locale>]/addons.json` | the addons for that language, every file hashed |
//! | `GET\|HEAD [/<locale>]/addons/<path>` | one addon file |
//!
//! Without a `/<locale>` prefix a request means the first `--data`, so an app that knows only
//! `/manifest.json` keeps working. With no `--data` folder holding archives the host still starts:
//! `/server.json` leaves `data` out, sign-up works, and the data requests are 404s. Hashing 5 GB
//! takes a minute or two; each manifest is cached (`.inilla-manifest-<locale>.json`, beside the
//! data or in `--cache-dir`) and rebuilt when an archive's size or mtime changes.
//!
//! The files are Blizzard's. Serve them only to yourself and people who own the game, and prefer
//! a LAN or a VPN (Tailscale, WireGuard) to the open internet — with a token there, which sign-up
//! then asks for too.

mod accounts;
mod addons;
mod admin;
mod http;
mod info;
mod manifest;

use std::fs;
use std::io::{Seek, SeekFrom};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Instant, UNIX_EPOCH};

use clap::builder::FalseyValueParser;
use clap::{Parser, Subcommand};

use crate::accounts::{DbConfig, Mode, Signup};
use crate::addons::AddonSet;
use crate::admin::Admin;
use crate::http::{Body, Head, Request};
use crate::manifest::Manifest;

#[derive(Parser)]
#[command(name = "inilla-datahost", version, about)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Serve the Data folder(s) and addons, /server.json, sign-up and the admin menu.
    Serve {
        /// A client's `Data` folder, as `DIR` or `LOCALE=DIR` (e.g. `deDE=/games/de/Data`).
        /// Repeat for more languages; the first is the default. A folder without archives is
        /// skipped, so an optional language can stay mounted empty; with none at all the host
        /// serves /server.json and sign-up only.
        #[arg(long = "data")]
        data: Vec<String>,
        /// A folder of addons (`<Addon>/<Addon>.toc`), as `DIR` for every language or
        /// `LOCALE=DIR` for one. Repeatable; later folders replace an addon of the same name.
        #[arg(long = "addons")]
        addons: Vec<String>,
        /// Address to listen on.
        #[arg(long, env = "INILLA_DATA_BIND", default_value = "0.0.0.0:8099")]
        bind: String,
        /// Require `Authorization: Bearer <token>` (or `?token=`) for the data and sign-up.
        /// Without one the host is open to everyone who can reach it: fine on a home LAN, not
        /// on the internet.
        #[arg(long, env = "INILLA_DATA_TOKEN", hide_env_values = true)]
        token: Option<String>,
        /// Accepted for compatibility: no token is now simply the default.
        #[arg(long, hide = true)]
        no_token: bool,
        /// The locale of a `--data` given without one.
        #[arg(long, env = "INILLA_DATA_LOCALE", default_value = "enUS")]
        locale: String,
        /// Where to cache the manifests, when the Data folders are read-only.
        #[arg(long, env = "INILLA_DATA_CACHE_DIR")]
        cache_dir: Option<PathBuf>,
        #[command(flatten)]
        server: Box<ServerArgs>,
    },
    /// Build (or refresh) the manifest and print it.
    Manifest {
        #[arg(long, env = "INILLA_DATA_DIR")]
        data: PathBuf,
        #[arg(long, env = "INILLA_DATA_LOCALE", default_value = "enUS")]
        locale: String,
        #[arg(long, env = "INILLA_DATA_CACHE_DIR")]
        cache_dir: Option<PathBuf>,
    },
}

/// What `/server.json` says about the server beside this host, and how sign-up reaches realmd's
/// database. An empty value (compose's `${X:-}`) is the same as none.
#[derive(clap::Args, Clone, Default)]
struct ServerArgs {
    /// The server's name in the app [default: the realm's, else Vanillish].
    #[arg(long, env = "INILLA_SERVER_NAME")]
    server_name: Option<String>,
    /// The core, shown in the app: vmangos, cmangos, … [default: not said].
    #[arg(long, env = "INILLA_CORE")]
    core: Option<String>,
    /// realmd's host, when it is not this machine [default: the address the app reached this
    /// host at].
    #[arg(long, env = "INILLA_AUTH_HOST")]
    auth_host: Option<String>,
    /// realmd's published port [default: 3724, said when a realm or a logon host is given].
    #[arg(long, env = "INILLA_AUTH_PORT")]
    auth_port: Option<String>,
    /// The realm's name, as realmd lists it.
    #[arg(long, env = "INILLA_REALM_NAME")]
    realm_name: Option<String>,
    /// The realm's type: pve, pvp, rp or rppvp.
    #[arg(long, env = "INILLA_REALM_TYPE")]
    realm_type: Option<String>,
    /// The world server's address as the realm list hands it out (REALM_ADDRESS).
    #[arg(long, env = "INILLA_WORLD_ADDRESS")]
    world_address: Option<String>,
    /// The world server's port [default: 8085].
    #[arg(long, env = "INILLA_WORLD_PORT")]
    world_port: Option<String>,
    /// The world server runs on this machine: the app then reaches it at the address it typed.
    #[arg(long, env = "INILLA_WORLD_SAME_HOST", value_parser = FalseyValueParser::new())]
    world_same_host: bool,
    /// A sentence for the players, kept in the app's notes for the server.
    #[arg(long, env = "INILLA_MESSAGE")]
    message: Option<String>,
    /// Account sign-up: off, lan (from private addresses only) or open.
    #[arg(long, env = "INILLA_SIGNUP", default_value = "off", value_parser = Mode::parse)]
    signup: Mode,
    /// Sign-up's database host [default: db].
    #[arg(long, env = "INILLA_SIGNUP_DB_HOST")]
    signup_db_host: Option<String>,
    /// Sign-up's database port [default: 3306].
    #[arg(long, env = "INILLA_SIGNUP_DB_PORT")]
    signup_db_port: Option<String>,
    /// Sign-up's database user, made by db-init with insert-only rights [default:
    /// vanillish_signup].
    #[arg(long, env = "INILLA_SIGNUP_DB_USER")]
    signup_db_user: Option<String>,
    /// That user's password (SIGNUP_DB_PASSWORD). Without one sign-up stays off.
    #[arg(long, env = "INILLA_SIGNUP_DB_PASSWORD", hide_env_values = true)]
    signup_db_password: Option<String>,
    /// realmd's database [default: realmd].
    #[arg(long, env = "INILLA_SIGNUP_DB_NAME")]
    signup_db_name: Option<String>,
    /// The admin menu: lan (from private addresses only), open or off. It is on only with
    /// --admin-agent and a usable code in the environment, INILLA_ADMIN_CODE (ADMIN_CODE: 16
    /// or more of the setup's symbols, or 20 or more hex digits; no flag, so it never shows in
    /// `ps`).
    #[arg(long, env = "INILLA_ADMIN", default_value = "lan", value_parser = admin_mode)]
    admin: Mode,
    /// The world server's admin agent, host:port (compose: mangosd:8091) [default: none, the
    /// admin menu off].
    #[arg(long, env = "INILLA_ADMIN_AGENT")]
    admin_agent: Option<String>,
}

/// `INILLA_ADMIN`: like sign-up's mode, but empty (compose's `${X:-}`) is the default, lan.
fn admin_mode(s: &str) -> Result<Mode, String> {
    if s.trim().is_empty() {
        Ok(Mode::Lan)
    } else {
        Mode::parse(s)
    }
}

/// A setting that was given: neither absent nor empty.
fn given(v: &Option<String>) -> Option<String> {
    v.as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn port(flag: &str, v: &Option<String>) -> Result<Option<u16>, String> {
    given(v)
        .map(|p| {
            p.parse::<u16>()
                .ok()
                .filter(|p| *p != 0)
                .ok_or_else(|| format!("{flag} {p:?} is not a port"))
        })
        .transpose()
}

impl ServerArgs {
    fn realm(&self) -> Result<info::Realm, String> {
        Ok(info::Realm {
            server_name: given(&self.server_name),
            core: given(&self.core),
            auth_host: given(&self.auth_host),
            auth_port: port("--auth-port", &self.auth_port)?,
            realm_name: given(&self.realm_name),
            realm_type: given(&self.realm_type),
            world_address: given(&self.world_address),
            world_port: port("--world-port", &self.world_port)?,
            world_same_host: self.world_same_host,
            message: given(&self.message),
        })
    }

    /// Sign-up as configured; `token` whether the host has one (sign-up then asks for it too).
    fn signup(&self, token: bool) -> Result<Signup, String> {
        if self.signup == Mode::Off {
            return Ok(Signup::off());
        }
        let Some(password) = self.signup_db_password.clone().filter(|p| !p.is_empty()) else {
            eprintln!(
                "inilla-datahost: sign-up stays off: INILLA_SIGNUP={} needs \
                 INILLA_SIGNUP_DB_PASSWORD, the sign-up user's database password",
                self.signup.as_str()
            );
            return Ok(Signup::off());
        };
        let db = DbConfig {
            host: given(&self.signup_db_host).unwrap_or_else(|| "db".into()),
            port: port("--signup-db-port", &self.signup_db_port)?.unwrap_or(3306),
            user: given(&self.signup_db_user).unwrap_or_else(|| "vanillish_signup".into()),
            password,
            name: given(&self.signup_db_name).unwrap_or_else(|| "realmd".into()),
        };
        Ok(Signup::new(
            self.signup,
            token,
            Box::new(accounts::Database::new(&db)),
        ))
    }

    /// The admin menu as configured; `code` is INILLA_ADMIN_CODE, read from the environment.
    fn admin(&self, code: Option<&str>) -> Result<Admin, String> {
        let agent = given(&self.admin_agent)
            .map(|a| match a.rsplit_once(':') {
                Some((host, p)) if !host.is_empty() && p.parse::<u16>().is_ok_and(|p| p != 0) => {
                    Ok(a)
                }
                _ => Err(format!("--admin-agent {a:?} is not host:port")),
            })
            .transpose()?;
        let code = code.unwrap_or_default();
        let backend =
            agent.map(|a| Box::new(admin::Agent::new(&a, code)) as Box<dyn admin::Backend>);
        Ok(Admin::new(self.admin, code, backend))
    }
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.cmd {
        Cmd::Serve {
            data,
            addons,
            bind,
            token,
            no_token: _,
            locale,
            cache_dir,
            server,
        } => serve(
            &data,
            &addons,
            &bind,
            token,
            &locale,
            cache_dir.as_deref(),
            &server,
        ),
        Cmd::Manifest {
            data,
            locale,
            cache_dir,
        } => manifest(&data, &locale, cache_dir.as_deref()).map(|m| {
            println!("{}", serde_json::to_string_pretty(&m).unwrap_or_default());
        }),
    };
    if let Err(e) = result {
        eprintln!("inilla-datahost: {e}");
        std::process::exit(1);
    }
}

/// `deDE=/path` → `(Some("deDE"), "/path")`; a plain path has no locale. A locale is four ASCII
/// letters, so a Windows drive (`C:\…`) or a path with `=` further in is still just a path.
fn split_locale(arg: &str) -> (Option<&str>, &str) {
    match arg.split_once('=') {
        Some((l, rest)) if l.len() == 4 && l.chars().all(|c| c.is_ascii_alphabetic()) => {
            (Some(l), rest)
        }
        _ => (None, arg),
    }
}

/// The cache's key: every archive's name, size and mtime, and the locale it was built for.
fn fingerprint(dir: &Path, locale: &str) -> String {
    let st = manifest::status(dir);
    let files = st
        .present
        .iter()
        .map(|(n, size)| {
            let mtime = fs::metadata(dir.join(n))
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs());
            format!("{n}:{size}:{mtime}")
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("{locale}|{files}")
}

fn manifest(dir: &Path, locale: &str, cache_dir: Option<&Path>) -> Result<Manifest, String> {
    let st = manifest::status(dir);
    if st.present.is_empty() {
        return Err(format!("no 1.12.1 archives in {}", dir.display()));
    }
    if !st.missing_required.is_empty() {
        eprintln!(
            "inilla-datahost: warning — {} is missing {} (the app will refuse this copy)",
            dir.display(),
            st.missing_required.join(", ")
        );
    }
    if !st.missing_expected.is_empty() {
        eprintln!(
            "inilla-datahost: note — {} has no {}",
            dir.display(),
            st.missing_expected.join(", ")
        );
    }
    let cache = cache_dir
        .unwrap_or(dir)
        .join(format!(".inilla-manifest-{locale}.json"));
    let fp = fingerprint(dir, locale);
    if let Ok(text) = fs::read_to_string(&cache) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            if v["fingerprint"].as_str() == Some(fp.as_str()) {
                if let Ok(m) = serde_json::from_value::<Manifest>(v["manifest"].clone()) {
                    eprintln!(
                        "inilla-datahost: {locale} manifest from cache {}",
                        cache.display()
                    );
                    return Ok(m);
                }
            }
        }
    }
    eprintln!(
        "inilla-datahost: hashing {} {locale} archives ({:.1} GB)…",
        st.present.len(),
        st.total_bytes() as f64 / 1e9
    );
    let mut last = u64::MAX;
    let m = Manifest::build_from(dir, Some(locale.to_string()), |name, done, total| {
        // Whole percent steps, one line each: this usually runs under `docker logs`, not a TTY.
        let pct = done * 100 / total.max(1);
        if pct != last && pct % 5 == 0 {
            last = pct;
            eprintln!("  {pct:3}%  {name}");
        }
    })
    .map_err(|e| e.to_string())?;
    let body = serde_json::json!({ "fingerprint": fp, "manifest": m });
    // A read-only install with no cache dir just goes uncached.
    if let Err(e) = fs::write(&cache, body.to_string()) {
        eprintln!(
            "inilla-datahost: not caching the manifest ({}: {e})",
            cache.display()
        );
    }
    Ok(m)
}

/// One language's game data and addons.
struct DataSet {
    locale: String,
    dir: PathBuf,
    manifest: Manifest,
    manifest_json: String,
    addons: AddonSet,
}

struct Host {
    /// The first is the default: what a request without a `/<locale>` prefix gets. Empty: this
    /// host serves no game data (yet).
    sets: Vec<DataSet>,
    /// `/locales.json`, when there is game data.
    locales_json: Option<String>,
    token: Option<String>,
    /// `/server.json` for a requester at a private address, and for any other: they differ only
    /// in whether sign-up and the admin menu are open to it.
    server_json_private: String,
    server_json_public: String,
    signup: Signup,
    admin: Admin,
}

impl Host {
    fn new(
        sets: Vec<DataSet>,
        token: Option<String>,
        realm: &info::Realm,
        signup: Signup,
        admin: Admin,
    ) -> Self {
        let locales = locales(&sets);
        let server_json = |private: bool| {
            let registration = signup.registration(private);
            let section = admin.section(private);
            info::server_json(
                realm,
                &locales,
                token.is_some(),
                registration.as_ref(),
                section.as_ref(),
            )
        };
        let (server_json_private, server_json_public) = (server_json(true), server_json(false));
        Host {
            locales_json: info::Locales::of(&locales).and_then(|l| serde_json::to_string(&l).ok()),
            sets,
            token,
            server_json_private,
            server_json_public,
            signup,
            admin,
        }
    }

    /// The data set a path names, and the rest of the path: `/deDE/files/x` → (deDE, `/files/x`).
    /// `None` without game data.
    fn route<'a>(&self, path: &'a str) -> Option<(&DataSet, &'a str)> {
        if let Some(rest) = path.strip_prefix('/') {
            let (first, tail) = rest.split_once('/').unwrap_or((rest, ""));
            if let Some(set) = self
                .sets
                .iter()
                .find(|s| s.locale.eq_ignore_ascii_case(first))
            {
                return Some((
                    set,
                    if tail.is_empty() {
                        "/"
                    } else {
                        &path[first.len() + 1..]
                    },
                ));
            }
        }
        self.sets.first().map(|set| (set, path))
    }
}

fn load(
    data: &[String],
    addons: &[String],
    default_locale: &str,
    cache_dir: Option<&Path>,
) -> Result<Vec<DataSet>, String> {
    let mut sets = Vec::new();
    for arg in data {
        let (locale, dir) = split_locale(arg);
        let locale = locale.unwrap_or(default_locale).to_string();
        let dir = PathBuf::from(dir);
        if manifest::status(&dir).present.is_empty() {
            eprintln!(
                "inilla-datahost: no archives in {} — {locale} is not offered",
                dir.display()
            );
            continue;
        }
        if sets
            .iter()
            .any(|s: &DataSet| s.locale.eq_ignore_ascii_case(&locale))
        {
            return Err(format!(
                "two --data folders with archives are both {locale}"
            ));
        }
        let m = manifest(&dir, &locale, cache_dir)?;
        let addon_dirs: Vec<PathBuf> = addons
            .iter()
            .filter_map(|a| match split_locale(a) {
                (None, d) => Some(PathBuf::from(d)),
                (Some(l), d) if l.eq_ignore_ascii_case(&locale) => Some(PathBuf::from(d)),
                _ => None,
            })
            .collect();
        let addons = addons::scan(&addon_dirs, &locale)?;
        sets.push(DataSet {
            manifest_json: serde_json::to_string(&m).map_err(|e| e.to_string())?,
            manifest: m,
            locale,
            dir,
            addons,
        });
    }
    Ok(sets)
}

/// The languages, as `/locales.json` and `server.json` list them.
fn locales(sets: &[DataSet]) -> Vec<info::Locale> {
    sets.iter()
        .map(|s| info::Locale {
            locale: s.locale.clone(),
            path: format!("{}/", s.locale),
            bytes: s.manifest.total_bytes(),
            addons: s.addons.count,
        })
        .collect()
}

fn serve(
    data: &[String],
    addons: &[String],
    bind: &str,
    token: Option<String>,
    locale: &str,
    cache_dir: Option<&Path>,
    server: &ServerArgs,
) -> Result<(), String> {
    let token = token.filter(|t| !t.trim().is_empty());
    let realm = server.realm()?;
    let signup = server.signup(token.is_some())?;
    let admin = server.admin(std::env::var("INILLA_ADMIN_CODE").ok().as_deref())?;
    let sets = load(data, addons, locale, cache_dir)?;
    let host = Arc::new(Host::new(sets, token, &realm, signup, admin));
    let listener = TcpListener::bind(bind).map_err(|e| format!("cannot listen on {bind}: {e}"))?;
    if host.sets.is_empty() {
        eprintln!(
            "inilla-datahost: no game data here (no --data folder holds 1.12.1 archives): \
             /server.json says so, and sign-up works when it is on"
        );
    }
    for s in &host.sets {
        eprintln!(
            "inilla-datahost: {} — {} archives ({:.1} GB) from {}, {} addon(s)",
            s.locale,
            s.manifest.files.len(),
            s.manifest.total_bytes() as f64 / 1e9,
            s.dir.display(),
            s.addons.count
        );
    }
    eprintln!(
        "inilla-datahost: serving on http://{bind}/  {}",
        if host.token.is_some() {
            "(token required)"
        } else {
            "(open: anyone who can reach this port can download)"
        }
    );
    eprintln!("inilla-datahost: {}", host.signup.describe());
    eprintln!("inilla-datahost: {}", host.admin.describe());
    http::serve(listener, move |req, stream| handle(&host, req, stream));
    Ok(())
}

fn handle(host: &Host, req: Request, w: &mut TcpStream) {
    let authorized = match &host.token {
        None => true,
        Some(t) => req.token().is_some_and(|given| http::token_eq(given, t)),
    };
    let peer = req.peer.parse::<SocketAddr>().ok().map(|a| a.ip());
    if req.path == "/accounts" {
        accounts(host, &req, peer, authorized, w);
        return;
    }
    // Its own signature, not the data token; and POSTs, so before the GET/HEAD gate.
    if req.path == "/admin" || req.path.starts_with("/admin/") {
        admin(host, &req, peer, w);
        return;
    }
    let head_only = req.method == "HEAD";
    if req.method != "GET" && !head_only {
        let _ = http::respond_text(w, 405, "GET or HEAD", false);
        if matches!(req.body, Body::TooLarge(_) | Body::Unreadable) {
            http::drain(w, 1 << 20);
        }
        return;
    }
    if req.path == "/server.json" {
        eprintln!("{} server.json", req.peer);
        let doc = if peer.is_some_and(accounts::is_private) {
            &host.server_json_private
        } else {
            &host.server_json_public
        };
        let _ = http::respond(w, 200, "application/json", doc.as_bytes(), head_only);
        return;
    }
    if req.path == "/" {
        // Apps that predate /server.json know a data host by this line's first word.
        let _ = http::respond_text(
            w,
            200,
            "inilla-datahost: type this machine's address in Vanillish, or this address in \
             iNilla → Game data.\n",
            head_only,
        );
        return;
    }
    if !authorized {
        let _ = http::respond_text(w, 401, "token required", head_only);
        return;
    }
    let no_data = "this host serves no game data";
    if req.path == "/locales.json" {
        let _ = match &host.locales_json {
            Some(json) => http::respond(w, 200, "application/json", json.as_bytes(), head_only),
            None => http::respond_text(w, 404, no_data, head_only),
        };
        return;
    }
    let Some((set, path)) = host.route(&req.path) else {
        let _ = http::respond_text(w, 404, no_data, head_only);
        return;
    };
    match path {
        "/manifest.json" => {
            eprintln!("{} {} manifest", req.peer, set.locale);
            let _ = http::respond(
                w,
                200,
                "application/json",
                set.manifest_json.as_bytes(),
                head_only,
            );
        }
        "/addons.json" => {
            eprintln!("{} {} addons", req.peer, set.locale);
            let _ = http::respond(
                w,
                200,
                "application/json",
                set.addons.manifest_json.as_bytes(),
                head_only,
            );
        }
        p if p.starts_with("/files/") => {
            let name = &p["/files/".len()..];
            // Only what the manifest lists — never a path into the rest of the disk.
            let Some(file) = set.manifest.files.iter().find(|f| f.name == name) else {
                let _ = http::respond_text(w, 404, "not an archive this host serves", head_only);
                return;
            };
            send_file(&set.dir.join(&file.name), file.size, &req, head_only, w);
        }
        p if p.starts_with("/addons/") => {
            let rel = &p["/addons/".len()..];
            let Some((path, size)) = set.addons.files.get(rel) else {
                let _ = http::respond_text(w, 404, "not an addon file this host serves", head_only);
                return;
            };
            send_file(path, *size, &req, head_only, w);
        }
        _ => {
            let _ = http::respond_text(w, 404, "not found", head_only);
        }
    }
}

/// `/accounts`: a sign-up when `POST`ed, JSON whatever the answer.
fn accounts(host: &Host, req: &Request, peer: Option<IpAddr>, authorized: bool, w: &mut TcpStream) {
    let reply = if req.method == "POST" {
        let ask = accounts::Request {
            peer,
            authorized,
            content_type: req.header("content-type"),
            body: &req.body,
        };
        host.signup.handle(&ask, Instant::now())
    } else {
        accounts::Reply::refuse(405, "bad_request", "POST a new account here.")
    };
    // The name and the outcome only: never a salt, a verifier or the token.
    eprintln!("{} accounts {} {}", req.peer, reply.status, reply.note);
    let mut extra = Vec::new();
    if let Some(secs) = reply.retry_after {
        extra.push(("Retry-After", secs.to_string()));
    }
    if reply.status == 405 {
        extra.push(("Allow", "POST".to_string()));
    }
    let _ = http::respond_with(
        w,
        reply.status,
        "application/json",
        reply.body.as_bytes(),
        req.method == "HEAD",
        &extra,
    );
    if matches!(req.body, Body::TooLarge(_) | Body::Unreadable) {
        http::drain(w, 1 << 20);
    }
}

/// `/admin/*`: the admin menu, JSON whatever the answer.
fn admin(host: &Host, req: &Request, peer: Option<IpAddr>, w: &mut TcpStream) {
    let ask = admin::Request {
        method: &req.method,
        path: &req.path,
        peer,
        authorization: req.header("authorization"),
        content_type: req.header("content-type"),
        body: &req.body,
    };
    let reply = host.admin.handle(&ask, Instant::now());
    eprintln!("{}", admin_log(&req.peer, &reply));
    let mut extra = Vec::new();
    if let Some(secs) = reply.retry_after {
        extra.push(("Retry-After", secs.to_string()));
    }
    if let Some(method) = reply.allow {
        extra.push(("Allow", method.to_string()));
    }
    let _ = http::respond_with(
        w,
        reply.status,
        "application/json",
        reply.body.as_bytes(),
        req.method == "HEAD",
        &extra,
    );
    if matches!(req.body, Body::TooLarge(_) | Body::Unreadable) {
        http::drain(w, 1 << 20);
    }
}

/// The route's fixed name and the status only: never the path, the header, the code or a nonce.
fn admin_log(peer: &str, reply: &admin::Reply) -> String {
    format!("{peer} admin {} {}", reply.route, reply.status)
}

fn send_file(path: &Path, size: u64, req: &Request, head_only: bool, w: &mut TcpStream) {
    let Ok(mut f) = fs::File::open(path) else {
        let _ = http::respond_text(w, 500, "the file vanished", head_only);
        return;
    };
    let range = req.header("range").map(|r| http::parse_range(r, size));
    let (status, start, len, mut extra) = match range {
        Some(Some((a, b))) => (
            206,
            a,
            b - a + 1,
            vec![("Content-Range", format!("bytes {a}-{b}/{size}"))],
        ),
        Some(None) => {
            let _ = http::write_head(
                w,
                &Head {
                    status: 416,
                    content_type: "text/plain",
                    length: Some(0),
                    extra: &[("Content-Range", format!("bytes */{size}"))],
                },
            );
            return;
        }
        None => (200, 0, size, Vec::new()),
    };
    extra.push(("Accept-Ranges", "bytes".to_string()));
    let _ = http::write_head(
        w,
        &Head {
            status,
            content_type: "application/octet-stream",
            length: Some(len),
            extra: &extra,
        },
    );
    if head_only {
        return;
    }
    if start > 0 && f.seek(SeekFrom::Start(start)).is_err() {
        return;
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    // Addon files are many and small; only the archives are worth a line each.
    let loud = len > 1_000_000;
    if loud {
        let from = if start > 0 {
            format!(" (from {start})")
        } else {
            String::new()
        };
        eprintln!("{} {name} {len}{from}", req.peer);
    }
    match http::copy_n(&mut f, w, len) {
        Ok(sent) if sent == len => {
            if loud {
                eprintln!("{} {name} done", req.peer);
            }
        }
        Ok(sent) => eprintln!("{} {name} stopped at {sent} of {len}", req.peer),
        Err(e) => eprintln!("{} {name} dropped: {e}", req.peer),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::tests::SALT;
    use crate::manifest::tests::{fill, tmp};
    use crate::manifest::REQUIRED;
    use serde_json::{json, Value};
    use std::io::{Read, Write};
    use std::time::Duration;

    fn data_dir(name: &str) -> PathBuf {
        let dir = tmp(name);
        fill(&dir, REQUIRED);
        fill(&dir, &["patch-2.MPQ", "base.MPQ", "realmlist.wtf"]);
        dir
    }

    /// A live host on an ephemeral port.
    fn host_with(data: &[String], addons: &[String], token: Option<&str>) -> SocketAddr {
        host_full(data, addons, token, &info::Realm::default(), Signup::off())
    }

    fn host_full(
        data: &[String],
        addons: &[String],
        token: Option<&str>,
        realm: &info::Realm,
        signup: Signup,
    ) -> SocketAddr {
        let sets = load(data, addons, "enUS", None).unwrap();
        let token = token.map(str::to_string);
        start(Host::new(sets, token, realm, signup, Admin::off()))
    }

    /// `host`, live on an ephemeral port.
    fn start(host: Host) -> SocketAddr {
        let host = Arc::new(host);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || http::serve(listener, move |r, s| handle(&host, r, s)));
        addr
    }

    /// A live host with no game data, behind the data token `s3cret`, and the admin menu `admin`.
    fn host_admin(admin: Admin) -> SocketAddr {
        let token = Some("s3cret".to_string());
        start(Host::new(
            Vec::new(),
            token,
            &stack_realm(),
            Signup::off(),
            admin,
        ))
    }

    fn host(name: &str, token: Option<&str>) -> (SocketAddr, PathBuf) {
        let dir = data_dir(name);
        (host_with(&[dir.display().to_string()], &[], token), dir)
    }

    /// Raw bytes out → (status, headers, body).
    fn send(addr: SocketAddr, raw: &[u8]) -> (u16, String, Vec<u8>) {
        let mut s = TcpStream::connect(addr).unwrap();
        s.write_all(raw).unwrap();
        let mut raw = Vec::new();
        s.read_to_end(&mut raw).unwrap();
        let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let headers = String::from_utf8_lossy(&raw[..split]).into_owned();
        let status = headers[9..12].parse().unwrap();
        (status, headers, raw[split + 4..].to_vec())
    }

    /// One raw request without a body → (status, headers, body).
    fn get(addr: SocketAddr, head: &str) -> (u16, String, Vec<u8>) {
        send(addr, format!("{head}\r\n\r\n").as_bytes())
    }

    /// `POST /accounts` with `headers` (each ending in CRLF) and `body`.
    fn post_account(addr: SocketAddr, headers: &str, body: &[u8]) -> (u16, String, Value) {
        let mut raw = format!(
            "POST /accounts HTTP/1.1\r\n{headers}Content-Length: {}\r\n\r\n",
            body.len()
        )
        .into_bytes();
        raw.extend_from_slice(body);
        let (status, headers, body) = send(addr, &raw);
        (status, headers, serde_json::from_slice(&body).unwrap())
    }

    fn account_json(username: &str, salt: &str) -> Vec<u8> {
        match accounts::tests::body(username, salt) {
            Body::Read(bytes) => bytes,
            other => panic!("{other:?}"),
        }
    }

    fn stack_realm() -> info::Realm {
        info::Realm {
            core: Some("vmangos".into()),
            auth_port: Some(3724),
            realm_name: Some("Vanillish".into()),
            realm_type: Some("pve".into()),
            world_address: Some("192.168.1.20".into()),
            world_port: Some(8085),
            world_same_host: true,
            ..info::Realm::default()
        }
    }

    const JSON: &str = "Content-Type: application/json\r\n";

    #[test]
    fn server_json_is_open_and_describes_the_server() {
        let dir = data_dir("info");
        let fake = accounts::tests::Fake::default();
        let addr = host_full(
            &[dir.display().to_string()],
            &[],
            Some("s3cret"),
            &stack_realm(),
            Signup::new(Mode::Lan, true, Box::new(fake)),
        );
        let (status, headers, body) = get(addr, "GET /server.json HTTP/1.1");
        assert_eq!(status, 200, "open even with a token");
        assert!(headers.contains("Content-Type: application/json"));
        assert!(headers.contains("Cache-Control: no-store"));
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["format"], 1);
        assert_eq!(v["name"], "Vanillish");
        assert_eq!(v["software"], "vanillish-datahost 0.4.0");
        assert!(v.get("admin").is_none(), "the admin menu is off");
        assert_eq!(v["auth"]["port"], 3724);
        assert_eq!(v["realms"][0]["world"]["same_host"], true);
        assert_eq!(v["data"]["url"], "");
        assert_eq!(v["data"]["token"], true);
        assert_eq!(v["data"]["locales"][0]["locale"], "enUS");
        assert_eq!(v["data"]["locales"][0]["path"], "enUS/");
        assert_eq!(v["registration"]["open"], true, "127.0.0.1 is private");
        assert_eq!(v["registration"]["token"], true);
        assert_eq!(v["registration"]["url"], "accounts");
        let (status, headers, body) = get(addr, "HEAD /server.json HTTP/1.1");
        assert_eq!(status, 200);
        assert!(headers.contains("Content-Length: "));
        assert!(body.is_empty());
        // A data host alone, sign-up off: no logon, no realm, no sign-up.
        let (alone, _) = host("info-alone", None);
        let (_, _, body) = get(alone, "GET /server.json HTTP/1.1");
        let v: Value = serde_json::from_slice(&body).unwrap();
        for absent in ["auth", "realms", "registration", "core"] {
            assert!(v.get(absent).is_none(), "{absent}: {v}");
        }
        assert_eq!(v["data"]["token"], false);
    }

    #[test]
    fn without_game_data_the_host_serves_server_json_and_sign_up() {
        let empty = tmp("no-data");
        assert!(load(&[], &[], "enUS", None).unwrap().is_empty());
        let fake = accounts::tests::Fake::default();
        let addr = host_full(
            &[empty.display().to_string()],
            &[],
            None,
            &stack_realm(),
            Signup::new(Mode::Lan, false, Box::new(fake.clone())),
        );
        let (status, _, body) = get(addr, "GET /server.json HTTP/1.1");
        assert_eq!(status, 200);
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert!(v.get("data").is_none(), "{v}");
        assert_eq!(v["registration"]["open"], true);
        for path in [
            "/locales.json",
            "/manifest.json",
            "/addons.json",
            "/files/dbc.MPQ",
            "/enUS/manifest.json",
        ] {
            assert_eq!(get(addr, &format!("GET {path} HTTP/1.1")).0, 404, "{path}");
        }
        let (status, _, body) = get(addr, "GET / HTTP/1.1");
        assert_eq!(status, 200);
        assert!(body.starts_with(b"inilla-datahost"));
        let (status, _, reply) = post_account(addr, JSON, &account_json("first", SALT));
        assert_eq!((status, &reply["username"]), (201, &Value::from("FIRST")));
        assert!(fake.accounts.lock().unwrap().contains_key("FIRST"));
    }

    #[test]
    fn sign_up_over_http() {
        let dir = data_dir("signup");
        let fake = accounts::tests::Fake::default();
        let addr = host_full(
            &[dir.display().to_string()],
            &[],
            Some("s3cret"),
            &stack_realm(),
            Signup::new(Mode::Lan, true, Box::new(fake.clone())),
        );
        let alice = account_json("alice", SALT);
        let (status, headers, reply) = post_account(addr, JSON, &alice);
        assert_eq!(status, 401);
        assert!(headers.contains("Content-Type: application/json"));
        assert_eq!(reply["error"], "token_required");
        let auth = format!("{JSON}Authorization: Bearer s3cret\r\n");
        let (status, _, reply) = post_account(addr, &auth, &alice);
        assert_eq!(
            (status, reply),
            (
                201,
                serde_json::json!({"ok": true, "username": "ALICE", "message": "Account ALICE created."})
            )
        );
        let (status, _, reply) = post_account(addr, &auth, &alice);
        assert_eq!((status, &reply["ok"]), (200, &Value::Bool(true)));
        let mallory = account_json("Alice", &format!("9{}", &SALT[1..]));
        let (status, _, reply) = post_account(addr, &auth, &mallory);
        assert_eq!((status, &reply["error"]), (409, &Value::from("name_taken")));
        let (status, _, reply) = post_account(
            addr,
            "Content-Type: text/plain\r\nAuthorization: Bearer s3cret\r\n",
            &alice,
        );
        assert_eq!(
            (status, &reply["error"]),
            (415, &Value::from("unsupported_media_type"))
        );
        let (status, _, reply) = post_account(addr, &auth, &account_json("x", SALT));
        assert_eq!(
            (status, &reply["error"]),
            (400, &Value::from("invalid_name"))
        );
        // A body over 4 KiB: refused unread, and the answer still reaches the client.
        let big = vec![b' '; 200_000];
        let (status, _, reply) = post_account(addr, &auth, &big);
        assert_eq!((status, &reply["error"]), (413, &Value::from("too_large")));
        let (status, headers, body) = get(addr, "GET /accounts HTTP/1.1");
        assert_eq!(status, 405);
        assert!(headers.contains("Allow: POST"));
        assert!(serde_json::from_slice::<Value>(&body).is_ok());
        // Three attempts reached the database; two more, then the limit, with Retry-After.
        for name in ["bob", "carol"] {
            assert_eq!(post_account(addr, &auth, &account_json(name, SALT)).0, 201);
        }
        let (status, headers, reply) = post_account(addr, &auth, &account_json("dave", SALT));
        assert_eq!(
            (status, &reply["error"]),
            (429, &Value::from("rate_limited"))
        );
        assert!(headers.contains("Retry-After: 600"), "{headers}");
        assert_eq!(reply["retry_after"], 600);
        assert_eq!(fake.calls.load(std::sync::atomic::Ordering::SeqCst), 5);
    }

    #[test]
    fn server_flags_read_empty_as_unset() {
        let cli = Cli::try_parse_from([
            "inilla-datahost",
            "serve",
            "--auth-port",
            "",
            "--realm-name",
            " ",
            "--core",
            "vmangos",
            "--world-same-host",
            "--signup",
            "lan",
            "--signup-db-password",
            "",
        ])
        .unwrap();
        let Cmd::Serve { data, server, .. } = cli.cmd else {
            panic!("serve");
        };
        assert!(data.is_empty(), "no --data is allowed");
        let realm = server.realm().unwrap();
        assert_eq!(
            (
                realm.auth_port,
                realm.realm_name.as_deref(),
                realm.core.as_deref()
            ),
            (None, None, Some("vmangos"))
        );
        assert!(realm.world_same_host);
        assert_eq!(server.signup, Mode::Lan);
        assert_eq!(
            server.signup(false).unwrap().registration(true),
            None,
            "lan without the database password stays off"
        );
        for bad in ["0", "70000", "x"] {
            let args = ServerArgs {
                world_port: Some(bad.into()),
                ..ServerArgs::default()
            };
            assert!(args.realm().is_err(), "{bad}");
        }
        assert!(Cli::try_parse_from(["inilla-datahost", "serve", "--signup", "yes"]).is_err());
    }

    #[test]
    fn locale_prefixes_are_four_letters() {
        assert_eq!(split_locale("deDE=/x"), (Some("deDE"), "/x"));
        assert_eq!(split_locale("/games/Data"), (None, "/games/Data"));
        assert_eq!(split_locale("C:=x"), (None, "C:=x"));
        assert_eq!(split_locale("/a=b/Data"), (None, "/a=b/Data"));
    }

    #[test]
    fn with_a_token_only_the_hint_is_open() {
        let (addr, _) = host("auth", Some("s3cret"));
        assert_eq!(get(addr, "GET / HTTP/1.1").0, 200);
        assert_eq!(get(addr, "GET /manifest.json HTTP/1.1").0, 401);
        assert_eq!(get(addr, "GET /locales.json HTTP/1.1").0, 401);
        assert_eq!(get(addr, "GET /files/dbc.MPQ HTTP/1.1").0, 401);
        assert_eq!(get(addr, "GET /manifest.json?token=wrong HTTP/1.1").0, 401);
        let (status, _, body) = get(
            addr,
            "GET /manifest.json HTTP/1.1\r\nAuthorization: Bearer s3cret",
        );
        assert_eq!(status, 200);
        let m: Manifest = serde_json::from_slice(&body).unwrap();
        assert_eq!((m.format, m.build), (1, 5875));
        assert_eq!(
            m.files.len(),
            REQUIRED.len() + 1,
            "the required archives and patch-2"
        );
        assert!(m
            .files
            .iter()
            .all(|f| f.name != "base.MPQ" && f.name != "realmlist.wtf"));
        assert_eq!(get(addr, "GET /files/dbc.MPQ?token=s3cret HTTP/1.1").0, 200);
    }

    #[test]
    fn without_a_token_the_host_is_open() {
        let (addr, _) = host("open", None);
        assert_eq!(get(addr, "GET /manifest.json HTTP/1.1").0, 200);
        assert_eq!(get(addr, "GET /files/dbc.MPQ HTTP/1.1").0, 200);
    }

    #[test]
    fn a_download_resumes_with_range() {
        let (addr, dir) = host("range", None);
        let whole = fs::read(dir.join("dbc.MPQ")).unwrap();
        let (status, headers, body) = get(addr, "GET /files/dbc.MPQ HTTP/1.1");
        assert_eq!(status, 200);
        assert!(headers.contains("Accept-Ranges: bytes"));
        assert_eq!(body, whole);
        let (status, headers, body) = get(addr, "GET /files/dbc.MPQ HTTP/1.1\r\nRange: bytes=4-");
        assert_eq!(status, 206);
        assert!(headers.contains(&format!(
            "Content-Range: bytes 4-{}/{}",
            whole.len() - 1,
            whole.len()
        )));
        assert_eq!(body, &whole[4..]);
        let (status, _, _) = get(addr, "GET /files/dbc.MPQ HTTP/1.1\r\nRange: bytes=999-");
        assert_eq!(status, 416);
        let (status, headers, body) = get(addr, "HEAD /files/dbc.MPQ HTTP/1.1");
        assert_eq!(status, 200);
        assert!(headers.contains(&format!("Content-Length: {}", whole.len())));
        assert!(body.is_empty());
    }

    #[test]
    fn nothing_outside_the_manifests_is_served() {
        let (addr, _) = host("escape", None);
        assert_eq!(get(addr, "GET /files/realmlist.wtf HTTP/1.1").0, 404);
        assert_eq!(get(addr, "GET /files/base.MPQ HTTP/1.1").0, 404);
        assert_eq!(
            get(addr, "GET /files/..%2F..%2Fetc%2Fpasswd HTTP/1.1").0,
            404
        );
        assert_eq!(
            get(addr, "GET /files/.inilla-manifest-enUS.json HTTP/1.1").0,
            404
        );
        assert_eq!(get(addr, "GET /addons/..%2Fdbc.MPQ HTTP/1.1").0, 404);
        assert_eq!(get(addr, "POST /manifest.json HTTP/1.1").0, 405);
    }

    #[test]
    fn two_languages_and_their_addons() {
        let en = data_dir("lang-en");
        let de = data_dir("lang-de");
        let missing = tmp("lang-fr-empty");
        let all = tmp("addons-all");
        fs::create_dir_all(all.join("Util")).unwrap();
        fs::write(all.join("Util/Util.toc"), "## Version: 1\n").unwrap();
        let de_addons = tmp("addons-de");
        fs::create_dir_all(de_addons.join("pfQuest/db/deDE")).unwrap();
        fs::write(de_addons.join("pfQuest/pfQuest.toc"), "## Version: 7.0.1\n").unwrap();
        fs::write(de_addons.join("pfQuest/db/deDE/quests.lua"), "Quests").unwrap();
        let addr = host_with(
            &[
                en.display().to_string(),
                format!("deDE={}", de.display()),
                format!("frFR={}", missing.display()),
            ],
            &[
                all.display().to_string(),
                format!("deDE={}", de_addons.display()),
            ],
            None,
        );
        let (_, _, body) = get(addr, "GET /locales.json HTTP/1.1");
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["default"], "enUS");
        let locales: Vec<&str> = v["locales"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l["locale"].as_str().unwrap())
            .collect();
        assert_eq!(locales, ["enUS", "deDE"], "an empty folder is not offered");
        let (_, _, body) = get(addr, "GET /deDE/manifest.json HTTP/1.1");
        let m: Manifest = serde_json::from_slice(&body).unwrap();
        assert_eq!(m.locale.as_deref(), Some("deDE"));
        assert_eq!(get(addr, "GET /dede/files/dbc.MPQ HTTP/1.1").0, 200);
        let (_, _, body) = get(addr, "GET /deDE/addons.json HTTP/1.1");
        let a: addons::AddonManifest = serde_json::from_slice(&body).unwrap();
        let names: Vec<&str> = a.addons.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["pfQuest", "Util"]);
        let (status, _, body) = get(addr, "GET /deDE/addons/pfQuest/db/deDE/quests.lua HTTP/1.1");
        assert_eq!((status, body.as_slice()), (200, b"Quests".as_slice()));
        let (_, _, body) = get(addr, "GET /addons.json HTTP/1.1");
        let a: addons::AddonManifest = serde_json::from_slice(&body).unwrap();
        assert_eq!(a.addons.len(), 1, "English gets only the shared addons");
        assert_eq!(get(addr, "GET /addons/pfQuest/pfQuest.toc HTTP/1.1").0, 404);
    }

    #[test]
    fn the_manifest_cache_is_reused_until_the_data_changes() {
        let dir = tmp("cache");
        let cache = tmp("cache-dir");
        fill(&dir, REQUIRED);
        let first = manifest(&dir, "enUS", Some(&cache)).unwrap();
        assert!(cache.join(".inilla-manifest-enUS.json").exists());
        assert_eq!(manifest(&dir, "enUS", Some(&cache)).unwrap(), first);
        fs::write(dir.join("dbc.MPQ"), "changed, and longer than before").unwrap();
        let second = manifest(&dir, "enUS", Some(&cache)).unwrap();
        assert_ne!(second, first);
        assert_eq!(
            manifest(&dir, "deDE", Some(&cache))
                .unwrap()
                .locale
                .as_deref(),
            Some("deDE")
        );
    }

    /// `method path` with `headers` (each ending in CRLF) and `body` → (status, headers, JSON).
    fn admin_req(
        addr: SocketAddr,
        method: &str,
        path: &str,
        headers: &str,
        body: &[u8],
    ) -> (u16, String, Value) {
        let mut raw = format!(
            "{method} {path} HTTP/1.1\r\n{headers}Content-Length: {}\r\n\r\n",
            body.len()
        )
        .into_bytes();
        raw.extend_from_slice(body);
        let (status, headers, body) = send(addr, &raw);
        (
            status,
            headers,
            serde_json::from_slice(&body).unwrap_or(Value::Null),
        )
    }

    fn nonce(addr: SocketAddr) -> String {
        let (status, _, v) = admin_req(addr, "GET", "/admin/nonce", "", b"");
        assert_eq!(status, 200, "{v}");
        v["nonce"].as_str().unwrap().to_string()
    }

    /// `method path` with a JSON `body`, signed with `code` over a fresh nonce.
    fn signed(
        addr: SocketAddr,
        code: &str,
        method: &str,
        path: &str,
        body: &[u8],
    ) -> (u16, String, Value) {
        let auth = admin::tests::auth_header(code, &nonce(addr), method, path, body);
        let headers = format!("{JSON}Authorization: {auth}\r\n");
        admin_req(addr, method, path, &headers, body)
    }

    fn error(v: &Value) -> &str {
        v["error"].as_str().unwrap_or("")
    }

    #[test]
    fn the_admin_menu_over_http() {
        use crate::admin::tests::{auth_header, say, tcp_agent, Act, CODE, RATES};
        let status_json =
            r#"{"ok":true,"world":"up","overrides":0,"rates":{},"players":[],"players_ok":true}"#;
        let (agent, seen) = tcp_agent(vec![
            say(200, status_json),
            say(200, r#"{"ok":true,"changed":["Rate.XP.Kill"]}"#),
            say(
                503,
                r#"{"ok":false,"error":"world_starting","message":"Starting."}"#,
            ),
            say(401, r#"{"ok":false,"error":"admin_auth","message":"No."}"#),
            Act::Say(b"HTTP/1.0 500 X\r\n\r\n<html>Traceback</html>".to_vec()),
            Act::Hold,
            // Read, then gone: mangosd stopped, and the agent with it.
            Act::Say(Vec::new()),
        ]);
        let backend = admin::Agent::new(&agent, CODE).with_deadline(Duration::from_millis(300));
        let admin = Admin::new(Mode::Lan, CODE, Some(Box::new(backend)));
        let addr = host_admin(admin);

        // server.json says where the menu is; none of it needs the data token.
        let (_, _, body) = get(addr, "GET /server.json HTTP/1.1");
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            v["admin"],
            json!({"url": "admin/", "open": true}),
            "127.0.0.1 is private"
        );
        let (status, headers, v) = admin_req(addr, "GET", "/admin/nonce", "", b"");
        assert_eq!(status, 200);
        assert!(headers.contains("Content-Type: application/json"));
        assert!(headers.contains("Cache-Control: no-store"));
        assert_eq!(
            (
                v["ok"].as_bool(),
                v["expires_in"].as_u64(),
                v["nonce"].as_str().map(str::len)
            ),
            (Some(true), Some(60), Some(32))
        );

        // Forwarded with this host's own code, normalised, the answers passed on.
        let status = auth_header(CODE, &nonce(addr), "GET", "/admin/status", b"");
        let status_auth = format!("Authorization: {status}\r\n");
        let (status, _, v) = admin_req(addr, "GET", "/admin/status", &status_auth, b"");
        assert_eq!(
            (status, v),
            (200, serde_json::from_str(status_json).unwrap())
        );
        let got = seen.recv().unwrap();
        assert_eq!((got.method.as_str(), got.path.as_str()), ("GET", "/status"));
        assert_eq!(got.header("authorization"), Some("Bearer ABCDEFGHJKLMNPQR"));
        assert_eq!(got.header("host"), Some("mangosd"));
        let rates = auth_header(CODE, &nonce(addr), "POST", "/admin/rates", RATES.as_bytes());
        let rates_auth = format!("{JSON}Authorization: {rates}\r\n");
        let (status, _, v) = admin_req(addr, "POST", "/admin/rates", &rates_auth, RATES.as_bytes());
        assert_eq!(
            (status, &v["changed"][0]),
            (200, &Value::from("Rate.XP.Kill"))
        );
        let got = seen.recv().unwrap();
        assert_eq!((got.method.as_str(), got.path.as_str()), ("POST", "/rates"));
        assert_eq!(got.body, Body::Read(RATES.as_bytes().to_vec()), "verbatim");
        let (status, _, v) = admin_req(addr, "GET", "/admin/status", &status_auth, b"");
        assert_eq!((status, error(&v)), (401, "admin_nonce"), "replayed");

        // The agent's own refusal passes through; its 401 and an answer without JSON do not; its
        // silence is a 504 at the deadline, and so is its hanging up once asked.
        let (status, _, v) = signed(addr, CODE, "GET", "/admin/status", b"");
        assert_eq!((status, error(&v)), (503, "world_starting"));
        let gold = br#"{"id":"g1","to":"online","copper":100}"#;
        let (status, headers, v) = signed(addr, CODE, "POST", "/admin/gold", gold);
        assert_eq!((status, error(&v)), (502, "admin_misconfigured"));
        assert!(headers.starts_with("HTTP/1.1 502 Bad Gateway"), "{headers}");
        assert!(v["message"]
            .as_str()
            .unwrap()
            .contains("disagree on ADMIN_CODE"));
        let reset = br#"{"announce":false}"#;
        let (status, _, v) = signed(addr, CODE, "POST", "/admin/rates/reset", reset);
        assert_eq!((status, error(&v)), (502, "admin_misconfigured"));
        let started = Instant::now();
        let (status, headers, v) = signed(addr, CODE, "GET", "/admin/status", b"");
        assert_eq!((status, error(&v)), (504, "no_answer"));
        assert!(
            headers.starts_with("HTTP/1.1 504 Gateway Timeout"),
            "{headers}"
        );
        assert!(started.elapsed() < Duration::from_secs(3));
        let (status, _, v) = signed(addr, CODE, "POST", "/admin/gold", gold);
        assert_eq!(
            (status, error(&v)),
            (504, "no_answer"),
            "it may have been done"
        );
        for want in ["/status", "/gold", "/rates/reset", "/status", "/gold"] {
            assert_eq!(seen.recv().unwrap().path, want);
        }

        // Refused here, never forwarded.
        let (status, headers, v) = admin_req(addr, "GET", "/admin/rates", "", b"");
        assert_eq!((status, error(&v)), (405, "bad_request"));
        assert!(headers.contains("Allow: POST"), "{headers}");
        let (status, headers, body) = get(addr, "HEAD /admin/status HTTP/1.1");
        assert_eq!((status, body.is_empty()), (405, true));
        assert!(headers.contains("Allow: GET"), "{headers}");
        let (status, _, v) = admin_req(addr, "GET", "/admin/status", "", b"");
        assert_eq!((status, error(&v)), (401, "admin_auth"));
        let (status, _, v) = admin_req(addr, "GET", "/admin/status?token=s3cret", "", b"");
        assert_eq!(
            (status, error(&v)),
            (401, "admin_auth"),
            "only the header signs"
        );
        let (status, _, v) = signed(addr, "ABCD-EFGH-JKLM-NPQX", "GET", "/admin/status", b"");
        assert_eq!((status, error(&v)), (401, "admin_code"));
        let auth = auth_header(CODE, &nonce(addr), "POST", "/admin/rates", RATES.as_bytes());
        let text = format!("Content-Type: text/plain\r\nAuthorization: {auth}\r\n");
        let (status, _, v) = admin_req(addr, "POST", "/admin/rates", &text, RATES.as_bytes());
        assert_eq!((status, error(&v)), (415, "json_only"));
        let auth = auth_header(CODE, &nonce(addr), "POST", "/admin/gold", b"");
        let raw = format!("POST /admin/gold HTTP/1.1\r\n{JSON}Authorization: {auth}\r\n\r\n");
        let (status, _, body) = send(addr, raw.as_bytes());
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!((status, error(&v)), (400, "bad_request"), "no body");
        // Over 4 KiB: refused unread, and the answer still reaches the client.
        let (status, _, v) = signed(addr, CODE, "POST", "/admin/gold", &vec![b' '; 200_000]);
        assert_eq!((status, error(&v)), (413, "too_large"));
        let (status, _, v) = admin_req(addr, "GET", "/admin/status%0a1.2.3.4", "", b"");
        assert_eq!((status, error(&v)), (404, "not_found"));
        assert!(seen.recv().is_err(), "nothing more reached the agent");
    }

    #[test]
    fn the_admin_menu_off_or_its_agent_down() {
        use crate::admin::tests::{nobody, CODE};
        let agent =
            || Some(Box::new(admin::Agent::new(&nobody(), CODE)) as Box<dyn admin::Backend>);
        for off in [
            Admin::new(Mode::Off, CODE, agent()),
            Admin::new(Mode::Lan, "change-me-admin-code", agent()),
            Admin::new(Mode::Lan, "ABCD-EFGH", agent()),
            Admin::new(Mode::Open, "vanillishadmin2026", agent()),
            Admin::new(Mode::Open, CODE, None),
        ] {
            let addr = host_admin(off);
            let (_, _, body) = get(addr, "GET /server.json HTTP/1.1");
            let v: Value = serde_json::from_slice(&body).unwrap();
            assert!(v.get("admin").is_none(), "{v}");
            for (method, path) in [
                ("GET", "/admin/nonce"),
                ("GET", "/admin/status"),
                ("POST", "/admin/gold"),
            ] {
                let (status, headers, v) = admin_req(addr, method, path, JSON, b"{}");
                assert_eq!((status, error(&v)), (404, "admin_off"), "{path}");
                assert!(headers.contains("Content-Type: application/json"));
            }
        }
        let addr = host_admin(Admin::new(Mode::Open, CODE, agent()));
        let (_, _, body) = get(addr, "GET /server.json HTTP/1.1");
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["admin"], json!({"url": "admin/", "open": true}));
        let (status, _, v) = signed(addr, CODE, "GET", "/admin/status", b"");
        assert_eq!((status, error(&v)), (503, "world_unavailable"));
        assert_eq!(
            v["message"],
            "The world server is not running (or still starting)."
        );
        // Ten wrong codes, then the wait, with Retry-After; the right code is still served.
        let wrong = "ABCD-EFGH-JKLM-NPQX";
        for _ in 0..10 {
            let (status, _, v) = signed(addr, wrong, "GET", "/admin/status", b"");
            assert_eq!((status, error(&v)), (401, "admin_code"));
        }
        let (status, headers, v) = signed(addr, wrong, "POST", "/admin/gold", b"{}");
        assert_eq!((status, error(&v)), (429, "too_many"));
        assert!(headers.contains("Retry-After: 600"), "{headers}");
        assert_eq!(v["retry_after"], 600);
        let (status, _, v) = signed(addr, CODE, "GET", "/admin/status", b"");
        assert_eq!((status, error(&v)), (503, "world_unavailable"));
    }

    #[test]
    fn admin_flags_and_the_code_from_the_environment() {
        use crate::admin::tests::CODE;
        let parse = |args: &[&str]| {
            let mut all = vec!["inilla-datahost", "serve"];
            all.extend_from_slice(args);
            match Cli::try_parse_from(all).map(|c| c.cmd) {
                Ok(Cmd::Serve { server, .. }) => Ok(server),
                Ok(_) => panic!("serve"),
                Err(e) => Err(e),
            }
        };
        let server = parse(&["--admin", "", "--admin-agent", "mangosd:8091"]).unwrap();
        assert_eq!(server.admin, Mode::Lan, "empty is the default");
        let admin = server.admin(Some(CODE)).unwrap();
        let said = admin.describe();
        assert!(
            said.starts_with("admin menu on, from private addresses only"),
            "{said}"
        );
        assert!(
            said.ends_with("agent at mangosd:8091") && !said.contains("ABCD"),
            "{said}"
        );
        assert_eq!(admin.section(false).map(|s| s.open), Some(false));
        let said = server.admin(None).unwrap().describe();
        assert_eq!(
            said,
            "admin menu off: no ADMIN_CODE (INILLA_ADMIN_CODE) is set"
        );
        for weak in [
            "",
            "1234",
            "CHANGEME-1234-5678-9ABC",
            "ÄBCD-EFGH-JKLM-NPQR",
            "VanillishServer2024",
        ] {
            let admin = server.admin(Some(weak)).unwrap();
            assert_eq!(admin.section(true), None, "{weak:?}");
            assert!(
                !admin.describe().contains(weak) || weak.is_empty(),
                "{weak:?}"
            );
        }
        for (flag, mode) in [
            ("off", Mode::Off),
            ("OPEN", Mode::Open),
            (" lan ", Mode::Lan),
        ] {
            assert_eq!(parse(&["--admin", flag]).unwrap().admin, mode, "{flag}");
        }
        let off = parse(&["--admin", "off", "--admin-agent", "mangosd:8091"]).unwrap();
        assert_eq!(
            off.admin(Some(CODE)).unwrap().describe(),
            "admin menu off: INILLA_ADMIN=off"
        );
        for bad in [
            "mangosd",
            "mangosd:0",
            ":8091",
            "mangosd:x",
            "mangosd:70000",
        ] {
            let args = ServerArgs {
                admin: Mode::Lan,
                admin_agent: Some(bad.into()),
                ..ServerArgs::default()
            };
            assert!(args.admin(Some(CODE)).is_err(), "{bad}");
        }
        let alone = ServerArgs {
            admin: Mode::Lan,
            admin_agent: Some(" ".into()),
            ..ServerArgs::default()
        };
        assert!(alone
            .admin(Some(CODE))
            .unwrap()
            .describe()
            .contains("no INILLA_ADMIN_AGENT"));
        assert!(parse(&["--admin", "yes"]).is_err());
        assert!(
            parse(&["--admin-code", CODE]).is_err(),
            "the code is never a flag"
        );
    }

    #[test]
    fn the_admin_log_line_names_the_route_never_the_path() {
        use crate::admin::tests::CODE;
        let admin = Admin::new(Mode::Lan, CODE, None);
        for (path, name) in [
            ("/admin/status", "status"),
            ("/admin/rates/reset", "rates/reset"),
            ("/admin/gold", "gold"),
            ("/admin/x\n10.0.0.1:1 admin status 200", "unknown"),
            ("/admin/\u{1b}[2J", "unknown"),
        ] {
            let ask = admin::Request {
                method: "GET",
                path,
                peer: None,
                authorization: Some("Vanillish-Admin 00112233445566778899aabbccddeeff:00"),
                content_type: None,
                body: &Body::None,
            };
            let reply = admin.handle(&ask, Instant::now());
            assert_eq!(
                admin_log("10.0.0.2:5000", &reply),
                format!("10.0.0.2:5000 admin {name} 404")
            );
        }
    }
}

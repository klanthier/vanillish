//! Vanillish: **quick connect** — from the address a player types to a server profile, the game
//! data's source and an account. A Vanillish (or iNilla) server stack describes itself on its data
//! host, `GET http://<host>:8099/server.json` ([`ServerInfo`]); a plain 1.12.1 server does not,
//! and the app falls back to the manual server form ([`manual_profile`], [`Probe`]).
//!
//! ```text
//! GET  <base>server.json   {"format":1,"name","core","auth":{"port"},"realms":[…],"data":{…},"registration":{…},"admin":{…}}
//! POST <base>accounts      {"username","salt","verifier"} → 201 {"ok":true,"username"} | 4xx/5xx {"ok":false,"error","message"}
//! ```
//!
//! `<base>` is where `server.json` answered: `http://<host>:8099/` unless the player typed another
//! port or a whole URL. The data and sign-up URLs in it are relative to `<base>` unless absolute
//! ([`resolve`]).
//!
//! A sign-up carries the account's SRP6 salt and verifier, computed on the phone — the password
//! never leaves it. The verifier comes in through a closure ([`MakeVerifier`]: the app passes
//! `benilla_srp::generate_account`), so this crate stays free of SRP and of the network: [`Fetch`]
//! and [`Post`] are the caller's.

use serde::{Deserialize, Deserializer, Serialize};

use crate::addons::{HostLocale, HostLocales};
use crate::data::{join_url, Fetch};
use crate::profiles::{
    split_host_port, Core, DataSource, Launcher, ServerProfile, DEFAULT_AUTH_PORT,
};

/// Where a Vanillish server's data host listens, and so where `server.json` is asked for first.
pub const DEFAULT_INFO_PORT: u16 = 8099;
/// The `server.json` layout this client reads; another number is a shape it cannot.
pub const SERVER_INFO_FORMAT: u32 = 1;
/// 1.12.1, the only build this client speaks.
pub const CLIENT_BUILD: u32 = 5875;
/// vmangos' `MAX_ACCOUNT_STR`: realmd drops a longer name, and a longer password is stored without
/// being upper-cased, so that account could never log in.
pub const MAX_ACCOUNT_LEN: usize = 16;
/// The prefix of an iNilla data host's `GET /` hint, from before `server.json` existed (it is how
/// such a host is told from any other web server).
pub const DATAHOST_HINT: &str = "inilla-datahost";

/// What the player typed, read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Address {
    /// The host alone: `192.168.1.20`, `mac-mini.local`, `fd00::2` (no brackets).
    pub host: String,
    /// Where `server.json` is asked for, with a trailing slash.
    pub base: String,
}

/// `192.168.1.20`, `192.168.1.20:8099`, `mac-mini.local`, `[fd00::2]:8099`,
/// `https://wow.example.com/vanillish/` → the host and the base to ask. A bare host, or the realm's
/// own `:3724` (the realmlist habit), means the data host's port 8099; any other port is the data
/// host's; a URL is the base itself (its query and fragment dropped).
pub fn parse_address(input: &str) -> Result<Address, String> {
    let s = input.trim();
    if s.is_empty() {
        return Err("Enter the server's address.".into());
    }
    if s.chars().any(char::is_whitespace) {
        return Err("The address cannot contain spaces.".into());
    }
    let lower = s.to_ascii_lowercase();
    if let Some(scheme) = ["http://", "https://"]
        .into_iter()
        .find(|p| lower.starts_with(p))
    {
        let rest = &s[scheme.len()..];
        let rest = rest.split(['?', '#']).next().unwrap_or(rest);
        let authority = rest.split('/').next().unwrap_or("");
        let (host, _) = host_port(authority)?;
        check_host(host)?;
        let base = format!("{scheme}{}/", rest.trim_end_matches('/'));
        return Ok(Address {
            host: host.to_string(),
            base,
        });
    }
    if s.contains('/') {
        return Err("Type the address alone (192.168.1.20), or a whole http:// link.".into());
    }
    let (host, port) = host_port(s)?;
    check_host(host)?;
    let port = match port {
        None | Some(DEFAULT_AUTH_PORT) => DEFAULT_INFO_PORT,
        Some(p) => p,
    };
    Ok(Address {
        host: host.to_string(),
        base: format!("http://{}:{port}/", bracketed(host)),
    })
}

/// `host`, `host:port`, `[v6]`, `[v6]:port` or a bare IPv6 literal → the host and the port typed.
/// Strict where [`split_host_port`] is lenient: a port that is not 1–65535, or anything after a
/// bracketed host but its port, is refused rather than dropped (`[fd00::2]:99999` would otherwise
/// become the default port, or a URL nothing can open).
fn host_port(s: &str) -> Result<(&str, Option<u16>), String> {
    let port = |p: &str| {
        p.parse::<u16>()
            .ok()
            .filter(|p| *p > 0)
            .ok_or_else(|| "The port must be between 1 and 65535.".to_string())
    };
    if let Some(rest) = s.strip_prefix('[') {
        let (host, tail) = rest
            .split_once(']')
            .filter(|(h, _)| h.parse::<std::net::Ipv6Addr>().is_ok())
            .ok_or_else(|| not_an_address(s))?;
        return match tail {
            "" => Ok((host, None)),
            _ => match tail.strip_prefix(':') {
                Some(p) => Ok((host, Some(port(p)?))),
                None => Err(not_an_address(s)),
            },
        };
    }
    match s.rsplit_once(':') {
        // One colon: `host:port`. More: an IPv6 literal (or a typo `check_host` refuses).
        Some((h, p)) if !h.contains(':') => Ok((h, Some(port(p)?))),
        _ => Ok((s, None)),
    }
}

fn not_an_address(s: &str) -> String {
    format!("\"{s}\" is not an address: type an IP (192.168.1.20) or a name (my-mac.local).")
}

fn check_host(host: &str) -> Result<(), String> {
    let ok = !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':'))
        && !host.starts_with(['.', '-'])
        // A colon only inside an IPv6 literal (`host:port:x`, `a:b:c:0` are typos).
        && (!host.contains(':') || host.parse::<std::net::Ipv6Addr>().is_ok());
    if ok {
        Ok(())
    } else {
        Err(not_an_address(host))
    }
}

/// An IPv6 literal in brackets, for a URL or a `host:port`; any other host as it is.
pub fn bracketed(host: &str) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

/// `url` resolved against `base` (a URL ending in `/`), the way a browser resolves a link on the
/// page at `base`: an absolute URL as it is, `/path` against the base's origin, anything else
/// against the base itself (`""` is the base). That lets a reverse proxy, another port or a website
/// host the data or the sign-up.
pub fn resolve(base: &str, url: &str) -> String {
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return url.to_string();
    }
    if let Some(rest) = url.strip_prefix("//") {
        let scheme = base.split("://").next().unwrap_or("http");
        return format!("{scheme}://{rest}");
    }
    if url.is_empty() || url == "." || url == "./" {
        return base.to_string();
    }
    if let Some(path) = url.strip_prefix('/') {
        return join_url(&origin(base), path);
    }
    join_url(base, url.strip_prefix("./").unwrap_or(url))
}

/// `scheme://authority` of `url`.
fn origin(url: &str) -> String {
    let after = url.find("://").map_or(0, |i| i + 3);
    let end = url[after..].find('/').map_or(url.len(), |i| after + i);
    url[..end].to_string()
}

/// A field that may be absent or `null` (an unset value in the server's environment): its default
/// either way, so one empty setting never makes the whole document unreadable.
pub(crate) fn or_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

/// `GET <base>server.json`: what a Vanillish server says about itself. Unknown fields are ignored
/// (a newer server of the same format); a section that is absent is a thing this server does not
/// do.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerInfo {
    pub format: u32,
    /// What the launcher calls the server.
    #[serde(default, deserialize_with = "or_default")]
    pub name: String,
    /// `vanillish-datahost 0.3.0`: who answered, for the session log.
    #[serde(default, deserialize_with = "or_default")]
    pub software: String,
    /// `vmangos`, `cmangos`, anything else.
    #[serde(default, deserialize_with = "or_default")]
    pub core: String,
    /// The client build the server is for; absent: 1.12.1's.
    #[serde(default)]
    pub build: Option<u32>,
    /// The logon server. Absent: a data host alone — the typed host on the stock port.
    #[serde(default)]
    pub auth: Option<AuthInfo>,
    #[serde(default, deserialize_with = "or_default")]
    pub realms: Vec<RealmInfo>,
    /// The game data this host serves. Absent: none (the player brings their own).
    #[serde(default)]
    pub data: Option<DataInfo>,
    /// Account sign-up. Absent: none.
    #[serde(default)]
    pub registration: Option<RegistrationInfo>,
    /// A line from the operator (the message of the day, a website), kept as the profile's notes.
    #[serde(default)]
    pub message: Option<String>,
    /// The server admin menu (`crate::admin`). Absent: none (an older data host, no `ADMIN_CODE`).
    #[serde(default)]
    pub admin: Option<AdminInfo>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthInfo {
    /// realmd's host. Absent: the host `server.json` was reached at, which is right wherever the
    /// player is (a LAN IP, a `.local` name, a VPN address, a public name).
    #[serde(default)]
    pub host: Option<String>,
    /// realmd's published port; absent: 3724.
    #[serde(default)]
    pub port: Option<u16>,
}

impl AuthInfo {
    pub fn port(&self) -> u16 {
        self.port.unwrap_or(DEFAULT_AUTH_PORT)
    }

    /// The host named, when it names one.
    fn named_host(&self) -> Option<&str> {
        self.host
            .as_deref()
            .map(str::trim)
            .filter(|h| !h.is_empty())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RealmInfo {
    #[serde(default, deserialize_with = "or_default")]
    pub name: String,
    /// `pve`, `pvp`, `rp`, `rppvp`: shown, never acted on.
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub world: Option<WorldInfo>,
}

impl RealmInfo {
    /// `PvE`, `PvP`, `RP`, `RP-PvP` — the kind as the realm list writes it.
    pub fn kind_label(&self) -> Option<&str> {
        let kind = self
            .kind
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty())?;
        Some(match kind.to_ascii_lowercase().as_str() {
            "pve" | "normal" => "PvE",
            "pvp" => "PvP",
            "rp" => "RP",
            "rppvp" | "rp-pvp" | "rp_pvp" => "RP-PvP",
            _ => kind,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldInfo {
    /// The address realmd's `realmlist` row hands out (the stack's `REALM_ADDRESS`).
    #[serde(default)]
    pub address: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    /// The world server runs on the machine that answered: the host the player typed reaches it,
    /// whatever `address` says.
    #[serde(default, deserialize_with = "or_default")]
    pub same_host: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataInfo {
    /// The data host's base (`manifest.json`, `locales.json`, `files/…`), relative to `<base>`.
    #[serde(default, deserialize_with = "or_default")]
    pub url: String,
    /// The manifest and the files need the operator's token (`INILLA_DATA_TOKEN`).
    #[serde(default, deserialize_with = "or_default")]
    pub token: bool,
    /// The default language, as `locales.json` says.
    #[serde(default)]
    pub default: Option<String>,
    /// The languages, as `locales.json` lists them — sizes included, so the size shows before the
    /// token is asked for.
    #[serde(default, deserialize_with = "or_default")]
    pub locales: Vec<LocaleInfo>,
}

/// One language of the game data, as `server.json` lists it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocaleInfo {
    #[serde(default, deserialize_with = "or_default")]
    pub locale: String,
    #[serde(default, deserialize_with = "or_default")]
    pub path: String,
    #[serde(default, deserialize_with = "or_default")]
    pub bytes: u64,
    #[serde(default, deserialize_with = "or_default")]
    pub addons: usize,
}

impl DataInfo {
    /// The languages in the shape the data download takes, when there are several to pick from.
    pub fn host_locales(&self) -> Option<HostLocales> {
        let locales: Vec<HostLocale> = self
            .locales
            .iter()
            .filter(|l| !l.locale.is_empty())
            .map(|l| HostLocale {
                locale: l.locale.clone(),
                path: l.path.clone(),
                bytes: l.bytes,
                addons: l.addons,
            })
            .collect();
        (locales.len() > 1).then(|| HostLocales {
            default: self
                .default
                .clone()
                .filter(|d| locales.iter().any(|l| l.locale == *d))
                .unwrap_or_else(|| locales[0].locale.clone()),
            locales,
        })
    }

    /// The size of `locale` (or the default) in bytes, when the server said.
    pub fn bytes(&self, locale: Option<&str>) -> Option<u64> {
        let want = locale.or(self.default.as_deref());
        self.locales
            .iter()
            .find(|l| Some(l.locale.as_str()) == want)
            .or(self.locales.first())
            .map(|l| l.bytes)
            .filter(|b| *b > 0)
    }

    fn has(&self, locale: &str) -> bool {
        self.locales.iter().any(|l| l.locale == locale)
    }
}

/// The server admin menu, as `server.json` announces it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminInfo {
    /// Where its routes are, relative to `<base>` (`admin/`; see `admin::admin_url`).
    #[serde(default, deserialize_with = "or_default")]
    pub url: String,
    /// It answers this requester: `false` from outside the server's network under
    /// `INILLA_ADMIN=lan`.
    #[serde(default, deserialize_with = "or_default")]
    pub open: bool,
}

/// The only sign-up credentials this client sends: the SRP6 salt and verifier.
pub const SRP6: &str = "srp6";

/// Account sign-up, as the server takes it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistrationInfo {
    /// Whether it takes sign-ups now. Absent: no.
    #[serde(default, deserialize_with = "or_default")]
    pub open: bool,
    /// `POST` here, relative to `<base>`; absent: `accounts`.
    #[serde(default)]
    pub url: Option<String>,
    /// Sign-up needs the operator's token too.
    #[serde(default, deserialize_with = "or_default")]
    pub token: bool,
    /// What a sign-up carries: [`SRP6`] (absent means it), the salt and verifier the phone
    /// computed. Anything else — a server that wants the password itself — this client does not
    /// send: the password never leaves the phone.
    #[serde(default)]
    pub credentials: Option<String>,
    #[serde(default)]
    pub name_min: Option<usize>,
    #[serde(default)]
    pub name_max: Option<usize>,
    #[serde(default)]
    pub password_min: Option<usize>,
    #[serde(default)]
    pub password_max: Option<usize>,
    /// The operator's words under the form ("Ask Kevin for a GM level").
    #[serde(default)]
    pub note: Option<String>,
}

impl RegistrationInfo {
    /// Open, for a salt and a verifier: what the Create tab needs.
    pub fn usable(&self) -> bool {
        self.open && self.takes_verifier()
    }

    pub fn takes_verifier(&self) -> bool {
        self.credentials
            .as_deref()
            .is_none_or(|c| c.trim().eq_ignore_ascii_case(SRP6))
    }

    pub fn url(&self) -> &str {
        self.url
            .as_deref()
            .map(str::trim)
            .filter(|u| !u.is_empty())
            .unwrap_or("accounts")
    }

    /// The name's length, clamped to what vmangos stores.
    pub fn name_range(&self) -> (usize, usize) {
        let max = self
            .name_max
            .unwrap_or(MAX_ACCOUNT_LEN)
            .clamp(1, MAX_ACCOUNT_LEN);
        (self.name_min.unwrap_or(3).clamp(1, max), max)
    }

    /// The password's length, clamped to what vmangos stores.
    pub fn password_range(&self) -> (usize, usize) {
        let max = self
            .password_max
            .unwrap_or(MAX_ACCOUNT_LEN)
            .clamp(1, MAX_ACCOUNT_LEN);
        (self.password_min.unwrap_or(6).clamp(1, max), max)
    }

    /// An account name the server and the 1.12 logon both take, upper-cased as it is stored:
    /// ASCII letters and digits only (the logon upper-cases ASCII alone, and vmangos writes the
    /// name into its `INSERT` unescaped).
    pub fn check_name(&self, name: &str) -> Result<String, String> {
        let name = name.trim();
        let (min, max) = self.name_range();
        if !name.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err("The account name takes letters and digits only.".into());
        }
        if name.len() < min || name.len() > max {
            return Err(format!(
                "The account name is {min} to {max} letters or digits."
            ));
        }
        Ok(name.to_ascii_uppercase())
    }

    /// A password the 1.12 logon can send: printable ASCII without spaces, quotes or backslashes
    /// (the logon upper-cases ASCII alone; a console's `account create` splits on spaces and
    /// quotes, so a server sharing these rules can make the same account by hand).
    pub fn check_password(&self, password: &str) -> Result<(), String> {
        let (min, max) = self.password_range();
        if !password
            .chars()
            .all(|c| c.is_ascii_graphic() && !matches!(c, '"' | '\'' | '\\'))
        {
            return Err(
                "The password takes letters, digits and symbols, without spaces or quotes.".into(),
            );
        }
        let n = password.len();
        if n < min || n > max {
            return Err(format!("The password is {min} to {max} characters."));
        }
        Ok(())
    }
}

/// `vmangos` / `cmangos` / anything else, as the launcher stores it.
pub fn core_from(wire: &str) -> Core {
    match wire.trim().to_ascii_lowercase().as_str() {
        "vmangos" => Core::Vmangos,
        "cmangos" => Core::Cmangos,
        _ => Core::Other,
    }
}

/// Why `server.json` could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InfoError {
    /// A format this client does not know: a newer server, an older app.
    Format(u32),
    /// Not the document at all (another web server's page, a broken file).
    Malformed(String),
}

/// Read `server.json`: the format first, so a newer server whose document no longer has this
/// shape still reads as "update the app", not as a stranger.
pub fn parse_server_info(text: &str) -> Result<ServerInfo, InfoError> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| InfoError::Malformed(e.to_string()))?;
    let format = value
        .get("format")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| InfoError::Malformed("no format".into()))?;
    if format != u64::from(SERVER_INFO_FORMAT) {
        return Err(InfoError::Format(u32::try_from(format).unwrap_or(u32::MAX)));
    }
    serde_json::from_value(value).map_err(|e| InfoError::Malformed(e.to_string()))
}

/// The sentence for a format this client cannot read.
fn format_problem(format: u32) -> String {
    format!(
        "it describes itself in format {format}, which this version does not read (it reads \
         {SERVER_INFO_FORMAT}): update the app"
    )
}

impl ServerInfo {
    /// What makes this description unusable for this client.
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.format != SERVER_INFO_FORMAT {
            out.push(format_problem(self.format));
        }
        if let Some(build) = self.build.filter(|b| *b != CLIENT_BUILD) {
            out.push(format!(
                "it is for client build {build}, and this client is 1.12.1 ({CLIENT_BUILD})"
            ));
        }
        if self.auth.as_ref().is_some_and(|a| a.port == Some(0)) {
            out.push("it names logon port 0".into());
        }
        out
    }

    /// The realm address override the profile needs, reached at `typed`: the host the player
    /// typed, when every realm runs on the machine that answered and one of them hands out
    /// another address (a LAN IP while the player came through a VPN or a public name, a
    /// container's address, `127.0.0.1`). `None` when the listed address is already that host, or
    /// a realm lives elsewhere. Bracketed for an IPv6 host, so the realm's port can follow it.
    pub fn world_override(&self, typed: &str) -> Option<String> {
        if self.realms.is_empty() {
            return None;
        }
        let same = self
            .realms
            .iter()
            .all(|r| r.world.as_ref().is_some_and(|w| w.same_host));
        let differs = self.realms.iter().any(|r| {
            r.world
                .as_ref()
                .and_then(|w| w.address.as_deref())
                .map(str::trim)
                .filter(|a| !a.is_empty())
                .is_none_or(|a| !split_host_port(a).0.eq_ignore_ascii_case(typed))
        });
        (same && differs).then(|| bracketed(typed))
    }

    /// The profile for this server, reached at `addr`: its logon (on the typed host unless the
    /// server names another), its one realm picked by name, the world override only when needed,
    /// its data source, and where to ask all this again (`info`).
    pub fn profile(&self, addr: &Address) -> ServerProfile {
        let auth = self.auth.clone().unwrap_or_default();
        let host = auth.named_host().unwrap_or(&addr.host).to_string();
        let name = Some(self.name.trim())
            .filter(|n| !n.is_empty())
            .unwrap_or(&addr.host);
        let mut p = ServerProfile::new(name, &host);
        p.port = auth.port();
        p.core = core_from(&self.core);
        if let [realm] = self.realms.as_slice() {
            p.realm_name = Some(realm.name.trim().to_string()).filter(|n| !n.is_empty());
        }
        // A realm the document places on another machine is reached where realmd says.
        p.realm_override = match auth.named_host() {
            Some(_) => None,
            None => self.world_override(&addr.host),
        };
        p.data = self.data.as_ref().map(|d| DataSource {
            url: resolve(&addr.base, &d.url),
            ..DataSource::default()
        });
        p.notes = self.message.clone().unwrap_or_default();
        p.info = Some(addr.base.clone());
        p
    }

    /// The sign-up rules, when the server takes sign-ups this client can send now.
    pub fn sign_up_rules(&self) -> Option<&RegistrationInfo> {
        self.registration.as_ref().filter(|r| r.usable())
    }

    /// Where to `POST` a sign-up, when the server takes them now.
    pub fn register_url(&self, base: &str) -> Option<String> {
        self.sign_up_rules().map(|r| resolve(base, r.url()))
    }
}

/// What the Connect screen does about the game data next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DataStep {
    /// The archives are here: straight to the account.
    Have,
    /// The server serves none: the player brings their own (Import from folder…, a PC over USB).
    BringYourOwn,
    /// Several languages and none chosen yet.
    PickLanguage(HostLocales),
    /// The data needs the operator's token and none is saved (or the saved one was refused).
    AskToken,
    /// Ask `url` for the manifest of `locale` (`None`: its default), then download.
    Fetch { url: String, locale: Option<String> },
}

/// The game data's next step for a server that described itself at `base`: `complete` whether
/// the archives are already here, `chosen` the language picked earlier (kept on the profile),
/// `have_token` whether one is saved for it and not refused.
pub fn data_step(
    info: &ServerInfo,
    base: &str,
    complete: bool,
    chosen: Option<&str>,
    have_token: bool,
) -> DataStep {
    if complete {
        return DataStep::Have;
    }
    let Some(d) = &info.data else {
        return DataStep::BringYourOwn;
    };
    // A language the server no longer has is no choice.
    let chosen = chosen.filter(|c| d.has(c));
    if let (Some(locales), None) = (d.host_locales(), chosen) {
        return DataStep::PickLanguage(locales);
    }
    if d.token && !have_token {
        return DataStep::AskToken;
    }
    DataStep::Fetch {
        url: resolve(base, &d.url),
        locale: chosen.map(str::to_string),
    }
}

/// A server that did not describe itself, as the manual form starts it: the typed host on the
/// stock logon port, and the data host when one answered.
pub fn manual_profile(addr: &Address, data: Option<&str>) -> ServerProfile {
    let mut p = ServerProfile::new(&addr.host, &addr.host);
    p.data = data.map(|url| DataSource {
        url: url.to_string(),
        ..DataSource::default()
    });
    p
}

/// Put `p` in the launcher and make it the current server: the profile already there for the
/// same logon (host and port) is updated in place — keeping its id (so its accounts and saved
/// secrets), its notes when the server has none, and the language picked for its data — so a
/// second Connect never makes a twin. Returns the profile's id.
pub fn adopt(launcher: &mut Launcher, mut p: ServerProfile) -> String {
    let same = same_logon(launcher, &p.host, p.port)
        .map(|s| s.id.clone())
        .and_then(|id| launcher.server_mut(&id));
    let id = match same {
        Some(s) => {
            if p.notes.is_empty() {
                p.notes = std::mem::take(&mut s.notes);
            }
            if let (Some(new), Some(old)) = (p.data.as_mut(), s.data.as_ref()) {
                // A resumed download stays in the language it started in.
                if new.locale.is_none() {
                    new.locale = old.locale.clone();
                }
                // A token written into the file by hand, for the same data host.
                if new.token.is_none() && new.url == old.url {
                    new.token = old.token.clone();
                }
            }
            p.id = s.id.clone();
            *s = p;
            s.id.clone()
        }
        None => launcher.add_server(p),
    };
    if launcher.last_server.as_deref() != Some(id.as_str()) {
        launcher.last_server = Some(id.clone());
        launcher.last_account = None;
    }
    id
}

/// An older data host (no `server.json`) answered at `base`, reached at `addr`: a new server for
/// its data on the stock logon — or, for a logon the launcher has already, that server kept as it
/// is (its realm, its world override, where it described itself before: a `server.json` that
/// failed once must not wipe them), given this data source only when it had none. Selected either
/// way; returns its id.
pub fn adopt_data_host(launcher: &mut Launcher, addr: &Address, base: &str) -> String {
    let p = manual_profile(addr, Some(base));
    let Some(id) = same_logon(launcher, &p.host, p.port).map(|s| s.id.clone()) else {
        return adopt(launcher, p);
    };
    if let Some(s) = launcher.server_mut(&id) {
        if s.data.is_none() {
            s.data = p.data;
        }
    }
    if launcher.last_server.as_deref() != Some(id.as_str()) {
        launcher.last_server = Some(id.clone());
        launcher.last_account = None;
    }
    id
}

/// The launcher's server for the logon at `host:port` (the host in any case), when it has one:
/// the one [`adopt`] updates, and the one a plain realm found again is edited as, not added twice.
pub fn same_logon<'a>(launcher: &'a Launcher, host: &str, port: u16) -> Option<&'a ServerProfile> {
    launcher
        .servers
        .iter()
        .find(|s| s.host.trim().eq_ignore_ascii_case(host.trim()) && s.port == port)
}

/// What the address led to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Probe {
    /// The server described itself at `base`.
    Described { info: Box<ServerInfo>, base: String },
    /// It did, in a way this client cannot use (a newer format, another build): the sentence.
    Unusable { why: String },
    /// A data host from before `server.json`: the game data at `base`, nothing else known.
    DataOnly { base: String },
    /// No usable HTTP answer, but a logon server listens on the stock port: a plain 1.12.1 realm.
    RealmOnly,
    /// Nothing answered; the network's reason.
    Nothing { why: String },
}

/// Ask the address what it is: `server.json`, then an older data host's `/`, then a logon server
/// on 3724. `realm_listens(host, port)` is a short TCP connect, the caller's (a plain realm has no
/// HTTP to ask).
pub fn probe(
    fetch: &dyn Fetch,
    addr: &Address,
    realm_listens: &dyn Fn(&str, u16) -> bool,
) -> Probe {
    let why = match fetch.text(&join_url(&addr.base, "server.json")) {
        Ok(text) => match parse_server_info(&text) {
            Ok(info) => {
                let problems = info.problems();
                return if problems.is_empty() {
                    Probe::Described {
                        info: Box::new(info),
                        base: addr.base.clone(),
                    }
                } else {
                    Probe::Unusable {
                        why: problems.join("; "),
                    }
                };
            }
            Err(InfoError::Format(n)) => {
                return Probe::Unusable {
                    why: format_problem(n),
                }
            }
            Err(InfoError::Malformed(_)) => {
                format!("something answered at {}, but not a game server", addr.base)
            }
        },
        Err(e) => e,
    };
    if fetch
        .text(&addr.base)
        .is_ok_and(|t| t.trim_start().starts_with(DATAHOST_HINT))
    {
        return Probe::DataOnly {
            base: addr.base.clone(),
        };
    }
    if realm_listens(&addr.host, DEFAULT_AUTH_PORT) {
        return Probe::RealmOnly;
    }
    Probe::Nothing { why }
}

/// The `POST` half of the network, so sign-up can be tested without one.
pub trait Post {
    /// `POST url` with a JSON body → the status and the body, whatever the status (the reply's
    /// JSON says why a 4xx refused). `Err` only when no answer came.
    fn post_json(&self, url: &str, json: &str) -> Result<(u16, String), String>;
}

/// A sign-up ready to send: the name as it will be stored, and the body. Kept for a retry, so a
/// request whose answer was lost is sent again unchanged — same salt — and the server answers the
/// replay with 200 instead of "name taken".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignUp {
    pub username: String,
    pub json: String,
}

/// `le` (SRP6's little-endian numbers) as vmangos stores them: big-endian upper-case hex.
pub fn big_endian_hex(le: &[u8]) -> String {
    le.iter().rev().map(|b| format!("{b:02X}")).collect()
}

/// A fresh SRP6 salt and its verifier, both little endian, from the upper-cased name and password
/// (`benilla_srp::generate_account`); `None` when it refuses them.
pub type MakeVerifier<'a> = &'a dyn Fn(&str, &str) -> Option<([u8; 32], [u8; 32])>;

/// Check `name` and `password` against the server's rules and build the body it takes:
/// `{"username","salt","verifier"}`, the salt and verifier made by `srp`. Nothing is built — so
/// nothing is sent — for what the server would refuse.
pub fn sign_up(
    rules: &RegistrationInfo,
    name: &str,
    password: &str,
    srp: MakeVerifier,
) -> Result<SignUp, RegisterError> {
    if !rules.usable() {
        return Err(RegisterError::Closed);
    }
    let username = rules.check_name(name).map_err(RegisterError::BadName)?;
    rules
        .check_password(password)
        .map_err(RegisterError::BadPassword)?;
    let (salt, verifier) = srp(&username, &password.to_ascii_uppercase())
        .ok_or_else(|| RegisterError::BadPassword("The password cannot be used.".into()))?;
    let json = serde_json::json!({
        "username": username,
        "salt": big_endian_hex(&salt),
        "verifier": big_endian_hex(&verifier),
    });
    Ok(SignUp {
        username,
        json: json.to_string(),
    })
}

/// `POST` the sign-up → the account name as created, or why not.
pub fn register(post: &dyn Post, url: &str, sign_up: &SignUp) -> Result<String, RegisterError> {
    let (status, text) = post
        .post_json(url, &sign_up.json)
        .map_err(RegisterError::Unreachable)?;
    read_reply(status, &text, &sign_up.username)
}

#[derive(Default, Deserialize)]
struct Reply {
    #[serde(default, deserialize_with = "or_default")]
    ok: bool,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    retry_after: Option<u64>,
}

/// Why a sign-up did not happen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegisterError {
    /// The name, refused here or by the server (its sentence).
    BadName(String),
    /// The password, refused here.
    BadPassword(String),
    NameTaken,
    /// Sign-up is off, or not for this caller (not from this network).
    Closed,
    TokenRequired,
    RateLimited {
        retry_after: Option<u64>,
    },
    /// The accounts database is not up yet (the server is still starting).
    Unavailable,
    /// No answer came: sent again unchanged, it is answered as a replay if it did arrive.
    Unreachable(String),
    /// Anything else, with the server's sentence.
    Failed(String),
}

/// A sign-up reply → the account name, or why not. The `error` code decides; an unknown code, or
/// none, falls back on the status, so the two sides only need to agree on statuses.
pub fn read_reply(status: u16, body: &str, asked: &str) -> Result<String, RegisterError> {
    let r: Reply = serde_json::from_str(body).unwrap_or_default();
    // 201 created; 200 the replay of a request that already created it. The name is the one
    // asked, whatever the reply echoes: the verifier was made for it, so no other logs in.
    if matches!(status, 200 | 201) && (r.ok || r.username.is_some() || status == 201) {
        return Ok(asked.to_string());
    }
    let message = r
        .message
        .clone()
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| format!("the server answered {status}"));
    Err(match (r.error.as_deref(), status) {
        (Some("invalid_name"), _) => RegisterError::BadName(message),
        (Some("name_taken"), _) | (None, 409) => RegisterError::NameTaken,
        (Some("token_required"), _) | (None, 401) => RegisterError::TokenRequired,
        (Some("registration_closed"), _) | (None, 403) => RegisterError::Closed,
        (Some("rate_limited"), _) | (None, 429) => RegisterError::RateLimited {
            retry_after: r.retry_after,
        },
        (Some("server_unavailable"), _) | (None, 502..=504) => RegisterError::Unavailable,
        // `invalid_credentials`, `bad_request`, `too_large`, `unsupported_media_type`,
        // `internal`: nothing the player can change; the server's sentence says what.
        (Some(code), _) if KNOWN_CODES.contains(&code) => RegisterError::Failed(message),
        // A code from a newer server: the status decides.
        (Some(_), 409) => RegisterError::NameTaken,
        (Some(_), 401) => RegisterError::TokenRequired,
        (Some(_), 403) => RegisterError::Closed,
        (Some(_), 429) => RegisterError::RateLimited {
            retry_after: r.retry_after,
        },
        (Some(_), 502..=504) => RegisterError::Unavailable,
        _ => RegisterError::Failed(message),
    })
}

/// The error codes of the contract that the player cannot act on.
const KNOWN_CODES: &[&str] = &[
    "invalid_credentials",
    "bad_request",
    "too_large",
    "unsupported_media_type",
    "internal",
];

/// The alphabet of a generated password: no `0`/`O` or `1`/`I` to misread, and capitals only —
/// 1.12 upper-cases passwords, so the case would not matter anyway. 32 symbols, so a random byte
/// picks one without bias.
pub const PASSWORD_ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

/// A generated password from `random` bytes, one symbol each (12 make a 60-bit password).
pub fn suggest_password(random: &[u8]) -> String {
    random
        .iter()
        .map(|b| PASSWORD_ALPHABET[usize::from(b % 32)] as char)
        .collect()
}

/// A generated account name: `PLAYER` and four digits.
pub fn suggest_name(random: u32) -> String {
    format!("PLAYER{:04}", random % 10_000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profiles::Account;
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::io::Read;

    /// What the full stack's data host serves (`CONTRACT.md`'s document, two languages).
    const STACK: &str = r#"{
        "format": 1,
        "name": "Vanillish",
        "software": "vanillish-datahost 0.3.0",
        "core": "vmangos",
        "build": 5875,
        "auth": { "port": 3724 },
        "realms": [ { "name": "Vanillish", "kind": "pve",
                      "world": { "address": "192.168.1.20", "port": 8085, "same_host": true } } ],
        "data": { "url": "", "token": false, "default": "enUS",
                  "locales": [ { "locale": "enUS", "path": "enUS/", "bytes": 5400000000, "addons": 1 },
                               { "locale": "deDE", "path": "deDE/", "bytes": 5300000000, "addons": 1 } ] },
        "registration": { "open": true, "url": "accounts", "token": false, "credentials": "srp6",
                          "name_min": 3, "name_max": 16, "password_min": 6, "password_max": 16 },
        "message": "Welcome home."
    }"#;

    fn stack() -> ServerInfo {
        parse_server_info(STACK).unwrap()
    }

    fn addr(s: &str) -> Address {
        parse_address(s).unwrap()
    }

    #[test]
    fn addresses_read_as_the_data_host_unless_they_say_otherwise() {
        assert_eq!(
            addr("192.168.1.20"),
            Address {
                host: "192.168.1.20".into(),
                base: "http://192.168.1.20:8099/".into()
            }
        );
        // The realmlist habit: the logon's port still means the data host's.
        assert_eq!(
            addr(" 192.168.1.20:3724 ").base,
            "http://192.168.1.20:8099/"
        );
        assert_eq!(
            addr("mac-mini.local:9000").base,
            "http://mac-mini.local:9000/"
        );
        assert_eq!(addr("mac-mini.local:9000").host, "mac-mini.local");
        // IPv6: bracketed in the URL, bare as the host.
        assert_eq!(addr("[fd00::2]:8099").host, "fd00::2");
        assert_eq!(addr("[fd00::2]:8099").base, "http://[fd00::2]:8099/");
        assert_eq!(addr("[fd00::2]:3724").base, "http://[fd00::2]:8099/");
        assert_eq!(addr("[fd00::2]").base, "http://[fd00::2]:8099/");
        assert_eq!(addr("::1").base, "http://[::1]:8099/");
        assert_eq!(addr("fd7a:115c::5").host, "fd7a:115c::5");
        // A link is the base itself: scheme lower-cased, query and fragment dropped, a slash added.
        let a = addr("HTTPS://wow.example.com/vanillish?x=1#top");
        assert_eq!(
            (a.host.as_str(), a.base.as_str()),
            ("wow.example.com", "https://wow.example.com/vanillish/")
        );
        assert_eq!(addr("http://10.0.0.5:8099").base, "http://10.0.0.5:8099/");
        assert_eq!(addr("http://10.0.0.5:8099/").base, "http://10.0.0.5:8099/");
        assert_eq!(addr("http://[fd00::2]:9000/x/").host, "fd00::2");
        assert_eq!(
            addr("http://[fd00::2]:9000/x/").base,
            "http://[fd00::2]:9000/x/"
        );
        assert_eq!(addr("http://localhost").base, "http://localhost/");
        for bad in [
            "",
            "  ",
            "my server",
            "host/path",
            "-x",
            ".x",
            "a:b:c:0",
            "x;rm",
            "host:0",
            "host:",
            "host:99999",
            "http://",
            "http://:8099/",
            "http://host:0/",
            "http://user:pw@host/",
            "ftp://host",
            // A bracketed host is an IPv6 literal with nothing after it but a port.
            "[fd00::2]:99999",
            "[fd00::2]:0",
            "[fd00::2]:",
            "[fd00::2]x",
            "[fd00::2",
            "[my-mac.local]:8099",
            "http://[fd00::2]:99999/",
            "http://[fd00::2]junk/",
            "http://host:/",
            "http://host:99999/",
        ] {
            assert!(parse_address(bad).is_err(), "{bad:?}");
        }
        assert_eq!(
            parse_address("host:99999"),
            Err("The port must be between 1 and 65535.".into())
        );
    }

    #[test]
    fn the_stack_describes_a_whole_profile() {
        let info = stack();
        assert!(info.problems().is_empty());
        let p = info.profile(&addr("192.168.1.20"));
        assert_eq!(
            (p.name.as_str(), p.host.as_str(), p.port),
            ("Vanillish", "192.168.1.20", 3724)
        );
        assert_eq!(p.core, Core::Vmangos);
        assert_eq!(p.realm_name.as_deref(), Some("Vanillish"));
        assert_eq!(
            p.realm_override, None,
            "the realm already hands out the typed host"
        );
        let d = p.data.as_ref().unwrap();
        assert_eq!(d.url, "http://192.168.1.20:8099/");
        assert_eq!((d.token.as_deref(), d.locale.as_deref()), (None, None));
        assert_eq!(p.info.as_deref(), Some("http://192.168.1.20:8099/"));
        assert_eq!(p.notes, "Welcome home.");
        assert!(p.validate().is_empty());
        let d = info.data.as_ref().unwrap();
        assert_eq!(d.host_locales().unwrap().locales.len(), 2);
        assert_eq!(d.bytes(Some("deDE")), Some(5_300_000_000));
        assert_eq!(d.bytes(None), Some(5_400_000_000));
        assert_eq!(info.realms[0].kind_label(), Some("PvE"));
        assert_eq!(
            info.register_url("http://192.168.1.20:8099/").as_deref(),
            Some("http://192.168.1.20:8099/accounts")
        );
    }

    #[test]
    fn the_world_is_dialled_where_the_player_came_from_only_when_it_must_be() {
        let info = stack();
        // A `.local` name, a VPN address: the realm row's LAN IP is overridden, its port kept.
        let p = info.profile(&addr("mac-mini.local"));
        assert_eq!(p.host, "mac-mini.local");
        assert_eq!(p.realm_override.as_deref(), Some("mac-mini.local"));
        assert_eq!(p.world_address("192.168.1.20:8085"), "mac-mini.local:8085");
        let p = info.profile(&addr("fd7a:115c::5"));
        assert_eq!(p.realm_override.as_deref(), Some("[fd7a:115c::5]"));
        assert_eq!(p.world_address("192.168.1.20:8085"), "[fd7a:115c::5]:8085");
        // The same host typed in another case is the same host.
        let mut named = info.clone();
        named.realms[0].world.as_mut().unwrap().address = Some("Mac-Mini.local:8085".into());
        assert_eq!(named.profile(&addr("mac-mini.local")).realm_override, None);
        // The realm hands out nothing it says: the typed host is the only one known to work.
        let mut blank = info.clone();
        blank.realms[0].world.as_mut().unwrap().address = None;
        assert_eq!(
            blank
                .profile(&addr("192.168.1.20"))
                .realm_override
                .as_deref(),
            Some("192.168.1.20")
        );
        // A realm elsewhere, or one realm of two elsewhere: never.
        let mut other = info.clone();
        other.realms[0].world.as_mut().unwrap().same_host = false;
        assert_eq!(other.profile(&addr("mac-mini.local")).realm_override, None);
        let mut no_world = info.clone();
        no_world.realms[0].world = None;
        assert_eq!(
            no_world.profile(&addr("mac-mini.local")).realm_override,
            None
        );
        let mut mixed = info.clone();
        mixed.realms.push(other.realms[0].clone());
        assert_eq!(mixed.profile(&addr("mac-mini.local")).realm_override, None);
        // The document names realmd's host: the logon goes there, and no override.
        let mut remote = info.clone();
        remote.auth = Some(AuthInfo {
            host: Some("wow.example.com".into()),
            port: Some(3725),
        });
        let p = remote.profile(&addr("files.example.com"));
        assert_eq!(
            (p.host.as_str(), p.port, p.realm_override.as_deref()),
            ("wow.example.com", 3725, None)
        );
        assert_eq!(p.data.unwrap().url, "http://files.example.com:8099/");
        // An empty `auth.host` is no host.
        let mut empty = info.clone();
        empty.auth = Some(AuthInfo {
            host: Some(" ".into()),
            port: None,
        });
        let p = empty.profile(&addr("mac-mini.local"));
        assert_eq!((p.host.as_str(), p.port), ("mac-mini.local", 3724));
        assert_eq!(p.realm_override.as_deref(), Some("mac-mini.local"));
        // Two realms: none picked by name, both same-host and listed elsewhere: overridden.
        let mut two = info;
        two.realms.push(two.realms[0].clone());
        two.realms[1].name = "Test".into();
        let p = two.profile(&addr("10.0.0.9"));
        assert_eq!(p.realm_name, None);
        assert_eq!(p.realm_override.as_deref(), Some("10.0.0.9"));
    }

    #[test]
    fn a_minimal_description_still_makes_a_profile() {
        let info = parse_server_info(r#"{"format":1}"#).unwrap();
        assert!(info.problems().is_empty());
        let p = info.profile(&addr("10.0.0.5"));
        assert_eq!(
            (p.name.as_str(), p.host.as_str(), p.port),
            ("10.0.0.5", "10.0.0.5", 3724)
        );
        assert_eq!((p.core, p.data, p.realm_name), (Core::Other, None, None));
        assert_eq!(p.realm_override, None);
        assert_eq!(info.register_url("http://10.0.0.5:8099/"), None);
        // Unknown fields from a newer server of the same format are ignored, and an unset
        // environment value (`null`) reads as absent.
        let info = parse_server_info(
            r#"{"format":1,"name":"x","future":{"a":1},"core":null,"realms":null,
                "auth":{"port":null,"host":null,"tls":true},
                "data":{"url":null,"token":null,"default":null,"locales":null,"cdn":"x"},
                "registration":{"open":true,"url":null,"credentials":null,"captcha":false}}"#,
        )
        .unwrap();
        assert_eq!(info.name, "x");
        assert_eq!(info.auth.as_ref().unwrap().port(), 3724);
        let d = info.data.as_ref().unwrap();
        assert!(d.locales.is_empty() && !d.token && d.url.is_empty());
        assert_eq!(d.bytes(None), None);
        assert_eq!(d.host_locales(), None);
        assert_eq!(
            info.register_url("http://h:8099/").as_deref(),
            Some("http://h:8099/accounts")
        );
        // Not the document: no format, not JSON at all.
        assert!(matches!(
            parse_server_info(r#"{"name":"router"}"#),
            Err(InfoError::Malformed(_))
        ));
        assert!(matches!(
            parse_server_info("<html>router</html>"),
            Err(InfoError::Malformed(_))
        ));
    }

    #[test]
    fn a_newer_format_or_another_build_asks_for_an_update() {
        // Format 2 in a shape this client cannot even read is still "update the app".
        assert_eq!(
            parse_server_info(r#"{"format":2,"realms":"one","auth":[1,2]}"#),
            Err(InfoError::Format(2))
        );
        assert!(format_problem(2).contains("update the app"));
        let other_build = parse_server_info(r#"{"format":1,"build":6005}"#).unwrap();
        assert_eq!(other_build.problems().len(), 1);
        assert!(other_build.problems()[0].contains("6005"));
        let port0 = parse_server_info(r#"{"format":1,"auth":{"port":0}}"#).unwrap();
        assert_eq!(port0.problems().len(), 1);
    }

    #[test]
    fn absent_sections_are_what_the_server_does_not_do() {
        // A data host alone (compose.datahost.yaml): no auth, no realms, no sign-up.
        let alone = parse_server_info(
            r#"{"format":1,"name":"Files","data":{"url":"","token":true,"default":"enUS",
                "locales":[{"locale":"enUS","path":"","bytes":5400000000}]}}"#,
        )
        .unwrap();
        let p = alone.profile(&addr("nas.local"));
        assert_eq!((p.host.as_str(), p.port), ("nas.local", 3724));
        assert_eq!((p.realm_name, p.realm_override), (None, None));
        assert_eq!(alone.register_url("http://nas.local:8099/"), None);
        assert_eq!(alone.sign_up_rules(), None);
        // A server with no data sets yet (sign-up before the import): no data.
        let empty =
            parse_server_info(r#"{"format":1,"auth":{"port":3724},"registration":{"open":true}}"#)
                .unwrap();
        assert_eq!(empty.profile(&addr("10.0.0.5")).data, None);
        assert!(empty.sign_up_rules().is_some());
        // Closed, or wanting the password itself: no Create tab.
        let closed = parse_server_info(r#"{"format":1,"registration":{"open":false}}"#).unwrap();
        assert_eq!(closed.sign_up_rules(), None);
        let no_open = parse_server_info(r#"{"format":1,"registration":{}}"#).unwrap();
        assert_eq!(no_open.sign_up_rules(), None);
        let password = parse_server_info(
            r#"{"format":1,"registration":{"open":true,"credentials":"password"}}"#,
        )
        .unwrap();
        assert_eq!(password.sign_up_rules(), None);
        assert_eq!(password.register_url("http://h/"), None);
        let upper =
            parse_server_info(r#"{"format":1,"registration":{"open":true,"credentials":"SRP6"}}"#)
                .unwrap();
        assert!(upper.sign_up_rules().is_some());
    }

    /// The admin menu is announced or not; an empty section reads as its defaults.
    #[test]
    fn the_admin_section_is_read_when_there() {
        assert_eq!(stack().admin, None);
        let empty = parse_server_info(r#"{"format":1,"admin":{}}"#).unwrap();
        assert_eq!(empty.admin, Some(AdminInfo::default()));
        let nulls = parse_server_info(r#"{"format":1,"admin":{"url":null,"open":null,"later":1}}"#)
            .unwrap();
        assert_eq!(nulls.admin, Some(AdminInfo::default()));
        let full =
            parse_server_info(r#"{"format":1,"admin":{"url":"admin/","open":true}}"#).unwrap();
        assert_eq!(
            full.admin,
            Some(AdminInfo {
                url: "admin/".into(),
                open: true
            })
        );
        let closed =
            parse_server_info(r#"{"format":1,"admin":{"url":"admin/","open":false}}"#).unwrap();
        assert!(closed.admin.is_some_and(|a| !a.open));
    }

    #[test]
    fn urls_resolve_against_the_base() {
        let b = "http://h:8099/";
        assert_eq!(resolve(b, ""), b);
        assert_eq!(resolve(b, " . "), b);
        assert_eq!(resolve(b, "./"), b);
        assert_eq!(resolve(b, "accounts"), "http://h:8099/accounts");
        assert_eq!(resolve(b, "./accounts"), "http://h:8099/accounts");
        assert_eq!(resolve(b, "/data/"), "http://h:8099/data/");
        assert_eq!(
            resolve(b, "https://cdn.example.com/d/"),
            "https://cdn.example.com/d/"
        );
        // Behind a reverse proxy: relative to the folder, absolute paths to the origin.
        let p = "https://wow.example.com/vanillish/";
        assert_eq!(
            resolve(p, "accounts"),
            "https://wow.example.com/vanillish/accounts"
        );
        assert_eq!(resolve(p, ""), p);
        assert_eq!(resolve(p, "/signup"), "https://wow.example.com/signup");
        assert_eq!(
            resolve(p, "//cdn.example.com/x/"),
            "https://cdn.example.com/x/"
        );
        assert_eq!(
            resolve("http://[fd00::2]:8099/", "/accounts"),
            "http://[fd00::2]:8099/accounts"
        );
        let info = stack();
        let mut there = info.clone();
        there.data.as_mut().unwrap().url = "/files/".into();
        there.registration.as_mut().unwrap().url = Some("https://site.example/join".into());
        let p = there.profile(&addr("https://wow.example.com/vanillish/"));
        assert_eq!(p.data.unwrap().url, "https://wow.example.com/files/");
        assert_eq!(
            there
                .register_url("https://wow.example.com/vanillish/")
                .as_deref(),
            Some("https://site.example/join")
        );
    }

    #[test]
    fn connecting_twice_updates_the_profile_in_place() {
        let info = stack();
        let mut l = Launcher::default();
        let first = adopt(&mut l, info.profile(&addr("192.168.1.20")));
        assert_eq!(l.last_server.as_deref(), Some(first.as_str()));
        l.add_account(Account::new(&first, "alice"));
        l.last_account = Some(l.accounts[0].id.clone());
        // The player picked a language, and wrote a token into the file by hand.
        {
            let d = l.servers[0].data.as_mut().unwrap();
            d.locale = Some("deDE".into());
            d.token = Some("by-hand".into());
        }
        let mut renamed = info.clone();
        renamed.name = "Home".into();
        renamed.message = None;
        let again = adopt(&mut l, renamed.profile(&addr("192.168.1.20")));
        assert_eq!(again, first);
        assert_eq!(l.servers.len(), 1);
        assert_eq!(l.servers[0].name, "Home");
        assert_eq!(
            l.servers[0].notes, "Welcome home.",
            "kept when the server says nothing"
        );
        let d = l.servers[0].data.as_ref().unwrap();
        assert_eq!(
            d.locale.as_deref(),
            Some("deDE"),
            "a resumed download keeps its language"
        );
        assert_eq!(d.token.as_deref(), Some("by-hand"));
        assert_eq!(l.accounts.len(), 1);
        assert!(l.last_account.is_some(), "the same server stays as it was");
        // A rename of the realm on the server follows on the next Connect.
        let mut realm = info.clone();
        realm.realms[0].name = "Azeroth".into();
        adopt(&mut l, realm.profile(&addr("192.168.1.20")));
        assert_eq!(l.servers[0].realm_name.as_deref(), Some("Azeroth"));
        // The same host reached on another logon port is another server; so is another host.
        let other = adopt(&mut l, manual_profile(&addr("nas.local"), None));
        assert_ne!(other, first);
        assert_eq!(l.last_server.as_deref(), Some(other.as_str()));
        assert_eq!(l.last_account, None);
        assert_eq!(l.servers.len(), 2);
        // The host as typed in another case is the same server.
        let back = adopt(&mut l, info.profile(&addr("192.168.1.20")));
        assert_eq!(back, first);
        let upper = manual_profile(&addr("NAS.local"), Some("http://NAS.local:8099/"));
        assert_eq!(adopt(&mut l, upper), other);
        assert_eq!(l.servers.len(), 2);
        // The server a logon belongs to, whatever case its host is typed in; none on another port.
        assert_eq!(
            same_logon(&l, "nas.LOCAL", 3724).map(|s| s.id.as_str()),
            Some(other.as_str())
        );
        assert_eq!(same_logon(&l, "nas.local", 3725), None);
        assert_eq!(same_logon(&l, "elsewhere", 3724), None);
    }

    #[test]
    fn an_older_answer_never_wipes_what_a_server_said_before() {
        let mut l = Launcher::default();
        let a = addr("mac-mini.local");
        let id = adopt(&mut l, stack().profile(&a));
        let before = l.server(&id).unwrap().clone();
        assert!(before.realm_override.is_some() && before.info.is_some());
        // `server.json` failed this once, the data host's `/` answered: the profile stays whole.
        l.last_server = None;
        assert_eq!(adopt_data_host(&mut l, &a, &a.base), id);
        assert_eq!(l.server(&id), Some(&before));
        assert_eq!(l.last_server.as_deref(), Some(id.as_str()));
        assert_eq!(l.servers.len(), 1);
        // A server entered by hand gets the data source it lacked, and nothing else changes.
        let mut by_hand = ServerProfile::new("Home", "10.0.0.7");
        by_hand.realm_override = Some("10.0.0.7".into());
        let hand = l.add_server(by_hand);
        let b = addr("10.0.0.7");
        assert_eq!(adopt_data_host(&mut l, &b, &b.base), hand);
        let s = l.server(&hand).unwrap();
        assert_eq!(s.data.as_ref().unwrap().url, "http://10.0.0.7:8099/");
        assert_eq!(
            (s.name.as_str(), s.realm_override.as_deref()),
            ("Home", Some("10.0.0.7"))
        );
        assert_eq!(l.last_account, None);
        // A new host is a new server, on the stock logon.
        let c = addr("nas.local:9000");
        let new = adopt_data_host(&mut l, &c, &c.base);
        let s = l.server(&new).unwrap();
        assert_eq!((s.host.as_str(), s.port), ("nas.local", 3724));
        assert_eq!(s.data.as_ref().unwrap().url, "http://nas.local:9000/");
        assert_eq!(l.servers.len(), 3);
    }

    /// A fake network: URL → body, anything else unreachable.
    struct Net(BTreeMap<String, String>);

    impl Fetch for Net {
        fn text(&self, url: &str) -> Result<String, String> {
            self.0
                .get(url)
                .cloned()
                .ok_or_else(|| format!("could not reach {url}"))
        }
        fn body(&self, _: &str, _: u64) -> Result<(Box<dyn Read + Send>, bool), String> {
            Err("no files here".into())
        }
    }

    fn net(pairs: &[(&str, &str)]) -> Net {
        Net(pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect())
    }

    #[test]
    fn the_probe_tells_a_described_server_from_an_older_or_plain_one() {
        let a = addr("192.168.1.20");
        let up = |_: &str, port: u16| port == 3724;
        let down = |_: &str, _: u16| false;
        let described = net(&[("http://192.168.1.20:8099/server.json", STACK)]);
        match probe(&described, &a, &down) {
            Probe::Described { info, base } => {
                assert_eq!(info.name, "Vanillish");
                assert_eq!(base, "http://192.168.1.20:8099/");
            }
            other => panic!("{other:?}"),
        }
        let older = net(&[(
            "http://192.168.1.20:8099/",
            "inilla-datahost: enter this address in iNilla → Game data.\n",
        )]);
        assert_eq!(
            probe(&older, &a, &up),
            Probe::DataOnly {
                base: "http://192.168.1.20:8099/".into()
            }
        );
        // A router's page on 8099 is not a data host; the realm on 3724 still counts.
        let html = net(&[
            (
                "http://192.168.1.20:8099/server.json",
                "<html>router</html>",
            ),
            ("http://192.168.1.20:8099/", "<html>router</html>"),
        ]);
        assert_eq!(probe(&html, &a, &up), Probe::RealmOnly);
        match probe(&html, &a, &down) {
            Probe::Nothing { why } => assert!(why.contains("not a game server"), "{why}"),
            other => panic!("{other:?}"),
        }
        match probe(&net(&[]), &a, &down) {
            Probe::Nothing { why } => assert!(why.contains("could not reach"), "{why}"),
            other => panic!("{other:?}"),
        }
        let newer = net(&[("http://192.168.1.20:8099/server.json", r#"{"format":2}"#)]);
        match probe(&newer, &a, &up) {
            Probe::Unusable { why } => assert!(why.contains("update the app"), "{why}"),
            other => panic!("{other:?}"),
        }
        let build = net(&[(
            "http://192.168.1.20:8099/server.json",
            r#"{"format":1,"build":8606}"#,
        )]);
        assert!(matches!(probe(&build, &a, &up), Probe::Unusable { .. }));
        // The realm is asked on the typed host, at the stock port.
        let asked = RefCell::new(Vec::new());
        let spy = |h: &str, p: u16| {
            asked.borrow_mut().push(format!("{h}:{p}"));
            false
        };
        probe(&net(&[]), &addr("[fd00::2]:9000"), &spy);
        assert_eq!(asked.borrow().as_slice(), ["fd00::2:3724"]);
    }

    #[test]
    fn the_data_step_asks_only_what_it_must() {
        let info = stack();
        let base = "http://h:8099/";
        assert_eq!(data_step(&info, base, true, None, false), DataStep::Have);
        assert!(
            matches!(data_step(&info, base, false, None, false), DataStep::PickLanguage(l) if l.default == "enUS" && l.locales.len() == 2)
        );
        assert!(
            matches!(
                data_step(&info, base, false, Some("frFR"), false),
                DataStep::PickLanguage(_)
            ),
            "a language the server no longer has"
        );
        assert_eq!(
            data_step(&info, base, false, Some("deDE"), false),
            DataStep::Fetch {
                url: base.into(),
                locale: Some("deDE".into())
            }
        );
        let mut one = info.clone();
        one.data.as_mut().unwrap().locales.truncate(1);
        one.data.as_mut().unwrap().token = true;
        assert_eq!(
            data_step(&one, base, false, None, false),
            DataStep::AskToken
        );
        assert_eq!(
            data_step(&one, base, false, None, true),
            DataStep::Fetch {
                url: base.into(),
                locale: None
            }
        );
        // The token is asked after the language, so the sizes show first.
        let mut both = info.clone();
        both.data.as_mut().unwrap().token = true;
        assert!(matches!(
            data_step(&both, base, false, None, false),
            DataStep::PickLanguage(_)
        ));
        assert_eq!(
            data_step(&both, base, false, Some("enUS"), false),
            DataStep::AskToken
        );
        // The data somewhere else.
        let mut elsewhere = one.clone();
        elsewhere.data.as_mut().unwrap().url = "files/".into();
        assert_eq!(
            data_step(&elsewhere, base, false, None, true),
            DataStep::Fetch {
                url: "http://h:8099/files/".into(),
                locale: None
            }
        );
        let mut none = info;
        none.data = None;
        assert_eq!(
            data_step(&none, base, false, None, true),
            DataStep::BringYourOwn
        );
        assert_eq!(data_step(&none, base, true, None, true), DataStep::Have);
    }

    #[test]
    fn sign_up_rules_follow_the_logon_and_vmangos() {
        let r = stack().registration.unwrap();
        assert_eq!(r.check_name(" alice7 "), Ok("ALICE7".into()));
        assert_eq!(
            r.check_name("abcdefghijklmnop"),
            Ok("ABCDEFGHIJKLMNOP".into())
        );
        for bad in [
            "al",
            "abcdefghijklmnopq",
            "seventeen-letters",
            "ab cd",
            "Zoë",
            "x'; --",
            "under_score",
            "",
        ] {
            assert!(r.check_name(bad).is_err(), "{bad:?}");
        }
        assert!(r.check_password("s3cret!").is_ok());
        assert!(r.check_password("123456").is_ok());
        assert!(r.check_password("~!@#$%^&*()_+{}").is_ok());
        for bad in [
            "short",
            "has space",
            "quote\"d",
            "apo'strophe",
            "back\\slash",
            "seventeen-chars!!",
            "pässwort",
            "tab\there",
        ] {
            assert!(r.check_password(bad).is_err(), "{bad:?}");
        }
        // A server cannot ask for more than vmangos stores, nor for nothing.
        let loose = RegistrationInfo {
            name_max: Some(64),
            password_max: Some(64),
            password_min: Some(0),
            name_min: Some(0),
            ..r.clone()
        };
        assert!(loose.check_password("seventeen-chars!!").is_err());
        assert!(loose.check_password("x").is_ok());
        assert!(loose.check_password("").is_err());
        assert_eq!(loose.name_range(), (1, 16));
        // The defaults are the contract's: 3 to 16, 6 to 16.
        let defaults = RegistrationInfo::default();
        assert_eq!(defaults.name_range(), (3, 16));
        assert_eq!(defaults.password_range(), (6, 16));
        assert_eq!(defaults.url(), "accounts");
    }

    /// A stand-in for `benilla_srp::generate_account`: a fixed salt, the "verifier" its inputs.
    fn fake_srp(user: &str, pass: &str) -> Option<([u8; 32], [u8; 32])> {
        let mut salt = [0u8; 32];
        salt[31] = 0x80;
        salt[0] = 1;
        let mut v = [0u8; 32];
        for (i, b) in format!("{user}:{pass}").bytes().take(32).enumerate() {
            v[31 - i] = b;
        }
        Some((salt, v))
    }

    #[test]
    fn sign_up_sends_the_verifier_not_the_password() {
        let r = stack().registration.unwrap();
        let up = sign_up(&r, "alice", "s3cret!", &fake_srp).unwrap();
        assert_eq!(up.username, "ALICE");
        let v: serde_json::Value = serde_json::from_str(&up.json).unwrap();
        assert_eq!(v.as_object().unwrap().len(), 3, "{v}");
        assert!(
            v.get("password").is_none(),
            "the password never leaves the phone"
        );
        assert!(!up.json.to_ascii_uppercase().contains("S3CRET"));
        let salt = v["salt"].as_str().unwrap();
        assert_eq!(salt.len(), 64);
        assert!(salt.starts_with("80"), "big endian, top bit set");
        assert!(salt.ends_with("01"));
        assert_eq!(salt, salt.to_ascii_uppercase());
        // The verifier is made from the upper-cased name and password, as the logon hashes them.
        assert!(
            v["verifier"]
                .as_str()
                .unwrap()
                .starts_with("414C4943453A533343524554"),
            "ALICE:S3CRET, most significant byte first"
        );
        let srp_inputs = RefCell::new(Vec::new());
        let spy = |u: &str, p: &str| {
            srp_inputs.borrow_mut().push(format!("{u}:{p}"));
            fake_srp(u, p)
        };
        sign_up(&r, "Bob7", "Pa55word", &spy).unwrap();
        assert_eq!(srp_inputs.borrow().as_slice(), ["BOB7:PA55WORD"]);
        // Nothing is built, so nothing is sent, for what the server would refuse.
        assert_eq!(
            sign_up(&r, "bo", "s3cret!", &fake_srp),
            Err(RegisterError::BadName(
                "The account name is 3 to 16 letters or digits.".into()
            ))
        );
        assert!(matches!(
            sign_up(&r, "alice", "short", &fake_srp),
            Err(RegisterError::BadPassword(_))
        ));
        assert!(matches!(
            sign_up(&r, "alice", "s3cret!", &|_, _| None),
            Err(RegisterError::BadPassword(_))
        ));
        // A closed server, or one that wants the password itself, gets nothing.
        let closed = RegistrationInfo {
            open: false,
            ..r.clone()
        };
        assert_eq!(
            sign_up(&closed, "alice", "s3cret!", &fake_srp),
            Err(RegisterError::Closed)
        );
        let console = RegistrationInfo {
            credentials: Some("password".into()),
            ..r
        };
        assert_eq!(
            sign_up(&console, "alice", "s3cret!", &fake_srp),
            Err(RegisterError::Closed)
        );
        assert_eq!(big_endian_hex(&[0x01, 0x02, 0xab]), "AB0201");
    }

    struct Server(
        RefCell<Vec<String>>,
        Result<(u16, &'static str), &'static str>,
    );

    impl Post for Server {
        fn post_json(&self, url: &str, json: &str) -> Result<(u16, String), String> {
            self.0.borrow_mut().push(format!("{url} {json}"));
            self.1
                .map(|(s, b)| (s, b.to_string()))
                .map_err(str::to_string)
        }
    }

    #[test]
    fn sign_up_posts_once_and_reads_every_reply_the_contract_has() {
        let r = stack().registration.unwrap();
        let up = sign_up(&r, "alice", "s3cret!", &fake_srp).unwrap();
        let ok = Server(
            RefCell::default(),
            Ok((
                201,
                r#"{"ok":true,"username":"ALICE","message":"Account ALICE created."}"#,
            )),
        );
        assert_eq!(register(&ok, "http://h/accounts", &up), Ok("ALICE".into()));
        assert_eq!(
            ok.0.borrow().as_slice(),
            [format!("http://h/accounts {}", up.json)]
        );
        // The replay of a request whose answer was lost.
        let replay = Server(
            RefCell::default(),
            Ok((200, r#"{"ok":true,"username":"ALICE","message":"x"}"#)),
        );
        assert_eq!(register(&replay, "u", &up), Ok("ALICE".into()));
        let lost = Server(
            RefCell::default(),
            Err("could not reach the server: timed out"),
        );
        assert_eq!(
            register(&lost, "u", &up),
            Err(RegisterError::Unreachable(
                "could not reach the server: timed out".into()
            ))
        );
        let reply = |status, body: &str| read_reply(status, body, "A");
        let refusal = |status, code: &str| {
            reply(
                status,
                &format!(r#"{{"ok":false,"error":"{code}","message":"Why {code}."}}"#),
            )
        };
        assert_eq!(
            refusal(400, "invalid_name"),
            Err(RegisterError::BadName("Why invalid_name.".into()))
        );
        for code in ["invalid_credentials", "bad_request"] {
            assert_eq!(
                refusal(400, code),
                Err(RegisterError::Failed(format!("Why {code}.")))
            );
        }
        assert_eq!(
            refusal(401, "token_required"),
            Err(RegisterError::TokenRequired)
        );
        assert_eq!(
            refusal(403, "registration_closed"),
            Err(RegisterError::Closed)
        );
        assert_eq!(refusal(409, "name_taken"), Err(RegisterError::NameTaken));
        assert_eq!(
            refusal(413, "too_large"),
            Err(RegisterError::Failed("Why too_large.".into()))
        );
        assert_eq!(
            refusal(415, "unsupported_media_type"),
            Err(RegisterError::Failed("Why unsupported_media_type.".into()))
        );
        assert_eq!(
            reply(
                429,
                r#"{"ok":false,"error":"rate_limited","message":"Slow down.","retry_after":600}"#
            ),
            Err(RegisterError::RateLimited {
                retry_after: Some(600)
            })
        );
        assert_eq!(
            refusal(503, "server_unavailable"),
            Err(RegisterError::Unavailable)
        );
        assert_eq!(
            refusal(500, "internal"),
            Err(RegisterError::Failed("Why internal.".into()))
        );
        // No body, or one that is not JSON (a proxy's page): the status decides.
        assert_eq!(reply(409, ""), Err(RegisterError::NameTaken));
        assert_eq!(
            reply(401, "token required"),
            Err(RegisterError::TokenRequired)
        );
        assert_eq!(reply(403, ""), Err(RegisterError::Closed));
        assert_eq!(
            reply(429, ""),
            Err(RegisterError::RateLimited { retry_after: None })
        );
        assert_eq!(
            reply(502, "<html>bad gateway</html>"),
            Err(RegisterError::Unavailable)
        );
        assert_eq!(
            reply(500, ""),
            Err(RegisterError::Failed("the server answered 500".into()))
        );
        assert_eq!(
            reply(404, ""),
            Err(RegisterError::Failed("the server answered 404".into()))
        );
        // A code from a newer server: the status still says what happened.
        assert_eq!(refusal(409, "name_reserved"), Err(RegisterError::NameTaken));
        assert_eq!(
            refusal(418, "teapot"),
            Err(RegisterError::Failed("Why teapot.".into()))
        );
        // Success without a name echoes the one asked for; a 200 that says no is no.
        assert_eq!(reply(200, r#"{"ok":true}"#), Ok("A".into()));
        assert_eq!(reply(201, ""), Ok("A".into()));
        // A reply that names another account still means the one asked: its verifier is that
        // name's, so the launcher must not save another name with this password.
        assert_eq!(
            reply(201, r#"{"ok":true,"username":"SOMEONE"}"#),
            Ok("A".into())
        );
        assert!(reply(200, r#"{"ok":false,"error":"internal"}"#).is_err());
    }

    #[test]
    fn the_profile_survives_the_launcher_file() {
        let mut l = Launcher::default();
        let id = adopt(&mut l, stack().profile(&addr("mac-mini.local")));
        l.servers[0].data.as_mut().unwrap().locale = Some("deDE".into());
        let text = toml::to_string(&l).unwrap();
        let back: Launcher = toml::from_str(&text).unwrap();
        assert_eq!(back, l);
        assert_eq!(
            back.server(&id).unwrap().info.as_deref(),
            Some("http://mac-mini.local:8099/")
        );
    }

    #[test]
    fn suggestions_pass_the_rules() {
        let r = RegistrationInfo::default();
        for seed in [0u32, 7, 9_999, 10_000, u32::MAX] {
            let name = suggest_name(seed);
            assert_eq!(r.check_name(&name), Ok(name.clone()), "{name}");
        }
        assert_eq!(suggest_name(42), "PLAYER0042");
        let all: Vec<u8> = (0..=255).collect();
        for chunk in all.chunks(12) {
            let p = suggest_password(chunk);
            assert!(r.check_password(&p).is_ok() || p.len() < 6, "{p}");
        }
        let p = suggest_password(&[0, 31, 32, 255, 1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(p.len(), 12);
        assert!(r.check_password(&p).is_ok());
        assert_eq!(&p[..4], "A9A9");
        // Every symbol is distinct, so a byte picks each with the same odds.
        let mut sorted = PASSWORD_ALPHABET.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 32);
    }

    /// The whole flow over real sockets on loopback, against a fake Vanillish data host built on
    /// this crate's own HTTP server: the probe, the profile, the token, the manifest and the
    /// download, the sign-up and its replay — what `inilla::connect` drives on the phone.
    mod over_the_wire {
        use super::*;
        use crate::data::{self as kd, Manifest};
        use crate::http::{self, Request};
        use std::collections::HashMap;
        use std::io::Write;
        use std::net::{TcpListener, TcpStream};
        use std::path::{Path, PathBuf};
        use std::sync::{Arc, Mutex};
        use std::time::Duration;

        const TOKEN: &str = "s3cret";

        /// The fake's `server.json`: a realm in a container (its own address is not the one the
        /// player reaches), the data and sign-up behind the token, and a field from the future.
        const FAKE_DOC: &str = r#"{"format":1,"name":"Fake","software":"vanillish-datahost test",
            "core":"vmangos","build":5875,"auth":{"port":3724},
            "realms":[{"name":"Fake","kind":"pvp",
                       "world":{"address":"172.18.0.3","port":8085,"same_host":true}}],
            "data":{"url":"","token":true,"default":"enUS",
                    "locales":[{"locale":"enUS","path":"","bytes":0}]},
            "registration":{"open":true,"url":"accounts","token":true,"credentials":"srp6"},
            "unknown":[1,2,3]}"#;

        /// A std-only HTTP/1.1 client (`Connection: close`), so the test needs no client crate.
        struct Client {
            token: Option<String>,
        }

        fn split_url(url: &str) -> (String, String) {
            let rest = url.strip_prefix("http://").expect("http only");
            let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
            (authority.to_string(), format!("/{path}"))
        }

        impl Client {
            fn send(
                &self,
                method: &str,
                url: &str,
                extra: &str,
                body: &[u8],
            ) -> Result<(u16, Vec<u8>), String> {
                let (authority, path) = split_url(url);
                let mut s = TcpStream::connect(&authority)
                    .map_err(|e| format!("could not reach {url}: {e}"))?;
                s.set_read_timeout(Some(Duration::from_secs(10))).ok();
                let auth = self
                    .token
                    .as_ref()
                    .map(|t| format!("Authorization: Bearer {t}\r\n"))
                    .unwrap_or_default();
                write!(
                    s,
                    "{method} {path} HTTP/1.1\r\nHost: {authority}\r\n{auth}{extra}Content-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .map_err(|e| e.to_string())?;
                s.write_all(body).map_err(|e| e.to_string())?;
                let mut all = Vec::new();
                s.read_to_end(&mut all).map_err(|e| e.to_string())?;
                let head_end = all
                    .windows(4)
                    .position(|w| w == b"\r\n\r\n")
                    .ok_or("no head")?;
                let head = String::from_utf8_lossy(&all[..head_end]).to_string();
                let status = head
                    .split_whitespace()
                    .nth(1)
                    .and_then(|s| s.parse().ok())
                    .ok_or("no status")?;
                Ok((status, all[head_end + 4..].to_vec()))
            }
        }

        impl Fetch for Client {
            fn text(&self, url: &str) -> Result<String, String> {
                match self.send("GET", url, "", b"")? {
                    (200, body) => String::from_utf8(body).map_err(|e| e.to_string()),
                    (status, _) => Err(format!("the server answered {status}")),
                }
            }
            fn body(&self, url: &str, offset: u64) -> Result<(Box<dyn Read + Send>, bool), String> {
                let range = if offset > 0 {
                    format!("Range: bytes={offset}-\r\n")
                } else {
                    String::new()
                };
                match self.send("GET", url, &range, b"")? {
                    (status @ (200 | 206), body) => {
                        Ok((Box::new(std::io::Cursor::new(body)), status == 206))
                    }
                    (status, _) => Err(format!("the server answered {status}")),
                }
            }
        }

        impl Post for Client {
            fn post_json(&self, url: &str, json: &str) -> Result<(u16, String), String> {
                let (status, body) = self.send(
                    "POST",
                    url,
                    "Content-Type: application/json\r\n",
                    json.as_bytes(),
                )?;
                Ok((status, String::from_utf8_lossy(&body).into_owned()))
            }
        }

        fn tmp(name: &str) -> PathBuf {
            let d = std::env::temp_dir()
                .join(format!("vanillish-connect-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            d
        }

        /// The fake data host: `server.json` open, the rest behind the token, sign-up as the
        /// contract says (a stored salt per name: the same one again is a replay).
        fn serve(files: PathBuf, manifest: Manifest) -> u16 {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let accounts: Arc<Mutex<HashMap<String, String>>> = Arc::default();
            let manifest = serde_json::to_string(&manifest).unwrap();
            std::thread::spawn(move || {
                http::serve(listener, move |req: Request, out: &mut TcpStream| {
                    let authorized = req.token().is_some_and(|t| http::token_eq(t, TOKEN));
                    let _ = match (req.method.as_str(), req.path.as_str()) {
                        ("GET", "/server.json") => http::respond_json(out, 200, FAKE_DOC),
                        ("GET", "/") => http::respond_text(out, 200, "inilla-datahost (fake)"),
                        (_, _) if !authorized => http::respond_json(
                            out,
                            401,
                            r#"{"ok":false,"error":"token_required","message":"The token is needed."}"#,
                        ),
                        ("GET", "/manifest.json") => http::respond_json(out, 200, &manifest),
                        ("GET", p) if p.starts_with("/files/") => {
                            let name = p.trim_start_matches("/files/");
                            let bytes = std::fs::read(files.join(name)).unwrap_or_default();
                            let range = req
                                .header("range")
                                .and_then(|r| http::parse_range(r, bytes.len() as u64));
                            match range {
                                Some((a, b)) => {
                                    let part = &bytes[a as usize..=b as usize];
                                    let extra = [(
                                        "Content-Range",
                                        format!("bytes {a}-{b}/{}", bytes.len()),
                                    )];
                                    http::write_head(
                                        out,
                                        &http::Head {
                                            status: 206,
                                            content_type: "application/octet-stream",
                                            length: Some(part.len() as u64),
                                            extra: &extra,
                                        },
                                    )
                                    .and_then(|_| out.write_all(part))
                                }
                                None => http::respond(out, 200, "application/octet-stream", &bytes),
                            }
                        }
                        ("POST", "/accounts") => {
                            if req.header("content-type") != Some("application/json") {
                                http::respond_json(
                                    out,
                                    415,
                                    r#"{"ok":false,"error":"unsupported_media_type","message":"JSON only."}"#,
                                )
                            } else {
                                let v: serde_json::Value =
                                    serde_json::from_slice(&req.body).unwrap_or_default();
                                let name = v["username"]
                                    .as_str()
                                    .unwrap_or_default()
                                    .to_ascii_uppercase();
                                let salt = v["salt"].as_str().unwrap_or_default().to_string();
                                let mut all = accounts.lock().unwrap();
                                match all.get(&name) {
                                    Some(s) if *s == salt => http::respond_json(
                                        out,
                                        200,
                                        &format!(
                                            r#"{{"ok":true,"username":"{name}","message":"Already created."}}"#
                                        ),
                                    ),
                                    Some(_) => http::respond_json(
                                        out,
                                        409,
                                        r#"{"ok":false,"error":"name_taken","message":"Taken."}"#,
                                    ),
                                    None => {
                                        all.insert(name.clone(), salt);
                                        http::respond_json(
                                            out,
                                            201,
                                            &format!(
                                                r#"{{"ok":true,"username":"{name}","message":"Created."}}"#
                                            ),
                                        )
                                    }
                                }
                            }
                        }
                        _ => http::respond_text(out, 404, "not found"),
                    };
                });
            });
            port
        }

        fn fill(dir: &Path) {
            for n in kd::REQUIRED {
                std::fs::write(dir.join(n), format!("mpq:{n}:{}", "x".repeat(300))).unwrap();
            }
        }

        #[test]
        fn a_fake_server_over_loopback_takes_the_whole_flow() {
            let src = tmp("src");
            fill(&src);
            let manifest = Manifest::build_from(&src, Some("enUS".into()), |_, _, _| {}).unwrap();
            let port = serve(src, manifest);
            let open = Client { token: None };
            let with_token = Client {
                token: Some(TOKEN.into()),
            };

            // Connect: the address typed, the server's description, the profile.
            let a = addr(&format!("127.0.0.1:{port}"));
            let Probe::Described { info, base } = probe(&open, &a, &|_, _| false) else {
                panic!("not described");
            };
            assert_eq!(base, format!("http://127.0.0.1:{port}/"));
            let mut l = Launcher::default();
            let id = adopt(&mut l, info.profile(&a));
            let p = l.current_server().unwrap();
            assert_eq!(
                (p.id.as_str(), p.host.as_str(), p.port),
                (id.as_str(), "127.0.0.1", 3724)
            );
            assert_eq!(p.realm_name.as_deref(), Some("Fake"));
            assert_eq!(
                p.realm_override.as_deref(),
                Some("127.0.0.1"),
                "the container's address is overridden"
            );
            assert_eq!(p.world_address("172.18.0.3:8085"), "127.0.0.1:8085");
            assert_eq!(info.realms[0].kind_label(), Some("PvP"));

            // The game data: the token first, then the manifest and the files, resumable.
            assert_eq!(
                data_step(&info, &base, false, None, false),
                DataStep::AskToken
            );
            let DataStep::Fetch { url, locale } = data_step(&info, &base, false, None, true) else {
                panic!("no fetch");
            };
            assert_eq!(locale, None);
            assert!(
                kd::fetch_manifest(&open, &url).is_err(),
                "the manifest is behind the token"
            );
            let m = kd::fetch_manifest(&with_token, &url).unwrap();
            let dst = tmp("dst");
            // A file cut short by an earlier attempt resumes with a range request.
            std::fs::write(kd::part_path(&dst, &m.files[0].name), &b"mpq:"[..]).unwrap();
            kd::download(&with_token, &url, &m, &dst, |_| {}, &|| true).unwrap();
            assert!(kd::status(&dst).complete());
            assert!(kd::verify(&m, &dst, |_, _, _| {}).unwrap().is_empty());
            assert_eq!(data_step(&info, &base, true, None, true), DataStep::Have);

            // The account: refused without the token, created with it, the lost answer replayed,
            // another salt for the same name refused, a JSON body or nothing.
            let rules = info.sign_up_rules().unwrap();
            let url = info.register_url(&base).unwrap();
            assert_eq!(url, format!("http://127.0.0.1:{port}/accounts"));
            let up = sign_up(rules, "alice", "s3cret!", &fake_srp).unwrap();
            assert_eq!(
                register(&open, &url, &up),
                Err(RegisterError::TokenRequired)
            );
            assert_eq!(register(&with_token, &url, &up), Ok("ALICE".into()));
            assert_eq!(
                register(&with_token, &url, &up),
                Ok("ALICE".into()),
                "the replay"
            );
            let mut other = up.clone();
            other.json = other.json.replace("\"salt\":\"80", "\"salt\":\"90");
            assert_eq!(
                register(&with_token, &url, &other),
                Err(RegisterError::NameTaken)
            );
            let (status, _) = with_token
                .send(
                    "POST",
                    &url,
                    "Content-Type: text/plain\r\n",
                    up.json.as_bytes(),
                )
                .unwrap();
            assert_eq!(status, 415);

            // The connection is refused where nothing listens; a data host from before
            // `server.json` is used for its data alone.
            let closed = TcpListener::bind("127.0.0.1:0").unwrap();
            let gone = closed.local_addr().unwrap().port();
            drop(closed);
            let nothing = probe(&open, &addr(&format!("127.0.0.1:{gone}")), &|_, _| false);
            assert!(matches!(nothing, Probe::Nothing { .. }), "{nothing:?}");
            let only_data = Net(BTreeMap::from([(base.clone(), open.text(&base).unwrap())]));
            assert_eq!(
                probe(&only_data, &a, &|_, _| false),
                Probe::DataOnly { base }
            );
        }

        /// A fake server to rehearse the Connect screen against on a desktop:
        /// `cargo test -p inilla-core serve_the_desktop_rehearsal -- --ignored --nocapture`, then
        /// `INILLA_QUICK_CONNECT=1 INILLA_TOUCH=1 INILLA_CONTAINER=/tmp/vc cargo run -p benilla`
        /// and type the address it prints. Its archives are stand-ins, so the game itself will not
        /// start on them: it rehearses the screens, the download and the sign-up.
        #[test]
        #[ignore = "serves until stopped"]
        fn serve_the_desktop_rehearsal() {
            let src = tmp("rehearsal");
            fill(&src);
            let manifest = Manifest::build_from(&src, Some("enUS".into()), |_, _, _| {}).unwrap();
            let port = serve(src, manifest);
            println!("type 127.0.0.1:{port} (access code {TOKEN})");
            loop {
                std::thread::sleep(Duration::from_secs(3600));
            }
        }
    }
}

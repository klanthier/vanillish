//! **`POST /accounts`**: a player account for the app, written straight into realmd's database.
//! 1.12 has no sign-up of its own; this is the server's.
//!
//! The phone works out the SRP6 salt and verifier itself (`benilla_srp::generate_account`, the
//! same code that later logs in), so the password never leaves it. This host checks the values and
//! inserts `username, v, s`, exactly what vmangos's own `account create` stores; realmd reads the
//! account fresh at every logon, so it works at once, before mangosd has loaded the world or the
//! game data is even here. The account is a player's: nothing here can grant a GM level.
//!
//! ```text
//! POST /accounts   Content-Type: application/json   (Authorization: Bearer <token> when the host has one)
//! {"username":"alice","salt":"<64 hex, top bit set>","verifier":"<1..64 hex, nonzero, < N>"}
//!   201 {"ok":true,"username":"ALICE","message":…}          created
//!   200 the same                                            a replay: the name is there with this salt
//!   400 invalid_name | invalid_credentials | bad_request    401 token_required
//!   403 registration_closed (off, or lan and not from a private address)
//!   409 name_taken    413 too_large    415 unsupported_media_type
//!   429 rate_limited (+ retry_after, Retry-After)   503 server_unavailable   500 internal
//! ```
//!
//! Every reply is `{"ok","username"?,"error"?,"message","retry_after"?}`. The database user
//! (`vanillish_signup`, made by db-init) may only read `account (id, username, s)`, insert
//! `account (username, v, s)`, read `realmlist (id)` and insert into `realmcharacters`. Salts,
//! verifiers and the database password are never logged.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::http::Body;

/// An account name is 3 to 16 ASCII letters or digits: realmd drops a name over 16 bytes, and
/// vmangos writes names into SQL unescaped in places.
pub const NAME_MIN: usize = 3;
pub const NAME_MAX: usize = 16;
/// The password rules the app applies (it never sends one): vmangos stores a password over 16
/// characters without upper-casing it, and such an account can never log in.
pub const PASSWORD_MIN: usize = 6;
pub const PASSWORD_MAX: usize = 16;

/// The 1.12 SRP6 modulus N, big-endian hex. A verifier is `g^x mod N`, so it is below N.
const N_HEX: &str = "894B645E89E1535BBDAD5B8B290650530801B18EBFBF5E8FAB3C82872A3E9BB7";

/// Who may sign up, or use the admin menu.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Off,
    /// Private source addresses only: RFC 1918, loopback, link-local, 100.64/10 (CGNAT,
    /// Tailscale), ULA, and their IPv4-mapped forms. Docker Desktop shows every client as its
    /// gateway, a private address: there it cannot tell the LAN from the internet.
    Lan,
    Open,
}

impl Mode {
    /// `off`, `lan` or `open` (any case); empty is `off`.
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "" | "off" => Ok(Mode::Off),
            "lan" => Ok(Mode::Lan),
            "open" => Ok(Mode::Open),
            other => Err(format!("{other:?} is not off, lan or open")),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Off => "off",
            Mode::Lan => "lan",
            Mode::Open => "open",
        }
    }
}

/// `server.json`'s `registration`: where and how to sign up, and the rules the app checks first.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Registration {
    pub open: bool,
    /// Relative to `server.json`'s base.
    pub url: &'static str,
    pub token: bool,
    /// What the body carries: the SRP6 salt and verifier.
    pub credentials: &'static str,
    pub name_min: usize,
    pub name_max: usize,
    pub password_min: usize,
    pub password_max: usize,
}

impl Registration {
    pub fn srp6(open: bool, token: bool) -> Self {
        Registration {
            open,
            url: "accounts",
            token,
            credentials: "srp6",
            name_min: NAME_MIN,
            name_max: NAME_MAX,
            password_min: PASSWORD_MIN,
            password_max: PASSWORD_MAX,
        }
    }
}

/// A private address: what a home network, a phone's hotspot or a VPN hands out.
pub fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || (a == 100 && (64..128).contains(&b))
        }
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => is_private(IpAddr::V4(v4)),
            None => {
                let first = v6.segments()[0];
                // ::1, fc00::/7 (ULA), fe80::/10 (link-local).
                v6.is_loopback() || first & 0xfe00 == 0xfc00 || first & 0xffc0 == 0xfe80
            }
        },
    }
}

/// The rate limit's key for a source: an IPv4 address itself (mapped or not), an IPv6 address
/// by its /64, the block one home or one phone is given.
pub fn source_key(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V4(_) => ip,
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => {
                let s = v6.segments();
                IpAddr::V6(Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0))
            }
        },
    }
}

/// An account as it is stored: the name upper-cased, salt and verifier upper-case big-endian hex.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub username: String,
    pub salt: String,
    pub verifier: String,
}

/// What a sign-up's values got wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Invalid {
    Name,
    Salt,
    Verifier,
}

fn is_hex(s: &str) -> bool {
    s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The checks the app makes too: the name `^[A-Za-z0-9]{3,16}$`; the salt exactly 64 hex digits
/// with the top bit set (vmangos draws it so, which keeps all 32 bytes significant for both
/// sides' hashes); the verifier 1 to 64 hex digits, nonzero and below N.
pub fn validate(username: &str, salt: &str, verifier: &str) -> Result<Account, Invalid> {
    if !(NAME_MIN..=NAME_MAX).contains(&username.len())
        || !username.bytes().all(|b| b.is_ascii_alphanumeric())
    {
        return Err(Invalid::Name);
    }
    if salt.len() != 64
        || !is_hex(salt)
        || !matches!(salt.as_bytes()[0], b'8'..=b'9' | b'a'..=b'f' | b'A'..=b'F')
    {
        return Err(Invalid::Salt);
    }
    if verifier.is_empty() || verifier.len() > 64 || !is_hex(verifier) {
        return Err(Invalid::Verifier);
    }
    let digits = verifier.trim_start_matches('0').to_ascii_uppercase();
    // N has 64 significant digits: fewer is below it, as many compares as text (upper-case hex
    // sorts as the numbers do).
    if digits.is_empty() || (digits.len() == N_HEX.len() && digits.as_str() >= N_HEX) {
        return Err(Invalid::Verifier);
    }
    Ok(Account {
        username: username.to_ascii_uppercase(),
        salt: salt.to_ascii_uppercase(),
        verifier: verifier.to_ascii_uppercase(),
    })
}

/// What the database made of an insert.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Created,
    /// The name is there with this very salt: the same request again (a retry after a lost
    /// answer), not someone else's account.
    Replayed,
    Taken,
}

/// Why the database did not answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DbError {
    /// Not reachable, or not ready (db-init has not made the user or the tables yet): try later.
    Unavailable(String),
    Failed(String),
}

/// Where accounts go.
pub trait Backend: Send + Sync {
    fn create(&self, account: &Account) -> Result<Outcome, DbError>;
    /// `user@host:port/database`, for the startup line.
    fn describe(&self) -> String;
}

/// The windows sign-up attempts are counted in, for the attempts that reach the database (a
/// malformed or refused request never does; a taken name does): per source 5 in 10 minutes and
/// 20 a day, everyone together 30 an hour. On Docker Desktop every client is the same source.
const PER_SOURCE: [(usize, Duration); 2] = [
    (5, Duration::from_secs(10 * 60)),
    (20, Duration::from_secs(24 * 3600)),
];
const EVERYONE: (usize, Duration) = (30, Duration::from_secs(3600));

/// Sliding windows over the attempts of the last day.
#[derive(Default)]
pub struct Limiter {
    sources: HashMap<IpAddr, Vec<Instant>>,
    all: Vec<Instant>,
}

/// How long until fewer than `n` of `times` (oldest first) fall within `window` of `now`.
pub fn wait_for(times: &[Instant], n: usize, window: Duration, now: Instant) -> Duration {
    let recent: Vec<&Instant> = times
        .iter()
        .filter(|t| now.saturating_duration_since(**t) < window)
        .collect();
    if recent.len() < n {
        return Duration::ZERO;
    }
    window.saturating_sub(now.saturating_duration_since(*recent[recent.len() - n]))
}

impl Limiter {
    /// Count one attempt from `source` (a [`source_key`]) at `now`, or say in how many seconds
    /// one is allowed.
    pub fn take(&mut self, source: IpAddr, now: Instant) -> Result<(), u64> {
        let day = PER_SOURCE[1].1;
        self.all
            .retain(|t| now.saturating_duration_since(*t) < EVERYONE.1);
        self.sources.retain(|_, times| {
            times.retain(|t| now.saturating_duration_since(*t) < day);
            !times.is_empty()
        });
        let mine = self.sources.get(&source).map_or(&[][..], Vec::as_slice);
        let wait = PER_SOURCE
            .iter()
            .map(|&(n, window)| wait_for(mine, n, window, now))
            .chain([wait_for(&self.all, EVERYONE.0, EVERYONE.1, now)])
            .max()
            .unwrap_or_default();
        if !wait.is_zero() {
            return Err((wait.as_secs() + u64::from(wait.subsec_nanos() > 0)).max(1));
        }
        self.sources.entry(source).or_default().push(now);
        self.all.push(now);
        Ok(())
    }

    /// Take back the attempt [`Limiter::take`] counted at `at`: the database never saw it.
    pub fn give_back(&mut self, source: IpAddr, at: Instant) {
        if let Some(times) = self.sources.get_mut(&source) {
            if let Some(i) = times.iter().rposition(|t| *t == at) {
                times.remove(i);
            }
        }
        if let Some(i) = self.all.iter().rposition(|t| *t == at) {
            self.all.remove(i);
        }
    }
}

/// An answer: the status, the JSON body, a `Retry-After`, and a line for the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub body: String,
    pub retry_after: Option<u64>,
    /// What happened, for the host's log: the name and the outcome, never a secret.
    pub note: String,
}

#[derive(Serialize)]
struct Wire<'a> {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    username: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a str>,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_after: Option<u64>,
}

impl Reply {
    fn done(status: u16, username: &str, message: &str, note: &str) -> Self {
        let wire = Wire {
            ok: true,
            username: Some(username),
            error: None,
            message,
            retry_after: None,
        };
        Reply {
            status,
            body: serde_json::to_string(&wire).unwrap_or_default(),
            retry_after: None,
            note: format!("{username} {note}"),
        }
    }

    /// A refusal: `error` is the contract's code, `message` a sentence the app may show.
    pub fn refuse(status: u16, error: &str, message: &str) -> Self {
        let wire = Wire {
            ok: false,
            username: None,
            error: Some(error),
            message,
            retry_after: None,
        };
        Reply {
            status,
            body: serde_json::to_string(&wire).unwrap_or_default(),
            retry_after: None,
            note: error.to_string(),
        }
    }

    fn rate_limited(secs: u64) -> Self {
        let minutes = secs.div_ceil(60);
        let when = match minutes {
            0 | 1 => "in a minute".to_string(),
            2..=90 => format!("in {minutes} minutes"),
            _ => format!("in {} hours", minutes.div_ceil(60)),
        };
        let message = format!("Too many new accounts from here: try again {when}.");
        let wire = Wire {
            ok: false,
            username: None,
            error: Some("rate_limited"),
            message: &message,
            retry_after: Some(secs),
        };
        Reply {
            status: 429,
            body: serde_json::to_string(&wire).unwrap_or_default(),
            retry_after: Some(secs),
            note: format!("rate_limited ({secs} s)"),
        }
    }
}

impl Invalid {
    fn reply(self) -> Reply {
        match self {
            Invalid::Name => Reply::refuse(
                400,
                "invalid_name",
                "An account name is 3 to 16 letters or digits.",
            ),
            Invalid::Salt => Reply::refuse(
                400,
                "invalid_credentials",
                "The salt is not 64 hex digits with the top bit set.",
            ),
            Invalid::Verifier => Reply::refuse(
                400,
                "invalid_credentials",
                "The verifier is not 1 to 64 hex digits, nonzero and below N.",
            ),
        }
    }
}

/// One `POST /accounts`, as the HTTP side saw it.
pub struct Request<'a> {
    pub peer: Option<IpAddr>,
    /// The host has no token, or this request gave it.
    pub authorized: bool,
    pub content_type: Option<&'a str>,
    pub body: &'a Body,
}

#[derive(Deserialize)]
struct Ask {
    username: String,
    salt: String,
    verifier: String,
}

/// Sign-up: who may, where accounts go, and how many.
pub struct Signup {
    mode: Mode,
    /// The host has a token, so sign-up asks for it too.
    token: bool,
    backend: Option<Box<dyn Backend>>,
    limiter: Mutex<Limiter>,
}

impl Signup {
    pub fn off() -> Self {
        Signup {
            mode: Mode::Off,
            token: false,
            backend: None,
            limiter: Mutex::default(),
        }
    }

    pub fn new(mode: Mode, token: bool, backend: Box<dyn Backend>) -> Self {
        Signup {
            mode,
            token,
            backend: (mode != Mode::Off).then_some(backend),
            limiter: Mutex::default(),
        }
    }

    /// `server.json`'s `registration` for a requester `private` or not: absent with sign-up off,
    /// `open: false` for a public address when only the LAN may sign up.
    pub fn registration(&self, private: bool) -> Option<Registration> {
        match self.mode {
            Mode::Off => None,
            Mode::Lan => Some(Registration::srp6(private, self.token)),
            Mode::Open => Some(Registration::srp6(true, self.token)),
        }
    }

    /// The posture, for the startup log.
    pub fn describe(&self) -> String {
        let (Some(backend), false) = (&self.backend, self.mode == Mode::Off) else {
            return "sign-up off (INILLA_SIGNUP=lan or open turns it on)".into();
        };
        let who = match self.mode {
            Mode::Lan => "on, from private addresses only (lan)",
            _ => "OPEN to every address that reaches this port",
        };
        let token = match (self.token, self.mode) {
            (true, _) => "the token required",
            (false, Mode::Open) => {
                "no token asked: set INILLA_DATA_TOKEN before this port is reachable from the internet"
            }
            (false, _) => "no token asked",
        };
        format!(
            "sign-up {who}, {token}; accounts go to {}; at most {} per address in 10 minutes, {} \
             a day, {} an hour in all",
            backend.describe(),
            PER_SOURCE[0].0,
            PER_SOURCE[1].0,
            EVERYONE.0
        )
    }

    /// Answer one `POST /accounts`. Refusals come first, in an order that tells a stranger
    /// nothing: closed, then the token, then the request's form; only then the database, and
    /// only an attempt that reaches it counts against the limits.
    pub fn handle(&self, req: &Request<'_>, now: Instant) -> Reply {
        let Some(backend) = self.backend.as_deref().filter(|_| self.mode != Mode::Off) else {
            return Reply::refuse(
                403,
                "registration_closed",
                "This server does not take sign-ups.",
            );
        };
        if self.mode == Mode::Lan && !req.peer.is_some_and(is_private) {
            return Reply::refuse(
                403,
                "registration_closed",
                "This server takes sign-ups from its own network only.",
            );
        }
        if !req.authorized {
            return Reply::refuse(
                401,
                "token_required",
                "Sign-up needs this server's access code.",
            );
        }
        let body = match req.body {
            Body::Read(bytes) => bytes,
            Body::TooLarge(_) => {
                return Reply::refuse(413, "too_large", "The request is too large.");
            }
            Body::None | Body::Unreadable => {
                return Reply::refuse(
                    400,
                    "bad_request",
                    "Send the account as a JSON body with a Content-Length.",
                );
            }
        };
        // A JSON content type keeps a web page from posting here behind the player's back: a
        // browser has to ask first (CORS), and this host never says yes.
        let media = req
            .content_type
            .and_then(|c| c.split(';').next())
            .map(str::trim);
        if !media.is_some_and(|m| m.eq_ignore_ascii_case("application/json")) {
            return Reply::refuse(
                415,
                "unsupported_media_type",
                "Send JSON (Content-Type: application/json).",
            );
        }
        let Ok(ask) = serde_json::from_slice::<Ask>(body) else {
            return Reply::refuse(
                400,
                "bad_request",
                "Send {\"username\", \"salt\", \"verifier\"} as JSON strings.",
            );
        };
        let account = match validate(&ask.username, &ask.salt, &ask.verifier) {
            Ok(a) => a,
            Err(invalid) => return invalid.reply(),
        };
        let source = source_key(req.peer.unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED)));
        let limited = self
            .limiter
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take(source, now);
        if let Err(secs) = limited {
            return Reply::rate_limited(secs);
        }
        let name = account.username.as_str();
        match backend.create(&account) {
            Ok(Outcome::Created) => {
                Reply::done(201, name, &format!("Account {name} created."), "created")
            }
            Ok(Outcome::Replayed) => Reply::done(
                200,
                name,
                &format!("Account {name} was already created by this request."),
                "replayed",
            ),
            Ok(Outcome::Taken) => {
                let mut r = Reply::refuse(409, "name_taken", "That account name is taken.");
                r.note = format!("{name} taken");
                r
            }
            Err(DbError::Unavailable(e)) => {
                self.limiter
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .give_back(source, now);
                eprintln!("inilla-datahost: sign-up {name}: the database is not ready: {e}");
                Reply::refuse(
                    503,
                    "server_unavailable",
                    "The server is still starting. Try again in a minute.",
                )
            }
            Err(DbError::Failed(e)) => {
                eprintln!("inilla-datahost: sign-up {name}: {e}");
                Reply::refuse(
                    500,
                    "internal",
                    "The server could not create the account. Its owner will find why in the \
                     data host's log.",
                )
            }
        }
    }
}

/// How to reach realmd's database as the sign-up user.
#[derive(Clone)]
pub struct DbConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub name: String,
}

/// realmd's database, through a connection per sign-up: they are rare, and a fresh connection
/// outlives a restarted database.
pub struct Database {
    opts: mysql::Opts,
    describe: String,
}

/// MySQL's "duplicate entry" (here: `idx_username`, which compares case-insensitively).
const ER_DUP_ENTRY: u16 = 1062;

/// Server errors that mean "not ready yet, or busy": too many connections, no such user or
/// database yet (db-init has not run; 1130 when no user at all may connect from this host yet),
/// shutting down, no such table yet, lock timeout, deadlock, connection killed.
const NOT_READY: &[u16] = &[1040, 1044, 1045, 1049, 1053, 1130, 1146, 1205, 1213, 1927];

const REALMCHARACTERS: &str = "INSERT IGNORE INTO realmcharacters (realmid, acctid, numchars) \
                               SELECT id, LAST_INSERT_ID(), 0 FROM realmlist";

fn classify(e: mysql::Error) -> DbError {
    use mysql::{DriverError as D, Error as E};
    let not_ready = match &e {
        E::IoError(_) | E::CodecError(_) => true,
        E::DriverError(d) => matches!(
            d,
            D::ConnectTimeout | D::CouldNotConnect(_) | D::Timeout | D::PacketOutOfSync
        ),
        E::MySqlError(m) => NOT_READY.contains(&m.code),
        _ => false,
    };
    // The statements carry their values as parameters, so no message holds a salt or verifier.
    if not_ready {
        DbError::Unavailable(e.to_string())
    } else {
        DbError::Failed(e.to_string())
    }
}

impl Database {
    pub fn new(c: &DbConfig) -> Self {
        let opts = mysql::OptsBuilder::new()
            .ip_or_hostname(Some(c.host.clone()))
            .tcp_port(c.port)
            .user(Some(c.user.clone()))
            .pass(Some(c.password.clone()))
            .db_name(Some(c.name.clone()))
            .prefer_socket(false)
            // Short: a database that is not up is a 503 now, not a phone waiting.
            .tcp_connect_timeout(Some(Duration::from_secs(3)))
            .read_timeout(Some(Duration::from_secs(10)))
            .write_timeout(Some(Duration::from_secs(10)));
        Database {
            opts: opts.into(),
            describe: format!("{}@{}:{}/{}", c.user, c.host, c.port, c.name),
        }
    }
}

impl Backend for Database {
    fn create(&self, a: &Account) -> Result<Outcome, DbError> {
        use mysql::prelude::Queryable;
        let mut c = mysql::Conn::new(self.opts.clone()).map_err(classify)?;
        // vmangos's tables rely on non-strict SQL (`last_login` defaults to a zero date), and
        // a strict session refuses the insert: set it here, whatever the server's global is.
        c.query_drop("SET SESSION sql_mode = 'NO_ENGINE_SUBSTITUTION'")
            .map_err(classify)?;
        match c.exec_drop(
            "INSERT INTO account (username, v, s) VALUES (?, ?, ?)",
            (&a.username, &a.verifier, &a.salt),
        ) {
            Ok(()) => {
                // Optional: realmd counts a missing row as no characters, and mangosd writes it
                // at the first character. The account stands either way.
                if let Err(e) = c.query_drop(REALMCHARACTERS) {
                    eprintln!(
                        "inilla-datahost: sign-up {}: created, without realmcharacters rows ({e})",
                        a.username
                    );
                }
                Ok(Outcome::Created)
            }
            Err(mysql::Error::MySqlError(e)) if e.code == ER_DUP_ENTRY => {
                let stored = c
                    .exec_first_opt::<Option<Vec<u8>>, _, _>(
                        "SELECT s FROM account WHERE username = ?",
                        (&a.username,),
                    )
                    .map_err(classify)?;
                Ok(match stored {
                    Some(Ok(Some(s))) if s.eq_ignore_ascii_case(a.salt.as_bytes()) => {
                        Outcome::Replayed
                    }
                    _ => Outcome::Taken,
                })
            }
            Err(e) => Err(classify(e)),
        }
    }

    fn describe(&self) -> String {
        self.describe.clone()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;

    /// N - 1, the largest valid verifier.
    const N_MINUS_1: &str = "894B645E89E1535BBDAD5B8B290650530801B18EBFBF5E8FAB3C82872A3E9BB6";
    pub(crate) const SALT: &str =
        "C0FFEE00112233445566778899AABBCCDDEEFF00112233445566778899AABBCC";
    pub(crate) const VERIFIER: &str =
        "0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF";

    pub(crate) fn body(username: &str, salt: &str) -> Body {
        Body::Read(
            format!(r#"{{"username":"{username}","salt":"{salt}","verifier":"{VERIFIER}"}}"#)
                .into_bytes(),
        )
    }

    /// realmd's `account` table in memory: the name compared case-insensitively, as its `_ci`
    /// collation does.
    #[derive(Clone, Default)]
    pub(crate) struct Fake {
        pub accounts: Arc<Mutex<HashMap<String, String>>>,
        pub calls: Arc<AtomicUsize>,
        pub down: Arc<AtomicBool>,
        pub broken: Arc<AtomicBool>,
    }

    impl Backend for Fake {
        fn create(&self, a: &Account) -> Result<Outcome, DbError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.down.load(Ordering::SeqCst) {
                return Err(DbError::Unavailable("connection refused".into()));
            }
            if self.broken.load(Ordering::SeqCst) {
                return Err(DbError::Failed("ERROR 1142: denied".into()));
            }
            let mut accounts = self.accounts.lock().unwrap();
            Ok(match accounts.get(&a.username.to_ascii_uppercase()) {
                Some(s) if s.eq_ignore_ascii_case(&a.salt) => Outcome::Replayed,
                Some(_) => Outcome::Taken,
                None => {
                    accounts.insert(a.username.to_ascii_uppercase(), a.salt.clone());
                    Outcome::Created
                }
            })
        }

        fn describe(&self) -> String {
            "fake".into()
        }
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn post<'a>(peer: &str, body: &'a Body) -> Request<'a> {
        Request {
            peer: Some(ip(peer)),
            authorized: true,
            content_type: Some("application/json"),
            body,
        }
    }

    #[test]
    fn names_follow_the_logon_rules() {
        for good in ["abc", "Alice", "a1B2c3D4e5F6g7H8", "007"] {
            assert_eq!(
                validate(good, SALT, VERIFIER).unwrap().username,
                good.to_ascii_uppercase()
            );
        }
        for bad in [
            "",
            "ab",
            "a1B2c3D4e5F6g7H8x",
            "al ice",
            "ali-ce",
            "alïce",
            " alice",
            "x'--",
        ] {
            assert_eq!(validate(bad, SALT, VERIFIER), Err(Invalid::Name), "{bad:?}");
        }
    }

    #[test]
    fn salts_are_64_hex_digits_with_the_top_bit_set() {
        let lower = SALT.to_ascii_lowercase();
        assert_eq!(validate("alice", &lower, VERIFIER).unwrap().salt, SALT);
        for first in ["8", "9", "a", "F"] {
            let s = format!("{first}{}", &SALT[1..]);
            assert!(validate("alice", &s, VERIFIER).is_ok(), "{s}");
        }
        for bad in [
            format!("7{}", &SALT[1..]),
            format!("0{}", &SALT[1..]),
            SALT[1..].to_string(),
            format!("{SALT}0"),
            format!("G{}", &SALT[1..]),
            format!("{}g", &SALT[..63]),
            String::new(),
        ] {
            assert_eq!(
                validate("alice", &bad, VERIFIER),
                Err(Invalid::Salt),
                "{bad}"
            );
        }
    }

    #[test]
    fn verifiers_are_nonzero_and_below_n() {
        for good in [
            "1",
            "0001",
            VERIFIER,
            N_MINUS_1,
            &N_MINUS_1.to_ascii_lowercase(),
            "7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF",
        ] {
            let a = validate("alice", SALT, good).unwrap();
            assert_eq!(a.verifier, good.to_ascii_uppercase(), "stored as sent");
        }
        for bad in [
            "",
            "0",
            "0000",
            N_HEX,
            "894B645E89E1535BBDAD5B8B290650530801B18EBFBF5E8FAB3C82872A3E9BB8",
            "FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF",
            "00894B645E89E1535BBDAD5B8B290650530801B18EBFBF5E8FAB3C82872A3E9BB6",
            "12 34",
            "-1",
            "0x12",
        ] {
            assert_eq!(
                validate("alice", SALT, bad),
                Err(Invalid::Verifier),
                "{bad}"
            );
        }
    }

    #[test]
    fn private_sources() {
        for good in [
            "10.0.0.2",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.65.1",
            "127.0.0.1",
            "169.254.1.2",
            "100.64.0.1",
            "100.127.255.255",
            "::1",
            "fc00::1",
            "fd7a:115c:a1e0::1",
            "fe80::1",
            "febf::1",
            "::ffff:192.168.1.5",
            "::ffff:100.100.1.1",
        ] {
            assert!(is_private(ip(good)), "{good}");
        }
        for bad in [
            "8.8.8.8",
            "172.15.255.255",
            "172.32.0.1",
            "100.63.255.255",
            "100.128.0.1",
            "0.0.0.0",
            "255.255.255.255",
            "2001:db8::1",
            "fec0::1",
            "fe00::1",
            "::ffff:8.8.8.8",
            "::",
        ] {
            assert!(!is_private(ip(bad)), "{bad}");
        }
    }

    #[test]
    fn sources_are_an_ipv4_address_or_an_ipv6_slash_64() {
        assert_eq!(source_key(ip("::ffff:10.1.2.3")), ip("10.1.2.3"));
        assert_eq!(source_key(ip("2001:db8:1:2:aaaa::1")), ip("2001:db8:1:2::"));
        assert_ne!(source_key(ip("2001:db8:1:3::1")), ip("2001:db8:1:2::"));
    }

    #[test]
    fn the_limiter_holds_a_source_and_everyone() {
        let mut l = Limiter::default();
        let t0 = Instant::now();
        let a = ip("10.0.0.2");
        for _ in 0..5 {
            assert_eq!(l.take(a, t0), Ok(()));
        }
        assert_eq!(l.take(a, t0 + Duration::from_secs(10)), Err(590));
        assert_eq!(l.take(a, t0 + Duration::from_millis(599_500)), Err(1));
        assert_eq!(l.take(a, t0 + Duration::from_secs(600)), Ok(()));
        // A refunded attempt is as if it never happened.
        let t1 = t0 + Duration::from_secs(601);
        l.give_back(a, t0 + Duration::from_secs(600));
        assert_eq!(l.take(a, t1), Ok(()));
        // Everyone together: 30 an hour (6 so far, from a); the 31st waits for t0's to age out.
        for i in 0..24u8 {
            assert_eq!(l.take(ip(&format!("10.0.1.{i}")), t1), Ok(()));
        }
        assert_eq!(l.take(ip("10.0.2.1"), t1), Err(3600 - 601));
    }

    #[test]
    fn twenty_a_day_from_one_source() {
        let mut l = Limiter::default();
        let t0 = Instant::now();
        let a = ip("192.168.1.9");
        // One every 700 s stays under 5 in 10 minutes and 30 an hour; the 21st waits for the
        // first to leave the day.
        let quick = Duration::from_secs(700);
        for i in 0..20u32 {
            assert_eq!(l.take(a, t0 + quick * i), Ok(()), "{i}");
        }
        let now = t0 + quick * 20;
        assert_eq!(l.take(a, now), Err(86_400 - 700 * 20));
        assert_eq!(l.take(ip("192.168.1.10"), now), Ok(()), "another source");
    }

    #[test]
    fn replies_have_the_contract_shape() {
        let fake = Fake::default();
        let s = Signup::new(Mode::Lan, false, Box::new(fake.clone()));
        let now = Instant::now();
        let b = body("alice", SALT);
        let r = s.handle(&post("192.168.1.2", &b), now);
        assert_eq!(
            (r.status, r.body.as_str(), r.retry_after),
            (
                201,
                r#"{"ok":true,"username":"ALICE","message":"Account ALICE created."}"#,
                None
            )
        );
        let r = s.handle(&post("192.168.1.2", &b), now);
        assert_eq!(
            (r.status, r.body.as_str()),
            (
                200,
                r#"{"ok":true,"username":"ALICE","message":"Account ALICE was already created by this request."}"#
            )
        );
        let other = body("Alice", &format!("9{}", &SALT[1..]));
        let r = s.handle(&post("192.168.1.2", &other), now);
        assert_eq!(
            (r.status, r.body.as_str()),
            (
                409,
                r#"{"ok":false,"error":"name_taken","message":"That account name is taken."}"#
            )
        );
        assert_eq!(r.note, "ALICE taken");
        let r = Reply::rate_limited(540);
        assert_eq!(
            (r.status, r.body.as_str(), r.retry_after),
            (
                429,
                r#"{"ok":false,"error":"rate_limited","message":"Too many new accounts from here: try again in 9 minutes.","retry_after":540}"#,
                Some(540)
            )
        );
        assert!(Reply::rate_limited(30).body.contains("in a minute"));
        assert!(Reply::rate_limited(80_000).body.contains("in 23 hours"));
        let off = Signup::off().handle(&post("192.168.1.2", &b), now);
        assert_eq!(
            (off.status, off.body.as_str()),
            (
                403,
                r#"{"ok":false,"error":"registration_closed","message":"This server does not take sign-ups."}"#
            )
        );
    }

    #[test]
    fn refusals_never_reach_the_database() {
        let fake = Fake::default();
        let lan = Signup::new(Mode::Lan, true, Box::new(fake.clone()));
        let now = Instant::now();
        let good = body("bob", SALT);
        let code = |r: Reply| {
            (
                r.status,
                serde_json::from_str::<serde_json::Value>(&r.body).unwrap()["error"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
            )
        };
        let mut req = post("8.8.8.8", &good);
        assert_eq!(
            code(lan.handle(&req, now)),
            (403, "registration_closed".into())
        );
        req.peer = None;
        assert_eq!(
            code(lan.handle(&req, now)),
            (403, "registration_closed".into())
        );
        req.peer = Some(ip("::ffff:192.168.1.4"));
        req.authorized = false;
        assert_eq!(code(lan.handle(&req, now)), (401, "token_required".into()));
        for (b, want) in [
            (Body::None, (400, "bad_request")),
            (Body::Unreadable, (400, "bad_request")),
            (Body::TooLarge(5000), (413, "too_large")),
            (Body::Read(b"username=bob".to_vec()), (400, "bad_request")),
            (
                Body::Read(br#"{"username":"bob","password":"s3cret!"}"#.to_vec()),
                (400, "bad_request"),
            ),
            (
                Body::Read(br#"{"username":"bob","salt":1,"verifier":"1"}"#.to_vec()),
                (400, "bad_request"),
            ),
            (body("bo", SALT), (400, "invalid_name")),
            (body("bob", &SALT[1..]), (400, "invalid_credentials")),
            (
                Body::Read(
                    format!(r#"{{"username":"bob","salt":"{SALT}","verifier":"{N_HEX}"}}"#)
                        .into_bytes(),
                ),
                (400, "invalid_credentials"),
            ),
        ] {
            let r = Request {
                body: &b,
                ..post("10.9.8.7", &good)
            };
            assert_eq!(code(lan.handle(&r, now)), (want.0, want.1.into()), "{b:?}");
        }
        for ct in [
            None,
            Some("text/plain"),
            Some("application/jsonx"),
            Some("multipart/form-data"),
        ] {
            let r = Request {
                content_type: ct,
                ..post("10.9.8.7", &good)
            };
            assert_eq!(
                code(lan.handle(&r, now)),
                (415, "unsupported_media_type".into()),
                "{ct:?}"
            );
        }
        assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
        let r = Request {
            content_type: Some("Application/JSON; charset=utf-8"),
            ..post("10.9.8.7", &good)
        };
        assert_eq!(lan.handle(&r, now).status, 201);
        let open = Signup::new(Mode::Open, false, Box::new(fake.clone()));
        assert_eq!(
            open.handle(&post("8.8.8.8", &body("carol", SALT)), now)
                .status,
            201
        );
        assert_eq!(fake.calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn database_trouble_is_a_503_or_a_500_and_a_503_costs_no_attempt() {
        let fake = Fake::default();
        let s = Signup::new(Mode::Open, false, Box::new(fake.clone()));
        let now = Instant::now();
        fake.down.store(true, Ordering::SeqCst);
        for _ in 0..8 {
            let r = s.handle(&post("10.0.0.1", &body("dave", SALT)), now);
            assert_eq!(r.status, 503);
            assert!(r.body.contains(r#""error":"server_unavailable""#));
        }
        fake.down.store(false, Ordering::SeqCst);
        fake.broken.store(true, Ordering::SeqCst);
        let r = s.handle(&post("10.0.0.1", &body("dave", SALT)), now);
        assert_eq!(r.status, 500);
        assert!(r.body.contains(r#""error":"internal""#));
        assert!(!r.body.contains("1142"), "never the database's words");
        fake.broken.store(false, Ordering::SeqCst);
        // One attempt (the 500) counted so far: four more, then the limit.
        for name in ["dave", "erin", "frank", "grace"] {
            assert_eq!(
                s.handle(&post("10.0.0.1", &body(name, SALT)), now).status,
                201
            );
        }
        let r = s.handle(&post("10.0.0.1", &body("heidi", SALT)), now);
        assert_eq!((r.status, r.retry_after), (429, Some(600)));
        // Another source, a taken name.
        let r = s.handle(
            &post("10.0.0.2", &body("dave", "9".repeat(64).as_str())),
            now,
        );
        assert_eq!(r.status, 409);
    }

    #[test]
    fn server_json_offers_sign_up_by_mode_and_source() {
        let fake = || Box::new(Fake::default());
        assert_eq!(Signup::off().registration(true), None);
        let lan = Signup::new(Mode::Lan, true, fake());
        assert_eq!(lan.registration(true), Some(Registration::srp6(true, true)));
        assert_eq!(lan.registration(false).map(|r| r.open), Some(false));
        let open = Signup::new(Mode::Open, false, fake());
        assert_eq!(
            open.registration(false),
            Some(Registration::srp6(true, false))
        );
        assert!(open.describe().contains("OPEN"));
        assert!(Signup::off().describe().starts_with("sign-up off"));
        assert_eq!(
            Signup::new(Mode::Off, false, fake()).registration(true),
            None
        );
    }

    #[test]
    fn modes_parse() {
        assert_eq!(Mode::parse(""), Ok(Mode::Off));
        assert_eq!(Mode::parse(" LAN "), Ok(Mode::Lan));
        assert_eq!(Mode::parse("open"), Ok(Mode::Open));
        assert!(Mode::parse("yes").is_err());
    }

    /// Against a real MariaDB, made for the purpose (it creates and drops the database
    /// `vanillish_signup_test` and the user `vanillish_signup_test`, and sets the global SQL mode
    /// strict for the run):
    ///
    /// ```sh
    /// docker run -d --name dh-test-db -e MARIADB_ROOT_PASSWORD=x -p 13306:3306 mariadb:11.4
    /// DATAHOST_TEST_MYSQL=127.0.0.1:13306 DATAHOST_TEST_MYSQL_ROOT_PASSWORD=x \
    ///   cargo test --manifest-path server/datahost/Cargo.toml -- --ignored
    /// docker rm -f dh-test-db
    /// ```
    #[test]
    #[ignore = "needs a MariaDB: DATAHOST_TEST_MYSQL=host:port (see the doc comment)"]
    fn against_mariadb() {
        use mysql::prelude::Queryable;
        let addr = std::env::var("DATAHOST_TEST_MYSQL")
            .expect("DATAHOST_TEST_MYSQL=host:port names the test database server");
        let (host, port) = addr.rsplit_once(':').expect("host:port");
        let port: u16 = port.parse().unwrap();
        let root_pw = std::env::var("DATAHOST_TEST_MYSQL_ROOT_PASSWORD").unwrap_or("x".into());
        let db = "vanillish_signup_test";
        let root_opts = mysql::OptsBuilder::new()
            .ip_or_hostname(Some(host))
            .tcp_port(port)
            .user(Some("root"))
            .pass(Some(root_pw))
            .prefer_socket(false);
        let mut root = {
            // A container just started takes a few seconds to take connections.
            let mut tries = 0;
            loop {
                match mysql::Conn::new(root_opts.clone()) {
                    Ok(c) => break c,
                    Err(e) if tries < 60 => {
                        tries += 1;
                        eprintln!("waiting for the database: {e}");
                        std::thread::sleep(Duration::from_secs(1));
                    }
                    Err(e) => panic!("no database at {addr}: {e}"),
                }
            }
        };
        let saved_mode: String = root
            .query_first("SELECT @@GLOBAL.sql_mode")
            .unwrap()
            .unwrap();
        // vmangos's schema (logon.sql, db_latest), the columns sign-up touches and the ones
        // whose defaults it relies on, under a session that takes its zero-date default.
        root.query_drop("SET SESSION sql_mode = 'NO_ENGINE_SUBSTITUTION'")
            .unwrap();
        for sql in [
            format!("DROP DATABASE IF EXISTS {db}"),
            format!("CREATE DATABASE {db}"),
            format!("USE {db}"),
            "CREATE TABLE account (
                id int(11) unsigned NOT NULL AUTO_INCREMENT,
                username varchar(32) NOT NULL,
                gmlevel tinyint(3) unsigned NOT NULL DEFAULT '0',
                sessionkey longtext, v longtext, s longtext,
                token_key varchar(100) NOT NULL DEFAULT '',
                email text,
                joindate timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,
                last_ip varchar(30) NOT NULL DEFAULT '0.0.0.0',
                locked tinyint(3) unsigned NOT NULL DEFAULT '0',
                lock_country varchar(2) NOT NULL DEFAULT '00',
                last_login timestamp NOT NULL DEFAULT '0000-00-00 00:00:00',
                online tinyint(4) NOT NULL DEFAULT '0',
                expansion tinyint(3) unsigned NOT NULL DEFAULT '0',
                mutetime bigint(40) NOT NULL DEFAULT '0',
                locale tinyint(3) unsigned NOT NULL DEFAULT '0',
                os varchar(4) NOT NULL DEFAULT '',
                platform varchar(4) NOT NULL DEFAULT '',
                current_realm tinyint(3) unsigned NOT NULL DEFAULT '0',
                flags int(10) unsigned NOT NULL DEFAULT '0',
                security varchar(255) DEFAULT NULL,
                email_verif tinyint(1) NOT NULL DEFAULT '0',
                geolock_pin int(11) DEFAULT '0',
                PRIMARY KEY (id), UNIQUE KEY idx_username (username), KEY idx_gmlevel (gmlevel)
            ) ENGINE=InnoDB DEFAULT CHARSET=utf8 ROW_FORMAT=DYNAMIC"
                .into(),
            "CREATE TABLE realmlist (
                id int(11) unsigned NOT NULL AUTO_INCREMENT,
                name varchar(32) NOT NULL DEFAULT '',
                address varchar(32) NOT NULL DEFAULT '127.0.0.1',
                localAddress varchar(255) NOT NULL DEFAULT '127.0.0.1',
                localSubnetMask varchar(255) NOT NULL DEFAULT '255.255.255.0',
                port int(11) NOT NULL DEFAULT '8085',
                icon tinyint(3) unsigned NOT NULL DEFAULT '0',
                realmflags tinyint(3) unsigned NOT NULL DEFAULT '2',
                timezone tinyint(3) unsigned NOT NULL DEFAULT '0',
                allowedSecurityLevel tinyint(3) unsigned NOT NULL DEFAULT '0',
                population float unsigned NOT NULL DEFAULT '0',
                gamebuild_min int(11) unsigned NOT NULL DEFAULT '0',
                gamebuild_max int(11) unsigned NOT NULL DEFAULT '0',
                flag tinyint(3) unsigned NOT NULL DEFAULT '2',
                realmbuilds varchar(64) NOT NULL DEFAULT '',
                PRIMARY KEY (id), UNIQUE KEY idx_name (name)
            ) ENGINE=MyISAM DEFAULT CHARSET=utf8 ROW_FORMAT=DYNAMIC"
                .into(),
            "CREATE TABLE realmcharacters (
                realmid int(11) unsigned NOT NULL DEFAULT '0',
                acctid bigint(20) unsigned NOT NULL,
                numchars tinyint(3) unsigned NOT NULL DEFAULT '0',
                PRIMARY KEY (realmid, acctid), KEY acctid (acctid)
            ) ENGINE=MyISAM DEFAULT CHARSET=utf8 ROW_FORMAT=DYNAMIC"
                .into(),
            "INSERT INTO realmlist (name, address) VALUES ('Vanillish', '192.168.1.20'), ('Second', '10.0.0.2')".into(),
            // The contract's db-init grants, on this test's database and user.
            format!("DROP USER IF EXISTS '{db}'@'%'"),
            format!("CREATE USER '{db}'@'%' IDENTIFIED BY 'signup-pw'"),
            format!("GRANT SELECT (`id`, `username`, `s`), INSERT (`username`, `v`, `s`) ON `{db}`.`account` TO '{db}'@'%'"),
            format!("GRANT SELECT (`id`) ON `{db}`.`realmlist` TO '{db}'@'%'"),
            format!("GRANT INSERT ON `{db}`.`realmcharacters` TO '{db}'@'%'"),
            "FLUSH PRIVILEGES".into(),
            // Strict, with zero dates refused: what makes the insert fail without its own mode.
            "SET GLOBAL sql_mode = 'STRICT_TRANS_TABLES,NO_ZERO_DATE,NO_ZERO_IN_DATE,ERROR_FOR_DIVISION_BY_ZERO,NO_ENGINE_SUBSTITUTION'".into(),
        ] {
            root.query_drop(&sql).unwrap_or_else(|e| panic!("{sql}: {e}"));
        }
        let cfg = DbConfig {
            host: host.into(),
            port,
            user: db.into(),
            password: "signup-pw".into(),
            name: db.into(),
        };
        let database = Database::new(&cfg);
        let alice = validate("alice", SALT, VERIFIER).unwrap();

        // Created: the row as realmd reads it, and a realmcharacters row per realm.
        assert_eq!(database.create(&alice), Ok(Outcome::Created));
        let row: (u32, String, String, String, u8) = root
            .query_first(format!(
                "SELECT id, username, v, s, gmlevel FROM {db}.account WHERE username = 'ALICE'"
            ))
            .unwrap()
            .unwrap();
        assert_eq!(
            (&row.1[..], &row.2[..], &row.3[..], row.4),
            ("ALICE", VERIFIER, SALT, 0)
        );
        let chars: Vec<(u32, u64, u8)> = root
            .query(format!(
                "SELECT realmid, acctid, numchars FROM {db}.realmcharacters ORDER BY realmid"
            ))
            .unwrap();
        assert_eq!(chars, [(1, row.0 as u64, 0), (2, row.0 as u64, 0)]);
        let locked: Option<u8> = root
            .query_first(format!(
                "SELECT locked FROM {db}.account WHERE id = {}",
                row.0
            ))
            .unwrap();
        assert_eq!(locked, Some(0), "every other column at its default");

        // The same request again (any case of the name), then someone else's.
        assert_eq!(database.create(&alice), Ok(Outcome::Replayed));
        let lower = validate("Alice", &SALT.to_ascii_lowercase(), VERIFIER).unwrap();
        assert_eq!(database.create(&lower), Ok(Outcome::Replayed));
        let other = validate("aLiCe", &format!("9{}", &SALT[1..]), VERIFIER).unwrap();
        assert_eq!(database.create(&other), Ok(Outcome::Taken));
        let n: u64 = root
            .query_first(format!("SELECT COUNT(*) FROM {db}.account"))
            .unwrap()
            .unwrap();
        assert_eq!(n, 1, "nothing overwritten, nothing added");

        // Through the handler: the replies the app reads.
        let signup = Signup::new(Mode::Lan, false, Box::new(Database::new(&cfg)));
        let now = Instant::now();
        let b = body("bob42", SALT);
        assert_eq!(signup.handle(&post("192.168.1.30", &b), now).status, 201);
        assert_eq!(signup.handle(&post("192.168.1.30", &b), now).status, 200);
        assert_eq!(
            signup
                .handle(&post("192.168.1.30", &body("bob42", &"8".repeat(64))), now)
                .status,
            409
        );

        // The user can do nothing else: no update, no GM level, no verifier read back.
        let mut signup_conn = mysql::Conn::new(
            mysql::OptsBuilder::new()
                .ip_or_hostname(Some(host))
                .tcp_port(port)
                .user(Some(db))
                .pass(Some("signup-pw"))
                .db_name(Some(db))
                .prefer_socket(false),
        )
        .unwrap();
        for (sql, code) in [
            ("UPDATE account SET v = '1' WHERE username = 'ALICE'", 1142),
            ("DELETE FROM account", 1142),
            ("SELECT v FROM account", 1143),
            (
                "INSERT INTO account (username, v, s, gmlevel) VALUES ('X1', '1', '1', 3)",
                1143,
            ),
        ] {
            match signup_conn.query_drop(sql) {
                Err(mysql::Error::MySqlError(e)) => assert_eq!(e.code, code, "{sql}"),
                other => panic!("{sql}: {other:?}"),
            }
        }

        // Not reachable, not made yet, wrong password: 503s, quickly.
        let started = Instant::now();
        for broken in [
            DbConfig {
                port: 1,
                ..cfg.clone()
            },
            DbConfig {
                host: "no-such-host.invalid".into(),
                ..cfg.clone()
            },
            DbConfig {
                password: "wrong".into(),
                ..cfg.clone()
            },
            DbConfig {
                name: "no_such_db".into(),
                ..cfg.clone()
            },
        ] {
            let broken = Database::new(&broken);
            let what = broken.describe();
            match broken.create(&alice) {
                Err(DbError::Unavailable(e)) => eprintln!("{what}: {e}"),
                other => panic!("{what}: {other:?}"),
            }
        }
        assert!(started.elapsed() < Duration::from_secs(15));
        // A schema without the table yet (db-init still importing): a 503 too.
        root.query_drop(format!("DROP TABLE {db}.account")).unwrap();
        assert!(matches!(
            database.create(&alice),
            Err(DbError::Unavailable(_))
        ));

        for sql in [
            format!("SET GLOBAL sql_mode = '{saved_mode}'"),
            format!("DROP USER '{db}'@'%'"),
            format!("DROP DATABASE {db}"),
        ] {
            root.query_drop(&sql).unwrap();
        }
    }
}

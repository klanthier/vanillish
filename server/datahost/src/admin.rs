//! **`/admin/*`**: the server admin menu's door. The owner's phone changes the realm's rates and
//! gives gold through here; this host checks each request and passes it on to the world server's
//! agent (`vanillish-admin serve` in the mangosd container, at `INILLA_ADMIN_AGENT`).
//!
//! The admin code never travels from the app: it asks for a nonce and signs its request with a
//! key made from the code; this host checks the signature, then forwards with its own code,
//! normalised.
//!
//! ```text
//! GET  /admin/nonce        200 {"ok":true,"nonce":"<32 hex>","expires_in":60}   single use, 60 s
//! GET  /admin/status       → the agent's GET  /status
//! POST /admin/rates        → the agent's POST /rates          (Content-Type: application/json)
//! POST /admin/rates/reset  → the agent's POST /rates/reset
//! POST /admin/gold         → the agent's POST /gold
//!   Authorization: Vanillish-Admin <nonce>:<mac>, lower-case hex
//!   key = SHA-256(normalised code)
//!   mac = HMAC-SHA256(key, nonce "\n" METHOD "\n" route "\n" hex(SHA-256(body)))
//!   404 not_found | admin_off    403 admin_closed    405 (+ Allow)
//!   401 admin_auth | admin_nonce | admin_code    429 too_many (+ retry_after, Retry-After)
//!   413 too_large    415 json_only    400 bad_request
//!   503 world_unavailable    504 no_answer    502 admin_misconfigured    else the agent's reply
//! ```
//!
//! This host's own refusals are `{"ok":false,"error","message","retry_after"?}`. The code, the
//! header and the nonces are never logged, and a request's log line names its route from a fixed
//! list.

use std::collections::HashMap;
use std::fs;
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, TcpStream, ToSocketAddrs};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::accounts::{self, is_private, source_key, Mode};
use crate::http::{self, Body};

/// The setup's alphabet: 32 symbols, no 0, 1, I or O; it makes 16 of them, 80 bits.
const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
/// The shortest usable code from [`ALPHABET`], and from hex digits (`openssl rand -hex 10`, also
/// 80 bits): a right code always passes the wrong-code limit, so these bits are the only guard.
pub const CODE_MIN: usize = 16;
const HEX_MIN: usize = 20;

/// The code as the app, this host and the agent all read it: ASCII whitespace and `-` dropped,
/// letters upper-cased ASCII-only (`ß` must not become `SS`); `None` for any other character.
pub fn normalize(code: &str) -> Option<String> {
    let mut out = String::with_capacity(code.len());
    for c in code.chars() {
        if c.is_ascii_whitespace() || c == '-' {
            continue;
        }
        if !c.is_ascii_alphanumeric() {
            return None;
        }
        out.push(c.to_ascii_uppercase());
    }
    Some(out)
}

/// The key a configured code signs with, `SHA-256(normalised code)`, or why it leaves the menu off.
/// Usable means random, 16 or more of [`ALPHABET`] or 20 or more hex digits: a word picked by hand
/// can be guessed, and the wrong-code limit stops no guesser.
pub fn key(code: &str) -> Result<[u8; 32], &'static str> {
    if code.is_empty() {
        return Err("no ADMIN_CODE (INILLA_ADMIN_CODE) is set");
    }
    let Some(normal) = normalize(code) else {
        return Err("ADMIN_CODE holds a character other than letters, digits, dashes and spaces");
    };
    if normal.len() < CODE_MIN {
        return Err("ADMIN_CODE is shorter than 16 letters and digits");
    }
    if normal.starts_with("CHANGEME") {
        return Err("ADMIN_CODE is still the example's change-me value");
    }
    let setup = normal.bytes().all(|b| ALPHABET.contains(&b));
    let hex = normal.len() >= HEX_MIN && normal.bytes().all(|b| b.is_ascii_hexdigit());
    if !setup && !hex {
        return Err(
            "ADMIN_CODE is not a random code: the setup's (16 or more of \
                    ABCDEFGHJKLMNPQRSTUVWXYZ23456789) or 20 or more hex digits (openssl rand \
                    -hex 10)",
        );
    }
    Ok(Sha256::digest(normal.as_bytes()).into())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// HMAC-SHA256 (RFC 2104): SHA-256 hashes 64-byte blocks, and a longer key is hashed first.
pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut block = [0u8; 64];
    if key.len() > block.len() {
        block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let inner = Sha256::new()
        .chain_update(block.map(|b| b ^ 0x36))
        .chain_update(msg)
        .finalize();
    Sha256::new()
        .chain_update(block.map(|b| b ^ 0x5c))
        .chain_update(inner)
        .finalize()
        .into()
}

/// The `mac` a request must carry: over the nonce, the method, the route and the body's hash.
pub fn mac(key: &[u8; 32], nonce: &str, method: &str, route: &str, body: &[u8]) -> String {
    let msg = format!("{nonce}\n{method}\n{route}\n{}", hex(&Sha256::digest(body)));
    hex(&hmac_sha256(key, msg.as_bytes()))
}

/// `Vanillish-Admin <nonce>:<mac>` → the nonce and the mac, each lower-case hex of its length.
fn parse_auth(header: &str) -> Option<(&str, &str)> {
    let (nonce, mac) = header
        .strip_prefix("Vanillish-Admin ")?
        .trim()
        .split_once(':')?;
    let hex_of = |s: &str, n: usize| {
        s.len() == n && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    };
    (hex_of(nonce, 32) && hex_of(mac, 64)).then_some((nonce, mac))
}

/// One `/admin` route: its fixed name (for the log), its exact path (signed as the route), the
/// one method it takes, and the agent's route it forwards to (`None`: answered here).
struct Route {
    name: &'static str,
    path: &'static str,
    method: &'static str,
    agent: Option<&'static str>,
}

const ROUTES: [Route; 5] = [
    Route {
        name: "nonce",
        path: "/admin/nonce",
        method: "GET",
        agent: None,
    },
    Route {
        name: "status",
        path: "/admin/status",
        method: "GET",
        agent: Some("/status"),
    },
    Route {
        name: "rates",
        path: "/admin/rates",
        method: "POST",
        agent: Some("/rates"),
    },
    Route {
        name: "rates/reset",
        path: "/admin/rates/reset",
        method: "POST",
        agent: Some("/rates/reset"),
    },
    Route {
        name: "gold",
        path: "/admin/gold",
        method: "POST",
        agent: Some("/gold"),
    },
];

/// How long a nonce is good for.
const NONCE_LIFE: Duration = Duration::from_secs(60);

/// Bytes from the kernel's random source.
fn random(bytes: &mut [u8]) -> io::Result<()> {
    fs::File::open("/dev/urandom")?.read_exact(bytes)
}

/// A nonce's first 10 bytes, what its tag signs: 6 random ones, then the second it was made.
type Made = [u8; 10];

/// Whether a nonce `made` is good at `at` (since this host started): not from later, and younger
/// than [`NONCE_LIFE`].
fn alive(made: &Made, at: Duration) -> bool {
    let second = u32::from_be_bytes([made[6], made[7], made[8], made[9]]);
    at.checked_sub(Duration::from_secs(second.into()))
        .is_some_and(|age| age < NONCE_LIFE)
}

/// The nonces, each its own proof: 6 random bytes, the second it was made (since this host
/// started, 4 bytes big-endian) and the first 6 bytes of HMAC-SHA256(this run's secret, those 10),
/// in hex. Handing one out stores nothing, so no one's asking pushes the owner's out; a nonce is
/// kept, as spent, only once a request signed with the code used it.
struct Nonces {
    secret: [u8; 32],
    born: Instant,
    spent: Mutex<Spent>,
}

/// The nonces spent in their minute, and the latest time a spend saw (what aged out of the list
/// by then is refused as expired, whatever an older `now` says).
#[derive(Default)]
struct Spent {
    latest: Duration,
    nonces: Vec<Made>,
}

impl Nonces {
    fn new() -> io::Result<Self> {
        let mut secret = [0u8; 32];
        random(&mut secret)?;
        Ok(Nonces {
            secret,
            born: Instant::now(),
            spent: Mutex::default(),
        })
    }

    fn tag(&self, made: &[u8]) -> String {
        hex(&hmac_sha256(&self.secret, made)[..6])
    }

    /// A new nonce, made at `now`.
    fn issue(&self, now: Instant) -> io::Result<String> {
        let mut made: Made = [0; 10];
        random(&mut made[..6])?;
        let secs = now.saturating_duration_since(self.born).as_secs();
        made[6..].copy_from_slice(&u32::try_from(secs).unwrap_or(u32::MAX).to_be_bytes());
        Ok(hex(&made) + &self.tag(&made))
    }

    /// What `nonce` (hex) signs, when this host made it (its tag) and it is good at `now`.
    fn check(&self, nonce: &str, now: Instant) -> Option<Made> {
        let mut made: Made = [0; 10];
        for (i, b) in made.iter_mut().enumerate() {
            *b = u8::from_str_radix(nonce.get(2 * i..2 * i + 2)?, 16).ok()?;
        }
        let good = http::token_eq(nonce.get(20..)?, &self.tag(&made))
            && alive(&made, now.saturating_duration_since(self.born));
        good.then_some(made)
    }

    /// Spend the nonce `made`, at `now`: false when it was spent before (or aged out of the list
    /// meanwhile).
    fn spend(&self, made: Made, now: Instant) -> bool {
        let mut spent = lock(&self.spent);
        let latest = spent.latest.max(now.saturating_duration_since(self.born));
        spent.latest = latest;
        spent.nonces.retain(|m| alive(m, latest));
        if !alive(&made, latest) || spent.nonces.contains(&made) {
            return false;
        }
        spent.nonces.push(made);
        true
    }
}

/// The wrong signatures one source may send in [`FAILURE_WINDOW`] before its wrong ones get 429,
/// not 401. Advisory: it tells an honest client to check the code, and slows no guesser, since a
/// right code is always served (where every peer shares one address, a hold would lock the owner
/// out); the code's 80 bits, which [`key`] requires, are the guard.
const FAILURES: usize = 10;
const FAILURE_WINDOW: Duration = Duration::from_secs(10 * 60);

/// The wrong signatures of the last [`FAILURE_WINDOW`], per source.
#[derive(Default)]
struct Failures(HashMap<IpAddr, Vec<Instant>>);

impl Failures {
    /// Count one wrong code from `source` (a [`source_key`]) at `now`, or say in how many seconds
    /// it may try again.
    fn count(&mut self, source: IpAddr, now: Instant) -> Result<(), u64> {
        self.0.retain(|_, times| {
            times.retain(|t| now.saturating_duration_since(*t) < FAILURE_WINDOW);
            !times.is_empty()
        });
        let mine = self.0.entry(source).or_default();
        let wait = accounts::wait_for(mine, FAILURES, FAILURE_WINDOW, now);
        if !wait.is_zero() {
            return Err((wait.as_secs() + u64::from(wait.subsec_nanos() > 0)).max(1));
        }
        mine.push(now);
        Ok(())
    }
}

/// `server.json`'s `admin`: where the menu is, relative to the document, and whether it answers
/// this requester.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Section {
    pub url: &'static str,
    pub open: bool,
}

/// An answer: the status, the JSON body, the route's fixed name for the log, and the headers the
/// status needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub body: String,
    /// `nonce`, `status`, `rates`, `rates/reset`, `gold`, or `unknown`: never the path itself.
    pub route: &'static str,
    pub retry_after: Option<u64>,
    /// The one method a 405 names.
    pub allow: Option<&'static str>,
}

#[derive(Serialize)]
struct Wire<'a> {
    ok: bool,
    error: &'a str,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_after: Option<u64>,
}

#[derive(Serialize)]
struct NonceWire<'a> {
    ok: bool,
    nonce: &'a str,
    expires_in: u64,
}

impl Reply {
    fn refuse(route: &'static str, status: u16, error: &str, message: &str) -> Self {
        let wire = Wire {
            ok: false,
            error,
            message,
            retry_after: None,
        };
        Reply {
            status,
            body: serde_json::to_string(&wire).unwrap_or_default(),
            route,
            retry_after: None,
            allow: None,
        }
    }

    fn too_many(route: &'static str, secs: u64) -> Self {
        let when = match secs.div_ceil(60) {
            0 | 1 => "minute".to_string(),
            minutes => format!("{minutes} minutes"),
        };
        let message = format!(
            "Too many wrong admin codes from here: check the code. Wrong ones get this answer for \
             the next {when}."
        );
        let wire = Wire {
            ok: false,
            error: "too_many",
            message: &message,
            retry_after: Some(secs),
        };
        Reply {
            status: 429,
            body: serde_json::to_string(&wire).unwrap_or_default(),
            route,
            retry_after: Some(secs),
            allow: None,
        }
    }

    fn misconfigured(route: &'static str) -> Self {
        Reply::refuse(
            route,
            502,
            "admin_misconfigured",
            "The world server's admin agent gave an answer the data host cannot read: run \
             `docker compose up -d` on the server.",
        )
    }
}

/// What the world server's agent answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentReply {
    pub status: u16,
    pub body: Vec<u8>,
}

/// Why the agent gave no answer to pass on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentError {
    /// No connection: the world server is not running, still starting, or not there at all.
    Unreachable(String),
    /// Asked, and no whole answer: none before the deadline, or the connection ended first (the
    /// agent dies with mangosd). What was asked may still have been done.
    NoAnswer,
    /// An answer that is not HTTP, or over 64 KiB: not the agent this host was told of.
    Unreadable(String),
}

/// Where admin requests go.
pub trait Backend: Send + Sync {
    /// `method` on the agent's `route`, with `body` verbatim.
    fn forward(&self, method: &str, route: &str, body: &[u8]) -> Result<AgentReply, AgentError>;
    /// `host:port`, for the startup line.
    fn describe(&self) -> String;
}

/// How long the agent is given to take the connection, then to answer; the agent stops its own
/// work at 20 s and the app waits 30.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const DEADLINE: Duration = Duration::from_secs(25);
/// The longest answer read: a status with 200 players online is about 15 KiB.
const MAX_REPLY: usize = 64 * 1024;

/// The agent, over TCP on the compose network: a connection per request, `Connection: close`.
pub struct Agent {
    addr: String,
    /// ADMIN_CODE normalised: the agent compares normalised codes, and nothing but A-Z and 0-9
    /// reaches the header (a CR or LF in the configured value would end it).
    code: String,
    deadline: Duration,
}

impl Agent {
    /// The agent at `addr`, asked with `code` (an unusable one leaves the menu off, and the agent
    /// unasked).
    pub fn new(addr: &str, code: &str) -> Self {
        Agent {
            addr: addr.to_string(),
            code: normalize(code).unwrap_or_default(),
            deadline: DEADLINE,
        }
    }

    #[cfg(test)]
    pub fn with_deadline(self, deadline: Duration) -> Self {
        Agent { deadline, ..self }
    }

    fn connect(&self) -> io::Result<TcpStream> {
        let mut last = io::Error::new(io::ErrorKind::NotFound, "the name has no address");
        for addr in self.addr.to_socket_addrs()? {
            match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
                Ok(s) => return Ok(s),
                Err(e) => last = e,
            }
        }
        Err(last)
    }
}

/// Where the body starts and the `Content-Length` the head announces, once the whole head is in.
fn head_of(raw: &[u8]) -> Option<(usize, Option<usize>)> {
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = String::from_utf8_lossy(&raw[..end]);
    let length = head.lines().skip(1).find_map(|line| {
        let (k, v) = line.split_once(':')?;
        k.trim()
            .eq_ignore_ascii_case("content-length")
            .then(|| v.trim().parse().ok())?
    });
    Some((end + 4, length))
}

/// `HTTP/1.x <status> …` and the body after the head.
fn parse_reply(raw: &[u8], body_at: usize) -> Result<AgentReply, AgentError> {
    let head = String::from_utf8_lossy(&raw[..body_at]);
    let mut words = head.split_whitespace();
    let status = words
        .next()
        .filter(|v| v.starts_with("HTTP/1."))
        .and_then(|_| words.next()?.parse::<u16>().ok())
        .filter(|s| (200..600).contains(s))
        .ok_or_else(|| AgentError::Unreadable("not an HTTP answer".into()))?;
    Ok(AgentReply {
        status,
        body: raw[body_at..].to_vec(),
    })
}

/// The connection ended before a whole answer, the request sent: the agent stopped at work, and
/// what was asked may have been done; unless what came first was never HTTP.
fn ended(raw: &[u8]) -> AgentError {
    if b"HTTP/1.".starts_with(&raw[..raw.len().min(7)]) {
        AgentError::NoAnswer
    } else {
        AgentError::Unreadable("not an HTTP answer".into())
    }
}

/// Read one answer off `s`, the request sent: up to its `Content-Length`, else to the end, before
/// `deadline`.
fn read_reply(s: &mut TcpStream, deadline: Instant) -> Result<AgentReply, AgentError> {
    let mut raw = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        if let Some((body_at, Some(len))) = head_of(&raw) {
            let end = match body_at.checked_add(len) {
                Some(end) if end <= MAX_REPLY => end,
                _ => return Err(AgentError::Unreadable("an answer over 64 KiB".into())),
            };
            if raw.len() >= end {
                return parse_reply(&raw[..end], body_at);
            }
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(AgentError::NoAnswer);
        }
        let _ = s.set_read_timeout(Some(left));
        match s.read(&mut buf) {
            Ok(0) => {
                return match head_of(&raw) {
                    Some((body_at, None)) => parse_reply(&raw, body_at),
                    _ => Err(ended(&raw)),
                };
            }
            Ok(n) => {
                raw.extend_from_slice(&buf[..n]);
                if raw.len() > MAX_REPLY {
                    return Err(AgentError::Unreadable("an answer over 64 KiB".into()));
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                return Err(AgentError::NoAnswer);
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::ConnectionReset
                        | io::ErrorKind::ConnectionAborted
                        | io::ErrorKind::BrokenPipe
                        | io::ErrorKind::UnexpectedEof
                ) =>
            {
                return Err(ended(&raw));
            }
            Err(e) => return Err(AgentError::Unreadable(e.to_string())),
        }
    }
}

impl Backend for Agent {
    fn forward(&self, method: &str, route: &str, body: &[u8]) -> Result<AgentReply, AgentError> {
        let mut s = self
            .connect()
            .map_err(|e| AgentError::Unreachable(e.to_string()))?;
        let deadline = Instant::now() + self.deadline;
        let mut ask = format!(
            "{method} {route} HTTP/1.1\r\nHost: mangosd\r\nAuthorization: Bearer {}\r\n\
             Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            self.code,
            body.len()
        )
        .into_bytes();
        ask.extend_from_slice(body);
        let _ = s.set_write_timeout(Some(CONNECT_TIMEOUT));
        s.write_all(&ask)
            .map_err(|e| AgentError::Unreachable(e.to_string()))?;
        read_reply(&mut s, deadline)
    }

    fn describe(&self) -> String {
        self.addr.clone()
    }
}

/// One `/admin/*` request, as the HTTP side saw it.
pub struct Request<'a> {
    pub method: &'a str,
    /// Percent-decoded: matched against the routes exactly, never logged.
    pub path: &'a str,
    pub peer: Option<IpAddr>,
    pub authorization: Option<&'a str>,
    pub content_type: Option<&'a str>,
    pub body: &'a Body,
}

/// A JSON content type keeps a web page from posting here behind the owner's back: a browser has
/// to ask first (CORS), and this host never says yes.
fn is_json(content_type: Option<&str>) -> bool {
    content_type
        .and_then(|c| c.split(';').next())
        .map(str::trim)
        .is_some_and(|m| m.eq_ignore_ascii_case("application/json"))
}

/// The menu when it is on: the key requests are signed with, the agent they go to, and the
/// nonces.
struct On {
    key: [u8; 32],
    backend: Box<dyn Backend>,
    nonces: Nonces,
}

/// The admin menu: who may use it, its key, agent and nonces (or why it is off), and the wrong
/// codes counted.
pub struct Admin {
    mode: Mode,
    on: Result<On, String>,
    failures: Mutex<Failures>,
}

impl Admin {
    /// On when `mode` is not off, `code` (ADMIN_CODE as configured) is usable and there is an
    /// agent to forward to; otherwise off, and [`Admin::describe`] says why.
    pub fn new(mode: Mode, code: &str, backend: Option<Box<dyn Backend>>) -> Self {
        let on = if mode == Mode::Off {
            Err("INILLA_ADMIN=off".to_string())
        } else {
            match (key(code), backend) {
                (Err(why), _) => Err(why.to_string()),
                (Ok(_), None) => {
                    Err("no INILLA_ADMIN_AGENT (the world server's admin agent, host:port)".into())
                }
                (Ok(key), Some(backend)) => Nonces::new()
                    .map(|nonces| On {
                        key,
                        backend,
                        nonces,
                    })
                    .map_err(|e| format!("no secret for the nonces from /dev/urandom: {e}")),
            }
        };
        Admin {
            mode,
            on,
            failures: Mutex::default(),
        }
    }

    #[cfg(test)]
    pub fn off() -> Self {
        Admin::new(Mode::Off, "", None)
    }

    /// `server.json`'s `admin` for a requester `private` or not: absent with the menu off,
    /// `open: false` for a public address when only the LAN may use it.
    pub fn section(&self, private: bool) -> Option<Section> {
        self.on.as_ref().ok().map(|_| Section {
            url: "admin/",
            open: self.mode != Mode::Lan || private,
        })
    }

    /// The posture, for the startup log: never the code.
    pub fn describe(&self) -> String {
        match &self.on {
            Err(why) => format!("admin menu off: {why}"),
            Ok(on) => {
                let who = match self.mode {
                    Mode::Lan => {
                        "from private addresses only (lan; behind Docker Desktop, OrbStack or a \
                         proxy every address looks private, and the code is the only guard)"
                    }
                    _ => "OPEN to every address that reaches this port, the code its guard",
                };
                format!(
                    "admin menu on, {who}; requests go to the world server's agent at {}",
                    on.backend.describe()
                )
            }
        }
    }

    /// Answer one `/admin/*` request. The refusals come in the contract's order: the route, off,
    /// the network, the method, the header, the nonce (made here, in its minute), the signature
    /// (only a wrong one counts against the source), the nonce spent (only a signed request
    /// spends one), then the body; only then the agent.
    pub fn handle(&self, req: &Request<'_>, now: Instant) -> Reply {
        let Some(route) = ROUTES.iter().find(|r| r.path == req.path) else {
            return Reply::refuse(
                "unknown",
                404,
                "not_found",
                "There is no such admin request.",
            );
        };
        let name = route.name;
        let on = match &self.on {
            Ok(on) => on,
            Err(_) => {
                return Reply::refuse(
                    name,
                    404,
                    "admin_off",
                    "The admin menu is off on this server.",
                )
            }
        };
        if self.mode == Mode::Lan && !req.peer.is_some_and(is_private) {
            return Reply::refuse(
                name,
                403,
                "admin_closed",
                "The admin menu answers only on the server's own network.",
            );
        }
        if req.method != route.method {
            let mut r = Reply::refuse(
                name,
                405,
                "bad_request",
                &format!("Use {} here.", route.method),
            );
            r.allow = Some(route.method);
            return r;
        }
        let Some(agent_route) = route.agent else {
            return on.nonce(now);
        };
        let Some((nonce, given)) = req.authorization.and_then(parse_auth) else {
            return Reply::refuse(
                name,
                401,
                "admin_auth",
                "Sign the request with the admin code (Authorization: Vanillish-Admin \
                 <nonce>:<mac>).",
            );
        };
        let used = || {
            Reply::refuse(
                name,
                401,
                "admin_nonce",
                "That nonce is unknown, expired or used: ask for a new one.",
            )
        };
        let Some(made) = on.nonces.check(nonce, now) else {
            return used();
        };
        // A body left unread cannot be checked against the signature.
        let body = match req.body {
            Body::Read(bytes) => bytes.as_slice(),
            Body::None => &[],
            Body::TooLarge(_) => {
                return Reply::refuse(name, 413, "too_large", "The request is too large.");
            }
            Body::Unreadable => {
                return Reply::refuse(
                    name,
                    400,
                    "bad_request",
                    "Send the request as a JSON body with a Content-Length.",
                );
            }
        };
        let want = mac(&on.key, nonce, route.method, route.path, body);
        if !http::token_eq(given, &want) {
            let source = source_key(req.peer.unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED)));
            return match lock(&self.failures).count(source, now) {
                Ok(()) => Reply::refuse(name, 401, "admin_code", "The admin code is wrong."),
                Err(secs) => Reply::too_many(name, secs),
            };
        }
        if !on.nonces.spend(made, now) {
            return used();
        }
        if route.method == "POST" {
            if !is_json(req.content_type) {
                return Reply::refuse(
                    name,
                    415,
                    "json_only",
                    "Send JSON (Content-Type: application/json).",
                );
            }
            if body.is_empty() {
                return Reply::refuse(
                    name,
                    400,
                    "bad_request",
                    "Send the request as a JSON body with a Content-Length.",
                );
            }
        }
        on.forward(route, agent_route, body)
    }
}

impl On {
    /// A new nonce, for `GET /admin/nonce`.
    fn nonce(&self, now: Instant) -> Reply {
        let nonce = match self.nonces.issue(now) {
            Ok(n) => n,
            Err(e) => {
                eprintln!("inilla-datahost: admin: no nonce from /dev/urandom: {e}");
                return Reply::refuse(
                    "nonce",
                    500,
                    "internal",
                    "The data host could not make a nonce.",
                );
            }
        };
        let wire = NonceWire {
            ok: true,
            nonce: &nonce,
            expires_in: NONCE_LIFE.as_secs(),
        };
        Reply {
            status: 200,
            body: serde_json::to_string(&wire).unwrap_or_default(),
            route: "nonce",
            retry_after: None,
            allow: None,
        }
    }

    /// The agent's answer as the app reads it: its own when it is JSON, else this host's.
    fn forward(&self, route: &Route, agent_route: &str, body: &[u8]) -> Reply {
        let (name, agent) = (route.name, self.backend.describe());
        match self.backend.forward(route.method, agent_route, body) {
            Err(AgentError::Unreachable(e)) => {
                eprintln!("inilla-datahost: admin {name}: no world server agent at {agent}: {e}");
                Reply::refuse(
                    name,
                    503,
                    "world_unavailable",
                    "The world server is not running (or still starting).",
                )
            }
            Err(AgentError::NoAnswer) => {
                eprintln!(
                    "inilla-datahost: admin {name}: no answer from {agent} (none in time, or it \
                     hung up)"
                );
                Reply::refuse(
                    name,
                    504,
                    "no_answer",
                    "No answer in time: it may still have been done. Refresh before trying again.",
                )
            }
            Err(AgentError::Unreadable(e)) => {
                eprintln!("inilla-datahost: admin {name}: no readable answer from {agent}: {e}");
                Reply::misconfigured(name)
            }
            Ok(r) if r.status == 401 || r.status == 403 => {
                eprintln!(
                    "inilla-datahost: admin {name}: {agent} refused this host's ADMIN_CODE ({})",
                    r.status
                );
                Reply::refuse(
                    name,
                    502,
                    "admin_misconfigured",
                    "The data host and the world server disagree on ADMIN_CODE: run `docker \
                     compose up -d` on the server.",
                )
            }
            Ok(r) => match String::from_utf8(r.body) {
                Ok(text) if serde_json::from_str::<serde::de::IgnoredAny>(&text).is_ok() => Reply {
                    status: r.status,
                    body: text,
                    route: name,
                    retry_after: None,
                    allow: None,
                },
                _ => {
                    eprintln!(
                        "inilla-datahost: admin {name}: {agent} answered {} without JSON",
                        r.status
                    );
                    Reply::misconfigured(name)
                }
            },
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::{HashSet, VecDeque};
    use std::io::BufReader;
    use std::net::TcpListener;
    use std::sync::{mpsc, Arc};

    /// The shared test vectors' code and nonce (the app's tests assert the same headers).
    pub(crate) const CODE: &str = "ABCD-EFGH-JKLM-NPQR";
    pub(crate) const NONCE: &str = "00112233445566778899aabbccddeeff";
    pub(crate) const RATES: &str = r#"{"rates":{"Rate.XP.Kill":2},"announce":true}"#;
    pub(crate) const STATUS_HEADER: &str = "Vanillish-Admin 00112233445566778899aabbccddeeff:2bfb9cca1911d025fc88f5222621f3ae5f2e9635d4c363fdc0231980353acbf4";
    pub(crate) const RATES_HEADER: &str = "Vanillish-Admin 00112233445566778899aabbccddeeff:af86f2b02789533c33b4c3a704701551d9517aa95581ecb26739906cb3efe176";

    /// The header the app sends.
    pub(crate) fn auth_header(
        code: &str,
        nonce: &str,
        method: &str,
        route: &str,
        body: &[u8],
    ) -> String {
        let key = key(code).unwrap();
        format!(
            "Vanillish-Admin {nonce}:{}",
            mac(&key, nonce, method, route, body)
        )
    }

    /// What a fake agent does with a connection.
    pub(crate) enum Act {
        /// Answers these bytes and closes.
        Say(Vec<u8>),
        /// Answers these bytes and keeps the connection open.
        SayAndHold(Vec<u8>),
        /// Reads the request and never answers.
        Hold,
        /// Takes one byte of the request and drops the connection, the rest unread: a reset.
        Reset,
    }

    /// An agent's answer, as `http.server` writes one.
    pub(crate) fn say(status: u16, body: &str) -> Act {
        Act::Say(
            format!(
                "HTTP/1.0 {status} X\r\nServer: BaseHTTP/0.6\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\n\r\n{body}",
                body.len()
            )
            .into_bytes(),
        )
    }

    /// A world server's agent on an ephemeral port: each connection gets the next act, and every
    /// request it read goes to the receiver.
    pub(crate) fn tcp_agent(acts: Vec<Act>) -> (String, mpsc::Receiver<http::Request>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for act in acts {
                let Ok((mut s, _)) = listener.accept() else {
                    return;
                };
                if let Act::Reset = act {
                    let _ = s.read(&mut [0u8; 1]);
                    continue;
                }
                let mut r = BufReader::new(s.try_clone().unwrap());
                if let Ok(req) = http::read_request(&mut r) {
                    let _ = tx.send(req);
                }
                let (say, hold) = match act {
                    Act::Say(b) => (b, false),
                    Act::SayAndHold(b) => (b, true),
                    Act::Hold => (Vec::new(), true),
                    Act::Reset => unreachable!(),
                };
                let _ = s.write_all(&say);
                if hold {
                    // Until the host gives up and closes.
                    let _ = io::copy(&mut s, &mut io::sink());
                }
            }
        });
        (addr, rx)
    }

    /// An address nobody listens on: a connection there is refused.
    pub(crate) fn nobody() -> String {
        TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .to_string()
    }

    /// What an agent was asked: the method, the route, the body.
    type Asked = (String, String, Vec<u8>);

    /// An agent in memory: the requests it got, and its next answers (then `{"ok":true}`).
    #[derive(Clone, Default)]
    struct Fake {
        seen: Arc<Mutex<Vec<Asked>>>,
        next: Arc<Mutex<VecDeque<Result<AgentReply, AgentError>>>>,
    }

    impl Fake {
        fn then(&self, r: Result<AgentReply, AgentError>) {
            self.next.lock().unwrap().push_back(r);
        }

        fn calls(&self) -> usize {
            self.seen.lock().unwrap().len()
        }
    }

    impl Backend for Fake {
        fn forward(
            &self,
            method: &str,
            route: &str,
            body: &[u8],
        ) -> Result<AgentReply, AgentError> {
            self.seen
                .lock()
                .unwrap()
                .push((method.into(), route.into(), body.to_vec()));
            self.next
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Ok(AgentReply {
                    status: 200,
                    body: br#"{"ok":true}"#.to_vec(),
                }))
        }

        fn describe(&self) -> String {
            "fake".into()
        }
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn on(mode: Mode, fake: &Fake) -> Admin {
        Admin::new(mode, CODE, Some(Box::new(fake.clone())))
    }

    fn ask<'a>(
        method: &'a str,
        path: &'a str,
        auth: Option<&'a str>,
        body: &'a Body,
    ) -> Request<'a> {
        Request {
            method,
            path,
            peer: Some(ip("192.168.1.2")),
            authorization: auth,
            content_type: Some("application/json"),
            body,
        }
    }

    fn error(r: &Reply) -> (u16, String) {
        let v: serde_json::Value = serde_json::from_str(&r.body).unwrap();
        (r.status, v["error"].as_str().unwrap_or("").to_string())
    }

    /// A nonce handed out at `now`.
    fn fresh(admin: &Admin, now: Instant) -> String {
        let r = admin.handle(&ask("GET", "/admin/nonce", None, &Body::None), now);
        assert_eq!(r.status, 200, "{}", r.body);
        let v: serde_json::Value = serde_json::from_str(&r.body).unwrap();
        v["nonce"].as_str().unwrap().to_string()
    }

    /// A request signed with `code` over a fresh nonce.
    fn signed(
        admin: &Admin,
        now: Instant,
        code: &str,
        method: &str,
        path: &str,
        body: &[u8],
    ) -> Reply {
        let header = auth_header(code, &fresh(admin, now), method, path, body);
        let b = if body.is_empty() && method == "GET" {
            Body::None
        } else {
            Body::Read(body.to_vec())
        };
        admin.handle(&ask(method, path, Some(&header), &b), now)
    }

    #[test]
    fn codes_normalise_as_the_app_and_the_agent_read_them() {
        for same in [
            "ABCD-EFGH-JKLM-NPQR",
            "abcd-efgh-jklm-npqr",
            "ABCDEFGHJKLMNPQR",
            " abcd efgh\tJKLM-npqr\n",
            "AB-CD--EF-GH-JK-LM-NP-QR",
        ] {
            assert_eq!(
                normalize(same).as_deref(),
                Some("ABCDEFGHJKLMNPQR"),
                "{same:?}"
            );
            assert_eq!(key(same), key(CODE), "{same:?}");
        }
        assert_eq!(
            hex(&key(CODE).unwrap()),
            "7cf629bb82226bfb6356859edb341b8f36a74d8997c4fe385dbef6dc85c5c4bb"
        );
        for bad in [
            "ABCD_EFGH_JKLM_NPQR",
            "ÄBCD-EFGH-JKLM-NPQR",
            "ßBCD-EFGH-JKLM-NPQR",
            "ABCD.EFGH.JKLM.NPQR",
        ] {
            assert_eq!(normalize(bad), None, "{bad:?}");
            assert!(key(bad).unwrap_err().contains("character"), "{bad:?}");
        }
        assert!(key("").unwrap_err().contains("no ADMIN_CODE"));
        for short in ["1234", "----", "   ", "ABCD-EFGH-JKLM-NPQ"] {
            assert!(key(short).unwrap_err().contains("shorter"), "{short:?}");
        }
        for example in [
            "change-me-admin-code",
            "CHANGEME-ABCD-EFGH-JKLM",
            "change me 1234567890",
        ] {
            assert!(
                key(example).unwrap_err().contains("change-me"),
                "{example:?}"
            );
        }
        assert!(
            key("ABCD-EFGH-JKLM-NPQR-CHANGEME").is_ok(),
            "only the start counts"
        );
        // Random only: the setup's 16 symbols or more, or 20 hex digits or more.
        for random in [
            "ABCD-EFGH-JKLM-NPQR-STUV",
            "23456789-WXYZ-abcd",
            "0123456789abcdef0123",
            "01234-56789-ABCDE-F0123",
            "0123456789abcdef0123456789abcdef",
        ] {
            assert!(key(random).is_ok(), "{random:?}");
        }
        for picked in [
            "VanillishServer2024",
            "vanillishadmin2026",
            "ABCD-EFGH-JKLM-NPQO",
            "ABCD-EFGH-JKLM-NPQ1",
            "0123456789ABCDEF",
            "0123456789abcdef012",
            "ABCDEFGHJKLMNPQRSTUVWXYZ0",
        ] {
            assert!(key(picked).unwrap_err().contains("random"), "{picked:?}");
        }
    }

    #[test]
    fn hmac_follows_rfc_4231() {
        // Case 1, case 2 and case 6 (a key longer than a block).
        assert_eq!(
            hex(&hmac_sha256(&[0x0b; 20], b"Hi There")),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        assert_eq!(
            hex(&hmac_sha256(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        assert_eq!(
            hex(&hmac_sha256(
                &[0xaa; 131],
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            )),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn the_shared_test_vectors() {
        assert_eq!(
            auth_header(CODE, NONCE, "GET", "/admin/status", b""),
            STATUS_HEADER
        );
        assert_eq!(
            auth_header(CODE, NONCE, "POST", "/admin/rates", RATES.as_bytes()),
            RATES_HEADER
        );
        // And this host reads them so: it serves only nonces it made (their tag), so the
        // vectors' fixed nonce stops at the nonce check, past which this is the signature checked.
        let key = key(CODE).unwrap();
        for (header, method, route, body) in [
            (STATUS_HEADER, "GET", "/admin/status", &b""[..]),
            (RATES_HEADER, "POST", "/admin/rates", RATES.as_bytes()),
        ] {
            let (nonce, given) = parse_auth(header).unwrap();
            assert_eq!(nonce, NONCE);
            assert!(http::token_eq(
                given,
                &mac(&key, nonce, method, route, body)
            ));
        }
    }

    #[test]
    fn the_header_is_read_strictly() {
        let mac = "2bfb9cca1911d025fc88f5222621f3ae5f2e9635d4c363fdc0231980353acbf4";
        assert_eq!(parse_auth(STATUS_HEADER), Some((NONCE, mac)));
        for bad in [
            String::new(),
            format!("Bearer {NONCE}:{mac}"),
            format!("vanillish-admin {NONCE}:{mac}"),
            format!("Vanillish-Admin {NONCE}"),
            format!("Vanillish-Admin {NONCE}:{}", mac.to_ascii_uppercase()),
            format!("Vanillish-Admin {}:{mac}", &NONCE[1..]),
            format!("Vanillish-Admin {NONCE}:{mac}0"),
            format!("Vanillish-Admin {NONCE}:{}g", &mac[1..]),
            format!("Vanillish-Admin nonce={NONCE}, mac={mac}"),
        ] {
            assert_eq!(parse_auth(&bad), None, "{bad}");
        }
    }

    #[test]
    fn refusals_come_in_the_contracts_order() {
        let fake = Fake::default();
        let now = Instant::now();
        let none = Body::None;
        // The route first: anything else under /admin is not_found, on or off.
        for admin in [on(Mode::Lan, &fake), Admin::off()] {
            for path in [
                "/admin",
                "/admin/",
                "/admin/Status",
                "/admin/status/",
                "/admin/x\n",
            ] {
                let r = admin.handle(&ask("GET", path, None, &none), now);
                assert_eq!(
                    (error(&r), r.route),
                    ((404, "not_found".into()), "unknown"),
                    "{path:?}"
                );
            }
        }
        // Off: every route, whatever else is wrong.
        let weak = Admin::new(Mode::Lan, "1234", Some(Box::new(fake.clone())));
        for admin in [
            Admin::off(),
            weak,
            Admin::new(Mode::Off, CODE, Some(Box::new(fake.clone()))),
        ] {
            for (method, path) in [
                ("GET", "/admin/nonce"),
                ("GET", "/admin/status"),
                ("PUT", "/admin/gold"),
            ] {
                let r = admin.handle(
                    &Request {
                        peer: Some(ip("8.8.8.8")),
                        ..ask(method, path, None, &none)
                    },
                    now,
                );
                assert_eq!(error(&r), (404, "admin_off".into()), "{path}");
            }
        }
        // lan: a public or unknown peer is closed before the method is looked at.
        let lan = on(Mode::Lan, &fake);
        for peer in [Some(ip("8.8.8.8")), Some(ip("::ffff:1.1.1.1")), None] {
            let r = lan.handle(
                &Request {
                    peer,
                    ..ask("DELETE", "/admin/status", None, &none)
                },
                now,
            );
            assert_eq!(error(&r), (403, "admin_closed".into()), "{peer:?}");
        }
        let open = on(Mode::Open, &fake);
        assert_eq!(
            open.handle(
                &Request {
                    peer: Some(ip("8.8.8.8")),
                    ..ask("GET", "/admin/nonce", None, &none)
                },
                now
            )
            .status,
            200
        );
        // The method, with Allow.
        for (method, path, allow) in [
            ("POST", "/admin/status", "GET"),
            ("HEAD", "/admin/nonce", "GET"),
            ("OPTIONS", "/admin/rates", "POST"),
            ("GET", "/admin/gold", "POST"),
            ("GET", "/admin/rates/reset", "POST"),
        ] {
            let r = lan.handle(&ask(method, path, None, &none), now);
            assert_eq!((r.status, r.allow), (405, Some(allow)), "{method} {path}");
        }
        // The header, then the nonce, then the signature: none of it reaches the agent.
        let r = lan.handle(&ask("GET", "/admin/status", None, &none), now);
        assert_eq!(error(&r), (401, "admin_auth".into()));
        let r = lan.handle(
            &ask(
                "GET",
                "/admin/status",
                Some("Bearer ABCD-EFGH-JKLM-NPQR"),
                &none,
            ),
            now,
        );
        assert_eq!(
            error(&r),
            (401, "admin_auth".into()),
            "never the code itself"
        );
        let r = lan.handle(
            &ask("GET", "/admin/status", Some(STATUS_HEADER), &none),
            now,
        );
        assert_eq!(
            error(&r),
            (401, "admin_nonce".into()),
            "a nonce this host never gave"
        );
        let r = signed(
            &lan,
            now,
            "ABCD-EFGH-JKLM-NPQX",
            "GET",
            "/admin/status",
            b"",
        );
        assert_eq!((error(&r), r.route), ((401, "admin_code".into()), "status"));
        // A signature over another route, method or body is wrong too.
        let nonce = fresh(&lan, now);
        let other = auth_header(CODE, &nonce, "GET", "/admin/gold", b"");
        let r = lan.handle(&ask("GET", "/admin/status", Some(&other), &none), now);
        assert_eq!(error(&r), (401, "admin_code".into()));
        let nonce = fresh(&lan, now);
        let other = auth_header(CODE, &nonce, "POST", "/admin/rates", b"{}");
        let tampered = Body::Read(br#"{"rates":{"Rate.XP.Kill":20}}"#.to_vec());
        let r = lan.handle(&ask("POST", "/admin/rates", Some(&other), &tampered), now);
        assert_eq!(error(&r), (401, "admin_code".into()));
        // A wrong signature is a 401 even with a body that is not JSON.
        let nonce = fresh(&lan, now);
        let header = auth_header("ABCD-EFGH-JKLM-NPQX", &nonce, "POST", "/admin/gold", b"x");
        let b = Body::Read(b"x".to_vec());
        let r = lan.handle(
            &Request {
                content_type: Some("text/plain"),
                ..ask("POST", "/admin/gold", Some(&header), &b)
            },
            now,
        );
        assert_eq!(error(&r), (401, "admin_code".into()));
        assert_eq!(fake.calls(), 0);
    }

    #[test]
    fn a_nonce_is_good_once_and_for_a_minute() {
        let fake = Fake::default();
        let admin = on(Mode::Lan, &fake);
        let t0 = Instant::now();
        let r = admin.handle(&ask("GET", "/admin/nonce", None, &Body::None), t0);
        let v: serde_json::Value = serde_json::from_str(&r.body).unwrap();
        assert_eq!(
            (v["ok"].as_bool(), v["expires_in"].as_u64()),
            (Some(true), Some(60))
        );
        let nonce = v["nonce"].as_str().unwrap();
        assert!(
            nonce.len() == 32
                && nonce
                    .bytes()
                    .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')),
            "{nonce}"
        );
        assert_ne!(fresh(&admin, t0), nonce, "every nonce is new");
        let header = auth_header(CODE, nonce, "GET", "/admin/status", b"");
        let get = ask("GET", "/admin/status", Some(&header), &Body::None);
        assert_eq!(admin.handle(&get, t0 + Duration::from_secs(59)).status, 200);
        assert_eq!(
            error(&admin.handle(&get, t0 + Duration::from_secs(59))),
            (401, "admin_nonce".into()),
            "replayed"
        );
        // Expired at 60 s; never good before it was made.
        let nonce = fresh(&admin, t0);
        let header = auth_header(CODE, &nonce, "GET", "/admin/status", b"");
        let get = ask("GET", "/admin/status", Some(&header), &Body::None);
        assert_eq!(
            error(&admin.handle(&get, t0 + Duration::from_secs(60))),
            (401, "admin_nonce".into())
        );
        let nonce = fresh(&admin, t0 + Duration::from_secs(5));
        let header = auth_header(CODE, &nonce, "GET", "/admin/status", b"");
        let get = ask("GET", "/admin/status", Some(&header), &Body::None);
        assert_eq!(error(&admin.handle(&get, t0)), (401, "admin_nonce".into()));
        // A nonce changed anywhere, or made by another host (or run), fails its tag.
        let nonce = fresh(&admin, t0);
        for i in [0, 12, 19, 20, 31] {
            let mut changed = nonce.clone().into_bytes();
            changed[i] = if changed[i] == b'0' { b'1' } else { b'0' };
            let changed = String::from_utf8(changed).unwrap();
            let header = auth_header(CODE, &changed, "GET", "/admin/status", b"");
            let get = ask("GET", "/admin/status", Some(&header), &Body::None);
            assert_eq!(
                error(&admin.handle(&get, t0)),
                (401, "admin_nonce".into()),
                "{i}"
            );
        }
        let elsewhere = fresh(&on(Mode::Lan, &Fake::default()), t0);
        let header = auth_header(CODE, &elsewhere, "GET", "/admin/status", b"");
        let get = ask("GET", "/admin/status", Some(&header), &Body::None);
        assert_eq!(error(&admin.handle(&get, t0)), (401, "admin_nonce".into()));
        assert_eq!(fake.calls(), 1);
    }

    #[test]
    fn handing_out_nonces_stores_nothing_and_pushes_none_out() {
        let fake = Fake::default();
        let admin = on(Mode::Open, &fake);
        let t0 = Instant::now();
        let first = fresh(&admin, t0);
        let others: HashSet<String> = (0..1000).map(|_| fresh(&admin, t0)).collect();
        assert!(others.len() == 1000 && !others.contains(&first));
        let header = auth_header(CODE, &first, "GET", "/admin/status", b"");
        let get = ask("GET", "/admin/status", Some(&header), &Body::None);
        let later = t0 + Duration::from_secs(1);
        assert_eq!(admin.handle(&get, later).status, 200);
        assert_eq!(
            error(&admin.handle(&get, later)),
            (401, "admin_nonce".into()),
            "once only"
        );
        assert_eq!(fake.calls(), 1);
        // Only a signed request keeps its nonce, and only for the nonce's minute.
        let nonces = &admin.on.as_ref().ok().unwrap().nonces;
        assert_eq!(lock(&nonces.spent).nonces.len(), 1);
        let wrong = signed(
            &admin,
            later,
            "ABCD-EFGH-JKLM-NPQX",
            "GET",
            "/admin/status",
            b"",
        );
        assert_eq!(error(&wrong), (401, "admin_code".into()));
        assert_eq!(lock(&nonces.spent).nonces.len(), 1);
        let late = t0 + Duration::from_secs(61);
        assert_eq!(
            signed(&admin, late, CODE, "GET", "/admin/status", b"").status,
            200
        );
        assert_eq!(lock(&nonces.spent).nonces.len(), 1, "the first aged out");
    }

    #[test]
    fn a_nonce_that_aged_out_of_the_spent_list_is_refused_whatever_its_request_says() {
        let n = Nonces::new().unwrap();
        let (t0, at) = (n.born, n.born + Duration::from_secs(61));
        let first = n.issue(t0).unwrap();
        assert_eq!(&first[12..20], "00000000", "made in second 0");
        let later = n.issue(at).unwrap();
        assert_eq!(&later[12..20], "0000003d", "made in second 61");
        let made = n.check(&later, at).unwrap();
        assert!(n.spend(made, at));
        assert!(!n.spend(made, at), "spent");
        // A request with the first nonce that waited, its `now` taken while that was good.
        let waited = t0 + Duration::from_secs(30);
        let made = n.check(&first, waited).unwrap();
        assert!(!n.spend(made, waited));
    }

    #[test]
    fn only_wrong_codes_count_and_a_right_one_always_passes() {
        let fake = Fake::default();
        let admin = on(Mode::Lan, &fake);
        let t0 = Instant::now();
        // Missing or malformed headers and unknown nonces cost nothing.
        for _ in 0..30 {
            let r = admin.handle(&ask("GET", "/admin/status", None, &Body::None), t0);
            assert_eq!(error(&r), (401, "admin_auth".into()));
            let r = admin.handle(
                &ask("GET", "/admin/status", Some(STATUS_HEADER), &Body::None),
                t0,
            );
            assert_eq!(error(&r), (401, "admin_nonce".into()));
        }
        let wrong = "ABCD-EFGH-JKLM-NPQX";
        for i in 0..10 {
            let r = signed(&admin, t0, wrong, "GET", "/admin/status", b"");
            assert_eq!(error(&r), (401, "admin_code".into()), "{i}");
        }
        let later = t0 + Duration::from_secs(30);
        let r = signed(&admin, later, wrong, "GET", "/admin/status", b"");
        assert_eq!(
            (error(&r), r.retry_after),
            ((429, "too_many".into()), Some(570))
        );
        let v: serde_json::Value = serde_json::from_str(&r.body).unwrap();
        assert_eq!(v["retry_after"], 570);
        assert_eq!(
            v["message"],
            "Too many wrong admin codes from here: check the code. Wrong ones get this answer for \
             the next 10 minutes."
        );
        // The right code, from the source that is held.
        assert_eq!(
            signed(&admin, later, CODE, "GET", "/admin/status", b"").status,
            200
        );
        // Another source is not held; this one is free again when the first ten age out.
        let elsewhere = Request {
            peer: Some(ip("10.0.0.9")),
            ..ask("GET", "/admin/status", None, &Body::None)
        };
        let nonce = fresh(&admin, later);
        let header = auth_header(wrong, &nonce, "GET", "/admin/status", b"");
        let r = admin.handle(
            &Request {
                authorization: Some(&header),
                ..elsewhere
            },
            later,
        );
        assert_eq!(error(&r), (401, "admin_code".into()));
        let r = signed(
            &admin,
            t0 + FAILURE_WINDOW,
            wrong,
            "GET",
            "/admin/status",
            b"",
        );
        assert_eq!(error(&r), (401, "admin_code".into()));
        assert_eq!(fake.calls(), 1);
    }

    #[test]
    fn a_post_needs_a_json_body_once_signed() {
        let fake = Fake::default();
        let admin = on(Mode::Lan, &fake);
        let now = Instant::now();
        let gold = br#"{"id":"a1","to":["Thrall"],"copper":10000}"#;
        let nonce = fresh(&admin, now);
        let header = auth_header(CODE, &nonce, "POST", "/admin/gold", gold);
        let b = Body::Read(gold.to_vec());
        let r = admin.handle(
            &Request {
                content_type: Some("text/plain"),
                ..ask("POST", "/admin/gold", Some(&header), &b)
            },
            now,
        );
        assert_eq!(error(&r), (415, "json_only".into()));
        let nonce = fresh(&admin, now);
        let header = auth_header(CODE, &nonce, "POST", "/admin/gold", b"");
        let r = admin.handle(&ask("POST", "/admin/gold", Some(&header), &Body::None), now);
        assert_eq!(error(&r), (400, "bad_request".into()));
        for (b, want) in [
            (Body::TooLarge(5000), (413, "too_large")),
            (Body::Unreadable, (400, "bad_request")),
        ] {
            let nonce = fresh(&admin, now);
            let header = auth_header(CODE, &nonce, "POST", "/admin/gold", b"");
            let r = admin.handle(&ask("POST", "/admin/gold", Some(&header), &b), now);
            assert_eq!(error(&r), (want.0, want.1.into()), "{b:?}");
        }
        assert_eq!(fake.calls(), 0);
        let nonce = fresh(&admin, now);
        let header = auth_header(CODE, &nonce, "POST", "/admin/gold", gold);
        let r = admin.handle(
            &Request {
                content_type: Some("Application/JSON; charset=utf-8"),
                ..ask("POST", "/admin/gold", Some(&header), &b)
            },
            now,
        );
        assert_eq!(r.status, 200);
        assert_eq!(
            fake.seen.lock().unwrap()[0],
            ("POST".into(), "/gold".into(), gold.to_vec())
        );
    }

    #[test]
    fn the_agents_answers_as_the_app_reads_them() {
        let fake = Fake::default();
        let admin = on(Mode::Lan, &fake);
        let now = Instant::now();
        let reply = |status: u16, body: &str| {
            Ok(AgentReply {
                status,
                body: body.as_bytes().to_vec(),
            })
        };
        let starting =
            r#"{"ok":false,"error":"world_starting","message":"The world is starting."}"#;
        for (answer, want) in [
            (
                Err(AgentError::Unreachable("refused".into())),
                (503, "world_unavailable"),
            ),
            (Err(AgentError::NoAnswer), (504, "no_answer")),
            (
                Err(AgentError::Unreadable("not an HTTP answer".into())),
                (502, "admin_misconfigured"),
            ),
            (
                reply(401, r#"{"ok":false,"error":"admin_auth"}"#),
                (502, "admin_misconfigured"),
            ),
            (reply(403, "{}"), (502, "admin_misconfigured")),
            (reply(200, "<html>"), (502, "admin_misconfigured")),
            (reply(500, ""), (502, "admin_misconfigured")),
            (reply(503, starting), (503, "world_starting")),
            (
                reply(
                    400,
                    r#"{"ok":false,"error":"out_of_range","message":"Rate.XP.Kill"}"#,
                ),
                (400, "out_of_range"),
            ),
        ] {
            fake.then(answer.clone());
            let r = signed(&admin, now, CODE, "GET", "/admin/status", b"");
            assert_eq!(error(&r), (want.0, want.1.into()), "{answer:?}");
            assert_eq!(r.route, "status");
        }
        fake.then(reply(503, starting));
        assert_eq!(
            signed(&admin, now, CODE, "GET", "/admin/status", b"").body,
            starting,
            "verbatim"
        );
        fake.then(Err(AgentError::Unreachable("refused".into())));
        let r = signed(&admin, now, CODE, "GET", "/admin/status", b"");
        assert_eq!(
            r.body,
            r#"{"ok":false,"error":"world_unavailable","message":"The world server is not running (or still starting)."}"#
        );
        fake.then(Err(AgentError::NoAnswer));
        let r = signed(&admin, now, CODE, "GET", "/admin/status", b"");
        assert_eq!(
            r.body,
            r#"{"ok":false,"error":"no_answer","message":"No answer in time: it may still have been done. Refresh before trying again."}"#
        );
        fake.then(reply(401, "{}"));
        let r = signed(&admin, now, CODE, "GET", "/admin/status", b"");
        assert_eq!(
            r.body,
            r#"{"ok":false,"error":"admin_misconfigured","message":"The data host and the world server disagree on ADMIN_CODE: run `docker compose up -d` on the server."}"#
        );
    }

    #[test]
    fn server_json_offers_the_menu_by_mode_and_source() {
        let fake = Fake::default();
        let open = |open| {
            Some(Section {
                url: "admin/",
                open,
            })
        };
        let lan = on(Mode::Lan, &fake);
        assert_eq!(
            (lan.section(true), lan.section(false)),
            (open(true), open(false))
        );
        let everyone = on(Mode::Open, &fake);
        assert_eq!(
            (everyone.section(true), everyone.section(false)),
            (open(true), open(true))
        );
        for off in [
            Admin::off(),
            Admin::new(Mode::Off, CODE, Some(Box::new(fake.clone()))),
            Admin::new(Mode::Lan, CODE, None),
            Admin::new(
                Mode::Lan,
                "CHANGEME-ABCD-EFGH-JKLM",
                Some(Box::new(fake.clone())),
            ),
            Admin::new(
                Mode::Open,
                "ÄBCD-EFGH-JKLM-NPQR",
                Some(Box::new(fake.clone())),
            ),
            Admin::new(Mode::Open, "ABCD-EFGH", Some(Box::new(fake.clone()))),
        ] {
            assert_eq!(off.section(true), None);
            assert!(
                off.describe().starts_with("admin menu off: "),
                "{}",
                off.describe()
            );
        }
        assert_eq!(Admin::off().describe(), "admin menu off: INILLA_ADMIN=off");
        assert!(Admin::new(Mode::Lan, CODE, None)
            .describe()
            .contains("INILLA_ADMIN_AGENT"));
        for d in [lan.describe(), everyone.describe()] {
            assert!(
                d.starts_with("admin menu on, ") && d.ends_with("agent at fake"),
                "{d}"
            );
            assert!(
                !d.contains("ABCD") && !d.contains(&CODE.to_ascii_lowercase()),
                "{d}"
            );
        }
        assert!(everyone.describe().contains("OPEN"));
    }

    #[test]
    fn the_agent_is_asked_with_this_hosts_own_code() {
        let (addr, seen) = tcp_agent(vec![
            say(200, r#"{"ok":true,"world":"up"}"#),
            say(200, "{}"),
        ]);
        let agent = Agent::new(&addr, " abcd-EFGH-jklm-NPQR ");
        assert_eq!(agent.describe(), addr);
        assert_eq!(
            agent.forward("GET", "/status", b""),
            Ok(AgentReply {
                status: 200,
                body: br#"{"ok":true,"world":"up"}"#.to_vec()
            })
        );
        let got = seen.recv().unwrap();
        assert_eq!((got.method.as_str(), got.path.as_str()), ("GET", "/status"));
        assert_eq!(got.header("host"), Some("mangosd"));
        // Normalised, as the agent compares it.
        assert_eq!(got.header("authorization"), Some("Bearer ABCDEFGHJKLMNPQR"));
        assert_eq!(got.header("content-type"), Some("application/json"));
        assert_eq!(got.header("content-length"), Some("0"));
        assert_eq!(got.header("connection"), Some("close"));
        assert_eq!(
            agent
                .forward("POST", "/rates", RATES.as_bytes())
                .map(|r| r.status),
            Ok(200)
        );
        let got = seen.recv().unwrap();
        assert_eq!((got.method.as_str(), got.path.as_str()), ("POST", "/rates"));
        assert_eq!(got.body, Body::Read(RATES.as_bytes().to_vec()));
    }

    #[test]
    fn a_code_ending_in_a_line_break_never_breaks_the_agents_header() {
        let ends = [
            "ABCD-EFGH-JKLM-NPQR\r",
            "ABCD-EFGH-JKLM-NPQR\n",
            "ABCD-EFGH-JKLM-NPQR\r\n",
        ];
        let (addr, seen) = tcp_agent(ends.iter().map(|_| say(200, "{}")).collect());
        for code in ends {
            assert_eq!(key(code), key(CODE), "{code:?}");
            let agent = Agent::new(&addr, code);
            assert_eq!(
                agent
                    .forward("POST", "/rates", RATES.as_bytes())
                    .map(|r| r.status),
                Ok(200),
                "{code:?}"
            );
            let got = seen.recv().unwrap();
            assert_eq!(got.header("authorization"), Some("Bearer ABCDEFGHJKLMNPQR"));
            let length = RATES.len().to_string();
            assert_eq!(
                got.header("content-length"),
                Some(length.as_str()),
                "{code:?}"
            );
            assert_eq!(got.header("content-type"), Some("application/json"));
            assert_eq!(got.body, Body::Read(RATES.as_bytes().to_vec()), "{code:?}");
        }
    }

    #[test]
    fn the_agent_answer_is_read_to_its_length_its_end_or_the_deadline() {
        let big = "x".repeat(MAX_REPLY);
        let (addr, _seen) = tcp_agent(vec![
            // A whole answer on a connection left open: its Content-Length ends it.
            Act::SayAndHold(b"HTTP/1.1 503 X\r\nContent-Length: 2\r\n\r\n{}".to_vec()),
            // No Content-Length: to the end.
            Act::Say(b"HTTP/1.0 200 OK\r\n\r\n{\"ok\":true}".to_vec()),
            Act::Say(b"SSH-2.0-OpenSSH\r\n\r\n".to_vec()),
            Act::Say(b"SSH-2.0-OpenSSH_9.6\r\n".to_vec()),
            say(200, &big),
            Act::Say(b"HTTP/1.1 200 OK\r\nContent-Length: 18446744073709551615\r\n\r\n{}".to_vec()),
            // Asked, then gone before a whole answer (mangosd, and the agent with it, stopped).
            Act::Say(b"HTTP/1.0 200 OK\r\nContent-Length: 10\r\n\r\n{}".to_vec()),
            Act::Say(b"HTTP/1.0 2".to_vec()),
            Act::Say(Vec::new()),
            Act::Reset,
            Act::Hold,
            Act::SayAndHold(b"HTTP/1.0 200 OK\r\nContent-Length: 10\r\n\r\n{}".to_vec()),
        ]);
        let agent = Agent::new(&addr, CODE).with_deadline(Duration::from_millis(300));
        let started = Instant::now();
        assert_eq!(
            agent.forward("GET", "/status", b""),
            Ok(AgentReply {
                status: 503,
                body: b"{}".to_vec()
            })
        );
        assert!(
            started.elapsed() < Duration::from_millis(300),
            "not held to the deadline"
        );
        assert_eq!(
            agent.forward("GET", "/status", b"").map(|r| r.body),
            Ok(br#"{"ok":true}"#.to_vec())
        );
        for what in [
            "not an HTTP answer",
            "not an HTTP answer",
            "over 64 KiB",
            "over 64 KiB",
        ] {
            match agent.forward("GET", "/status", b"") {
                Err(AgentError::Unreadable(e)) => assert!(e.contains(what), "{e}"),
                other => panic!("{what}: {other:?}"),
            }
        }
        for what in ["cut short", "the head cut short", "closed", "reset"] {
            let started = Instant::now();
            assert_eq!(
                agent.forward("POST", "/gold", b"{}"),
                Err(AgentError::NoAnswer),
                "{what}"
            );
            assert!(
                started.elapsed() < Duration::from_millis(300),
                "{what}: not held to the deadline"
            );
        }
        for _ in 0..2 {
            let started = Instant::now();
            assert_eq!(
                agent.forward("POST", "/gold", b"{}"),
                Err(AgentError::NoAnswer)
            );
            let took = started.elapsed();
            assert!(
                took >= Duration::from_millis(300) && took < Duration::from_secs(3),
                "{took:?}"
            );
        }
        assert!(matches!(
            Agent::new(&nobody(), CODE).forward("GET", "/status", b""),
            Err(AgentError::Unreachable(_))
        ));
        assert!(matches!(
            Agent::new("no-such-host.invalid:8091", CODE).forward("GET", "/status", b""),
            Err(AgentError::Unreachable(_))
        ));
        assert_eq!(Agent::new("mangosd:8091", CODE).deadline, DEADLINE);
    }
}

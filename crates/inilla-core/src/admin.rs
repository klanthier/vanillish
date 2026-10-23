//! Vanillish: the **server admin menu**, the engine-free half — the admin code, the signature an
//! admin request carries, the data host's replies, the knobs the Server tab's sliders move, gold
//! in whole gold. The app (`inilla::admin`) asks the data host on a worker thread.
//!
//! ```text
//! GET  <base>admin/nonce                                          → {"ok":true,"nonce":"<32 hex>","expires_in":60}
//! GET  <base>admin/status       Authorization: Vanillish-Admin <nonce>:<mac>   → Status
//! POST <base>admin/rates        {"id","rates":{key:number},"announce":bool}     → Status + "changed"
//! POST <base>admin/rates/reset  {"id","announce":bool}                          → Status + "changed"
//! POST <base>admin/gold         {"id","to":[names] | "online","copper":int}    → GoldReply
//! ```
//!
//! The code never travels: `mac = HMAC-SHA256(SHA-256(code), nonce + "\n" + METHOD + "\n" + route
//! + "\n" + hex(SHA-256(body)))`, lowercase hex, over a single-use nonce. `route` is the data host's
//! own path (`/admin/status`), whatever `<base>` a proxy puts in front of it.

use std::collections::BTreeMap;
use std::fmt;
use std::ops::RangeInclusive;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::connect::{or_default, resolve, AdminInfo};

/// A code in the setup's form is at least this many symbols of [`ALPHABET`] (80 bits)…
pub const MIN_CODE_LEN: usize = 16;
/// …and one made by hand at least this many hex digits (`openssl rand -hex 10`: 80 bits).
pub const MIN_HEX_CODE_LEN: usize = 20;
/// The setup's 32 symbols: no 0, 1, I or O.
pub const ALPHABET: &str = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
/// The prefix of `.env.example`'s placeholder: a code that starts with it is no code.
const PLACEHOLDER: &str = "CHANGEME";

/// Why a typed admin code is refused before anything is sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodeError {
    /// A character other than ASCII letters, digits, spaces and dashes.
    Symbol(char),
    /// Neither the setup's form nor 20 hex digits: a word picked by hand can be guessed, and
    /// the wrong-code limit does not stop a guesser (a right code always passes).
    Form,
    /// The setup's length with a 0, 1, I or O in it, which no setup code has: one misread.
    Lookalike,
    /// The example's placeholder (`change-me…`).
    Placeholder,
}

impl fmt::Display for CodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CodeError::Symbol(c) => write!(
                f,
                "The admin code takes letters and digits only, not \"{c}\"."
            ),
            CodeError::Form => f.write_str(
                "An admin code is 16 letters and digits, as the setup prints it (or 20 hex digits).",
            ),
            CodeError::Lookalike => f.write_str(
                "The setup's codes have no 0, 1, I or O: check those letters (an O is a Q or D, an \
                 I is an L or J).",
            ),
            CodeError::Placeholder => f.write_str(
                "That is the example's placeholder: the code is ADMIN_CODE in the server's .env.",
            ),
        }
    }
}

/// `code` without its ASCII whitespace and dashes, ASCII upper-cased.
fn strip(code: &str) -> String {
    code.chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != '-')
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// The code as the data host and the agent read it: whitespace and dashes dropped, upper-cased,
/// and usable only as the setup makes it (16 or more of [`ALPHABET`]) or as 20 or more hex
/// digits; any other character, any other form or the placeholder refused — the setup's length
/// with only 0, 1, I or O outside its alphabet (and not all hex) as a code misread.
pub fn normalize_code(code: &str) -> Result<String, CodeError> {
    let n = strip(code);
    if let Some(c) = n.chars().find(|c| !c.is_ascii_alphanumeric()) {
        return Err(CodeError::Symbol(c));
    }
    if n.starts_with(PLACEHOLDER) {
        return Err(CodeError::Placeholder);
    }
    let setup = n.len() >= MIN_CODE_LEN && n.chars().all(|c| ALPHABET.contains(c));
    let hex = n.len() >= MIN_HEX_CODE_LEN && n.chars().all(|c| c.is_ascii_hexdigit());
    let misread = n.len() == MIN_CODE_LEN
        && n.chars()
            .all(|c| ALPHABET.contains(c) || "01IO".contains(c))
        && !n.chars().all(|c| c.is_ascii_hexdigit());
    if setup || hex {
        Ok(n)
    } else if misread {
        Err(CodeError::Lookalike)
    } else {
        Err(CodeError::Form)
    }
}

/// `ABCD-EFGH-JKLM-NPQR`: the code in groups of four, as the setup prints it.
pub fn group_code(code: &str) -> String {
    let chars: Vec<char> = strip(code).chars().collect();
    chars
        .chunks(4)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("-")
}

/// The MAC key: SHA-256 of the normalised code's ASCII (a grouped or lower-case code is the same).
pub fn key(code: &str) -> [u8; 32] {
    Sha256::digest(strip(code).as_bytes()).into()
}

/// HMAC-SHA256 (RFC 2104): a key longer than SHA-256's 64-byte block is hashed first.
pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let inner = Sha256::new()
        .chain_update(k.map(|b| b ^ 0x36))
        .chain_update(msg)
        .finalize();
    Sha256::new()
        .chain_update(k.map(|b| b ^ 0x5c))
        .chain_update(inner)
        .finalize()
        .into()
}

/// Lowercase hex, as every hex on the admin wire is.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The `Authorization` header of one request: `Vanillish-Admin <nonce>:<mac>`, the MAC over the
/// nonce, the method, the data host's route and the body's SHA-256 (of nothing for a `GET`).
pub fn auth_header(code: &str, nonce: &str, method: &str, route: &str, body: &[u8]) -> String {
    let msg = format!("{nonce}\n{method}\n{route}\n{}", hex(&Sha256::digest(body)));
    format!(
        "Vanillish-Admin {nonce}:{}",
        hex(&hmac_sha256(&key(code), msg.as_bytes()))
    )
}

/// The Keystore key of the admin code saved for a server (`DataSource::token_key`'s sibling).
pub fn code_key(server_id: &str) -> String {
    format!("admin-code:{server_id}")
}

/// A request id the agent answers a repeat of with its stored reply (`[A-Za-z0-9-]`, ≤ 64).
pub fn request_id(random: u64) -> String {
    format!("{random:016x}")
}

/// Where the nonce is asked for, relative to the admin URL.
pub const NONCE: &str = "nonce";

/// The signed routes, exactly as the data host serves and signs them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    Status,
    Rates,
    Reset,
    Gold,
}

impl Route {
    pub fn method(self) -> &'static str {
        match self {
            Route::Status => "GET",
            Route::Rates | Route::Reset | Route::Gold => "POST",
        }
    }

    /// The data host's path, the one the MAC covers.
    pub fn path(self) -> &'static str {
        match self {
            Route::Status => "/admin/status",
            Route::Rates => "/admin/rates",
            Route::Reset => "/admin/rates/reset",
            Route::Gold => "/admin/gold",
        }
    }

    /// The path relative to the admin URL (`<base>admin/`).
    pub fn relative(self) -> &'static str {
        &self.path()["/admin/".len()..]
    }
}

/// The admin menu's URL: `server.json`'s `admin.url` against the base when it is relative — no
/// scheme, no `//`, no `..`, so a signed request never leaves the host that described itself —
/// else `admin/`. Always with a trailing slash.
pub fn admin_url(base: &str, info: &AdminInfo) -> String {
    let url = info.url.trim();
    let safe = !url.is_empty()
        && url.chars().all(|c| c.is_ascii_graphic())
        && !url.contains([':', '\\', '?', '#'])
        && !url.contains("//")
        && !url.contains("..");
    let mut out = resolve(base, if safe { url } else { "admin/" });
    if !out.ends_with('/') {
        out.push('/');
    }
    out
}

/// `GET /admin/nonce`'s answer.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NonceReply {
    #[serde(default, deserialize_with = "or_default")]
    pub nonce: String,
    #[serde(default, deserialize_with = "or_default")]
    pub expires_in: u64,
}

/// One knob as the agent reports it: what the live conf holds, the base conf's value, the API's
/// range.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rate {
    #[serde(default, deserialize_with = "or_default")]
    pub value: f64,
    #[serde(default, deserialize_with = "or_default")]
    pub server: f64,
    #[serde(default, deserialize_with = "or_default")]
    pub min: f64,
    #[serde(default, deserialize_with = "or_default")]
    pub max: f64,
}

impl Rate {
    /// The live value is not the server's own.
    pub fn overridden(&self) -> bool {
        (self.value - self.server).abs() > TOLERANCE
    }
}

/// An online character.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Player {
    #[serde(default, deserialize_with = "or_default")]
    pub name: String,
    #[serde(default, deserialize_with = "or_default")]
    pub level: u32,
    #[serde(default, deserialize_with = "or_default")]
    pub race: u32,
    #[serde(default, deserialize_with = "or_default")]
    pub class: u32,
}

/// `GET /admin/status`, and a rates change's reply (which adds `changed`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Status {
    /// `up`, or `starting` (mangosd's world port not listening yet).
    #[serde(default, deserialize_with = "or_default")]
    pub world: String,
    /// How many rates the server keeps for the menu (lines for knobs this vmangos lacks included).
    #[serde(default, deserialize_with = "or_default")]
    pub overrides: u32,
    /// Every knob this server has (the speed only on a patched image).
    #[serde(default, deserialize_with = "or_default")]
    pub rates: BTreeMap<String, Rate>,
    #[serde(default, deserialize_with = "or_default")]
    pub players: Vec<Player>,
    /// `false`: the characters database did not answer, and `players` is empty for that.
    #[serde(default, deserialize_with = "or_default")]
    pub players_ok: bool,
    /// The keys a `POST /rates` or `/rates/reset` changed.
    #[serde(default, deserialize_with = "or_default")]
    pub changed: Vec<String>,
}

impl Status {
    pub fn world_up(&self) -> bool {
        self.world == "up"
    }
}

/// `POST /admin/gold`'s answer, by name.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GoldReply {
    /// The console said `Mail sent to`.
    #[serde(default, deserialize_with = "or_default")]
    pub sent: Vec<String>,
    /// No such character.
    #[serde(default, deserialize_with = "or_default")]
    pub missing: Vec<String>,
    /// The console answered without saying, or did not answer in time.
    #[serde(default, deserialize_with = "or_default")]
    pub unsure: Vec<String>,
    /// Left when the agent's deadline passed.
    #[serde(default, deserialize_with = "or_default")]
    pub not_sent: Vec<String>,
}

/// Every refusal on the admin wire: `{"ok":false,"error","message"}`, `retry_after` on a 429.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ErrorReply {
    #[serde(default, deserialize_with = "or_default")]
    pub error: String,
    #[serde(default, deserialize_with = "or_default")]
    pub message: String,
    #[serde(default)]
    pub retry_after: Option<u64>,
}

/// Why an admin request did not do what it asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdminError {
    /// No answer, and nothing was done (the network's reason).
    Unreachable(String),
    /// The request went out and no answer came, or the data host's own wait ran out (504
    /// `no_answer`): it may still have been done.
    NoAnswer,
    /// 404 `admin_off`: the menu is switched off on the server.
    Off,
    /// 403 `admin_closed`: only from the server's own network (`INILLA_ADMIN=lan`).
    Closed,
    /// 401 `admin_code`: not this server's code.
    WrongCode,
    /// 401 `admin_nonce`: the nonce was spent or had expired.
    Nonce,
    /// 401 `admin_auth`: the data host saw no signature it could read (a proxy dropping it).
    Unsigned,
    /// 429 `too_many`: a wrong code, after too many from here lately (advisory: a right code always
    /// passes).
    TooMany { retry_after: Option<u64> },
    /// Any other 429, JSON or not: a limit in front of the data host (a proxy's), which says
    /// nothing of the code.
    Throttled,
    /// 503 `world_unavailable` / `world_starting`: the world server is not up (its sentence).
    WorldDown(String),
    /// Any other refusal: the code and the server's sentence (502 `admin_misconfigured`, a 400,
    /// `console_busy`, `internal`…).
    Refused { code: String, message: String },
    /// Not JSON: a server from before the admin menu, or another web server (the status).
    Older(u16),
    /// `server.json` in a format this app does not read: a newer server (the format).
    Newer(u32),
}

/// A reply → `T`, or why not. The `error` code decides; a body that is not JSON is a server older
/// than the menu — but a bare 504 is a gateway that stopped waiting, so it may have been done, and
/// a bare 429 a limit in front of the data host.
pub fn read_reply<T: DeserializeOwned>(status: u16, body: &str) -> Result<T, AdminError> {
    let value = match serde_json::from_str::<serde_json::Value>(body) {
        Ok(v @ serde_json::Value::Object(_)) => v,
        _ if status == 504 => return Err(AdminError::NoAnswer),
        _ if status == 429 => return Err(AdminError::Throttled),
        _ => return Err(AdminError::Older(status)),
    };
    if status == 200 && value.get("ok") != Some(&serde_json::Value::Bool(false)) {
        return serde_json::from_value(value).map_err(|e| AdminError::Refused {
            code: "bad_reply".into(),
            message: format!("The server's answer did not read: {e}."),
        });
    }
    let r: ErrorReply = serde_json::from_value(value).unwrap_or_default();
    let message = Some(r.message.trim())
        .filter(|m| !m.is_empty())
        .map_or_else(|| format!("The server answered {status}."), str::to_string);
    Err(match r.error.as_str() {
        "admin_off" => AdminError::Off,
        "admin_closed" => AdminError::Closed,
        "admin_code" => AdminError::WrongCode,
        "admin_nonce" => AdminError::Nonce,
        "admin_auth" => AdminError::Unsigned,
        "too_many" => AdminError::TooMany {
            retry_after: r.retry_after,
        },
        "world_unavailable" | "world_starting" => AdminError::WorldDown(message),
        "no_answer" => AdminError::NoAnswer,
        _ if status == 429 => AdminError::Throttled,
        code => AdminError::Refused {
            code: if code.is_empty() {
                status.to_string()
            } else {
                code.to_string()
            },
            message,
        },
    })
}

/// `GET /admin/nonce` → the nonce: 32 lowercase hex digits, or the answer is refused (it goes
/// into a header).
pub fn read_nonce(status: u16, body: &str) -> Result<String, AdminError> {
    let r: NonceReply = read_reply(status, body)?;
    let hex = |c: char| c.is_ascii_digit() || ('a'..='f').contains(&c);
    if r.nonce.len() == 32 && r.nonce.chars().all(hex) {
        Ok(r.nonce)
    } else {
        Err(AdminError::Refused {
            code: "bad_nonce".into(),
            message: "The server's one-time number is not 32 hex digits.".into(),
        })
    }
}

/// The Server tab's three groups.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Experience,
    Loot,
    Movement,
}

/// One knob: its `mangosd.conf` key, its section, its label, its slider's own range (the API takes
/// more, and a value outside it widens it: [`slider_range`]) and step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Knob {
    pub key: &'static str,
    pub section: Section,
    pub label: &'static str,
    pub lo: f64,
    pub hi: f64,
    pub step: f64,
}

const fn rate(key: &'static str, section: Section, label: &'static str) -> Knob {
    Knob {
        key,
        section,
        label,
        lo: 0.5,
        hi: 10.0,
        step: 0.05,
    }
}

pub const XP_KILL: &str = "Rate.XP.Kill";
pub const XP_QUEST: &str = "Rate.XP.Quest";
pub const XP_EXPLORE: &str = "Rate.XP.Explore";
pub const DROP_POOR: &str = "Rate.Drop.Item.Poor";
pub const DROP_NORMAL: &str = "Rate.Drop.Item.Normal";
pub const DROP_UNCOMMON: &str = "Rate.Drop.Item.Uncommon";
pub const DROP_RARE: &str = "Rate.Drop.Item.Rare";
pub const DROP_EPIC: &str = "Rate.Drop.Item.Epic";
pub const DROP_LEGENDARY: &str = "Rate.Drop.Item.Legendary";
pub const DROP_REFERENCED: &str = "Rate.Drop.Item.Referenced";
pub const DROP_MONEY: &str = "Rate.Drop.Money";
pub const MOVE_SPEED: &str = "Rate.MoveSpeed.Player";

/// Every knob the agent allows, in the tab's order. vmangos rolls an item row at
/// `chance × the rate of its quality`, a reference row at `chance × Referenced`, and multiplies
/// money from loot and quest rewards by `Rate.Drop.Money` (`LootMgr.cpp`, `QuestDef.cpp`).
pub const KNOBS: &[Knob] = &[
    rate(XP_KILL, Section::Experience, "Kills"),
    rate(XP_QUEST, Section::Experience, "Quests"),
    rate(XP_EXPLORE, Section::Experience, "Exploration"),
    rate(DROP_POOR, Section::Loot, "Grey (poor)"),
    rate(DROP_NORMAL, Section::Loot, "White (and quest items)"),
    rate(DROP_UNCOMMON, Section::Loot, "Green"),
    rate(DROP_RARE, Section::Loot, "Blue"),
    rate(DROP_EPIC, Section::Loot, "Purple"),
    rate(DROP_LEGENDARY, Section::Loot, "Orange"),
    rate(
        DROP_REFERENCED,
        Section::Loot,
        "Shared loot tables (how often one is rolled)",
    ),
    rate(
        DROP_MONEY,
        Section::Loot,
        "Gold from loot and quest rewards",
    ),
    Knob {
        key: MOVE_SPEED,
        section: Section::Movement,
        label: "Player speed",
        lo: 0.75,
        hi: 2.0,
        step: 0.01,
    },
];

pub fn knob(key: &str) -> Option<&'static Knob> {
    KNOBS.iter().find(|k| k.key == key)
}

/// One slider that sets several knobs at once (or one, under its own label).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Group {
    pub label: &'static str,
    pub section: Section,
    pub keys: &'static [&'static str],
}

pub const GROUPS: &[Group] = &[
    Group {
        label: "All experience",
        section: Section::Experience,
        keys: &[XP_KILL, XP_QUEST, XP_EXPLORE],
    },
    Group {
        label: "Item drops",
        section: Section::Loot,
        keys: &[
            DROP_POOR,
            DROP_NORMAL,
            DROP_UNCOMMON,
            DROP_RARE,
            DROP_EPIC,
            DROP_LEGENDARY,
            DROP_REFERENCED,
        ],
    },
    Group {
        label: "Gold",
        section: Section::Loot,
        keys: &[DROP_MONEY],
    },
    Group {
        label: "Player speed",
        section: Section::Movement,
        keys: &[MOVE_SPEED],
    },
];

/// Two values closer than this are the same (the agent rounds to 0.01).
pub const TOLERANCE: f64 = 0.005;

/// The sliders' values, by key, before Apply.
pub type Draft = BTreeMap<String, f64>;

/// The draft a status starts: the live values as they are, in range or not.
pub fn draft(status: &Status) -> Draft {
    status
        .rates
        .iter()
        .map(|(k, r)| (k.clone(), r.value))
        .collect()
}

/// What Apply sends: the keys whose draft moved off the live value, rounded to 0.01.
pub fn changes(draft: &Draft, status: &Status) -> BTreeMap<String, f64> {
    draft
        .iter()
        .filter_map(|(k, v)| {
            let live = status.rates.get(k)?.value;
            ((v - live).abs() > TOLERANCE).then(|| (k.clone(), round(*v)))
        })
        .collect()
}

/// A refreshed status under a draft: a key the player moved off the old live value keeps its
/// value, every other key takes the new live one (another device's change, a restart with an
/// edited conf); a key the server lost goes, one it gained comes in at its live value.
pub fn rebase(draft: &Draft, old: &Status, new: &Status) -> Draft {
    new.rates
        .iter()
        .map(|(k, r)| {
            let moved = match (draft.get(k), old.rates.get(k)) {
                (Some(v), Some(o)) if (v - o.value).abs() > TOLERANCE => Some(*v),
                _ => None,
            };
            (k.clone(), moved.unwrap_or(r.value))
        })
        .collect()
}

/// The draft holds something Apply would send.
pub fn dirty(draft: &Draft, status: &Status) -> bool {
    !changes(draft, status).is_empty()
}

fn round(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// A knob's slider range: its own, widened to take in `value` (so drawing never moves it), kept
/// within the server's `min`–`max`.
pub fn slider_range(knob: &Knob, value: f64, rate: &Rate) -> RangeInclusive<f64> {
    let (mut lo, mut hi) = (knob.lo.min(value), knob.hi.max(value));
    if rate.min < rate.max {
        lo = lo.clamp(rate.min, rate.max);
        hi = hi.clamp(rate.min, rate.max);
    }
    lo..=hi
}

/// A group's slider: the value its keys share and `false`, or — they differ — `true` and the
/// geometric mean for the handle (a zero counts as 0.01). `None`: the server has none of them.
pub fn group_value(group: &Group, draft: &Draft) -> Option<(f64, bool)> {
    let values: Vec<f64> = group
        .keys
        .iter()
        .filter_map(|k| draft.get(*k))
        .copied()
        .collect();
    let first = *values.first()?;
    if values.iter().all(|v| (v - first).abs() <= 0.01) {
        return Some((first, false));
    }
    let mean = values.iter().map(|v| v.max(0.01).ln()).sum::<f64>() / values.len() as f64;
    Some((mean.exp(), true))
}

/// A group slider's range: every key's [`slider_range`] together.
pub fn group_range(group: &Group, draft: &Draft, status: &Status) -> Option<RangeInclusive<f64>> {
    group
        .keys
        .iter()
        .filter_map(|k| {
            Some(slider_range(
                knob(k)?,
                *draft.get(*k)?,
                status.rates.get(*k)?,
            ))
        })
        .reduce(|a, b| a.start().min(*b.start())..=a.end().max(*b.end()))
}

/// Copper in a gold coin.
pub const COPPER_PER_GOLD: u64 = 10_000;
/// The tab's amounts, in whole gold: its buttons, and its slider's range.
pub const GOLD_STEPS: [u32; 4] = [1, 10, 100, 1000];
pub const GOLD_RANGE: RangeInclusive<u32> = 1..=1000;

/// Whole gold in copper.
pub fn gold(gold: u32) -> u64 {
    u64::from(gold) * COPPER_PER_GOLD
}

/// `250g`, `12g 50s`, `75c`, as the game writes money.
pub fn format_gold(copper: u64) -> String {
    let (g, s, c) = (copper / COPPER_PER_GOLD, copper / 100 % 100, copper % 100);
    let mut parts = Vec::new();
    if g > 0 {
        parts.push(format!("{g}g"));
    }
    if s > 0 {
        parts.push(format!("{s}s"));
    }
    if c > 0 || parts.is_empty() {
        parts.push(format!("{c}c"));
    }
    parts.join(" ")
}

/// The most names one gold send takes (the agent's `MAX_NAMES`: past it, it refuses the send).
pub const MAX_GOLD_NAMES: usize = 50;

/// A letter the agent's `str.isalpha()` takes (a letter category: never a combining mark, a
/// number or a symbol): a cased one (Latin, Greek, Cyrillic…), or a Han, Kana or Hangul one.
/// vmangos names a character in Latin, Cyrillic or East Asian letters only
/// (`ObjectMgr::CheckPlayerName`), so the scripts left out, whose vowel signs are combining marks,
/// hold no character's name.
fn letter(c: char) -> bool {
    // U+0345 is a combining mark that counts as lower case; U+24B6… and U+1F130… are symbols.
    let symbol = matches!(c, '\u{345}' | '\u{24b6}'..='\u{24e9}' | '\u{1f130}'..='\u{1f189}');
    let cased = (c.is_lowercase() || c.is_uppercase()) && !c.is_numeric() && !symbol;
    // Hangul Jamo, Kana, Bopomofo, Hangul compatibility Jamo, Kana extensions, Han (and its
    // compatibility block), Hangul syllables, half-width Kana and Hangul.
    let east_asian = matches!(c,
        '\u{1100}'..='\u{11ff}'
            | '\u{3041}'..='\u{30ff}'
            | '\u{3105}'..='\u{312f}'
            | '\u{3131}'..='\u{318e}'
            | '\u{31f0}'..='\u{31ff}'
            | '\u{3400}'..='\u{4dbf}'
            | '\u{4e00}'..='\u{9fff}'
            | '\u{f900}'..='\u{faff}'
            | '\u{ac00}'..='\u{d7a3}'
            | '\u{ff66}'..='\u{ffdc}');
    c.is_alphabetic() && (cased || east_asian)
}

/// 2 to 12 [`letter`]s, the agent's rule for a name.
fn name_ok(name: &str) -> bool {
    (2..=12).contains(&name.chars().count()) && name.chars().all(letter)
}

/// A character name the agent takes, written the way the game stores it (capital first, the rest
/// lower case) when that is still one, else as typed: `İ` lower-cases to `i` and a combining dot,
/// `ß` upper-cases to `SS`.
pub fn check_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if !name_ok(name) {
        return Err("A character's name is 2 to 12 letters.".into());
    }
    let mut chars = name.chars();
    let stored: String = chars
        .next()
        .into_iter()
        .flat_map(char::to_uppercase)
        .chain(chars.flat_map(char::to_lowercase))
        .collect();
    Ok(if name_ok(&stored) {
        stored
    } else {
        name.to_string()
    })
}

/// Who the gold goes to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GoldTo {
    /// Every character online when it is sent.
    Online,
    Names(Vec<String>),
}

/// A rate as JSON: rounded to 0.01, a whole number written without a fraction.
fn number(v: f64) -> serde_json::Value {
    let v = round(v);
    if v.fract() == 0.0 && v.abs() < 1e15 {
        serde_json::json!(v as i64)
    } else {
        serde_json::json!(v)
    }
}

/// `POST /admin/rates`'s body.
pub fn rates_body(id: &str, rates: &BTreeMap<String, f64>, announce: bool) -> String {
    let rates: serde_json::Map<String, serde_json::Value> =
        rates.iter().map(|(k, v)| (k.clone(), number(*v))).collect();
    serde_json::json!({ "announce": announce, "id": id, "rates": rates }).to_string()
}

/// `POST /admin/rates/reset`'s body.
pub fn reset_body(id: &str, announce: bool) -> String {
    serde_json::json!({ "announce": announce, "id": id }).to_string()
}

/// `POST /admin/gold`'s body.
pub fn gold_body(id: &str, to: &GoldTo, copper: u64) -> String {
    let to = match to {
        GoldTo::Online => serde_json::json!("online"),
        GoldTo::Names(names) => serde_json::json!(names),
    };
    serde_json::json!({ "copper": copper, "id": id, "to": to }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CODE: &str = "ABCD-EFGH-JKLM-NPQR";
    const NONCE_HEX: &str = "00112233445566778899aabbccddeeff";

    /// The vectors the data host's tests assert too (computed with Python's `hmac`/`hashlib`).
    #[test]
    fn the_signature_matches_the_data_hosts() {
        assert_eq!(
            auth_header(CODE, NONCE_HEX, "GET", "/admin/status", b""),
            "Vanillish-Admin 00112233445566778899aabbccddeeff:2bfb9cca1911d025fc88f5222621f3ae5f2e9635d4c363fdc0231980353acbf4"
        );
        assert_eq!(
            auth_header(
                CODE,
                NONCE_HEX,
                "POST",
                "/admin/rates",
                br#"{"rates":{"Rate.XP.Kill":2},"announce":true}"#
            ),
            "Vanillish-Admin 00112233445566778899aabbccddeeff:af86f2b02789533c33b4c3a704701551d9517aa95581ecb26739906cb3efe176"
        );
        // A code typed in lower case, without dashes, is the same key.
        assert_eq!(key("abcd efgh jklm npqr"), key(CODE));
        assert_eq!(key(CODE), key("ABCDEFGHJKLMNPQR"));
        assert_eq!(
            Route::Status.path(),
            "/admin/status",
            "the route signed is the data host's"
        );
    }

    /// RFC 4231's test cases 2 (a short key) and 6 (a key longer than the block, hashed first).
    #[test]
    fn hmac_is_rfc_4231s() {
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
    fn codes_read_as_the_server_reads_them() {
        for typed in [
            CODE,
            "abcd-efgh-jklm-npqr",
            " ABCD EFGH\tJKLM-NPQR\n",
            "ABCDEFGHJKLMNPQR",
            "a-b-c-d-e-f-g-h-j-k-l-m-n-p-q-r",
        ] {
            assert_eq!(
                normalize_code(typed).as_deref(),
                Ok("ABCDEFGHJKLMNPQR"),
                "{typed:?}"
            );
        }
        // Longer is fine; digits are letters here.
        assert_eq!(
            normalize_code("2345-6789-ABCD-EFGH-JK").as_deref(),
            Ok("23456789ABCDEFGHJK")
        );
        assert_eq!(normalize_code("ABCD-EFGH-JKLM-NPQ"), Err(CodeError::Form));
        assert_eq!(normalize_code(""), Err(CodeError::Form));
        assert_eq!(normalize_code(" - "), Err(CodeError::Form));
        // The setup's alphabet has no 0, 1, I or O: at its length, such a code is one misread…
        for typed in [
            "ABCD-EFGH-IJKL-MNOP",
            "0BCD-EFGH-JKLM-NPQR",
            "1BCD-EFGH-JKLM-NPQR",
            "abcd-efgh-jkim-npqr",
        ] {
            assert_eq!(
                normalize_code(typed),
                Err(CodeError::Lookalike),
                "{typed:?}"
            );
        }
        assert_eq!(
            CodeError::Lookalike.to_string(),
            "The setup's codes have no 0, 1, I or O: check those letters (an O is a Q or D, an I \
             is an L or J)."
        );
        // …at any other length, or all hex (a hex code cut short), no form at all.
        for typed in [
            "0BCD-EFGH-JKLM-NPQR-S",
            "0BCD-EFGH-JKLM-NPQ",
            "0123-4567-89AB-CDEF",
        ] {
            assert_eq!(normalize_code(typed), Err(CodeError::Form), "{typed:?}");
        }
        // Hex, as `openssl rand -hex 10` makes it: 20 digits or more, any case, 0 and 1 too.
        assert_eq!(
            normalize_code("0123456789abcdef0123").as_deref(),
            Ok("0123456789ABCDEF0123")
        );
        assert_eq!(
            normalize_code("0123 4567 89AB CDEF 0123 45").as_deref(),
            Ok("0123456789ABCDEF012345")
        );
        assert_eq!(normalize_code("0123456789abcdef012"), Err(CodeError::Form));
        // Neither: a word picked by hand, or the two alphabets mixed.
        for typed in [
            "VanillishServer2024",
            "vanillishadmin2026",
            "0123456789ABCDEFGHJK",
            "correct-horse-battery-staple",
        ] {
            assert_eq!(normalize_code(typed), Err(CodeError::Form), "{typed:?}");
        }
        assert_eq!(
            normalize_code("ABCD-EFGH-JKLM-NPQ!"),
            Err(CodeError::Symbol('!'))
        );
        assert_eq!(
            normalize_code("ÄBCD-EFGH-JKLM-NPQR"),
            Err(CodeError::Symbol('Ä'))
        );
        assert_eq!(
            normalize_code("ABCD_EFGH_JKLM_NPQR"),
            Err(CodeError::Symbol('_'))
        );
        assert_eq!(
            normalize_code("change-me-to-something-long"),
            Err(CodeError::Placeholder)
        );
        assert_eq!(normalize_code("change-me"), Err(CodeError::Placeholder));
        // The placeholder is refused even in the setup's form.
        assert_eq!(
            normalize_code("CHANGEME-ABCD-EFGH"),
            Err(CodeError::Placeholder)
        );
        assert_eq!(
            CodeError::Form.to_string(),
            "An admin code is 16 letters and digits, as the setup prints it (or 20 hex digits)."
        );
        assert_eq!(group_code("abcdefghjklmnpqr"), CODE);
        assert_eq!(group_code(" ab-cdef ghjk "), "ABCD-EFGH-JK");
        assert_eq!(group_code(""), "");
        assert_eq!(code_key("home-1a2b3c"), "admin-code:home-1a2b3c");
        let id = request_id(0x1234);
        assert_eq!(id, "0000000000001234");
        assert!(id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    /// A signed request goes only to the host that described itself.
    #[test]
    fn the_admin_url_never_leaves_the_base() {
        let base = "http://192.168.1.20:8099/";
        let at = |url: &str| {
            admin_url(
                base,
                &AdminInfo {
                    url: url.into(),
                    open: true,
                },
            )
        };
        assert_eq!(at("admin/"), "http://192.168.1.20:8099/admin/");
        assert_eq!(at(""), "http://192.168.1.20:8099/admin/");
        assert_eq!(at(" admin "), "http://192.168.1.20:8099/admin/");
        assert_eq!(at("./manage/"), "http://192.168.1.20:8099/manage/");
        assert_eq!(at("/x/admin/"), "http://192.168.1.20:8099/x/admin/");
        for unsafe_url in [
            "http://evil.example/admin/",
            "https://evil.example/",
            "//evil.example/admin/",
            "../admin/",
            "a/../../admin/",
            "javascript:alert(1)",
            "evil.example:80/admin/",
            "\\\\evil\\admin",
            "admin/?x=1",
            "admin/#x",
            "ad min/",
            "admin/\n",
        ] {
            assert_eq!(
                at(unsafe_url),
                "http://192.168.1.20:8099/admin/",
                "{unsafe_url:?}"
            );
        }
        // Behind a proxy's path: relative to it.
        assert_eq!(
            admin_url("https://wow.example.com/v/", &AdminInfo::default()),
            "https://wow.example.com/v/admin/"
        );
        assert_eq!(Route::Reset.relative(), "rates/reset");
        assert_eq!(Route::Gold.method(), "POST");
    }

    fn status_with(values: &[(&str, f64, f64)]) -> Status {
        Status {
            world: "up".into(),
            rates: values
                .iter()
                .map(|(k, v, server)| {
                    let k = knob(k).unwrap();
                    let (min, max) = if k.section == Section::Movement {
                        (0.5, 3.0)
                    } else {
                        (0.0, 20.0)
                    };
                    (
                        k.key.to_string(),
                        Rate {
                            value: *v,
                            server: *server,
                            min,
                            max,
                        },
                    )
                })
                .collect(),
            ..Status::default()
        }
    }

    /// A server set outside the sliders' own ranges is shown as it is: not dirty, not moved.
    #[test]
    fn values_outside_the_sliders_are_left_alone() {
        let s = status_with(&[
            (DROP_POOR, 0.0, 0.0),
            (XP_KILL, 15.0, 1.0),
            (XP_QUEST, 25.0, 25.0),
            (MOVE_SPEED, 0.6, 1.0),
        ]);
        let mut d = draft(&s);
        assert!(!dirty(&d, &s));
        let k = |key| knob(key).unwrap();
        assert_eq!(
            slider_range(k(XP_KILL), 15.0, &s.rates[XP_KILL]),
            0.5..=15.0
        );
        assert_eq!(
            slider_range(k(DROP_POOR), 0.0, &s.rates[DROP_POOR]),
            0.0..=10.0
        );
        // Beyond the API's range: the slider stops at the server's max.
        assert_eq!(
            slider_range(k(XP_QUEST), 25.0, &s.rates[XP_QUEST]),
            0.5..=20.0
        );
        assert_eq!(
            slider_range(k(MOVE_SPEED), 0.6, &s.rates[MOVE_SPEED]),
            0.6..=2.0
        );
        assert_eq!(
            slider_range(k(MOVE_SPEED), 1.0, &s.rates[MOVE_SPEED]),
            0.75..=2.0
        );
        // No usable bounds: the knob's own range, widened.
        assert_eq!(slider_range(k(XP_KILL), 12.0, &Rate::default()), 0.5..=12.0);
        // Within the tolerance is no change; past it, the move is sent rounded.
        d.insert(XP_KILL.into(), 15.004);
        assert!(!dirty(&d, &s));
        d.insert(XP_KILL.into(), 2.0049);
        assert!(dirty(&d, &s));
        assert_eq!(
            changes(&d, &s),
            BTreeMap::from([(XP_KILL.to_string(), 2.0)])
        );
        // A key the server does not have is never sent.
        d.insert(DROP_EPIC.into(), 3.0);
        assert_eq!(changes(&d, &s).len(), 1);
        assert!(s.rates[XP_KILL].overridden() && !s.rates[DROP_POOR].overridden());
    }

    /// A refresh under moved sliders keeps the moves and takes the server's every other value.
    #[test]
    fn a_refresh_keeps_only_the_keys_moved() {
        let old = status_with(&[
            (XP_KILL, 1.0, 1.0),
            (DROP_RARE, 1.0, 1.0),
            (DROP_EPIC, 1.0, 1.0),
        ]);
        let mut d = draft(&old);
        d.insert(XP_KILL.into(), 3.0);
        // Meanwhile another device sets the blue drops to 2, and the epic one goes away while the
        // speed comes.
        let new = status_with(&[
            (XP_KILL, 1.0, 1.0),
            (DROP_RARE, 2.0, 1.0),
            (MOVE_SPEED, 1.2, 1.0),
        ]);
        let d = rebase(&d, &old, &new);
        assert_eq!(
            d,
            Draft::from([
                (XP_KILL.to_string(), 3.0),
                (DROP_RARE.to_string(), 2.0),
                (MOVE_SPEED.to_string(), 1.2)
            ])
        );
        assert_eq!(
            changes(&d, &new),
            BTreeMap::from([(XP_KILL.to_string(), 3.0)])
        );
        // Nothing moved: the new status's draft as it is.
        assert_eq!(rebase(&draft(&old), &old, &new), draft(&new));
        // A move within the tolerance is no move.
        let mut d = draft(&old);
        d.insert(DROP_RARE.into(), 1.004);
        assert_eq!(rebase(&d, &old, &new)[DROP_RARE], 2.0);
    }

    #[test]
    fn a_group_says_mixed_when_its_keys_differ() {
        let g = &GROUPS[0];
        let s = status_with(&[
            (XP_KILL, 2.0, 1.0),
            (XP_QUEST, 2.0, 1.0),
            (XP_EXPLORE, 2.004, 1.0),
        ]);
        let d = draft(&s);
        assert_eq!(group_value(g, &d), Some((2.0, false)));
        let s = status_with(&[
            (XP_KILL, 1.0, 1.0),
            (XP_QUEST, 4.0, 1.0),
            (XP_EXPLORE, 2.0, 1.0),
        ]);
        let d = draft(&s);
        let (v, mixed) = group_value(g, &d).unwrap();
        assert!(mixed && (v - 2.0).abs() < 1e-9, "{v}");
        assert_eq!(group_range(g, &d, &s), Some(0.5..=10.0));
        // A zero counts as 0.01, and widens the range down to it.
        let s = status_with(&[(DROP_POOR, 0.0, 1.0), (DROP_NORMAL, 1.0, 1.0)]);
        let d = draft(&s);
        let (v, mixed) = group_value(&GROUPS[1], &d).unwrap();
        assert!(mixed && (v - 0.1).abs() < 1e-9, "{v}");
        assert_eq!(group_range(&GROUPS[1], &d, &s), Some(0.0..=10.0));
        // A server without the speed: no slider.
        assert_eq!(group_value(&GROUPS[3], &d), None);
        assert_eq!(group_range(&GROUPS[3], &d, &s), None);
        // Every group's keys are knobs, in their group's section.
        for g in GROUPS {
            for k in g.keys {
                assert_eq!(knob(k).map(|k| k.section), Some(g.section), "{k}");
            }
        }
        assert_eq!(
            GROUPS.iter().map(|g| g.keys.len()).sum::<usize>(),
            KNOBS.len()
        );
    }

    #[test]
    fn money_reads_as_the_game_writes_it() {
        assert_eq!(format_gold(2_500_000), "250g");
        assert_eq!(format_gold(125_000), "12g 50s");
        assert_eq!(format_gold(75), "75c");
        assert_eq!(format_gold(0), "0c");
        assert_eq!(format_gold(1_234_567), "123g 45s 67c");
        assert_eq!(format_gold(10_005), "1g 5c");
        assert_eq!(gold(100), 1_000_000);
        assert_eq!(gold(*GOLD_RANGE.end()), 10_000_000);
        assert_eq!(format_gold(gold(1000)), "1000g");
        assert_eq!(check_name("thrall").as_deref(), Ok("Thrall"));
        assert_eq!(check_name(" JAINA ").as_deref(), Ok("Jaina"));
        assert_eq!(check_name("Ünter").as_deref(), Ok("Ünter"));
        for bad in ["", "A", "Thrall2", "Thr all", "Abcdefghijklm", "x;--"] {
            assert!(check_name(bad).is_err(), "{bad:?}");
        }
    }

    /// A name the app sends is one the agent takes (`str.isalpha()` on every char, 2 to 12): a
    /// case mapping that would break the rule is not made.
    #[test]
    fn names_keep_the_agents_rule() {
        // Capital dotted I lower-cases to i and a combining dot (no letter): sent as typed.
        assert_eq!(check_name("K\u{130}RAZ").as_deref(), Ok("K\u{130}RAZ"));
        assert_eq!(check_name(" k\u{130}raz ").as_deref(), Ok("k\u{130}raz"));
        // Sharp s upper-cases to SS: 12 letters would come out 13.
        let sharp = "\u{df}abcdefghijk";
        assert_eq!(sharp.chars().count(), 12);
        assert_eq!(check_name(sharp).as_deref(), Ok(sharp));
        assert_eq!(check_name("\u{df}abc").as_deref(), Ok("SSabc"));
        for name in ["K\u{130}raz", sharp, "\u{416}\u{430}\u{43d}\u{43d}\u{430}"] {
            let sent = check_name(name).unwrap();
            assert!(name_ok(&sent), "{sent:?}");
        }
        // Cyrillic, Han, Kana, Hangul, Latin with marks built in: letters.
        for c in [
            'a', 'Z', '\u{e9}', '\u{df}', '\u{416}', '\u{4e2d}', '\u{30a2}', '\u{ac00}',
        ] {
            assert!(letter(c), "{c:?}");
        }
        // A combining mark (as typed, or alphabetic like a vowel sign), a number, a symbol.
        for c in [
            '\u{301}', '\u{307}', '\u{345}', '\u{93e}', '\u{2160}', '\u{24b6}', '1', '-',
        ] {
            assert!(!letter(c), "{c:?}");
        }
        assert!(check_name("Jo\u{301}se").is_err());
    }

    /// The agent refuses a whole send of more names than this.
    #[test]
    fn the_gold_limit_is_the_agents() {
        let agent = include_str!("../../../server/docker/vmangos/bin/vanillish-admin");
        let line = format!("MAX_NAMES = {MAX_GOLD_NAMES}");
        assert!(agent.lines().any(|l| l == line), "{line}");
    }

    #[test]
    fn the_bodies_are_what_the_agent_reads() {
        let rates = BTreeMap::from([(XP_KILL.to_string(), 2.0), (DROP_RARE.to_string(), 1.254)]);
        assert_eq!(
            rates_body("00000000000000ab", &rates, true),
            r#"{"announce":true,"id":"00000000000000ab","rates":{"Rate.Drop.Item.Rare":1.25,"Rate.XP.Kill":2}}"#
        );
        assert_eq!(reset_body("1", false), r#"{"announce":false,"id":"1"}"#);
        assert_eq!(
            gold_body("7", &GoldTo::Online, gold(100)),
            r#"{"copper":1000000,"id":"7","to":"online"}"#
        );
        assert_eq!(
            gold_body(
                "7",
                &GoldTo::Names(vec!["Thrall".into(), "Jaina".into()]),
                125_000
            ),
            r#"{"copper":125000,"id":"7","to":["Thrall","Jaina"]}"#
        );
    }

    /// The agent's and the data host's JSON, as spec'd (`.context/admin-design.md` §2–3).
    #[test]
    fn the_replies_read() {
        let status = r#"{"ok":true,"world":"up","overrides":2,
            "rates":{"Rate.XP.Kill":{"value":2,"server":1,"min":0,"max":20},
                     "Rate.Drop.Money":{"value":1.5,"server":1,"min":0,"max":20},
                     "Rate.MoveSpeed.Player":{"value":1,"server":1,"min":0.5,"max":3}},
            "players":[{"name":"Thrall","level":12,"race":2,"class":1}],
            "players_ok":true}"#;
        let s: Status = read_reply(200, status).unwrap();
        assert!(s.world_up());
        assert_eq!((s.overrides, s.players_ok), (2, true));
        assert_eq!(
            s.rates[XP_KILL],
            Rate {
                value: 2.0,
                server: 1.0,
                min: 0.0,
                max: 20.0
            }
        );
        assert_eq!(s.rates[MOVE_SPEED].min, 0.5);
        assert_eq!(
            s.players,
            vec![Player {
                name: "Thrall".into(),
                level: 12,
                race: 2,
                class: 1
            }]
        );
        assert!(s.changed.is_empty());
        // A rates change: the status and what changed; the world still starting; no database.
        let changed: Status = read_reply(
            200,
            r#"{"ok":true,"world":"starting","overrides":0,"rates":{},"players":[],"players_ok":false,"changed":["Rate.XP.Kill"],"future":1}"#,
        )
        .unwrap();
        assert!(!changed.world_up() && !changed.players_ok);
        assert_eq!(changed.changed, vec![XP_KILL.to_string()]);
        let gold: GoldReply = read_reply(
            200,
            r#"{"ok":true,"sent":["Thrall"],"missing":["Nobody"],"unsure":[],"not_sent":["Jaina"]}"#,
        )
        .unwrap();
        assert_eq!(gold.sent, vec!["Thrall".to_string()]);
        assert_eq!(gold.missing, vec!["Nobody".to_string()]);
        assert_eq!(gold.not_sent, vec!["Jaina".to_string()]);
        assert_eq!(
            read_nonce(
                200,
                r#"{"ok":true,"nonce":"00112233445566778899aabbccddeeff","expires_in":60}"#
            )
            .as_deref(),
            Ok(NONCE_HEX)
        );
        for bad in [
            r#"{"ok":true,"nonce":"00112233445566778899AABBCCDDEEFF"}"#,
            r#"{"ok":true,"nonce":"0011\r\nX: y"}"#,
            r#"{"ok":true}"#,
        ] {
            assert!(
                matches!(read_nonce(200, bad), Err(AdminError::Refused { .. })),
                "{bad}"
            );
        }

        let refusal = |status, code: &str| {
            read_reply::<Status>(
                status,
                &format!(r#"{{"ok":false,"error":"{code}","message":"Why {code}."}}"#),
            )
            .unwrap_err()
        };
        assert_eq!(refusal(404, "admin_off"), AdminError::Off);
        assert_eq!(refusal(403, "admin_closed"), AdminError::Closed);
        assert_eq!(refusal(401, "admin_code"), AdminError::WrongCode);
        assert_eq!(refusal(401, "admin_nonce"), AdminError::Nonce);
        assert_eq!(refusal(401, "admin_auth"), AdminError::Unsigned);
        assert_eq!(
            refusal(503, "world_unavailable"),
            AdminError::WorldDown("Why world_unavailable.".into())
        );
        assert_eq!(
            refusal(503, "world_starting"),
            AdminError::WorldDown("Why world_starting.".into())
        );
        assert_eq!(refusal(504, "no_answer"), AdminError::NoAnswer);
        for (status, code) in [
            (502, "admin_misconfigured"),
            (400, "out_of_range"),
            (400, "unknown_key"),
            (503, "console_busy"),
            (500, "internal"),
            (413, "too_large"),
            (415, "json_only"),
            (405, "method"),
            (404, "not_found"),
        ] {
            assert_eq!(
                refusal(status, code),
                AdminError::Refused {
                    code: code.into(),
                    message: format!("Why {code}.")
                }
            );
        }
        assert_eq!(
            read_reply::<Status>(
                429,
                r#"{"ok":false,"error":"too_many","message":"Later.","retry_after":540}"#
            ),
            Err(AdminError::TooMany {
                retry_after: Some(540)
            })
        );
        // Another's 429, JSON or not, says nothing of the code.
        for body in [
            r#"{"ok":false,"error":"rate_limited","message":"Slow down."}"#,
            r#"{"message":"Too Many Requests"}"#,
            "<html>429 Too Many Requests</html>",
            "",
        ] {
            assert_eq!(
                read_reply::<Status>(429, body),
                Err(AdminError::Throttled),
                "{body}"
            );
        }
        // `ok:false` with a 200 is a refusal; JSON without a code says the status.
        assert_eq!(
            read_reply::<Status>(200, r#"{"ok":false,"error":"internal","message":" "}"#),
            Err(AdminError::Refused {
                code: "internal".into(),
                message: "The server answered 200.".into()
            })
        );
        assert_eq!(
            read_reply::<Status>(500, "{}"),
            Err(AdminError::Refused {
                code: "500".into(),
                message: "The server answered 500.".into()
            })
        );
        // Not JSON: a data host from before the menu (`token required`, a 404 page)…
        assert_eq!(
            read_reply::<Status>(401, "token required"),
            Err(AdminError::Older(401))
        );
        assert_eq!(read_reply::<Status>(404, ""), Err(AdminError::Older(404)));
        assert_eq!(
            read_reply::<Status>(200, "<html></html>"),
            Err(AdminError::Older(200))
        );
        assert_eq!(
            read_reply::<Status>(200, "[1]"),
            Err(AdminError::Older(200))
        );
        // …except a gateway that stopped waiting: it may have been done.
        assert_eq!(
            read_reply::<GoldReply>(504, "<html>Gateway Timeout</html>"),
            Err(AdminError::NoAnswer)
        );
        // A 200 in a shape that does not read.
        assert!(matches!(
            read_reply::<Status>(200, r#"{"ok":true,"rates":[1,2]}"#),
            Err(AdminError::Refused { .. })
        ));
    }
}

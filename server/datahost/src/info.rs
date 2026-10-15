//! **`GET /server.json`**: what this server is, for the app's one-field Connect: the logon port,
//! the realm, the core, the game data it serves and whether it takes sign-ups. Always open, like
//! `/`: it holds no secret, and it is how the app learns that the data needs a token.
//!
//! ```json
//! {"format":1,"name":"Vanillish","software":"vanillish-datahost 0.3.0","core":"vmangos","build":5875,
//!  "auth":{"port":3724},
//!  "realms":[{"name":"Vanillish","kind":"pve","world":{"address":"192.168.1.20","port":8085,"same_host":true}}],
//!  "data":{"url":"","token":false,"default":"enUS","locales":[{"locale":"enUS","path":"enUS/","bytes":5400000000,"addons":1}]},
//!  "registration":{"open":true,"url":"accounts","token":false,"credentials":"srp6",
//!                  "name_min":3,"name_max":16,"password_min":6,"password_max":16},
//!  "message":"optional sentence"}
//! ```
//!
//! Only what the operator said is written: no `auth` without a realm or a logon port (a data host
//! alone), no `data` without archives, no `registration` with sign-up off. The `url`s are relative
//! to where the app fetched this document, so a reverse proxy or another port can host either.
//! The app reads it as `inilla_core::connect::ServerInfo`; a change of shape lands there too.

use serde::Serialize;

use crate::accounts::Registration;
use crate::manifest::CLIENT_BUILD;

/// The layout of the document. The app refuses another number ("update the app").
pub const SERVER_INFO_FORMAT: u32 = 1;

/// The layout of `/locales.json`.
pub const LOCALES_FORMAT: u32 = 1;

/// What the operator said about the realm beside this host (compose passes the stack's `.env`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Realm {
    /// The app's name for the server (default: the realm's).
    pub server_name: Option<String>,
    /// `vmangos`, `cmangos`, … Left out when not given.
    pub core: Option<String>,
    /// realmd's host, when it is not the machine the app reached this one at.
    pub auth_host: Option<String>,
    /// realmd's published port.
    pub auth_port: Option<u16>,
    pub realm_name: Option<String>,
    /// `pve`, `pvp`, `rp`, `rppvp`: the app shows it.
    pub realm_type: Option<String>,
    /// What the realm list hands out as the world server (`REALM_ADDRESS`, `MANGOSD_PORT`).
    pub world_address: Option<String>,
    pub world_port: Option<u16>,
    /// The world server runs on this machine: the app then dials the host it typed, whatever
    /// address the realm list hands out.
    pub world_same_host: bool,
    /// A sentence from the operator, kept as the server profile's notes.
    pub message: Option<String>,
}

/// One language this host serves game data in: an entry of `/locales.json` and of
/// `server.json`'s `data.locales`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Locale {
    pub locale: String,
    /// Where its manifest, files and addons live, relative to the host (`deDE/`).
    pub path: String,
    pub bytes: u64,
    pub addons: usize,
}

/// `GET /locales.json`: the languages, the first one the default.
#[derive(Serialize)]
pub struct Locales<'a> {
    pub format: u32,
    pub default: &'a str,
    pub locales: &'a [Locale],
}

impl<'a> Locales<'a> {
    /// `None` without game data.
    pub fn of(locales: &'a [Locale]) -> Option<Self> {
        Some(Locales {
            format: LOCALES_FORMAT,
            default: &locales.first()?.locale,
            locales,
        })
    }
}

#[derive(Serialize)]
struct Document<'a> {
    format: u32,
    name: &'a str,
    software: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    core: Option<&'a str>,
    build: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth: Option<Auth<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    realms: Vec<RealmEntry<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Data<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    registration: Option<&'a Registration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<&'a str>,
}

#[derive(Serialize)]
struct Auth<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    host: Option<&'a str>,
    port: u16,
}

#[derive(Serialize)]
struct RealmEntry<'a> {
    name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<&'a str>,
    world: World<'a>,
}

#[derive(Serialize)]
struct World<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    address: Option<&'a str>,
    port: u16,
    same_host: bool,
}

#[derive(Serialize)]
struct Data<'a> {
    /// The data's base, relative to the document's: here.
    url: &'static str,
    token: bool,
    default: &'a str,
    locales: &'a [Locale],
}

/// The document. `locales` are the languages this host serves game data in (none: no `data`),
/// `data_token` whether the data needs the host's token, `registration` the sign-up block when
/// sign-up is on.
pub fn server_json(
    realm: &Realm,
    locales: &[Locale],
    data_token: bool,
    registration: Option<&Registration>,
) -> String {
    let knows_logon =
        realm.realm_name.is_some() || realm.auth_port.is_some() || realm.auth_host.is_some();
    let doc = Document {
        format: SERVER_INFO_FORMAT,
        name: realm
            .server_name
            .as_deref()
            .or(realm.realm_name.as_deref())
            .unwrap_or("Vanillish"),
        software: concat!("vanillish-datahost ", env!("CARGO_PKG_VERSION")),
        core: realm.core.as_deref(),
        build: CLIENT_BUILD,
        auth: knows_logon.then(|| Auth {
            host: realm.auth_host.as_deref(),
            port: realm.auth_port.unwrap_or(3724),
        }),
        realms: realm
            .realm_name
            .as_deref()
            .map(|name| RealmEntry {
                name,
                kind: realm.realm_type.as_deref().and_then(realm_kind),
                world: World {
                    address: realm.world_address.as_deref(),
                    port: realm.world_port.unwrap_or(8085),
                    same_host: realm.world_same_host,
                },
            })
            .into_iter()
            .collect(),
        data: Locales::of(locales).map(|l| Data {
            url: "",
            token: data_token,
            default: l.default,
            locales: l.locales,
        }),
        registration,
        message: realm.message.as_deref(),
    };
    serde_json::to_string(&doc).unwrap_or_default()
}

/// The realm's kind as the contract spells it (`pve`, `pvp`, `rp`, `rppvp`), from whatever
/// `REALM_TYPE` the operator wrote: db-init accepts any case and the `normal` / `rp-pvp` aliases.
/// Anything else is left out rather than passed on.
pub fn realm_kind(t: &str) -> Option<&'static str> {
    match t.trim().to_ascii_lowercase().as_str() {
        "pve" | "normal" => Some("pve"),
        "pvp" => Some("pvp"),
        "rp" => Some("rp"),
        "rppvp" | "rp-pvp" | "rp_pvp" => Some("rppvp"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn stack() -> Realm {
        Realm {
            core: Some("vmangos".into()),
            auth_port: Some(3724),
            realm_name: Some("Vanillish".into()),
            realm_type: Some("pve".into()),
            world_address: Some("192.168.1.20".into()),
            world_port: Some(8085),
            world_same_host: true,
            message: Some("optional sentence".into()),
            ..Realm::default()
        }
    }

    fn en() -> Vec<Locale> {
        vec![Locale {
            locale: "enUS".into(),
            path: "enUS/".into(),
            bytes: 5_400_000_000,
            addons: 1,
        }]
    }

    fn parse(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn the_full_stack_reads_as_the_contract_shows_it() {
        let text = server_json(
            &stack(),
            &en(),
            false,
            Some(&Registration::srp6(true, false)),
        );
        assert_eq!(
            text,
            r#"{"format":1,"name":"Vanillish","software":"vanillish-datahost 0.3.0","core":"vmangos","build":5875,"auth":{"port":3724},"realms":[{"name":"Vanillish","kind":"pve","world":{"address":"192.168.1.20","port":8085,"same_host":true}}],"data":{"url":"","token":false,"default":"enUS","locales":[{"locale":"enUS","path":"enUS/","bytes":5400000000,"addons":1}]},"registration":{"open":true,"url":"accounts","token":false,"credentials":"srp6","name_min":3,"name_max":16,"password_min":6,"password_max":16},"message":"optional sentence"}"#
        );
    }

    #[test]
    fn a_data_host_alone_says_only_what_it_knows() {
        let v = parse(&server_json(&Realm::default(), &en(), true, None));
        assert_eq!(v["name"], "Vanillish");
        assert_eq!(v["build"], 5875);
        assert_eq!(v["data"]["token"], true);
        assert_eq!(v["data"]["default"], "enUS");
        for absent in ["core", "auth", "realms", "registration", "message"] {
            assert!(v.get(absent).is_none(), "{absent}: {v}");
        }
    }

    #[test]
    fn without_archives_there_is_no_data_block() {
        let v = parse(&server_json(&stack(), &[], false, None));
        assert!(v.get("data").is_none());
        assert_eq!(v["realms"][0]["name"], "Vanillish");
        assert!(Locales::of(&[]).is_none());
    }

    #[test]
    fn the_realm_and_logon_say_only_what_was_given() {
        let realm = Realm {
            server_name: Some("Kevin's".into()),
            auth_host: Some("realm.example".into()),
            realm_name: Some("Elsewhere".into()),
            ..Realm::default()
        };
        let v = parse(&server_json(&realm, &[], false, None));
        assert_eq!(
            v["name"], "Kevin's",
            "the server's name wins over the realm's"
        );
        assert_eq!(v["auth"], json!({"host": "realm.example", "port": 3724}));
        assert_eq!(
            v["realms"],
            json!([{"name": "Elsewhere", "world": {"port": 8085, "same_host": false}}])
        );
        let port_only = Realm {
            auth_port: Some(3725),
            ..Realm::default()
        };
        let v = parse(&server_json(&port_only, &[], false, None));
        assert_eq!(v["auth"], json!({"port": 3725}));
        assert!(v.get("realms").is_none());
    }

    #[test]
    fn locales_json_keeps_its_shape() {
        let two = [
            en().remove(0),
            Locale {
                locale: "deDE".into(),
                path: "deDE/".into(),
                bytes: 1,
                addons: 0,
            },
        ];
        assert_eq!(
            serde_json::to_string(&Locales::of(&two).unwrap()).unwrap(),
            r#"{"format":1,"default":"enUS","locales":[{"locale":"enUS","path":"enUS/","bytes":5400000000,"addons":1},{"locale":"deDE","path":"deDE/","bytes":1,"addons":0}]}"#
        );
    }

    #[test]
    fn the_realm_kind_is_the_contracts_spelling() {
        assert_eq!(super::realm_kind("PvP"), Some("pvp"));
        assert_eq!(super::realm_kind("normal"), Some("pve"));
        assert_eq!(super::realm_kind("RP-PvP"), Some("rppvp"));
        assert_eq!(super::realm_kind("rp"), Some("rp"));
        assert_eq!(super::realm_kind("ffa"), None);
    }
}

//! Account passwords on Android: one file in the app's **private** storage (`filesDir`), sealed
//! with an AES-256-GCM key that lives in the Android Keystore (`Bridge.seal` / `unseal`, alias
//! `vanillish.secrets.v1`). The key never leaves the TEE; the file is useless without it, and a
//! reinstall or a restored backup (the key is gone) degrades to "type the password again".
//!
//! The map inside is the desktop fallback's TOML, so the two stand-ins share their logic.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use jni::objects::{JByteArray, JValue};

/// Serializes read-modify-write cycles of the sealed file.
static LOCK: Mutex<()> = Mutex::new(());

fn path() -> Option<PathBuf> {
    super::paths::dirs()
        .internal
        .as_ref()
        .map(|d| d.join("secrets.bin"))
}

fn crypt(method: &str, input: &[u8]) -> Option<Vec<u8>> {
    super::with_env(|env| {
        let class = super::bridge(env)?;
        let arr = env.byte_array_from_slice(input)?;
        let out = env
            .call_static_method(&class, method, "([B)[B", &[JValue::Object(&arr)])?
            .l()?;
        if out.is_null() {
            return Ok(None);
        }
        let out = JByteArray::from(out);
        Ok(Some(env.convert_byte_array(&out)?))
    })
    .flatten()
}

fn load() -> BTreeMap<String, String> {
    let Some(p) = path() else {
        return BTreeMap::new();
    };
    let Ok(sealed) = std::fs::read(&p) else {
        return BTreeMap::new();
    };
    match crypt("unseal", &sealed)
        .and_then(|plain| String::from_utf8(plain).ok())
        .and_then(|text| toml::from_str(&text).ok())
    {
        Some(map) => map,
        None => {
            // The Keystore key is gone (reinstall, restore) or the file is damaged: nothing in it
            // can ever be read again, so start over rather than fail every login.
            eprintln!("vanillish: saved passwords could not be unsealed; they are forgotten");
            let _ = std::fs::remove_file(&p);
            BTreeMap::new()
        }
    }
}

fn save(all: &BTreeMap<String, String>) -> Result<(), String> {
    let p = path().ok_or("no private storage")?;
    let text = toml::to_string(all).map_err(|e| e.to_string())?;
    let sealed = crypt("seal", text.as_bytes()).ok_or("the Android Keystore refused the key")?;
    let tmp = p.with_extension("bin.tmp");
    std::fs::write(&tmp, sealed).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &p).map_err(|e| e.to_string())
}

pub(crate) fn get(key: &str) -> Option<String> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    load().remove(key)
}

pub(crate) fn set(key: &str, secret: &str) -> Result<(), String> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut all = load();
    all.insert(key.to_string(), secret.to_string());
    save(&all)
}

pub(crate) fn delete(key: &str) {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut all = load();
    if all.remove(key).is_some() {
        let _ = save(&all);
    }
}

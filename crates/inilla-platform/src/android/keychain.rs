//! Account passwords on Android: one file in the app's **private** storage (`filesDir`), sealed
//! with an AES-256-GCM key that lives in the Android Keystore (`Bridge.seal` / `unseal`, alias
//! `vanillish.secrets.v1`). The key never leaves the TEE; the file is useless without it, and a
//! reinstall or a restored backup (the key is gone) degrades to "type the password again".
//!
//! The map inside is the desktop fallback's TOML, so the two stand-ins share their logic.
//!
//! Only a *permanent* failure forgets the file ([`crate::keychain::judge`]): a Keystore that is
//! busy or restarting (right after boot, under load) fails one read, not the passwords, so the
//! file is kept and the launcher asks for the password this once.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use jni::objects::{JByteArray, JValue};
use jni::JNIEnv;

use crate::keychain::{judge, Unsealed};

/// Serializes read-modify-write cycles of the sealed file.
static LOCK: Mutex<()> = Mutex::new(());

/// What a failed read says to the launcher (`set` reports it as "not saved").
const BUSY: &str = "saved passwords are unavailable right now (the Android Keystore is busy); \
                    nothing was changed";

fn path() -> Option<PathBuf> {
    super::paths::dirs()
        .internal
        .as_ref()
        .map(|d| d.join("secrets.bin"))
}

/// `Bridge.<method>(input)`: the bytes, or `None` for Java's null.
fn call(env: &mut JNIEnv, method: &str, input: &[u8]) -> jni::errors::Result<Option<Vec<u8>>> {
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
}

fn seal(plain: &[u8]) -> Option<Vec<u8>> {
    super::with_env(|env| call(env, "seal", plain)).flatten()
}

/// Tri-state, unlike [`seal`]: `Ok(None)` is Java's null, gone for good (no key, a bad tag);
/// `Err` is this time only — `Bridge.unseal` throws for a busy Keystore, and a JNI failure says
/// nothing about the file either.
fn unseal(sealed: &[u8]) -> Result<Option<Vec<u8>>, ()> {
    super::with_env(|env| call(env, "unseal", sealed)).ok_or(())
}

/// The saved map. `Err` when it could not be read this time; the file is then left as it is.
fn load() -> Result<BTreeMap<String, String>, String> {
    let Some(p) = path() else {
        return Ok(BTreeMap::new());
    };
    let sealed = match std::fs::read(&p) {
        Ok(sealed) => sealed,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(format!("saved passwords: {e}")),
    };
    match judge(unseal(&sealed)) {
        Unsealed::Map(map) => Ok(map),
        Unsealed::Gone => {
            // The Keystore key is gone (reinstall, restore) or the file is damaged: nothing in it
            // can ever be read again, so start over rather than fail every login. Set aside, not
            // deleted: the next save writes a fresh file beside it.
            eprintln!("vanillish: saved passwords could not be unsealed; they are forgotten");
            let _ = std::fs::rename(&p, p.with_extension("bin.unreadable"));
            Ok(BTreeMap::new())
        }
        Unsealed::Failed => {
            eprintln!("vanillish: saved passwords could not be read this time; kept for later");
            Err(BUSY.into())
        }
    }
}

fn save(all: &BTreeMap<String, String>) -> Result<(), String> {
    let p = path().ok_or("no private storage")?;
    let text = toml::to_string(all).map_err(|e| e.to_string())?;
    let sealed = seal(text.as_bytes()).ok_or("the Android Keystore refused the key")?;
    let tmp = p.with_extension("bin.tmp");
    // On disk before the rename: a crash must not leave a short file that later reads as damaged
    // (and so forgotten).
    let mut f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    f.write_all(&sealed).map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())?;
    drop(f);
    std::fs::rename(&tmp, &p).map_err(|e| e.to_string())
}

pub(crate) fn get(key: &str) -> Option<String> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // A failed read asks for the password this once; the file stays for the next login.
    load().ok()?.remove(key)
}

pub(crate) fn set(key: &str, secret: &str) -> Result<(), String> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Never save over a file that could not be read: that would keep only this password.
    let mut all = load()?;
    all.insert(key.to_string(), secret.to_string());
    save(&all)
}

pub(crate) fn delete(key: &str) {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Ok(mut all) = load() else {
        return;
    };
    if all.remove(key).is_some() {
        let _ = save(&all);
    }
}

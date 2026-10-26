//! DPAPI for [`crate::keychain`]: the saved passwords as one file, `secrets.bin` in the fallback
//! folder, sealed for this Windows account on this PC (`CryptProtectData`, the user's own key).
//! The file is useless to anyone else: another account, or the folder copied to another PC,
//! cannot unseal it, and the launcher asks for the passwords again.
//!
//! Android's sealed file, with DPAPI in the Keystore's place (`crate::android::keychain`): the
//! desktop's TOML map inside, written tmp-then-rename, and only a permanent refusal forgets it
//! ([`crate::keychain::dpapi_answer`], [`crate::keychain::judge`]). A plain `secrets.toml` an
//! earlier desktop build left beside it is sealed in, then deleted.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

use windows_sys::Win32::Foundation::{GetLastError, LocalFree};
use windows_sys::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};

use crate::keychain::{dpapi_answer, judge, Unsealed};

/// Serializes read-modify-write cycles of the sealed file.
static LOCK: Mutex<()> = Mutex::new(());

/// Mixed into every seal, so a blob another program sealed for this account is not ours.
const ENTROPY: &[u8] = b"Vanillish keychain v1";

const SEALED: &str = "secrets.bin";
/// The desktop fallback's plain file (`crate::keychain`'s `file`), from before the seal.
const PLAIN: &str = "secrets.toml";

/// What a failed read says to the launcher (`set` reports it as "not saved").
const BUSY: &str = "saved passwords are unavailable right now (Windows could not unseal them); \
                    nothing was changed";

/// `bytes` as the blob DPAPI reads (it never writes through an input blob).
fn blob(bytes: &[u8]) -> Option<CRYPT_INTEGER_BLOB> {
    Some(CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(bytes.len()).ok()?,
        pbData: bytes.as_ptr().cast_mut(),
    })
}

/// The bytes of a blob DPAPI allocated, copied out; the blob is freed.
///
/// # Safety
/// `out` is what a successful `CryptProtectData` or `CryptUnprotectData` wrote.
unsafe fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    if out.pbData.is_null() {
        return Vec::new();
    }
    // SAFETY: DPAPI's own allocation of `cbData` bytes, freed with `LocalFree` as it asks.
    unsafe {
        let bytes = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        LocalFree(out.pbData.cast());
        bytes
    }
}

/// `plain` sealed for this account, or the error `GetLastError` names.
fn seal(plain: &[u8]) -> Result<Vec<u8>, u32> {
    const ERROR_INVALID_PARAMETER: u32 = 87;
    let (Some(input), Some(entropy)) = (blob(plain), blob(ENTROPY)) else {
        return Err(ERROR_INVALID_PARAMETER);
    };
    let mut out = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    // SAFETY: both input blobs point at live slices; no prompt (UI forbidden); `out` is written
    // only on success.
    let ok = unsafe {
        CryptProtectData(
            &input,
            std::ptr::null(),
            &entropy,
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        )
    };
    if ok == 0 {
        // SAFETY: reads this thread's last error, which the failed call set.
        return Err(unsafe { GetLastError() });
    }
    // SAFETY: the call succeeded.
    Ok(unsafe { take(out) })
}

/// The plain bytes in `sealed`, or the error `GetLastError` names.
fn unseal(sealed: &[u8]) -> Result<Vec<u8>, u32> {
    const ERROR_INVALID_DATA: u32 = 13;
    let (Some(input), Some(entropy)) = (blob(sealed), blob(ENTROPY)) else {
        return Err(ERROR_INVALID_DATA);
    };
    let mut out = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    // SAFETY: as in `seal`; no description is asked for.
    let ok = unsafe {
        CryptUnprotectData(
            &input,
            std::ptr::null_mut(),
            &entropy,
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        )
    };
    if ok == 0 {
        // SAFETY: as in `seal`.
        return Err(unsafe { GetLastError() });
    }
    // SAFETY: the call succeeded.
    Ok(unsafe { take(out) })
}

/// The sealed map at `p`. `Err` when it could not be read this time; the file is then left as
/// it is.
fn load_sealed(p: &Path) -> Result<BTreeMap<String, String>, String> {
    let sealed = match std::fs::read(p) {
        Ok(sealed) => sealed,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(format!("saved passwords: {e}")),
    };
    match judge(dpapi_answer(unseal(&sealed))) {
        Unsealed::Map(map) => Ok(map),
        Unsealed::Gone => {
            // Another account's or another PC's file, or a damaged one: nothing in it can ever
            // be read here, so start over rather than fail every login. Set aside, not deleted.
            eprintln!("vanillish: saved passwords could not be unsealed; they are forgotten");
            let _ = std::fs::rename(p, p.with_extension("bin.unreadable"));
            Ok(BTreeMap::new())
        }
        Unsealed::Failed => {
            eprintln!("vanillish: saved passwords could not be read this time; kept for later");
            Err(BUSY.into())
        }
    }
}

/// The saved map in `dir`, with an earlier build's plain `secrets.toml` sealed in (what the
/// sealed file already holds wins) and then deleted. A plain file that does not parse is left
/// alone; one whose entries could not be sealed stays for the next read.
fn load(dir: &Path) -> Result<BTreeMap<String, String>, String> {
    let sealed = dir.join(SEALED);
    let mut all = load_sealed(&sealed)?;
    let plain = dir.join(PLAIN);
    let old: Option<BTreeMap<String, String>> = std::fs::read_to_string(&plain)
        .ok()
        .and_then(|text| toml::from_str(&text).ok());
    if let Some(old) = old {
        for (key, secret) in old {
            all.entry(key).or_insert(secret);
        }
        if save(&sealed, &all).is_ok() {
            let _ = std::fs::remove_file(&plain);
        }
    }
    Ok(all)
}

fn save(p: &Path, all: &BTreeMap<String, String>) -> Result<(), String> {
    // No passwords left: no file, rather than a seal of nothing.
    if all.is_empty() {
        return match std::fs::remove_file(p) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        };
    }
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = toml::to_string(all).map_err(|e| e.to_string())?;
    let sealed = seal(text.as_bytes())
        .map_err(|code| format!("Windows refused to seal the passwords (error {code:#x})"))?;
    let tmp = p.with_extension("bin.tmp");
    // On disk before the rename: a crash must not leave a short file that later reads as damaged
    // (and so forgotten).
    let mut f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    f.write_all(&sealed).map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())?;
    drop(f);
    std::fs::rename(&tmp, p).map_err(|e| e.to_string())
}

pub(crate) fn get(key: &str) -> Option<String> {
    let dir = crate::keychain::fallback_dir()?;
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // A failed read asks for the password this once; the file stays for the next login.
    load(&dir).ok()?.remove(key)
}

pub(crate) fn set(key: &str, secret: &str) -> Result<(), String> {
    let dir = crate::keychain::fallback_dir().ok_or("no secrets folder set")?;
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Never save over a file that could not be read: that would keep only this password.
    let mut all = load(&dir)?;
    all.insert(key.to_string(), secret.to_string());
    save(&dir.join(SEALED), &all)
}

pub(crate) fn delete(key: &str) {
    let Some(dir) = crate::keychain::fallback_dir() else {
        return;
    };
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Ok(mut all) = load(&dir) else {
        return;
    };
    if all.remove(key).is_some() {
        let _ = save(&dir.join(SEALED), &all);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh scratch folder (the tests never touch the process-wide fallback dir).
    fn scratch(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("inilla-dpapi-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_sealed_map_round_trips_and_a_tampered_one_is_gone() {
        let plain = b"\"account:a\" = \"hunter2\"\n";
        let sealed = seal(plain).expect("DPAPI seals for the current user");
        assert!(!sealed.windows(plain.len()).any(|w| w == plain));
        assert_eq!(unseal(&sealed).as_deref(), Ok(&plain[..]));
        let mut bad = sealed.clone();
        let last = bad.len() - 1;
        bad[last] ^= 0x5a;
        assert_eq!(dpapi_answer(unseal(&bad)), Ok(None));
        assert_eq!(dpapi_answer(unseal(b"not a blob")), Ok(None));
    }

    #[test]
    fn the_plain_file_is_sealed_in_and_deleted() {
        let dir = scratch("migrate");
        std::fs::write(
            dir.join(PLAIN),
            "# iNilla desktop build: saved passwords, in PLAIN TEXT.\n\
             \"account:a\" = \"old\"\n\"account:b\" = \"bee\"\n",
        )
        .unwrap();
        let mut newer = BTreeMap::new();
        newer.insert("account:a".to_string(), "new".to_string());
        save(&dir.join(SEALED), &newer).unwrap();
        let all = load(&dir).unwrap();
        assert_eq!(all.get("account:a").map(String::as_str), Some("new"));
        assert_eq!(all.get("account:b").map(String::as_str), Some("bee"));
        assert!(!dir.join(PLAIN).exists());
        assert!(!std::fs::read(dir.join(SEALED))
            .unwrap()
            .windows(3)
            .any(|w| w == b"bee"));
        assert_eq!(load(&dir).unwrap(), all);
        save(&dir.join(SEALED), &BTreeMap::new()).unwrap();
        assert!(!dir.join(SEALED).exists());
        assert_eq!(load(&dir), Ok(BTreeMap::new()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_that_never_unseals_is_set_aside() {
        let dir = scratch("damaged");
        std::fs::write(dir.join(SEALED), b"damaged").unwrap();
        assert_eq!(load(&dir), Ok(BTreeMap::new()));
        assert!(dir.join("secrets.bin.unreadable").exists());
        assert!(!dir.join(SEALED).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

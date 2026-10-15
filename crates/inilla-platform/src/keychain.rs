//! Account passwords and access tokens. On iOS they live in the **Keychain** (a generic password
//! item per key, service `inilla`): encrypted at rest, kept across app updates and SideStore
//! re-signs, never in a file the Files app can see.
//!
//! Off iOS there is no keychain wired up, so secrets fall back to one TOML file in a directory
//! the caller names ([`set_fallback_dir`]) — the desktop build is a development tool, and the
//! file says so in its header. Nothing is stored anywhere until a fallback dir is set.
//!
//! Vanillish: on Android they are sealed with a key in the **Android Keystore** and kept in the
//! app's private storage (`crate::android::keychain`); the fallback dir — on Android a folder a
//! PC can browse — is never used there.

use std::path::PathBuf;
use std::sync::Mutex;

#[cfg(target_os = "ios")]
const SERVICE: &str = "inilla";

static FALLBACK: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Where the desktop fallback file lives (`<dir>/secrets.toml`). Ignored on iOS.
pub fn set_fallback_dir(dir: PathBuf) {
    if let Ok(mut g) = FALLBACK.lock() {
        *g = Some(dir);
    }
}

pub fn get(key: &str) -> Option<String> {
    #[cfg(target_os = "ios")]
    {
        security_framework::passwords::get_generic_password(SERVICE, key)
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
    }
    #[cfg(target_os = "android")]
    {
        crate::android::keychain::get(key)
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        file::load().remove(key)
    }
}

pub fn set(key: &str, secret: &str) -> Result<(), String> {
    #[cfg(target_os = "ios")]
    {
        security_framework::passwords::set_generic_password(SERVICE, key, secret.as_bytes())
            .map_err(|e| e.to_string())
    }
    #[cfg(target_os = "android")]
    {
        crate::android::keychain::set(key, secret)
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        let mut all = file::load();
        all.insert(key.to_string(), secret.to_string());
        file::save(&all)
    }
}

pub fn delete(key: &str) {
    #[cfg(target_os = "ios")]
    {
        let _ = security_framework::passwords::delete_generic_password(SERVICE, key);
    }
    #[cfg(target_os = "android")]
    {
        crate::android::keychain::delete(key);
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        let mut all = file::load();
        if all.remove(key).is_some() {
            let _ = file::save(&all);
        }
    }
}

/// Vanillish: what one read of Android's sealed password file comes to
/// (`crate::android::keychain`). Pure, so the decision that may forget every saved password is
/// tested on the host.
#[cfg(not(target_os = "ios"))]
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
#[derive(Debug, PartialEq)]
pub(crate) enum Unsealed {
    /// The saved passwords.
    Map(std::collections::BTreeMap<String, String>),
    /// Gone for good: the Keystore has no key any more (a reinstall, a restored backup), the tag
    /// does not match, or the authenticated bytes are not the map. Nothing in the file can ever
    /// be read again, so it is set aside and the map starts over.
    Gone,
    /// Failed this time only: the Keystore was busy or restarting (Java threw), or the JNI call
    /// itself failed. That says nothing about the key or the file, so both are kept: forgetting
    /// here would lose every password a retry a moment later would have read.
    Failed,
}

/// Judge `Bridge.unseal`'s answer: `Ok(Some)` the plain bytes, `Ok(None)` gone for good, `Err`
/// failed this time.
#[cfg(not(target_os = "ios"))]
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub(crate) fn judge(unsealed: Result<Option<Vec<u8>>, ()>) -> Unsealed {
    match unsealed {
        Err(()) => Unsealed::Failed,
        Ok(None) => Unsealed::Gone,
        // The bytes passed GCM's tag: a map that does not parse now never will.
        Ok(Some(plain)) => String::from_utf8(plain)
            .ok()
            .and_then(|text| toml::from_str(&text).ok())
            .map_or(Unsealed::Gone, Unsealed::Map),
    }
}

#[cfg(not(any(target_os = "ios", target_os = "android")))]
mod file {
    use std::collections::BTreeMap;

    fn path() -> Option<std::path::PathBuf> {
        super::FALLBACK
            .lock()
            .ok()
            .and_then(|g| g.clone())
            .map(|d| d.join("secrets.toml"))
    }

    pub(super) fn load() -> BTreeMap<String, String> {
        path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| toml::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub(super) fn save(all: &BTreeMap<String, String>) -> Result<(), String> {
        let Some(p) = path() else {
            return Err("no secrets folder set".into());
        };
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let body = toml::to_string(all).map_err(|e| e.to_string())?;
        let text = format!(
            "# iNilla desktop build: saved passwords, in PLAIN TEXT.\n\
             # The iOS app keeps these in the Keychain; this file exists only off iOS.\n{body}"
        );
        std::fs::write(&p, text).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }
}

#[cfg(all(test, not(any(target_os = "ios", target_os = "android"))))]
mod tests {
    use super::*;

    #[test]
    fn the_fallback_round_trips() {
        let dir = std::env::temp_dir().join(format!("inilla-keychain-{}", std::process::id()));
        set_fallback_dir(dir.clone());
        set("account:a", "hunter2").unwrap();
        assert_eq!(get("account:a").as_deref(), Some("hunter2"));
        delete("account:a");
        assert_eq!(get("account:a"), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn only_a_permanent_unseal_failure_forgets_the_passwords() {
        let plain = b"\"account:a\" = \"hunter2\"\n".to_vec();
        let Unsealed::Map(map) = judge(Ok(Some(plain))) else {
            panic!("a good unseal is the map");
        };
        assert_eq!(map.get("account:a").map(String::as_str), Some("hunter2"));
        assert_eq!(
            judge(Ok(Some(Vec::new()))),
            Unsealed::Map(Default::default())
        );
        // No key any more, a bad tag: gone for good.
        assert_eq!(judge(Ok(None)), Unsealed::Gone);
        // Authenticated bytes that are not the map never will be.
        assert_eq!(judge(Ok(Some(vec![0xff, 0xfe]))), Unsealed::Gone);
        assert_eq!(judge(Ok(Some(b"not = [toml".to_vec()))), Unsealed::Gone);
        // A busy Keystore or a JNI failure keeps everything for the next try.
        assert_eq!(judge(Err(())), Unsealed::Failed);
    }
}

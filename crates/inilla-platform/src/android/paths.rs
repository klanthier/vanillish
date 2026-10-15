//! The app's folders, from the `Context` (`getFilesDir`, `getExternalFilesDir`, `getCacheDir`),
//! asked once and kept.

use std::path::PathBuf;
use std::sync::OnceLock;

use jni::objects::{JObject, JValue};
use jni::JNIEnv;

#[derive(Debug, Default)]
pub(crate) struct Dirs {
    /// `/data/user/0/<pkg>/files` — private, never visible from outside.
    pub internal: Option<PathBuf>,
    /// `/storage/emulated/0/Android/data/<pkg>/files` — the app's own corner of shared storage,
    /// which a PC sees over USB (File transfer). `None` while shared storage is not mounted.
    pub external: Option<PathBuf>,
    /// `/data/user/0/<pkg>/cache` — the OS may clear it under storage pressure.
    pub cache: Option<PathBuf>,
}

fn file_path(env: &mut JNIEnv, file: JObject) -> jni::errors::Result<Option<PathBuf>> {
    if file.is_null() {
        return Ok(None);
    }
    let path = env
        .call_method(&file, "getAbsolutePath", "()Ljava/lang/String;", &[])?
        .l()?;
    Ok(super::rust_string(env, path)?.map(PathBuf::from))
}

/// How often, and how far apart, `getExternalFilesDir` is asked again when it answers null: it
/// does so while the external dir cannot be stat'd or made at that moment (the MediaProvider's
/// FUSE daemon restarting after a Play system update), and each call tries again. About three
/// seconds, on the boot's thread (`android_main`), never the UI thread.
const EXTERNAL_TRIES: u32 = 10;
const EXTERNAL_WAIT: std::time::Duration = std::time::Duration::from_millis(300);

fn external_files_dir(env: &mut JNIEnv) -> jni::errors::Result<Option<PathBuf>> {
    let app = super::application();
    let external = env
        .call_method(
            &app,
            "getExternalFilesDir",
            "(Ljava/lang/String;)Ljava/io/File;",
            &[JValue::Object(&JObject::null())],
        )?
        .l()?;
    file_path(env, external)
}

pub(crate) fn dirs() -> &'static Dirs {
    static DIRS: OnceLock<Dirs> = OnceLock::new();
    DIRS.get_or_init(|| {
        let mut dirs = super::with_env(|env| {
            let app = super::application();
            let internal = env
                .call_method(&app, "getFilesDir", "()Ljava/io/File;", &[])?
                .l()?;
            let internal = file_path(env, internal)?;
            let external = external_files_dir(env)?;
            let cache = env
                .call_method(&app, "getCacheDir", "()Ljava/io/File;", &[])?
                .l()?;
            let cache = file_path(env, cache)?;
            Ok(Dirs {
                internal,
                external,
                cache,
            })
        })
        .unwrap_or_default();
        // Vanillish: shared storage briefly away is not shared storage gone (`paths::android_roots`).
        for _ in 1..EXTERNAL_TRIES {
            if dirs.external.is_some() || dirs.internal.is_none() {
                break;
            }
            std::thread::sleep(EXTERNAL_WAIT);
            dirs.external = super::with_env(external_files_dir).flatten();
        }
        if dirs.external.is_none() && dirs.internal.is_some() {
            super::log::write_line(
                "vanillish: shared storage (Android/data) is not available; the game's folders \
                 are not there this time",
            );
        }
        dirs
    })
}

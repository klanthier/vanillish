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

pub(crate) fn dirs() -> &'static Dirs {
    static DIRS: OnceLock<Dirs> = OnceLock::new();
    DIRS.get_or_init(|| {
        super::with_env(|env| {
            let app = super::application();
            let internal = env
                .call_method(&app, "getFilesDir", "()Ljava/io/File;", &[])?
                .l()?;
            let internal = file_path(env, internal)?;
            let external = env
                .call_method(
                    &app,
                    "getExternalFilesDir",
                    "(Ljava/lang/String;)Ljava/io/File;",
                    &[JValue::Object(&JObject::null())],
                )?
                .l()?;
            let external = file_path(env, external)?;
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
        .unwrap_or_default()
    })
}

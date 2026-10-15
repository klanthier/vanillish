//! The Storage Access Framework and the share sheet, through `Bridge`'s `[data]` methods (which
//! forward to `FolderImport.java`). `list` and `open` are binder calls into the document provider:
//! they run on the import's worker thread, never Bevy's main one.

use std::fs::File;
use std::os::fd::FromRawFd;
use std::path::Path;

use jni::objects::JValue;

/// Post the folder picker to the UI thread; `false` when there is no Activity to show it.
pub(crate) fn pick() -> bool {
    super::call_bridge("pickFolder", "()Z", &[], |_, v| v.z()).unwrap_or(false)
}

/// The JSON of the picked folder whose copy has not finished, if any.
pub(crate) fn pending() -> Option<String> {
    super::call_bridge("pendingFolder", "()Ljava/lang/String;", &[], |env, v| {
        let obj = v.l()?;
        super::rust_string(env, obj)
    })
    .flatten()
}

/// Why the last `list` or `open` failed, as the Java side caught it.
fn last_error(what: &str) -> String {
    let why = super::call_bridge("folderError", "()Ljava/lang/String;", &[], |env, v| {
        let obj = v.l()?;
        super::rust_string(env, obj)
    })
    .flatten()
    .filter(|s| !s.is_empty());
    match why {
        Some(why) => format!("{what}: {why}"),
        None => format!("{what} (the folder may have been moved, or the USB drive unplugged)"),
    }
}

/// The folder's archives as the listing JSON (`inilla_core::import::Listing`).
pub(crate) fn list(tree: &str, max_depth: u32) -> Result<String, String> {
    super::with_env(|env| {
        let class = super::bridge(env)?;
        let t = env.new_string(tree)?;
        let v = env
            .call_static_method(
                &class,
                "listFolder",
                "(Ljava/lang/String;I)Ljava/lang/String;",
                &[JValue::Object(&t), JValue::Int(max_depth.min(16) as i32)],
            )?
            .l()?;
        super::rust_string(env, v)
    })
    .flatten()
    .ok_or_else(|| last_error("the folder could not be read"))
}

/// One document of the folder, as a file this side owns: the Java side detaches the descriptor
/// from its `ParcelFileDescriptor`, so closing the `File` is what closes it.
pub(crate) fn open(tree: &str, id: &str) -> Result<File, String> {
    let fd = super::with_env(|env| {
        let class = super::bridge(env)?;
        let t = env.new_string(tree)?;
        let d = env.new_string(id)?;
        env.call_static_method(
            &class,
            "openDocument",
            "(Ljava/lang/String;Ljava/lang/String;)I",
            &[JValue::Object(&t), JValue::Object(&d)],
        )?
        .i()
    })
    .unwrap_or(-1);
    if fd < 0 {
        return Err(last_error(&format!("{id} could not be opened")));
    }
    // SAFETY: a descriptor detached for us alone (`ParcelFileDescriptor.detachFd`): nothing else
    // owns or closes it.
    Ok(unsafe { File::from_raw_fd(fd) })
}

/// Give the folder's grant back and drop it as the pending one.
pub(crate) fn forget(tree: &str) {
    super::with_env(|env| {
        let class = super::bridge(env)?;
        let t = env.new_string(tree)?;
        env.call_static_method(
            &class,
            "forgetFolder",
            "(Ljava/lang/String;)V",
            &[JValue::Object(&t)],
        )?;
        Ok(())
    });
}

/// Offer a file to other apps through the share sheet (a `FileProvider` URI, read-only).
pub(crate) fn share_file(path: &Path, mime: &str) -> bool {
    let Some(path) = path.to_str() else {
        return false;
    };
    super::with_env(|env| {
        let class = super::bridge(env)?;
        let p = env.new_string(path)?;
        let m = env.new_string(mime)?;
        env.call_static_method(
            &class,
            "shareFile",
            "(Ljava/lang/String;Ljava/lang/String;)Z",
            &[JValue::Object(&p), JValue::Object(&m)],
        )?
        .z()
    })
    .unwrap_or(false)
}

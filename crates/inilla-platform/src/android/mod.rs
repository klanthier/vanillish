//! **Vanillish** — the Android host, the counterpart of `objc.rs`: everything `inilla-platform`
//! asks of Android goes through here.
//!
//! - **Java**: one app class, `io.github.klanthier.vanillish.Bridge` (`android/app/src/main/java`),
//!   whose static methods do what must run in Java (the UI thread, the Keystore, the clipboard,
//!   the Vibrator) and cache what Android pushes (insets, the fold posture). Rust *pulls*, as iOS
//!   reads `safeAreaInsets`: there is no "native not registered yet" race and no `JNI_OnLoad`
//!   (GameActivity defines its own).
//! - **The NDK** where it has the call: logcat (`liblog`), the thermal state (`AThermal`), system
//!   properties — no JNI at all.
//!
//! The JavaVM and the `Application` come from `ndk_context`, which android-activity fills in before
//! `android_main` runs, so nothing here needs an init call — the boot (`inilla_boot`) can already
//! ask for the paths. The Activity is never cached on this side: it can be recreated, the
//! Application and the class cannot.
//!
//! JNI rules kept by [`with_env`]: a thread is attached once, as a daemon (Bevy's pool threads
//! are not attached; a per-call attach/detach would cost every time), every call runs in its own
//! local frame (a native thread never returns to Java, so its local refs would otherwise pile up
//! until the table overflows), and a Java exception is logged and cleared, never left pending.

pub(crate) mod clipboard;
pub(crate) mod device;
pub(crate) mod fold;
pub(crate) mod keychain;
pub(crate) mod log;
pub(crate) mod paths;
/// The Storage Access Framework's folder picker and the share sheet (`crate::picker`).
pub(crate) mod picker;
pub(crate) mod thermal;

use std::sync::OnceLock;

use jni::objects::{GlobalRef, JClass, JObject, JValue};
use jni::{JNIEnv, JavaVM};

/// The Java side's bridge class (`android/app/src/main/java/io/github/klanthier/vanillish`).
pub const BRIDGE: &str = "io.github.klanthier.vanillish.Bridge";

/// The process's JavaVM, from android-activity's `ndk_context`.
pub(crate) fn vm() -> Option<&'static JavaVM> {
    static VM: OnceLock<Option<JavaVM>> = OnceLock::new();
    VM.get_or_init(|| {
        let ctx = ndk_context::android_context();
        if ctx.vm().is_null() {
            return None;
        }
        // SAFETY: android-activity stores the process's JavaVM pointer before `android_main`.
        unsafe { JavaVM::from_raw(ctx.vm().cast()) }.ok()
    })
    .as_ref()
}

/// The `Application` context: a global reference android-activity keeps for the whole process
/// (and never deletes), so it outlives every call made here.
pub(crate) fn application<'a>() -> JObject<'a> {
    // SAFETY: a live global reference (see above); `JObject` does not delete it on drop.
    unsafe { JObject::from_raw(ndk_context::android_context().context().cast()) }
}

/// Run `f` with this thread's `JNIEnv`, inside a local frame. `None` when there is no VM (a test
/// on the host) or the call failed; a Java exception is described to logcat and cleared.
pub(crate) fn with_env<R>(f: impl FnOnce(&mut JNIEnv) -> jni::errors::Result<R>) -> Option<R> {
    let mut env = vm()?.attach_current_thread_as_daemon().ok()?;
    let out = env.with_local_frame(16, |env| f(env));
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }
    out.ok()
}

/// The bridge class, loaded once through the Application's class loader: `FindClass` on a natively
/// attached thread only sees the system classes.
pub(crate) fn bridge<'a>(env: &mut JNIEnv<'a>) -> jni::errors::Result<JClass<'a>> {
    static CLASS: OnceLock<GlobalRef> = OnceLock::new();
    let global = match CLASS.get() {
        Some(g) => g,
        None => {
            let loader = env
                .call_method(
                    application(),
                    "getClassLoader",
                    "()Ljava/lang/ClassLoader;",
                    &[],
                )?
                .l()?;
            let name = env.new_string(BRIDGE)?;
            let class = env
                .call_method(
                    &loader,
                    "loadClass",
                    "(Ljava/lang/String;)Ljava/lang/Class;",
                    &[JValue::Object(&name)],
                )?
                .l()?;
            let global = env.new_global_ref(class)?;
            CLASS.get_or_init(|| global)
        }
    };
    // A fresh local reference to the class, owned by the caller's local frame.
    let local = env.new_local_ref(global.as_obj())?;
    Ok(JClass::from(local))
}

/// `Bridge.<name><sig>(args)`, with the frame, attach and exception rules of [`with_env`].
pub(crate) fn call_bridge<R>(
    name: &str,
    sig: &str,
    args: &[JValue],
    read: impl FnOnce(&mut JNIEnv, jni::objects::JValueOwned) -> jni::errors::Result<R>,
) -> Option<R> {
    with_env(|env| {
        let class = bridge(env)?;
        let v = env.call_static_method(&class, name, sig, args)?;
        read(env, v)
    })
}

/// A `void` bridge call; answers whether it ran.
pub(crate) fn call_bridge_void(name: &str, sig: &str, args: &[JValue]) -> bool {
    call_bridge(name, sig, args, |_, _| Ok(())).is_some()
}

/// A Java `String` (possibly null) into Rust.
pub(crate) fn rust_string(env: &mut JNIEnv, obj: JObject) -> jni::errors::Result<Option<String>> {
    if obj.is_null() {
        return Ok(None);
    }
    let s = jni::objects::JString::from(obj);
    let out: String = env.get_string(&s)?.into();
    Ok(Some(out))
}

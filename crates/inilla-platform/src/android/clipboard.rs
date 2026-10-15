//! `ClipboardManager`, through `Bridge.clipboardGet` / `clipboardSet` (binder calls, fine from
//! any thread; Android 10+ answers only while the game has the input focus, which a paste chord
//! implies).

use jni::objects::JValue;

pub(crate) fn get() -> Option<String> {
    super::call_bridge("clipboardGet", "()Ljava/lang/String;", &[], |env, v| {
        let obj = v.l()?;
        super::rust_string(env, obj)
    })
    .flatten()
    .filter(|s| !s.is_empty())
}

pub(crate) fn set(text: &str) -> bool {
    super::with_env(|env| {
        let class = super::bridge(env)?;
        let s = env.new_string(text)?;
        env.call_static_method(
            &class,
            "clipboardSet",
            "(Ljava/lang/String;)Z",
            &[JValue::Object(&s)],
        )?
        .z()
    })
    .unwrap_or(false)
}

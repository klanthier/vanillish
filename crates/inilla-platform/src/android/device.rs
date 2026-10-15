//! The window and the device: screen-awake, haptics, the display-cutout insets, memory, the
//! frame rate, URLs and relaunching — `Bridge` calls — and the device's identity, read from the
//! system properties with no JNI.

use std::ffi::{c_char, CStr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use jni::objects::{JFloatArray, JLongArray, JValue};

use crate::device::{Haptic, Side};

pub(crate) fn keep_screen_on(on: bool) {
    super::call_bridge_void("keepScreenOn", "(Z)V", &[JValue::Bool(on.into())]);
}

pub(crate) fn haptic(kind: Haptic) {
    let k = match kind {
        Haptic::Light => 0,
        Haptic::Medium => 1,
        Haptic::Heavy => 2,
        Haptic::Selection => 3,
    };
    super::call_bridge_void("haptic", "(I)V", &[JValue::Int(k)]);
}

/// A `float[]` bridge getter, copied out.
pub(crate) fn floats(name: &str) -> Option<Vec<f32>> {
    super::call_bridge(name, "()[F", &[], |env, v| {
        let obj = v.l()?;
        if obj.is_null() {
            return Ok(None);
        }
        let arr = JFloatArray::from(obj);
        let n = env.get_array_length(&arr)? as usize;
        let mut out = vec![0.0f32; n];
        env.get_float_array_region(&arr, 0, &mut out)?;
        Ok(Some(out))
    })
    .flatten()
}

/// The display-cutout insets in dp (Bevy's logical pixels), `[top, left, bottom, right]`, and the
/// side the camera hole is on in landscape. The system bars are hidden (immersive), so only the
/// cutout counts — as on iOS, where the home indicator is hidden too.
pub(crate) fn safe_area() -> ([f32; 4], Option<Side>) {
    let Some(v) = floats("safeArea") else {
        return ([0.0; 4], None);
    };
    if v.len() < 5 {
        return ([0.0; 4], None);
    }
    let side = match v[4] as i32 {
        0 => Some(Side::Left),
        1 => Some(Side::Right),
        _ => None,
    };
    ([v[0], v[1], v[2], v[3]], side)
}

/// `ActivityManager.MemoryInfo`: `{availMem, threshold, totalMem, lowMemory}`.
fn memory_info() -> Option<[i64; 4]> {
    super::call_bridge("memoryInfo", "()[J", &[], |env, v| {
        let obj = v.l()?;
        if obj.is_null() {
            return Ok(None);
        }
        let arr = JLongArray::from(obj);
        let mut out = [0i64; 4];
        if env.get_array_length(&arr)? >= 4 {
            env.get_long_array_region(&arr, 0, &mut out)?;
            Ok(Some(out))
        } else {
            Ok(None)
        }
    })
    .flatten()
}

/// `MemAvailable` from `/proc/meminfo`, in bytes.
fn mem_available() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    crate::device::parse_mem_available(&text)
}

/// Bytes the system can still hand out before its low-memory killer starts on background apps
/// (`MemAvailable` − `MemoryInfo.threshold`). Android has no per-process limit like iOS's
/// `os_proc_available_memory`; this is the nearest figure, system-wide. Cached for a second: the
/// perf overlay asks every frame.
pub(crate) fn available_memory() -> Option<u64> {
    static CACHE: Mutex<Option<(Instant, Option<u64>)>> = Mutex::new(None);
    static THRESHOLD: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, v)) = *cache {
        if at.elapsed() < Duration::from_secs(1) {
            return v;
        }
    }
    let threshold = *THRESHOLD.get_or_init(|| {
        memory_info()
            .map(|m| m[1].max(0) as u64)
            .unwrap_or_default()
    });
    let v = mem_available()
        .or_else(|| memory_info().map(|m| m[0].max(0) as u64))
        .map(|avail| avail.saturating_sub(threshold));
    *cache = Some((Instant::now(), v));
    v
}

pub(crate) fn open_url(url: &str) -> bool {
    super::with_env(|env| {
        let class = super::bridge(env)?;
        let s = env.new_string(url)?;
        env.call_static_method(&class, "openUrl", "(Ljava/lang/String;)Z", &[JValue::Object(&s)])?
            .z()
    })
    .unwrap_or(false)
}

/// Start the app again in a fresh process (Android keeps a finished app's process around, and a
/// Bevy app cannot be built twice in one).
pub(crate) fn relaunch() -> bool {
    super::call_bridge_void("relaunch", "()V", &[])
}

/// Ask the display for `hz` (0 = the panel's highest), so a 120 Hz panel runs at 120 in the High
/// preset and drops to 60/30 with the frame cap instead of idling at a mismatched rate.
pub(crate) fn set_frame_rate(hz: f32) {
    super::call_bridge_void("setFrameRate", "(F)V", &[JValue::Float(hz)]);
}

/// The panel's refresh rate now and the highest it offers, in Hz.
pub(crate) fn refresh_rate() -> Option<(f32, f32)> {
    let v = floats("refreshRate")?;
    (v.len() >= 2 && v[1] > 0.0).then(|| (v[0], v[1]))
}

/// Game Dashboard's mode (`GameManager.GAME_MODE_*`).
pub(crate) fn game_mode() -> i32 {
    super::call_bridge("gameMode", "()I", &[], |_, v| v.i()).unwrap_or(0)
}

/// Back presses since the last call.
pub(crate) fn take_back_presses() -> u32 {
    super::call_bridge("takeBackPresses", "()I", &[], |_, v| v.i())
        .unwrap_or(0)
        .max(0) as u32
}

/// Which orientations the game may take: `0` landscape (either way up, by the sensor), `1` any.
pub(crate) fn set_orientation_mode(mode: i32) {
    super::call_bridge_void("setOrientationMode", "(I)V", &[JValue::Int(mode)]);
}

fn property(name: &CStr) -> String {
    let mut buf = [0 as c_char; libc::PROP_VALUE_MAX as usize];
    // SAFETY: `buf` is PROP_VALUE_MAX bytes, as `__system_property_get` requires.
    let n = unsafe { libc::__system_property_get(name.as_ptr(), buf.as_mut_ptr()) };
    if n <= 0 {
        return String::new();
    }
    // SAFETY: the property call NUL-terminates within PROP_VALUE_MAX.
    unsafe { CStr::from_ptr(buf.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

/// `"samsung SM-F9xxx · Android 16 (API 36) · SM8850"`.
pub(crate) fn describe() -> String {
    let maker = property(c"ro.product.manufacturer");
    let model = property(c"ro.product.model");
    let release = property(c"ro.build.version.release");
    let sdk = property(c"ro.build.version.sdk");
    let soc = property(c"ro.soc.model");
    let mut s = format!("{maker} {model} · Android {release} (API {sdk})");
    if !soc.is_empty() {
        s.push_str(" · ");
        s.push_str(&soc);
    }
    s
}

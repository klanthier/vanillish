//! The thermal state, straight from the NDK (`AThermal_*`, libandroid, API 30/31): a status
//! listener keeps an atomic current, so the perf overlay can ask every frame for the price of a
//! load. The headroom forecast is rate-limited by Android (about once a second), so it is cached.

use std::ffi::{c_int, c_void};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[repr(C)]
struct AThermalManager {
    _private: [u8; 0],
}

type StatusCallback = extern "C" fn(data: *mut c_void, status: c_int);

#[link(name = "android")]
extern "C" {
    fn AThermal_acquireManager() -> *mut AThermalManager;
    fn AThermal_getCurrentThermalStatus(manager: *mut AThermalManager) -> c_int;
    fn AThermal_registerThermalStatusListener(
        manager: *mut AThermalManager,
        callback: StatusCallback,
        data: *mut c_void,
    ) -> c_int;
    fn AThermal_getThermalHeadroom(manager: *mut AThermalManager, forecast_seconds: c_int) -> f32;
}

/// `ATHERMAL_STATUS_*`; `-1` is ERROR.
static STATUS: AtomicI32 = AtomicI32::new(0);

struct Manager(*mut AThermalManager);
// SAFETY: the NDK thermal manager is documented as thread-safe; the pointer is never freed.
unsafe impl Send for Manager {}
unsafe impl Sync for Manager {}

extern "C" fn on_status(_data: *mut c_void, status: c_int) {
    STATUS.store(status, Ordering::Relaxed);
}

fn manager() -> Option<&'static Manager> {
    static M: OnceLock<Option<Manager>> = OnceLock::new();
    M.get_or_init(|| {
        // SAFETY: plain NDK calls; the manager lives for the process (never released).
        unsafe {
            let m = AThermal_acquireManager();
            if m.is_null() {
                return None;
            }
            STATUS.store(AThermal_getCurrentThermalStatus(m), Ordering::Relaxed);
            AThermal_registerThermalStatusListener(m, on_status, std::ptr::null_mut());
            Some(Manager(m))
        }
    })
    .as_ref()
}

/// The current `ATHERMAL_STATUS_*` (0 none … 6 shutdown, -1 error).
pub(crate) fn status() -> i32 {
    if manager().is_none() {
        return 0;
    }
    STATUS.load(Ordering::Relaxed)
}

/// How close to severe throttling the device will be in `forecast` seconds (1.0 = severe).
pub(crate) fn headroom(forecast: i32) -> Option<f32> {
    static CACHE: Mutex<Option<(Instant, i32, Option<f32>)>> = Mutex::new(None);
    let m = manager()?;
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, f, v)) = *cache {
        if f == forecast && at.elapsed() < Duration::from_millis(1500) {
            return v;
        }
    }
    // SAFETY: a live manager (above).
    let v = unsafe { AThermal_getThermalHeadroom(m.0, forecast) };
    let v = (!v.is_nan()).then_some(v);
    *cache = Some((Instant::now(), forecast, v));
    v
}

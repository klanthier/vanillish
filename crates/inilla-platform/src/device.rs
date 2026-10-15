//! The device: screen-awake, safe area, haptics, thermal state, memory headroom, backup
//! exclusion, identity. Every call is a cheap no-op off iOS.
//!
//! Vanillish: each call also has an Android arm (`crate::android::device`), and Android adds the
//! fold ([`fold`]), the display's frame rate ([`set_frame_rate`]), the orientation policy and
//! [`relaunch`] — no-ops on iOS and the desktop.

use std::ffi::c_void;
use std::path::Path;

/// Keep the screen from dimming and locking (`UIApplication.idleTimerDisabled`). A controller
/// player never touches the screen, so without this the phone locks mid-fight.
pub fn set_idle_timer_disabled(disabled: bool) {
    #[cfg(target_os = "ios")]
    ios::set_idle_timer_disabled(disabled);
    // Vanillish: `FLAG_KEEP_SCREEN_ON` on the game's window.
    #[cfg(target_os = "android")]
    crate::android::device::keep_screen_on(disabled);
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    let _ = disabled;
}

/// The safe-area insets of `ui_view` (the game's `UIView`, from Bevy's raw window handle) in
/// points: `[top, left, bottom, right]`. Zero off iOS or for a null view.
pub fn safe_area_insets(ui_view: *mut c_void) -> [f32; 4] {
    #[cfg(target_os = "ios")]
    {
        ios::safe_area_insets(ui_view)
    }
    // Vanillish: the display-cutout insets in dp; `ui_view` is not used (pass null).
    #[cfg(target_os = "android")]
    {
        let _ = ui_view;
        crate::android::device::safe_area().0
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        let _ = ui_view;
        [0.0; 4]
    }
}

/// A side of the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// Which side of the screen the camera housing (the notch or the Dynamic Island) is on, from the
/// interface orientation of `ui_view`'s window scene: landscape-right puts the top of the phone on
/// the left, landscape-left on the right. `None` in portrait, off iOS, or before a scene exists.
/// (The safe-area insets cannot say: in landscape iOS reports the same inset on both sides.)
pub fn camera_side(ui_view: *mut c_void) -> Option<Side> {
    #[cfg(target_os = "ios")]
    {
        match ios::interface_orientation(ui_view) {
            // UIInterfaceOrientationLandscapeRight: the home side is on the right.
            3 => Some(Side::Left),
            // UIInterfaceOrientationLandscapeLeft.
            4 => Some(Side::Right),
            _ => None,
        }
    }
    // Vanillish: Android's cutout insets are per side, so the side with the hole is the one with
    // an inset.
    #[cfg(target_os = "android")]
    {
        let _ = ui_view;
        crate::android::device::safe_area().1
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        let _ = ui_view;
        None
    }
}

/// Have UIKit re-read the supported orientations of the view controller behind `ui_view` (iOS
/// 16+), rotating away from one it no longer allows.
pub fn refresh_orientations(ui_view: *mut c_void) {
    #[cfg(target_os = "ios")]
    ios::refresh_orientations(ui_view);
    #[cfg(not(target_os = "ios"))]
    let _ = ui_view;
}

/// Taptic feedback styles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Haptic {
    Light,
    Medium,
    Heavy,
    /// A selection tick (a radial wedge, a slider step).
    Selection,
}

pub fn haptic(kind: Haptic) {
    #[cfg(target_os = "ios")]
    ios::haptic(kind);
    // Vanillish: the Vibrator's click/tick primitives.
    #[cfg(target_os = "android")]
    crate::android::device::haptic(kind);
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    let _ = kind;
}

/// `NSProcessInfo.thermalState`: how hard the phone is throttling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Thermal {
    #[default]
    Nominal,
    Fair,
    Serious,
    Critical,
}

pub fn thermal_state() -> Thermal {
    #[cfg(target_os = "ios")]
    {
        ios::thermal_state()
    }
    // Vanillish: `AThermal`'s status, kept current by a listener (an atomic load).
    #[cfg(target_os = "android")]
    {
        thermal_from_android(crate::android::thermal::status())
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        Thermal::Nominal
    }
}

/// `ATHERMAL_STATUS_*` → [`Thermal`]: light is "warm", moderate is where throttling begins.
pub fn thermal_from_android(status: i32) -> Thermal {
    match status {
        1 => Thermal::Fair,
        2 => Thermal::Serious,
        3..=6 => Thermal::Critical,
        _ => Thermal::Nominal,
    }
}

/// How close the device is to severe throttling in `forecast_secs` (`1.0` = severe), where the
/// OS says (Android's thermal headroom). `None` elsewhere.
pub fn thermal_headroom(forecast_secs: i32) -> Option<f32> {
    #[cfg(target_os = "android")]
    {
        crate::android::thermal::headroom(forecast_secs)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = forecast_secs;
        None
    }
}

/// Bytes the process can still allocate before iOS kills it (`os_proc_available_memory`).
pub fn available_memory() -> Option<u64> {
    #[cfg(target_os = "ios")]
    {
        extern "C" {
            fn os_proc_available_memory() -> usize;
        }
        // SAFETY: a libSystem query with no arguments, available since iOS 13.
        Some(unsafe { os_proc_available_memory() } as u64)
    }
    // Vanillish: system-wide room before the low-memory killer (Android has no per-app limit).
    #[cfg(target_os = "android")]
    {
        crate::android::device::available_memory()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        None
    }
}

/// `MemAvailable` from the text of `/proc/meminfo`, in bytes.
pub fn parse_mem_available(meminfo: &str) -> Option<u64> {
    meminfo.lines().find_map(|l| {
        let rest = l.strip_prefix("MemAvailable:")?;
        let kb: u64 = rest.trim().trim_end_matches("kB").trim().parse().ok()?;
        Some(kb * 1024)
    })
}

/// Keep `path` (and, for a folder, everything in it) out of iCloud/iTunes backups.
pub fn exclude_from_backup(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "ios")]
    {
        ios::exclude_from_backup(path)
    }
    #[cfg(not(target_os = "ios"))]
    {
        let _ = path;
        Ok(())
    }
}

/// `"iPhone16,2 · iOS 27.0"` (and `" · in LiveContainer"` as a guest), for logs and bug reports.
pub fn describe() -> String {
    #[cfg(target_os = "ios")]
    {
        match crate::paths::host() {
            crate::paths::Host::LiveContainer => format!("{} · in LiveContainer", ios::describe()),
            _ => ios::describe(),
        }
    }
    // Vanillish: maker, model, Android release and the SoC, from the system properties.
    #[cfg(target_os = "android")]
    {
        crate::android::device::describe()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        format!("{} {}", std::env::consts::OS, std::env::consts::ARCH)
    }
}

/// Open a URL in the system (a web page, the Settings app with `app-settings:`).
pub fn open_url(url: &str) {
    #[cfg(target_os = "ios")]
    ios::open_url(url);
    // Vanillish: an `ACTION_VIEW` intent; `app-settings:` opens the app's system settings page.
    #[cfg(target_os = "android")]
    {
        crate::android::device::open_url(url);
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    let _ = url;
}

/// Start the app over in a fresh process, where the OS allows an app to (Android). `false` where
/// it does not: the caller then exits and the player opens the app again.
pub fn relaunch() -> bool {
    #[cfg(target_os = "android")]
    {
        crate::android::device::relaunch()
    }
    #[cfg(not(target_os = "android"))]
    {
        false
    }
}

/// Ask the display to refresh at `hz` (`0` = its highest), so a 120 Hz panel runs the High preset
/// at 120 and settles to the frame cap's rate otherwise. A no-op where the OS picks by itself.
pub fn set_frame_rate(hz: f32) {
    #[cfg(target_os = "android")]
    crate::android::device::set_frame_rate(hz);
    #[cfg(not(target_os = "android"))]
    let _ = hz;
}

/// Which orientations the game may take.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Orientation {
    /// Landscape, either way up (the sensor picks).
    #[default]
    Landscape,
    /// Whatever the device is held in — on a foldable's near-square inner screen, portrait plays
    /// too.
    Any,
}

pub fn set_orientation(o: Orientation) {
    #[cfg(target_os = "android")]
    crate::android::device::set_orientation_mode(match o {
        Orientation::Landscape => 0,
        Orientation::Any => 1,
    });
    #[cfg(not(target_os = "android"))]
    let _ = o;
}

/// A foldable's hinge, as the OS reports it (Jetpack WindowManager's `FoldingFeature`).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Fold {
    pub state: FoldState,
    /// The hinge runs left to right across the window (the device is held like a laptop: the
    /// tabletop posture when half open), rather than top to bottom (a book).
    pub horizontal: bool,
    /// The hinge's bounds in the window, in logical pixels: `[left, top, right, bottom]`.
    pub bounds: [f32; 4],
    /// The fold splits the window into two areas (half open, or a physical gap).
    pub separating: bool,
    /// The hinge hides content (`OcclusionType.FULL`), rather than only creasing it.
    pub occluding: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FoldState {
    /// Fully open (180°).
    #[default]
    Flat,
    /// Somewhere between closed and flat: the posture a game can split around.
    HalfOpen,
}

impl Fold {
    /// Half open with the hinge horizontal: the laptop / "Flex mode" posture.
    pub fn tabletop(&self) -> bool {
        self.state == FoldState::HalfOpen && self.horizontal
    }

    /// Half open with the hinge vertical: held like a book.
    pub fn book(&self) -> bool {
        self.state == FoldState::HalfOpen && !self.horizontal
    }
}

/// `Bridge.fold()`'s array: `{present, halfOpen, horizontal, left, top, right, bottom, separating,
/// occluding}`. `None` when the window has no hinge across it (the cover screen, a closed device,
/// a phone that does not fold).
pub fn fold_from_floats(v: &[f32]) -> Option<Fold> {
    if v.len() < 9 || v[0] < 0.5 {
        return None;
    }
    Some(Fold {
        state: if v[1] > 0.5 {
            FoldState::HalfOpen
        } else {
            FoldState::Flat
        },
        horizontal: v[2] > 0.5,
        bounds: [v[3], v[4], v[5], v[6]],
        separating: v[7] > 0.5,
        occluding: v[8] > 0.5,
    })
}

/// The hinge across the game's window, if the device folds and the window spans it. Desktop runs
/// rehearse one with `$INILLA_FOLD=tabletop` / `book` (a hinge across the middle of the window,
/// sized when the window size is known: see the app's `inilla::fold`).
pub fn fold() -> Option<Fold> {
    #[cfg(target_os = "android")]
    {
        crate::android::fold::fold()
    }
    #[cfg(not(target_os = "android"))]
    {
        None
    }
}

#[cfg(target_os = "ios")]
mod ios {
    use std::cell::RefCell;
    use std::ffi::c_void;
    use std::path::Path;

    use objc2::encode::{Encode, Encoding};
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, Bool};
    use objc2::{msg_send, ClassType};
    use objc2_foundation::{NSNumber, NSString, NSURL};

    use super::{Haptic, Thermal};
    use crate::objc::{class, shared_application};

    pub(super) fn set_idle_timer_disabled(disabled: bool) {
        if let Some(app) = shared_application() {
            let _: () = unsafe { msg_send![&*app, setIdleTimerDisabled: Bool::new(disabled)] };
        }
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct UIEdgeInsets {
        top: f64,
        left: f64,
        bottom: f64,
        right: f64,
    }

    // SAFETY: matches UIKit's `UIEdgeInsets` (four CGFloats, which are f64 on arm64).
    unsafe impl Encode for UIEdgeInsets {
        const ENCODING: Encoding = Encoding::Struct(
            "UIEdgeInsets",
            &[f64::ENCODING, f64::ENCODING, f64::ENCODING, f64::ENCODING],
        );
    }

    pub(super) fn safe_area_insets(ui_view: *mut c_void) -> [f32; 4] {
        if ui_view.is_null() {
            return [0.0; 4];
        }
        // SAFETY: the pointer is the live UIView from winit's raw window handle.
        let view = unsafe { &*(ui_view as *const AnyObject) };
        let i: UIEdgeInsets = unsafe { msg_send![view, safeAreaInsets] };
        [i.top as f32, i.left as f32, i.bottom as f32, i.right as f32]
    }

    /// `ui_view.window.windowScene.interfaceOrientation` (iOS 13+), `0` (unknown) when any link
    /// of that chain is missing.
    pub(super) fn interface_orientation(ui_view: *mut c_void) -> isize {
        if ui_view.is_null() {
            return 0;
        }
        // SAFETY: the pointer is the live UIView from winit's raw window handle; `window`,
        // `windowScene` and `interfaceOrientation` are UIKit properties on iOS 13 and later.
        let view = unsafe { &*(ui_view as *const AnyObject) };
        let window: Option<Retained<AnyObject>> = unsafe { msg_send![view, window] };
        let Some(window) = window else {
            return 0;
        };
        let scene: Option<Retained<AnyObject>> = unsafe { msg_send![&*window, windowScene] };
        let Some(scene) = scene else {
            return 0;
        };
        unsafe { msg_send![&*scene, interfaceOrientation] }
    }

    pub(super) fn refresh_orientations(ui_view: *mut c_void) {
        if ui_view.is_null() {
            return;
        }
        // SAFETY: the pointer is the live UIView from winit's raw window handle; every message
        // below is a UIKit property or method that exists on iOS 16 and later.
        let view = unsafe { &*(ui_view as *const AnyObject) };
        let window: Option<Retained<AnyObject>> = unsafe { msg_send![view, window] };
        let Some(window) = window else {
            return;
        };
        let root: Option<Retained<AnyObject>> = unsafe { msg_send![&*window, rootViewController] };
        if let Some(root) = root {
            let _: () =
                unsafe { msg_send![&*root, setNeedsUpdateOfSupportedInterfaceOrientations] };
        }
    }

    thread_local! {
        static IMPACT: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
        static SELECTION: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
    }

    /// One generator per kind, created on first use and kept: creating one per tap adds latency.
    fn generator(
        slot: &'static std::thread::LocalKey<RefCell<Option<Retained<AnyObject>>>>,
        class_name: &str,
        f: impl FnOnce(&AnyObject),
    ) {
        slot.with(|g| {
            let mut g = g.borrow_mut();
            if g.is_none() {
                if let Some(cls) = class(class_name) {
                    let obj: Option<Retained<AnyObject>> = unsafe { msg_send![cls, new] };
                    *g = obj;
                }
            }
            if let Some(obj) = g.as_deref() {
                f(obj);
            }
        });
    }

    pub(super) fn haptic(kind: Haptic) {
        let intensity: f64 = match kind {
            Haptic::Light => 0.4,
            Haptic::Medium => 0.7,
            Haptic::Heavy => 1.0,
            Haptic::Selection => {
                generator(&SELECTION, "UISelectionFeedbackGenerator", |g| {
                    let _: () = unsafe { msg_send![g, selectionChanged] };
                });
                return;
            }
        };
        generator(&IMPACT, "UIImpactFeedbackGenerator", |g| {
            let _: () = unsafe { msg_send![g, impactOccurredWithIntensity: intensity] };
        });
    }

    pub(super) fn thermal_state() -> Thermal {
        let Some(cls) = class("NSProcessInfo") else {
            return Thermal::Nominal;
        };
        let info: Option<Retained<AnyObject>> = unsafe { msg_send![cls, processInfo] };
        let Some(info) = info else {
            return Thermal::Nominal;
        };
        let state: isize = unsafe { msg_send![&*info, thermalState] };
        match state {
            1 => Thermal::Fair,
            2 => Thermal::Serious,
            3 => Thermal::Critical,
            _ => Thermal::Nominal,
        }
    }

    extern "C" {
        static NSURLIsExcludedFromBackupKey: &'static NSString;
    }

    pub(super) fn exclude_from_backup(path: &Path) -> Result<(), String> {
        let p = NSString::from_str(&path.to_string_lossy());
        let url: Retained<NSURL> = unsafe { msg_send![NSURL::class(), fileURLWithPath: &*p] };
        let yes = NSNumber::new_bool(true);
        let mut err: *mut AnyObject = std::ptr::null_mut();
        // SAFETY: a Foundation constant, linked from Foundation.
        let key = unsafe { NSURLIsExcludedFromBackupKey };
        let ok: Bool =
            unsafe { msg_send![&*url, setResourceValue: &*yes, forKey: key, error: &mut err] };
        if ok.as_bool() {
            Ok(())
        } else {
            Err(format!("could not exclude {} from backup", path.display()))
        }
    }

    fn uname_machine() -> String {
        let mut u: libc::utsname = unsafe { std::mem::zeroed() };
        if unsafe { libc::uname(&mut u) } != 0 {
            return "unknown".into();
        }
        let bytes: Vec<u8> = u
            .machine
            .iter()
            .take_while(|c| **c != 0)
            .map(|c| *c as u8)
            .collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    pub(super) fn describe() -> String {
        let version = class("UIDevice")
            .and_then(|cls| {
                let dev: Option<Retained<AnyObject>> = unsafe { msg_send![cls, currentDevice] };
                dev
            })
            .and_then(|dev| {
                let v: Option<Retained<NSString>> = unsafe { msg_send![&*dev, systemVersion] };
                v
            })
            .map(|v| v.to_string())
            .unwrap_or_default();
        format!("{} · iOS {version}", uname_machine())
    }

    pub(super) fn open_url(url: &str) {
        let Some(app) = shared_application() else {
            return;
        };
        let s = NSString::from_str(url);
        let u: Option<Retained<NSURL>> = unsafe { msg_send![NSURL::class(), URLWithString: &*s] };
        let Some(u) = u else {
            return;
        };
        let Some(options): Option<Retained<AnyObject>> =
            class("NSDictionary").and_then(|cls| unsafe { msg_send![cls, dictionary] })
        else {
            return;
        };
        let done: Option<&block2::Block<dyn Fn(Bool)>> = None;
        let _: () =
            unsafe { msg_send![&*app, openURL: &*u, options: &*options, completionHandler: done] };
    }
}

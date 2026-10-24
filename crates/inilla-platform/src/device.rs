//! The device: screen-awake, safe area, haptics, thermal state, memory headroom, backup
//! exclusion, identity. Every call is a cheap no-op off iOS.
//!
//! Vanillish: each call also has an Android arm (`crate::android::device`), and Android adds the
//! fold ([`fold`]), the display's frame rate ([`set_frame_rate`]), the orientation policy, the
//! background session ([`set_session_service`]), the edges the system's swipes hold off
//! ([`set_gesture_exclusion`]), [`relaunch`] and whether the network is metered
//! ([`network_metered`]) — no-ops on iOS and the desktop, where `$INILLA_FOLD` rehearses a hinge
//! ([`fold_in_window`]). Windows (`crate::windows::device`) answers the screen-awake, memory,
//! identity, URL, relaunch and metered calls, and adds [`open_folder`].

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
    // Vanillish: a power request for the display and the system, held by the process.
    #[cfg(windows)]
    crate::windows::device::keep_awake(disabled);
    #[cfg(not(any(target_os = "ios", target_os = "android", windows)))]
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
    // Vanillish: the free physical memory, as on Android system-wide.
    #[cfg(windows)]
    {
        crate::windows::device::available_memory()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android", windows)))]
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
    // Vanillish: the Windows release and build and the CPU, from the registry ([`describe_windows`]).
    #[cfg(windows)]
    {
        crate::windows::device::describe()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android", windows)))]
    {
        format!("{} {}", std::env::consts::OS, std::env::consts::ARCH)
    }
}

/// Vanillish: `"Windows 11 Pro 24H2 (build 26100.1234) · x86_64 · AMD Ryzen 7 7840U"` from the
/// registry's `ProductName`, `DisplayVersion`, `CurrentBuildNumber` and `UBR`, the build's
/// architecture and the CPU's name, empty parts left out. `ProductName` still says "Windows 10"
/// on Windows 11, which is build 22000 and later.
pub fn describe_windows(
    product: &str,
    version: &str,
    build: &str,
    ubr: Option<u32>,
    arch: &str,
    cpu: &str,
) -> String {
    let build = build.trim();
    let mut name = product.trim().to_string();
    if build.parse::<u32>().is_ok_and(|b| b >= 22_000) {
        if let Some(edition) = name.strip_prefix("Windows 10") {
            name = format!("Windows 11{edition}");
        }
    }
    let version = version.trim();
    if !version.is_empty() {
        name = format!("{name} {version}");
    }
    if !build.is_empty() {
        name = match ubr {
            Some(ubr) => format!("{name} (build {build}.{ubr})"),
            None => format!("{name} (build {build})"),
        };
    }
    let cpu = cpu.split_whitespace().collect::<Vec<_>>().join(" ");
    [name, arch.to_string(), cpu]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
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
    // Vanillish: the default browser, for a web address only ([`is_web_url`]).
    #[cfg(windows)]
    crate::windows::device::open_url(url);
    #[cfg(not(any(target_os = "ios", target_os = "android", windows)))]
    let _ = url;
}

/// Vanillish: whether `url` is a web page's address, the one thing Windows' `ShellExecuteW` is
/// handed — given a path, it would run a program. `http://` or `https://` in any case, then no
/// space, quote or control character (a URL carries those percent-encoded).
pub fn is_web_url(url: &str) -> bool {
    let scheme = ["http://", "https://"].iter().any(|s| {
        url.len() > s.len() && url.as_bytes()[..s.len()].eq_ignore_ascii_case(s.as_bytes())
    });
    scheme
        && !url
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '"')
}

/// Vanillish (Windows): show the folder at `path` in the system's file manager (File Explorer).
/// `false` where there is none to ask, or it could not be opened.
pub fn open_folder(path: &std::path::Path) -> bool {
    #[cfg(windows)]
    {
        crate::windows::device::open_folder(path)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

/// The game has quit: close the app and end the process the way the OS wants it (Android: the
/// task closes and the process ends from the UI thread). A no-op elsewhere — the caller returns.
pub fn quit() {
    #[cfg(target_os = "android")]
    crate::android::call_bridge_void("quit", "()V", &[]);
}

/// Start the app over in a fresh process, where the OS allows an app to (Android; Vanillish:
/// Windows, the exe again with the same arguments). `false` where it does not: the caller then
/// exits and the player opens the app again.
pub fn relaunch() -> bool {
    #[cfg(target_os = "android")]
    {
        crate::android::device::relaunch()
    }
    #[cfg(windows)]
    {
        crate::windows::device::relaunch()
    }
    #[cfg(not(any(target_os = "android", windows)))]
    {
        false
    }
}

/// Vanillish: whether the network the device is on now costs by the byte — mobile data, or a
/// hotspot marked so (Android's `ConnectivityManager.isActiveNetworkMetered`; Windows' connection
/// cost, [`metered_from_windows`]). Quick connect starts a 5 GB download by itself only on
/// `Some(false)`. `None` where the OS does not say, or with no network; a desktop rehearses
/// either answer with `$INILLA_METERED=1` or `0`, which Windows asks first.
pub fn network_metered() -> Option<bool> {
    #[cfg(target_os = "android")]
    {
        crate::android::device::network_metered()
    }
    #[cfg(target_os = "ios")]
    {
        None
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        match std::env::var("INILLA_METERED").as_deref() {
            Ok("1") => Some(true),
            Ok("0") => Some(false),
            #[cfg(windows)]
            _ => crate::windows::device::network_metered(),
            #[cfg(not(windows))]
            _ => None,
        }
    }
}

/// Vanillish: Windows' `ConnectionCost` as [`network_metered`] answers it. `NetworkCostType`
/// `Unrestricted` (1) is free unless roaming or over the plan's data limit; `Fixed` (2) and
/// `Variable` (3) cost by the byte; `Unknown` (0) says nothing.
pub fn metered_from_windows(cost_type: i32, roaming: bool, over_data_limit: bool) -> Option<bool> {
    match cost_type {
        1 => Some(roaming || over_data_limit),
        2 | 3 => Some(true),
        _ => None,
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

/// The display's refresh rate now and the highest it offers at its resolution, in Hz, where the
/// OS says (Android). `None` elsewhere.
pub fn refresh_rate() -> Option<(f32, f32)> {
    #[cfg(target_os = "android")]
    {
        crate::android::device::refresh_rate()
    }
    #[cfg(not(target_os = "android"))]
    {
        None
    }
}

/// The OS's game mode, where it has one (Android's Game Dashboard): the player asked the system
/// for performance or battery life.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GameMode {
    #[default]
    Standard,
    Performance,
    Battery,
}

pub fn game_mode() -> GameMode {
    #[cfg(target_os = "android")]
    {
        match crate::android::device::game_mode() {
            2 => GameMode::Performance,
            3 => GameMode::Battery,
            _ => GameMode::Standard,
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        GameMode::Standard
    }
}

/// The system Back presses the Activity's back callback caught since the last call — Android's
/// stand-in for the Escape key iOS has none of. A safety net: with predictive back off (the
/// manifest), the key and the gesture reach the game as winit's key, and GameActivity takes it
/// before Android would fall back to the callback. Always 0 where there is no such thing.
pub fn take_back_presses() -> u32 {
    #[cfg(target_os = "android")]
    {
        crate::android::device::take_back_presses()
    }
    #[cfg(not(target_os = "android"))]
    {
        0
    }
}

/// Keep the app's process — and with it the game's connection to its server — alive while the app
/// is in the background, where the OS would otherwise freeze or reclaim it: Android's foreground
/// service, with the notification the OS requires (tapping it returns to the game). A no-op where
/// there is no such thing (iOS suspends a backgrounded app regardless).
pub fn set_session_service(on: bool) {
    #[cfg(target_os = "android")]
    crate::android::device::set_session_service(on);
    #[cfg(not(target_os = "android"))]
    let _ = on;
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

    /// The hinge as the HUD's screen model takes it ([`inilla_core::fold::Fold`]): the same
    /// logical-pixel bounds, as a rect.
    pub fn to_core(&self) -> inilla_core::fold::Fold {
        use inilla_core::fold::{Fold as CoreFold, FoldState as CoreState};
        let [l, t, r, b] = self.bounds;
        CoreFold {
            state: match self.state {
                FoldState::Flat => CoreState::Flat,
                FoldState::HalfOpen => CoreState::HalfOpened,
            },
            horizontal: self.horizontal,
            bounds: inilla_core::touch::Rect::new([l, t], [r, b]),
        }
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

/// The hinge across the game's window, if the device folds and the window spans it (Android's,
/// cached by the Java side as it changes). `None` elsewhere: a desktop run rehearses one through
/// [`fold_in_window`], which knows the window's size.
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

/// The hinge across a window `window` logical pixels big: on Android the OS's ([`fold`]); on a
/// desktop the rehearsal `$INILLA_FOLD` names, a hinge through the middle of the window — sized
/// from it, which is why the caller passes it:
///
/// | `$INILLA_FOLD` | rehearses |
/// |---|---|
/// | `half` (or `tabletop`) | half open, the hinge across: tabletop (Samsung's Flex mode) |
/// | `flat` | fully open, the hinge across: the inner screen lying flat |
/// | `book` | half open, the hinge down the middle: held like a book |
///
/// `INILLA_TOUCH=1 INILLA_FOLD=half` in a near-square window (the inner screen is about 954 × 859)
/// plays the tabletop split on a desktop. Never on iOS.
pub fn fold_in_window(window: [f32; 2]) -> Option<Fold> {
    #[cfg(target_os = "android")]
    {
        let _ = window;
        fold()
    }
    #[cfg(target_os = "ios")]
    {
        let _ = window;
        None
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        static SPEC: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        SPEC.get_or_init(|| std::env::var("INILLA_FOLD").ok())
            .as_deref()
            .and_then(|spec| rehearsed_fold(spec, window))
    }
}

/// The hinge `$INILLA_FOLD=spec` rehearses in a window `window` logical pixels big (see
/// [`fold_in_window`]): a line of no thickness through the middle, as a flexible screen's crease
/// reports itself. `None` for a spec it does not know, or an empty window.
pub fn rehearsed_fold(spec: &str, window: [f32; 2]) -> Option<Fold> {
    let [w, h] = window;
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let (state, horizontal) = match spec.trim().to_ascii_lowercase().as_str() {
        "half" | "tabletop" => (FoldState::HalfOpen, true),
        "flat" => (FoldState::Flat, true),
        "book" => (FoldState::HalfOpen, false),
        _ => return None,
    };
    let bounds = if horizontal {
        [0.0, h / 2.0, w, h / 2.0]
    } else {
        [w / 2.0, 0.0, w / 2.0, h]
    };
    Some(Fold {
        state,
        horizontal,
        bounds,
        separating: state == FoldState::HalfOpen,
        occluding: false,
    })
}

/// Where a swipe in from a screen edge stays the game's instead of starting the system's gesture
/// (Android's back swipe from either side): `rects` in logical pixels (dp), each
/// `[left, top, right, bottom]` in the window; empty clears them. Android honours at most 200 dp
/// of height per side edge, and never holds off the bottom edge's home gesture. A no-op elsewhere
/// (iOS defers every edge's gesture already: the app's `window_options`).
pub fn set_gesture_exclusion(rects: &[[f32; 4]]) {
    #[cfg(target_os = "android")]
    crate::android::device::set_gesture_exclusion(rects);
    #[cfg(not(target_os = "android"))]
    let _ = rects;
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

#[cfg(test)]
mod tests {
    use super::*;
    use inilla_core::fold::{classify, ScreenClass};

    /// A Galaxy Z Fold8 Ultra's inner screen in landscape, in dp.
    const INNER: [f32; 2] = [953.9, 859.4];

    #[test]
    fn windows_names_itself_by_its_build() {
        assert_eq!(
            describe_windows(
                "Windows 10 Pro",
                "24H2",
                "26100",
                Some(1234),
                "x86_64",
                "  AMD Ryzen 7 7840U w/ Radeon  780M Graphics   "
            ),
            "Windows 11 Pro 24H2 (build 26100.1234) · x86_64 · AMD Ryzen 7 7840U w/ Radeon 780M \
             Graphics"
        );
        // Windows 10 stays Windows 10; what the registry lacks is left out.
        assert_eq!(
            describe_windows("Windows 10 Home", "22H2", "19045", None, "x86_64", ""),
            "Windows 10 Home 22H2 (build 19045) · x86_64"
        );
        assert_eq!(
            describe_windows("Windows 10 Pro", "", "", None, "aarch64", ""),
            "Windows 10 Pro · aarch64"
        );
        assert_eq!(
            describe_windows("Windows 11 Home", "25H2", "26200", Some(7), "x86_64", "x"),
            "Windows 11 Home 25H2 (build 26200.7) · x86_64 · x"
        );
    }

    #[test]
    fn windows_hands_the_shell_only_web_addresses() {
        assert!(is_web_url("https://github.com/klanthier/vanillish"));
        assert!(is_web_url("HTTP://192.168.1.20:8099/server.json"));
        assert!(!is_web_url("https://"));
        assert!(!is_web_url(r"C:\Windows\System32\calc.exe"));
        assert!(!is_web_url("file:///C:/Windows/System32/calc.exe"));
        assert!(!is_web_url("ms-settings:privacy"));
        assert!(!is_web_url("https://x\" -p"));
        assert!(!is_web_url("https://x y"));
        assert!(!is_web_url("https://x\ny"));
    }

    #[test]
    fn a_windows_connection_costs_by_its_type() {
        assert_eq!(metered_from_windows(1, false, false), Some(false));
        assert_eq!(metered_from_windows(1, true, false), Some(true));
        assert_eq!(metered_from_windows(1, false, true), Some(true));
        assert_eq!(metered_from_windows(2, false, false), Some(true));
        assert_eq!(metered_from_windows(3, false, false), Some(true));
        assert_eq!(metered_from_windows(0, true, true), None);
    }

    #[test]
    fn the_bridges_fold_reaches_the_hud_as_the_same_hinge() {
        let v = [1.0, 1.0, 1.0, 0.0, 429.7, 953.9, 429.7, 1.0, 0.0];
        let f = fold_from_floats(&v).expect("a hinge");
        assert!(f.tabletop() && !f.book());
        let core = f.to_core();
        assert_eq!(core.bounds.min, [0.0, 429.7]);
        assert_eq!(core.bounds.max, [953.9, 429.7]);
        assert_eq!(classify(INNER, Some(&core)), ScreenClass::Tabletop);
        assert_eq!(fold_from_floats(&[0.0; 9]), None, "no hinge in the window");
        assert_eq!(fold_from_floats(&[1.0; 4]), None, "a short array");
    }

    #[test]
    fn the_desktop_rehearses_each_posture_across_the_middle_of_its_window() {
        let half = rehearsed_fold("half", INNER).unwrap();
        assert!(half.tabletop());
        assert_eq!(half.bounds, [0.0, INNER[1] / 2.0, INNER[0], INNER[1] / 2.0]);
        assert_eq!(
            classify(INNER, Some(&half.to_core())),
            ScreenClass::Tabletop
        );
        assert_eq!(rehearsed_fold("tabletop", INNER), Some(half));
        let flat = rehearsed_fold("flat", INNER).unwrap();
        assert_eq!(flat.state, FoldState::Flat);
        assert_eq!(classify(INNER, Some(&flat.to_core())), ScreenClass::Tablet);
        let book = rehearsed_fold(" Book ", INNER).unwrap();
        assert!(book.book());
        assert_eq!(book.bounds[0], INNER[0] / 2.0);
        assert_eq!(classify(INNER, Some(&book.to_core())), ScreenClass::Tablet);
        assert_eq!(rehearsed_fold("open", INNER), None);
        assert_eq!(rehearsed_fold("half", [0.0, 0.0]), None);
    }
}

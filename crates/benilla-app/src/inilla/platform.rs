//! **The iOS host**, as Bevy systems: the window's iOS options, the screen kept awake, the audio
//! session, the safe area and the Dynamic Island, the on-screen keyboard, the frame cap, and the
//! iNilla settings that land on benilla CVars (render scale, UI scale, the quality preset).
//!
//! Vanillish: the same systems run on Android, through `inilla_platform`'s Android arms; what only
//! Android has (the soft keyboard's text, Back, the native window, the lifecycle, the frame-rate
//! vote) is `inilla::android`.

use std::time::{Duration, Instant};

use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::prelude::*;
use bevy::window::{PrimaryWindow, ScreenEdge};

use inilla_core::settings::{InillaSettings, Quality};
use inilla_core::touch::Screen;
use inilla_platform::device::Side;
use inilla_platform::{audio, device};

use super::screen::HudScreen;
use super::Inilla;
use crate::char_select::ClientState;
use crate::cvars::Cvars;
use crate::ui_script::UiKeyboardCapture;

/// The safe-area insets in points, `[top, left, bottom, right]` — the notch / Dynamic Island in
/// landscape, the home indicator — and which side the camera housing is on.
#[derive(Resource, Default, Clone, Copy, PartialEq)]
pub(crate) struct SafeArea(pub(crate) [f32; 4], pub(crate) Option<Side>);

impl SafeArea {
    /// The screen the HUD lays out on: the window, its safe area, and the strip beside the camera
    /// housing where the system buttons go.
    pub(crate) fn screen(&self, width: f32, height: f32) -> Screen {
        let s = Screen {
            safe: self.0,
            ..Screen::new(width, height)
        };
        match self.1 {
            Some(side) => s.with_camera_on_left(side == Side::Left),
            None => s,
        }
    }
}

/// The on-screen keyboard was asked for by a HUD button (glue screens, which have no focus event).
#[derive(Resource, Default)]
pub(crate) struct KeyboardRequest(pub(crate) bool);

/// Vanillish: what the focused text field holds, set each frame by the screen that owns it, so the
/// Android soft keyboard opens as the right kind (a password is never learned or suggested; an
/// address gets the URL layout). Reset to `Text` every frame before the screens run.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ImePurpose {
    #[default]
    Text,
    Password,
    Url,
}

/// Vanillish: system Back presses (Android's key or gesture) not yet handled this frame. The
/// screens take theirs first (close a form, a panel); what is left becomes the game's Escape.
#[derive(Resource, Default, Debug)]
pub(crate) struct BackPresses(pub(crate) u32);

/// Keys a HUD button presses for the player: the chat bar's Send (`Enter`) and Close (`Escape`),
/// which the iOS keyboard has no key for. Each goes down in one frame and up in the next, ahead of
/// Bevy's input systems, so the text law sees an ordinary key.
#[derive(Resource, Default)]
pub(crate) struct SyntheticKeys {
    queued: Vec<KeyCode>,
    down: Vec<KeyCode>,
}

impl SyntheticKeys {
    pub(crate) fn tap(&mut self, key: KeyCode) {
        self.queued.push(key);
    }

    /// Vanillish: an Enter or an Escape waits to go down — the line being typed is about to be sent
    /// or dropped, so the word the Android keyboard is still composing must reach the box first.
    #[cfg(target_os = "android")]
    pub(crate) fn ends_line(&self) -> bool {
        self.queued
            .iter()
            .any(|k| matches!(k, KeyCode::Enter | KeyCode::Escape))
    }
}

pub(crate) struct PlatformPlugin;

impl Plugin for PlatformPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SafeArea>()
            .init_resource::<IslandInset>()
            .init_resource::<KeyboardRequest>()
            .init_resource::<ImePurpose>()
            .init_resource::<BackPresses>()
            .init_resource::<SyntheticKeys>()
            .add_systems(Startup, window_options)
            .add_systems(
                PreUpdate,
                (return_key_is_enter, synthetic_keys)
                    .chain()
                    .before(bevy::input::InputSystems),
            )
            .add_systems(
                Update,
                (
                    track_safe_area,
                    lock_landscape,
                    keep_screen_awake,
                    audio_session,
                    keyboard_policy.after(crate::ui_script::UiInput),
                    apply_cvar_settings,
                    compact_stock_ui.after(crate::ui_script::UiInput),
                    clear_the_island.after(crate::ui_script::UiInput),
                ),
            )
            .add_systems(Last, frame_cap);
    }
}

/// Full screen as a game wants it: no home indicator, no status bar, and the screen edges defer
/// the system gestures (a swipe from an edge shows the grabber first instead of leaving the game).
///
/// Vanillish: nothing here on Android — the Activity does the same in Java (immersive, drawn under
/// the camera cut-out, the keyboard floating over the game rather than resizing it).
fn window_options(mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    if !cfg!(target_os = "ios") {
        return;
    }
    for mut w in &mut windows {
        w.prefers_home_indicator_hidden = true;
        w.prefers_status_bar_hidden = true;
        w.preferred_screen_edges_deferring_system_gestures = ScreenEdge::All;
        // iNilla reads raw touches; UIKit's recognisers would only steal them.
        w.recognize_pinch_gesture = false;
        w.recognize_rotation_gesture = false;
        w.recognize_doubletap_gesture = false;
        w.recognize_pan_gesture = None;
    }
}

/// Landscape only, held by the view controller itself: the installed app's `Info.plist` already
/// says so, but a host that runs iNilla inside itself (LiveContainer) applies its own plist,
/// which rotates to portrait.
#[cfg(target_os = "ios")]
fn lock_landscape(
    windows: Query<(Entity, &bevy::window::RawHandleWrapper), With<PrimaryWindow>>,
    _main: bevy::ecs::system::NonSendMarker,
    mut done: Local<bool>,
) {
    use winit::platform::ios::{ValidOrientations, WindowExtIOS};
    if *done {
        return;
    }
    let Ok((entity, handle)) = windows.single() else {
        return;
    };
    *done = bevy::winit::WINIT_WINDOWS.with_borrow(|winit_windows| {
        winit_windows
            .get_window(entity)
            .map(|window| window.set_valid_orientations(ValidOrientations::Landscape))
            .is_some()
    });
    // winit applies the mask with `attemptRotationToDeviceOrientation`, a no-op since iOS 16.
    if !*done {
        return;
    }
    if let raw_window_handle::RawWindowHandle::UiKit(h) = handle.get_window_handle() {
        device::refresh_orientations(h.ui_view.as_ptr());
    }
}

#[cfg(not(target_os = "ios"))]
fn lock_landscape() {}

/// The iOS keyboard's Return arrives as the text `"\n"` with no key code; benilla's text law keys
/// submit on `Enter`. Rewrite it before Bevy's keyboard system sees it.
///
/// Vanillish: Android's soft keyboard never reaches winit (the IME bridge in `inilla::android`
/// turns its newline into Enter), and a hardware keyboard's Enter is rewritten there too.
fn return_key_is_enter(mut keys: ResMut<Messages<KeyboardInput>>) {
    if !cfg!(target_os = "ios") {
        return;
    }
    let needs = keys.iter_current_update_messages().any(|k| {
        matches!(k.text.as_deref(), Some("\n") | Some("\r")) || is_newline_key(&k.logical_key)
    });
    if !needs {
        return;
    }
    // Messages cannot be edited in place: take this update's, drop the buffer (last frame's half
    // has been read by every reader already) and write this update's back, rewritten.
    let current: Vec<KeyboardInput> = keys.iter_current_update_messages().cloned().collect();
    keys.drain().for_each(drop);
    for mut k in current {
        if matches!(k.text.as_deref(), Some("\n") | Some("\r")) || is_newline_key(&k.logical_key) {
            k.key_code = KeyCode::Enter;
            k.logical_key = Key::Enter;
            k.text = None;
        }
        keys.write(k);
    }
}

pub(super) fn synthetic_keys(
    mut keys: ResMut<SyntheticKeys>,
    mut out: ResMut<Messages<KeyboardInput>>,
    windows: Query<Entity, With<PrimaryWindow>>,
) {
    if keys.queued.is_empty() && keys.down.is_empty() {
        return;
    }
    let Ok(window) = windows.single() else {
        return;
    };
    let logical = |k: KeyCode| match k {
        KeyCode::Enter => Key::Enter,
        KeyCode::Escape => Key::Escape,
        // Vanillish: the Android keyboard's tab (a hardware keyboard typing into its buffer).
        KeyCode::Tab => Key::Tab,
        _ => Key::Unidentified(bevy::input::keyboard::NativeKey::Unidentified),
    };
    for key in std::mem::take(&mut keys.down) {
        out.write(KeyboardInput {
            key_code: key,
            logical_key: logical(key),
            state: bevy::input::ButtonState::Released,
            text: None,
            repeat: false,
            window,
        });
    }
    for key in std::mem::take(&mut keys.queued) {
        out.write(KeyboardInput {
            key_code: key,
            logical_key: logical(key),
            state: bevy::input::ButtonState::Pressed,
            text: None,
            repeat: false,
            window,
        });
        keys.down.push(key);
    }
}

fn is_newline_key(k: &Key) -> bool {
    matches!(k, Key::Character(c) if c.as_str() == "\n" || c.as_str() == "\r")
}

/// The safe area, once a second. Vanillish: on Android also at once when the screen changes (a
/// fold moves the game to the other panel, with its own cut-out), and then every quarter second
/// for a second — the new insets can reach the Java side a moment after the new size.
fn track_safe_area(
    windows: Query<&bevy::window::RawHandleWrapper, With<PrimaryWindow>>,
    hud: Res<HudScreen>,
    mut area: ResMut<SafeArea>,
    time: Res<Time<Real>>,
    mut next: Local<f64>,
    mut quick: Local<u8>,
) {
    let now = time.elapsed_secs_f64();
    if !cfg!(target_os = "ios") && hud.is_changed() {
        *next = now;
        *quick = 4;
    }
    if now < *next {
        return;
    }
    *next = now + 1.0;
    if *quick > 0 {
        *quick -= 1;
        *next = now + 0.25;
    }
    // Vanillish: Android's cut-out insets are dp already, Bevy's logical pixels; no view needed.
    #[cfg(target_os = "android")]
    {
        let _ = &windows;
        let insets = device::safe_area_insets(std::ptr::null_mut());
        if area.0 != insets {
            area.0 = insets;
        }
        let side = device::camera_side(std::ptr::null_mut());
        if area.1 != side {
            area.1 = side;
        }
    }
    #[cfg(target_os = "ios")]
    {
        let Ok(handle) = windows.single() else {
            return;
        };
        if let raw_window_handle::RawWindowHandle::UiKit(h) = handle.get_window_handle() {
            let insets = device::safe_area_insets(h.ui_view.as_ptr());
            if area.0 != insets {
                area.0 = insets;
            }
            let side = device::camera_side(h.ui_view.as_ptr());
            if area.1 != side {
                area.1 = side;
            }
        }
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        let _ = (&windows, &mut area);
    }
}

/// Keep the screen on in the world (the setting), and — Vanillish — while a data job runs: a
/// 5 GB download or import outlives the screen timeout, and a locked phone suspends the app.
fn keep_screen_awake(
    state: Res<Inilla>,
    client: Res<State<ClientState>>,
    data: Res<super::data::DataState>,
    mut busy: Local<bool>,
    mut last: Local<Option<bool>>,
) {
    // A job starts and ends through a mutable borrow of the data state, so its busy flag can only
    // move on a frame where the resource changed.
    if data.is_changed() {
        *busy = data.busy().is_some();
    }
    let want = (state.settings.keep_screen_on && *client.get() == ClientState::InWorld) || *busy;
    if *last != Some(want) {
        device::set_idle_timer_disabled(want);
        *last = Some(want);
    }
}

fn audio_session(state: Res<Inilla>, mut last: Local<Option<(bool, bool)>>) {
    let want = (
        state.settings.mix_with_other_audio,
        state.settings.ignore_silent_switch,
    );
    if *last == Some(want) {
        return;
    }
    *last = Some(want);
    if let Err(e) = audio::configure_session(want.0, want.1) {
        warn!("inilla: audio session: {e}");
    }
}

/// Show the keyboard while a text box has focus (chat, the stock popups) or a HUD button asked.
///
/// Vanillish: an iNilla screen's own field (the launcher's host and account, the setup's address)
/// asks too. Its focus is egui's, which no stock edit box sees; the egui context remembers it from
/// the last pass, and only a running lane has a live one.
fn keyboard_policy(
    state: Res<Inilla>,
    capture: Res<UiKeyboardCapture>,
    request: Res<KeyboardRequest>,
    lane: Res<super::EguiWanted>,
    mut egui: Query<&mut bevy_egui::EguiContext, With<bevy_egui::PrimaryEguiContext>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut last: Local<bool>,
) {
    if !super::touch_device() {
        return;
    }
    // A read: `get_mut` is the only getter bevy_egui offers without its `immutable_ctx` feature.
    let egui_typing = lane.0
        && egui.iter_mut().any(|mut ctx| {
            ctx.bypass_change_detection()
                .get_mut()
                .wants_keyboard_input()
        });
    let want = request.0 || (state.settings.auto_keyboard && (capture.typing || egui_typing));
    if want == *last {
        return;
    }
    *last = want;
    for mut w in &mut windows {
        w.ime_enabled = want;
    }
}

/// The interface scale for the screen the game is on: iNilla's rule on a phone; Vanillish: on a
/// foldable's inner screen and on a tabletop, `fold::ui_scale`'s (the 1024-unit main bar on the
/// near-square screen; about 600 units of `UIParent` above the hinge, near a phone's 614).
fn applied_ui_scale(settings: &InillaSettings, hud: &HudScreen) -> f32 {
    let [w, h] = hud.screen.size;
    let world_h = hud.screen.world.map_or(h, |r| r.size()[1]);
    inilla_core::fold::ui_scale(settings, hud.class, [w, h], world_h)
}

/// iNilla settings → benilla CVars, when they move.
fn apply_cvar_settings(
    state: Res<Inilla>,
    hud: Res<HudScreen>,
    mut cvars: ResMut<Cvars>,
    mut last: Local<Option<(u32, u32)>>,
    mut quality: Local<Option<Quality>>,
) {
    if !super::touch_device() {
        return;
    }
    let ui = applied_ui_scale(&state.settings, &hud);
    let key = (
        (state.settings.render_scale * 100.0).round() as u32,
        (ui * 100.0).round() as u32,
    );
    if *last != Some(key) {
        *last = Some(key);
        cvars.set(
            "renderScale",
            &format!("{:.2}", state.settings.render_scale.clamp(0.5, 1.0)),
        );
        cvars.set("useUiScale", "1");
        cvars.set("uiScale", &format!("{ui:.2}"));
    }
    // The preset writes its CVars when the player picks it: not on every launch — a detail the
    // player then changed in the stock options must survive a restart — and (Vanillish) not when
    // the screen changes either, which moves the UI scale on every fold.
    let q = state.settings.quality;
    if *quality != Some(q) {
        let first = quality.is_none();
        *quality = Some(q);
        if !first {
            for (name, value) in q.cvars() {
                cvars.set(name, value);
            }
        }
    }
}

/// The preset's frame cap (`0` = the display's rate). Sleeping at the end of the frame is crude,
/// but it is the one limiter that works the same on every platform, and at 30 fps it halves the
/// work the phone does — which is the whole point of the battery preset.
///
/// Vanillish: Android also asks the display for the cap's rate (the frame-rate vote in
/// `inilla::android`). Where the panel then runs at it, presenting (Fifo) already paces every
/// frame, and a sleep on top only shifts frames against the refresh — two land in one interval,
/// none in the next. So while the panel reports the cap's rate, and the frames measure as paced,
/// the sleep stands down ([`Pacing`]).
fn frame_cap(state: Res<Inilla>, mut last: Local<Option<Instant>>, mut pacing: Local<Pacing>) {
    if !super::touch_device() {
        return;
    }
    let cap = state.settings.quality.fps_cap();
    let now = Instant::now();
    if cap > 0 {
        let budget = Duration::from_secs_f64(1.0 / f64::from(cap));
        if let Some(prev) = *last {
            let spent = now.duration_since(prev);
            let paced = cfg!(target_os = "android")
                && pacing.display_paces(cap, spent, now, || {
                    device::refresh_rate().map_or(0.0, |(current, _)| current)
                });
            if spent < budget && !paced {
                std::thread::sleep(budget - spent);
            }
        }
    }
    *last = Some(Instant::now());
}

/// Vanillish: does the display already pace frames at the cap (Android)? The panel's rate is read
/// once a second (a Java call). While it is at or under the cap the sleep stands down, and the
/// unslept frame interval is measured: should frames still come clearly faster than the cap (a
/// rate override that throttles the app's vsync but not its presents), the report is not trusted
/// for a while and the sleep caps again.
#[derive(Default)]
struct Pacing {
    cap: u32,
    display_hz: f32,
    read_at: Option<Instant>,
    /// The smoothed frame interval while not sleeping (s), and how many frames it has seen.
    interval: f64,
    samples: u32,
    distrust_until: Option<Instant>,
}

impl Pacing {
    /// Frames to measure before judging the display's pacing.
    const SAMPLES: u32 = 30;
    /// How long a disproved report is ignored.
    const DISTRUST: Duration = Duration::from_secs(10);

    /// `display_hz` reads the panel's current rate (`0` = unknown).
    fn display_paces(
        &mut self,
        cap: u32,
        spent: Duration,
        now: Instant,
        display_hz: impl FnOnce() -> f32,
    ) -> bool {
        if self.cap != cap {
            *self = Pacing { cap, ..default() };
        }
        if self
            .read_at
            .is_none_or(|t| now.duration_since(t) >= Duration::from_secs(1))
        {
            self.display_hz = display_hz();
            self.read_at = Some(now);
        }
        let at_cap = self.display_hz > 0.0 && self.display_hz <= cap as f32 * 1.05;
        if !at_cap || self.distrust_until.is_some_and(|t| now < t) {
            self.interval = 0.0;
            self.samples = 0;
            return false;
        }
        // Not sleeping, so `spent` is the whole frame; a hitch is clamped so it cannot pass for
        // pacing on its own.
        let budget = 1.0 / f64::from(cap);
        let s = spent.as_secs_f64().min(2.0 * budget);
        self.interval = if self.samples == 0 {
            s
        } else {
            self.interval * 0.9 + s * 0.1
        };
        self.samples = self.samples.saturating_add(1);
        if self.samples >= Self::SAMPLES && self.interval < 0.8 * budget {
            self.distrust_until = Some(now + Self::DISTRUST);
            self.interval = 0.0;
            self.samples = 0;
            return false;
        }
        true
    }
}

#[cfg(test)]
mod pacing_tests {
    use super::*;

    /// `frames` unslept frames `interval` apart under a panel at `hz`; what the last one decided.
    fn run(p: &mut Pacing, t: &mut Instant, cap: u32, hz: f32, interval: f64, frames: u32) -> bool {
        let mut paced = false;
        for _ in 0..frames {
            *t += Duration::from_secs_f64(interval);
            paced = p.display_paces(cap, Duration::from_secs_f64(interval), *t, || hz);
        }
        paced
    }

    #[test]
    fn a_panel_at_the_cap_pacing_the_frames_stands_the_sleep_down() {
        let (mut p, mut t) = (Pacing::default(), Instant::now());
        assert!(run(&mut p, &mut t, 60, 60.0, 1.0 / 60.0, 300));
        // A panel under the cap paces too (the frames only come slower).
        assert!(run(&mut p, &mut t, 60, 30.0, 1.0 / 30.0, 300));
    }

    #[test]
    fn a_panel_over_the_cap_or_unknown_keeps_the_sleep() {
        let (mut p, mut t) = (Pacing::default(), Instant::now());
        assert!(!run(&mut p, &mut t, 60, 120.0, 1.0 / 120.0, 10));
        let mut p = Pacing::default();
        assert!(!run(&mut p, &mut t, 60, 0.0, 1.0 / 60.0, 10));
    }

    #[test]
    fn frames_faster_than_the_reported_rate_are_not_trusted_for_a_while() {
        let (mut p, mut t) = (Pacing::default(), Instant::now());
        // The panel says 60, frames come at 120: trusted until measured, then not.
        assert!(run(
            &mut p,
            &mut t,
            60,
            60.0,
            1.0 / 120.0,
            Pacing::SAMPLES - 1
        ));
        assert!(!run(&mut p, &mut t, 60, 60.0, 1.0 / 120.0, 1));
        // While distrusted, frames that do come paced change nothing...
        assert!(!run(&mut p, &mut t, 60, 60.0, 1.0 / 60.0, 300));
        // ...until the distrust runs out.
        t += Pacing::DISTRUST;
        assert!(run(&mut p, &mut t, 60, 60.0, 1.0 / 60.0, 1));
    }

    #[test]
    fn a_new_cap_starts_over() {
        let (mut p, mut t) = (Pacing::default(), Instant::now());
        run(&mut p, &mut t, 60, 60.0, 1.0 / 120.0, Pacing::SAMPLES);
        assert!(p.distrust_until.is_some());
        assert!(run(&mut p, &mut t, 30, 30.0, 1.0 / 30.0, 1));
    }
}

/// The main action bar's gryphons (`MainMenuBarLeftEndCap`/`RightEndCap`) are 128 units of
/// decoration at each end — on a phone, exactly the corners the thumbs and the HUD use. Hidden or
/// shown every couple of seconds so a `/reload` (a fresh interface) gets the setting too.
fn compact_stock_ui(
    state: Res<Inilla>,
    script: Option<NonSend<benilla_ui::script::UiScript>>,
    client: Res<State<ClientState>>,
    time: Res<Time<Real>>,
    mut next: Local<f64>,
) {
    let now = time.elapsed_secs_f64();
    if now < *next || *client.get() != ClientState::InWorld {
        return;
    }
    *next = now + 2.0;
    let Some(script) = script else {
        return;
    };
    let verb = if state.settings.compact_stock_ui && super::touch_device() {
        "Hide"
    } else {
        "Show"
    };
    crate::ui_script::run_or_warn(
        &script,
        &format!(
            "if MainMenuBarLeftEndCap then MainMenuBarLeftEndCap:{verb}() end \
             if MainMenuBarRightEndCap then MainMenuBarRightEndCap:{verb}() end"
        ),
    );
}

/// When to next check the stock interface's inset (real seconds).
#[derive(Resource, Default)]
struct IslandInset {
    next: f64,
}

/// Keep the stock windows clear of the Dynamic Island: `UIParent` is anchored to the screen with
/// the safe-area inset on the camera's side only, so the bags, a vendor and the quest log stop
/// short of it while the 3D world, which is not `UIParent`'s, still runs edge to edge. The applied
/// inset is kept in the VM, so a reloaded interface gets it again; an interface never inset is
/// left as the stock anchors it.
///
/// Vanillish: two more edges. A cut-out on the top edge (a foldable's inner screen in landscape
/// can put its camera there) insets the top. On a tabletop the interface ends at the hinge with
/// the world ([`HudScreen`]'s `world`): `UIParent` is the half above it, and the stock windows,
/// the action bars and the chat with it. Runs at once when the screen changes (a fold, the split).
#[allow(clippy::too_many_arguments)]
fn clear_the_island(
    state: Res<Inilla>,
    script: Option<NonSend<benilla_ui::script::UiScript>>,
    safe: Res<SafeArea>,
    hud: Res<HudScreen>,
    ui_scale: Option<Res<crate::ui_script::UiScaleCvar>>,
    client: Res<State<ClientState>>,
    time: Res<Time<Real>>,
    mut inset: ResMut<IslandInset>,
) {
    let now = time.elapsed_secs_f64();
    if *client.get() != ClientState::InWorld || (now < inset.next && !hud.is_changed()) {
        return;
    }
    inset.next = now + 0.5;
    let Some(script) = script else {
        return;
    };
    let [_, height] = hud.screen.size;
    if height <= 0.0 {
        return;
    }
    let seam = crate::ui_script::seam_scale(height);
    let on = state.settings.island_margin && super::touch_device();
    let points = |side: Side, inset: f32| {
        if on && safe.1 == Some(side) {
            inset
        } else {
            0.0
        }
    };
    // Root units (768 to the window's height) …
    let left = points(Side::Left, safe.0[1]) / seam;
    let right = points(Side::Right, safe.0[3]) / seam;
    // (iOS reports no top inset in landscape; iNilla never used one.)
    let top = if on && !cfg!(target_os = "ios") {
        safe.0[0] / seam
    } else {
        0.0
    };
    let bottom = hud
        .screen
        .world
        .map_or(0.0, |w| (height - w.max[1]).max(0.0) / seam);
    // … in `UIParent`'s own units: the resolver multiplies an anchor's offsets by the frame's
    // effective scale (`benilla-ui` `anchor_resolve_x`), which for `UIParent` is `uiScale` as the
    // client applies it (`fold::ui_parent_scale`: capped below 4:3, floored at 0.64). That is the
    // one `apply_cvar_settings` asks for on a touch device, else the player's own `uiScale`. iOS
    // keeps the root-unit inset it has shipped with until it is measured on a device.
    let requested = if super::touch_device() {
        applied_ui_scale(&state.settings, &hud)
    } else {
        ui_scale.map_or(crate::ui_script::DEFAULT_UI_SCALE, |s| s.0)
    };
    let scale = inilla_core::fold::ui_parent_scale(requested, hud.screen.size);
    let unit = if cfg!(target_os = "ios") { 1.0 } else { scale };
    let [left, right, top, bottom] = [left, right, top, bottom].map(|v| v / unit);
    let key = if [left, right, top, bottom].iter().all(|v| *v < 0.05) {
        "0".to_string()
    } else {
        format!("{left:.1}:{right:.1}:{top:.1}:{bottom:.1}:{scale:.2}")
    };
    crate::ui_script::run_or_warn(&script, &island_chunk(&key, left, top, right, bottom));
}

/// The Lua that seats `UIParent` inside the insets (`UIParent`'s units; `key` names them, `"0"`
/// none), once per change.
///
/// Vanillish: with an inset, the stock open-bag stack measures `UIParent` rather than the screen.
/// `updateContainerFrameAnchors` starts a new column of bags once the stack passes
/// `GetScreenHeight()`, which answers the screen root's height, not `UIParent`'s; on a tabletop
/// `UIParent` is half of it, and a stack as tall as the whole screen ran off its top. Only that
/// function is wrapped — the rest of the interface (the fullscreen quads, the world map) needs the
/// root's size — and only while an inset is applied; re-wrapped after a reload, and run again
/// after a re-anchor so open bags wrap again. `GetTop`/`GetBottom` answer in `UIParent`'s units;
/// `GetHeight` would not (it answers the root units its rect spans).
fn island_chunk(key: &str, left: f32, top: f32, right: f32, bottom: f32) -> String {
    format!(
        "if updateContainerFrameAnchors and updateContainerFrameAnchors ~= INILLA_BAG_ANCHORS then \
           local stock = updateContainerFrameAnchors \
           INILLA_BAG_ANCHORS = function() \
             if (INILLA_INSET or \"0\") == \"0\" or not UIParent or not UIParent:GetTop() then \
               return stock() \
             end \
             local screen = GetScreenHeight \
             GetScreenHeight = function() return UIParent:GetTop() - UIParent:GetBottom() end \
             local ok, err = pcall(stock) \
             GetScreenHeight = screen \
             if not ok then error(err, 0) end \
           end \
           updateContainerFrameAnchors = INILLA_BAG_ANCHORS \
         end \
         if (INILLA_INSET or \"0\") ~= \"{key}\" and UIParent then \
           UIParent:ClearAllPoints() \
           UIParent:SetPoint(\"TOPLEFT\", nil, \"TOPLEFT\", {left:.1}, {t:.1}) \
           UIParent:SetPoint(\"BOTTOMRIGHT\", nil, \"BOTTOMRIGHT\", {r:.1}, {bottom:.1}) \
           INILLA_INSET = \"{key}\" \
           if updateContainerFrameAnchors then updateContainerFrameAnchors() end \
         end",
        t = -top,
        r = -right,
    )
}

/// Vanillish: the tabletop's inset and the stock open-bag stack.
#[cfg(test)]
mod island_tests {
    use super::island_chunk;
    use benilla_ui::script::UiScript;

    /// The stock column wrap's shape (1.12 `updateContainerFrameAnchors`): a new column once the
    /// stack passes `GetScreenHeight() - CONTAINER_OFFSET_Y`, each bag on the one below.
    const BAGS: &str = r#"
        CreateFrame("Frame", "UIParent") UIParent:SetScale(0.64)
        UIParent:SetPoint("TOPLEFT", nil, "TOPLEFT", 0, 0)
        UIParent:SetPoint("BOTTOMRIGHT", nil, "BOTTOMRIGHT", 0, 0)
        BAGS = {}
        for i = 1, 5 do
          local f = CreateFrame("Frame", "ContainerFrame" .. i, UIParent)
          f:SetWidth(192) f:SetHeight(225) BAGS[i] = f
        end
        function updateContainerFrameAnchors()
          local free = GetScreenHeight() - 70
          local column = 0
          for i, frame in ipairs(BAGS) do
            frame:ClearAllPoints()
            if i == 1 then
              frame:SetPoint("BOTTOMRIGHT", UIParent, "BOTTOMRIGHT", 0, 70)
            elseif free < frame:GetHeight() then
              column = column + 1
              free = GetScreenHeight() - 70
              frame:SetPoint("BOTTOMRIGHT", UIParent, "BOTTOMRIGHT", -column * 192, 70)
            else
              frame:SetPoint("BOTTOMRIGHT", BAGS[i - 1], "TOPRIGHT", 0, 0)
            end
            free = free - frame:GetHeight() - 3
          end
        end
    "#;

    /// The highest bag's top over `UIParent`'s, in `UIParent`'s units.
    const OVERSHOOT: &str = r#"
        local top = 0
        for _, f in ipairs(BAGS) do top = math.max(top, f:GetTop()) end
        return top - UIParent:GetTop()
    "#;

    #[test]
    fn on_a_tabletop_open_bags_wrap_within_uiparent() {
        let mut s = UiScript::new().unwrap();
        s.set_screen_size(1365.0, 768.0);
        s.run(BAGS).unwrap();
        // Uninset, the stock stack fits the whole screen (1200 units at 0.64) in one column.
        s.run("updateContainerFrameAnchors()").unwrap();
        assert!(s.eval::<f32>(OVERSHOOT).unwrap() <= 0.01);
        // The tabletop: UIParent is the half above the hinge, 600 of its units tall.
        s.run(&island_chunk("t", 0.0, 0.0, 0.0, 600.0)).unwrap();
        assert_eq!(
            s.eval::<f32>("return UIParent:GetTop() - UIParent:GetBottom()")
                .unwrap()
                .round(),
            600.0
        );
        let over = s.eval::<f32>(OVERSHOOT).unwrap();
        assert!(
            over <= 0.01,
            "the open bags rose {over} past UIParent's top"
        );
        assert!(
            s.eval::<bool>("return BAGS[3]:GetRight() < BAGS[1]:GetRight()")
                .unwrap(),
            "a second column"
        );
        // Opening a bag later (the stock calls the global) wraps the same; the rest of the
        // interface still reads the root's size.
        s.run("updateContainerFrameAnchors()").unwrap();
        assert!(s.eval::<f32>(OVERSHOOT).unwrap() <= 0.01);
        assert_eq!(
            s.eval::<f32>("return GetScreenHeight()").unwrap().round(),
            1200.0
        );
        // Run again (every half second), it wraps nothing twice.
        s.run(&island_chunk("t", 0.0, 0.0, 0.0, 600.0)).unwrap();
        assert!(s
            .eval::<bool>("return updateContainerFrameAnchors == INILLA_BAG_ANCHORS")
            .unwrap());
    }
}

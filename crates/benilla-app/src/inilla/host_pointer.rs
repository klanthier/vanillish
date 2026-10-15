//! Vanillish: **a hardware mouse and a hovering pen** on Android (Samsung DeX, a Bluetooth mouse
//! or trackpad, a stylus) reach the game through the [`VirtualMouse`], as every other input does.
//!
//! winit has no mouse on Android. The Activity takes the mouse's events before GameActivity sees
//! them and queues them in `inilla_platform::pointer`, in px. Once a frame, [`drain`] does three
//! things:
//! - It turns the mouse's reports into the virtual mouse's operations ([`HostMouse`]): the cursor
//!   in points, a press or a release for each button that changed, the wheel, and motion for
//!   benilla's mouse-look (the raw counts while the pointer is captured). All five buttons count,
//!   so BUTTON4 is autorun as on the desktop.
//! - It keeps the pen's hover for the touch pass, which hands it to the gesture engine
//!   (`TouchEngine::hover`). Under the pen come the stock tooltips and the world's mouseover, and
//!   over a HUD button its tooltip, without pressing it.
//! - It notes when a mouse or a hardware keyboard was last used. The touch HUD steps aside while
//!   one is in use (`hide_touch_with_mouse`), until the next touch.
//!
//! The other way ([`follow_cursor`]): for mouse-look benilla grabs and hides the cursor
//! (`player::camera`), and it sets the 1.12 cursor images (`cursor`). winit refuses all three on
//! Android. Here a grab is pointer capture, and the visibility and the image are the pointer's
//! icon.
//!
//! Every system runs on Android only (`pointer::AVAILABLE`): the desktop has winit's real mouse,
//! and iOS keeps its behaviour.

use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::window::{
    CursorGrabMode, CursorIcon, CursorOptions, CustomCursor, CustomCursorImage, PrimaryWindow,
    WindowFocused,
};

use inilla_core::touch::{MouseButton as KButton, MouseOp, Vec2 as Point};
use inilla_platform::pointer::{self, HostMouse, HostPointerEvent, MouseBackKeys, PointerImage};

use super::mouse::VirtualMouse;
use super::platform::BackPresses;
use super::InputProducers;

/// The host's mouse, pen and keyboard between the queue and the game.
#[derive(Resource, Default)]
pub(crate) struct HostPointer {
    mouse: HostMouse,
    /// The Back presses Android makes of the mouse's back button, taken back.
    back_keys: MouseBackKeys,
    /// Real-time seconds of the last report from a hardware mouse, and of the last key of a
    /// hardware keyboard (`0` = never).
    last_mouse: f64,
    last_keyboard: f64,
    /// The pen since the touch pass last took it: `Some(Some(p))` above `p` (points), `Some(None)`
    /// gone from above the screen.
    pen: Option<Option<Point>>,
    /// A hardware mouse has been seen: from then on its pointer follows the game's cursor.
    mouse_seen: bool,
    /// Pointer capture is on: a grab this mouse started (it held a button).
    captured: bool,
    /// The grab benilla asked for while the pointer is captured, put back after bevy_winit undoes
    /// it ([`keep_grab`]).
    grab_wish: Option<CursorGrabMode>,
    /// The pointer's visibility as last sent to Android.
    shown: Option<bool>,
    /// The cursor's image has to be sent again (it changed, or the mouse just appeared).
    icon_stale: bool,
}

impl HostPointer {
    /// Seconds (real time) of the last use of a hardware mouse or keyboard; `0` = never.
    pub(crate) fn last_used(&self) -> f64 {
        self.last_mouse.max(self.last_keyboard)
    }

    /// The pen's hover since the last call: `Some(None)` when it left.
    pub(crate) fn take_pen(&mut self) -> Option<Option<Point>> {
        self.pen.take()
    }
}

pub(crate) struct HostPointerPlugin;

impl Plugin for HostPointerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HostPointer>()
            .add_systems(PreUpdate, drain.in_set(InputProducers).run_if(on_android))
            // After the game's `Update`, which writes the wishes, and before bevy_winit's `Last`,
            // which undoes a grab it cannot make.
            .add_systems(PostUpdate, follow_cursor.run_if(on_android))
            .add_systems(First, keep_grab.run_if(on_android));
    }
}

fn on_android() -> bool {
    pointer::AVAILABLE
}

/// The host's reports since the last frame, into the virtual mouse and [`HostPointer`]. In
/// `InputProducers`, ahead of the touch pass, which takes the pen's hover from here, and after
/// Android's Back presses are counted (`android`), so the back button's own can be taken out
/// before the screens take theirs.
pub(crate) fn drain(
    mut host: ResMut<HostPointer>,
    mut vm: ResMut<VirtualMouse>,
    mut back: ResMut<BackPresses>,
    windows: Query<&Window, With<PrimaryWindow>>,
    time: Res<Time<Real>>,
    mut focus: MessageReader<WindowFocused>,
    mut ops: Local<Vec<MouseOp>>,
) {
    let host = &mut *host;
    let now = time.elapsed_secs_f64();
    ops.clear();
    // Releases made while the window is unfocused go elsewhere: let go of everything now. The next
    // report presses again whatever is still held.
    if focus.read().any(|f| !f.focused) {
        host.mouse.release_all(&mut ops);
    }
    let events = pointer::drain();
    if let Ok(window) = windows.single() {
        let scale = window.resolution.scale_factor();
        let size = [window.width(), window.height()];
        for ev in &events {
            match *ev {
                HostPointerEvent::PenHover(at) => {
                    host.pen = Some(
                        at.map(|p| [p[0] / scale, p[1] / scale])
                            .filter(|p| p[0].is_finite() && p[1].is_finite()),
                    );
                }
                HostPointerEvent::Keyboard => host.last_keyboard = now,
                HostPointerEvent::Mouse { .. } | HostPointerEvent::Relative { .. } => {
                    host.mouse.feed(ev, scale, size, &mut ops);
                    host.last_mouse = now;
                    if !host.mouse_seen {
                        host.mouse_seen = true;
                        host.icon_stale = true;
                    }
                }
            }
        }
    }
    // Android 16 hands the Back key of the back button to the back callback, which counts it with
    // the gesture's: taken out here, so the button is BUTTON5 alone, as on the desktop.
    let back_releases = ops.iter().filter(|op| **op == MouseOp::Up(KButton::Back));
    for _ in back_releases {
        host.back_keys.released(now);
    }
    let presses = host.back_keys.filter(back.0, now);
    if presses != back.0 {
        back.0 = presses;
    }
    vm.extend(ops.drain(..));
}

/// benilla's wishes for the cursor, as Android can grant them, on every change:
/// - A grab is mouse-look: pointer capture, while this mouse started it (holding a button). A
///   touch drag or a controller's look grabs too, and has no pointer to capture.
/// - The visibility and the cursor's image are the pointer's icon. The 1.12 cursors are
///   `CursorIcon::Custom` images (`cursor`); anything else is the system's arrow.
fn follow_cursor(
    mut host: ResMut<HostPointer>,
    options: Query<Ref<CursorOptions>, With<PrimaryWindow>>,
    icons: Query<Ref<CursorIcon>, With<PrimaryWindow>>,
    images: Res<Assets<Image>>,
) {
    let Ok(opts) = options.single() else {
        return;
    };
    let icon = icons.single().ok();
    let host = &mut *host;
    if opts.is_changed() {
        let grab = opts.grab_mode != CursorGrabMode::None;
        let capture = grab && (host.captured || host.mouse.holds_a_button());
        if capture != host.captured {
            host.captured = capture;
            pointer::set_pointer_capture(capture);
        }
        host.grab_wish = capture.then_some(opts.grab_mode);
    }
    if !host.mouse_seen {
        return;
    }
    if host.shown != Some(opts.visible) {
        host.shown = Some(opts.visible);
        pointer::set_pointer_visible(opts.visible);
    }
    if icon.as_ref().is_some_and(|i| i.is_changed()) {
        host.icon_stale = true;
    }
    if !host.icon_stale {
        return;
    }
    match icon.as_deref() {
        Some(CursorIcon::Custom(CustomCursor::Image(c))) => {
            // Not loaded yet: asked again next frame.
            let Some(image) = images.get(&c.handle) else {
                return;
            };
            pointer::set_pointer_icon(pointer_image(image, c).as_ref());
        }
        _ => pointer::set_pointer_icon(None),
    }
    host.icon_stale = false;
}

/// A custom cursor's image as Android's pointer icon takes it; `None` (the system's arrow) for
/// what benilla never sets: a sheet's region, an atlas, a format other than RGBA8.
fn pointer_image(image: &Image, c: &CustomCursorImage) -> Option<PointerImage> {
    if c.texture_atlas.is_some() || c.rect.is_some() {
        return None;
    }
    if !matches!(
        image.texture_descriptor.format,
        TextureFormat::Rgba8UnormSrgb | TextureFormat::Rgba8Unorm
    ) {
        return None;
    }
    PointerImage::from_rgba(
        image.width(),
        image.height(),
        image.data.as_deref()?,
        c.hotspot,
        c.flip_x,
        c.flip_y,
    )
}

/// winit cannot grab on Android, and bevy_winit then puts the grab mode back as it was, without
/// a change tick. The pointer is captured all the same, so benilla's grab is put back too, before
/// anything reads it this frame. Otherwise a later write of the visibility alone (a cinematic
/// starting during mouse-look) would read as the end of the grab.
fn keep_grab(host: Res<HostPointer>, mut windows: Query<&mut CursorOptions, With<PrimaryWindow>>) {
    let Some(wish) = host.grab_wish else {
        return;
    };
    if let Ok(mut opts) = windows.single_mut() {
        let opts = opts.bypass_change_detection();
        if opts.grab_mode != wish {
            opts.grab_mode = wish;
        }
    }
}

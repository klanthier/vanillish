//! Vanillish: **a hardware mouse and a hovering pen** — Samsung DeX, a Bluetooth mouse or
//! trackpad, a stylus above the screen — as the Java side reports them, for the app's virtual
//! mouse.
//!
//! winit 0.30 has no mouse on Android. GameActivity's input filter lets a mouse's events through
//! as pointer events, and winit makes a finger of a held button and drops everything else: the
//! hover, the wheel, which button it was. A pen's hover is dropped the same way. So the Activity
//! takes these events before GameActivity sees them (`PointerBridge.java`). Every mouse event,
//! and a pen's hover (a pen touching the screen stays a touch), comes to [`push`] through
//! `libvanillish`'s JNI shims and is consumed there. No fake finger is made, and no button press
//! is left for Android to turn into Back. (The Back key Android also makes of the back button is
//! dropped too: [`MouseBackKeys`].) The app [`drain`]s the queue once a frame
//! (`inilla::host_pointer`), and [`HostMouse`] turns the mouse's reports into the virtual mouse's
//! operations. Input still reaches the game through that one seam.
//!
//! The other way, benilla's wishes for the cursor reach Android:
//! - a grab becomes pointer capture ([`set_pointer_capture`]): the mouse's raw motion for
//!   mouse-look, with the pointer hidden;
//! - the cursor's visibility and its 1.12 image become the pointer icon ([`set_pointer_visible`],
//!   [`set_pointer_icon`]).
//!
//! Off Android nothing fills the queue and the calls do nothing: the desktop has winit's real
//! mouse, and iOS has none.
//!
//! The writers run on Android's UI thread (JNI) and the reader on Bevy's. Each holds the one lock
//! for microseconds, the lock is recovered from poisoning, and nothing here may panic, because an
//! `extern "system"` function aborts on a panic.

use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard, PoisonError};

use inilla_core::touch::{MouseButton, MouseOp};

/// This OS reports a hardware mouse here (Android); elsewhere winit has it, or there is none.
pub const AVAILABLE: bool = cfg!(target_os = "android");

/// The `MotionEvent.ACTION_*` values a mouse report carries.
pub mod action {
    /// A button went down: the first one, or (on a trackpad) the fake finger of a swipe.
    pub const DOWN: i32 = 0;
    pub const UP: i32 = 1;
    /// The pointer moved with a button held.
    pub const MOVE: i32 = 2;
    /// The stream was taken elsewhere: says nothing about the buttons.
    pub const CANCEL: i32 = 3;
    /// The pointer moved with no button held.
    pub const HOVER_MOVE: i32 = 7;
    /// The wheel turned.
    pub const SCROLL: i32 = 8;
    pub const HOVER_ENTER: i32 = 9;
    /// The hover ended: the pointer left the window, or a button went down.
    pub const HOVER_EXIT: i32 = 10;
    pub const BUTTON_PRESS: i32 = 11;
    pub const BUTTON_RELEASE: i32 = 12;
}

/// `MotionEvent.BUTTON_*` bits and the buttons they are.
pub const BUTTONS: [(u32, MouseButton); 5] = [
    (1, MouseButton::Left),     // BUTTON_PRIMARY
    (2, MouseButton::Right),    // BUTTON_SECONDARY
    (4, MouseButton::Middle),   // BUTTON_TERTIARY
    (8, MouseButton::Back),     // BUTTON_BACK
    (16, MouseButton::Forward), // BUTTON_FORWARD
];

/// One report from the Java side. Positions are px in the window's coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HostPointerEvent {
    /// The mouse, or a trackpad's pointer: the event's action ([`action`]), every button held
    /// after it (`BUTTON_*` bits), where it is, and the wheel's notches (`+` away from the
    /// player; a high-resolution wheel or a trackpad's swipe gives fractions).
    Mouse {
        action: i32,
        buttons: u32,
        pos: [f32; 2],
        wheel: f32,
    },
    /// The captured mouse (pointer capture): its motion since the last report in the device's
    /// own counts, like winit's `MouseMotion` on the desktop, every button held, and the wheel.
    Relative {
        buttons: u32,
        delta: [f32; 2],
        wheel: f32,
    },
    /// A pen hovering above the screen, or gone from above it.
    PenHover(Option<[f32; 2]>),
    /// A key of a hardware keyboard went down. The soft keyboard's keys do not count.
    Keyboard,
}

impl HostPointerEvent {
    /// The player used the mouse or the keyboard: what the touch HUD steps aside for. A cancel and
    /// a hover's end are Android's own doing (a touch on the screen, pointer capture starting, a
    /// button going down), not use; a pen above the screen is not a mouse.
    pub fn is_use(&self) -> bool {
        match *self {
            HostPointerEvent::Mouse { action: a, .. } => {
                a != action::CANCEL && a != action::HOVER_EXIT
            }
            HostPointerEvent::Relative { .. } | HostPointerEvent::Keyboard => true,
            HostPointerEvent::PenHover(_) => false,
        }
    }
}

/// At most this many reports wait; past that, the oldest is dropped. A mouse report carries
/// every button's state, so a lost one costs a step of motion, never a stuck button.
pub const QUEUE_CAP: usize = 256;

/// The reports waiting for the next frame.
#[derive(Debug, Default)]
pub struct Queue(VecDeque<HostPointerEvent>);

impl Queue {
    pub const fn new() -> Self {
        Self(VecDeque::new())
    }

    /// Add a report. Where nothing is lost it is folded into the last one, so a mouse polled at
    /// 1000 Hz stays one report a frame: a move after a move with the same buttons, captured
    /// motion after captured motion, a hover after a hover, a key after a key.
    pub fn push(&mut self, ev: HostPointerEvent) {
        use HostPointerEvent::{Keyboard, Mouse, PenHover, Relative};
        let is_move = |a: i32| a == action::MOVE || a == action::HOVER_MOVE;
        match (self.0.back_mut(), ev) {
            (
                Some(Mouse {
                    action: last,
                    buttons: held,
                    pos,
                    wheel,
                }),
                Mouse {
                    action: a,
                    buttons: b,
                    pos: p,
                    wheel: w,
                },
            ) if is_move(*last) && *last == a && *held == b && *wheel == 0.0 && w == 0.0 => {
                *pos = p;
            }
            (
                Some(Relative {
                    buttons: held,
                    delta,
                    wheel,
                }),
                Relative {
                    buttons: b,
                    delta: d,
                    wheel: w,
                },
            ) if *held == b && *wheel == 0.0 && w == 0.0 => {
                delta[0] += d[0];
                delta[1] += d[1];
            }
            (Some(PenHover(Some(at))), PenHover(Some(p))) => *at = p,
            (Some(Keyboard), Keyboard) => {}
            _ => {
                if self.0.len() >= QUEUE_CAP {
                    self.0.pop_front();
                }
                self.0.push_back(ev);
            }
        }
    }

    /// Everything waiting, oldest first.
    pub fn drain(&mut self) -> Vec<HostPointerEvent> {
        self.0.drain(..).collect()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

static QUEUE: Mutex<Queue> = Mutex::new(Queue::new());

/// The queue, whatever happened to a thread that held it: a JNI shim must never panic.
fn queue() -> MutexGuard<'static, Queue> {
    QUEUE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A report from the Java side (the JNI shims call this on Android's UI thread).
pub fn push(ev: HostPointerEvent) {
    queue().push(ev);
}

/// Every report since the last call, oldest first. Always empty off Android.
pub fn drain() -> Vec<HostPointerEvent> {
    if !AVAILABLE {
        return Vec::new();
    }
    queue().drain()
}

/// A hardware mouse as the virtual mouse's operations: the cursor where the pointer is, a Down or
/// an Up for each button whose state changed since the last report, motion like the desktop's
/// `MouseMotion`, and the wheel.
#[derive(Clone, Debug, Default)]
pub struct HostMouse {
    /// The `BUTTON_*` bits already pressed (each was sent as a Down).
    held: u32,
    /// Where the cursor was put (logical points), while the pointer is over the window.
    at: Option<[f32; 2]>,
}

impl HostMouse {
    /// A button is down: mouse-look started now comes from this mouse.
    pub fn holds_a_button(&self) -> bool {
        self.held != 0
    }

    /// One report into `out`. `scale` is px per logical point (the window's scale factor);
    /// `size` is the window in points.
    pub fn feed(
        &mut self,
        ev: &HostPointerEvent,
        scale: f32,
        size: [f32; 2],
        out: &mut Vec<MouseOp>,
    ) {
        match *ev {
            HostPointerEvent::Mouse {
                action,
                buttons,
                pos,
                wheel,
            } => {
                let scale = if scale.is_finite() && scale > 0.0 {
                    scale
                } else {
                    1.0
                };
                let p = [pos[0] / scale, pos[1] / scale];
                // A position that is no number moves nothing, but the buttons still count: a
                // release lost here would leave its button held.
                let placed = p[0].is_finite() && p[1].is_finite();
                if action == action::HOVER_EXIT {
                    // Android also ends the hover when a button goes down, with the pointer where
                    // it was. Only an exit across the window's edge takes the cursor away: a DeX
                    // window left behind, or the pointer moved to another display.
                    if placed && self.at.is_some() && outside(p, size) {
                        self.at = None;
                        out.push(MouseOp::Leave);
                    }
                    return;
                }
                // A cancel only says the stream went elsewhere. Its button state is not a
                // release: a release that matters comes as one (or as the captured mouse's).
                let clicks = action != action::CANCEL && (buttons & KNOWN_BUTTONS) != self.held;
                if placed && self.at != Some(p) {
                    // The desktop's `MouseMotion` comes with every move, in the device's counts.
                    // The nearest thing here is the pointer's px.
                    if let Some(old) = self.at {
                        out.push(MouseOp::Motion([
                            (p[0] - old[0]) * scale,
                            (p[1] - old[1]) * scale,
                        ]));
                    }
                    out.push(MouseOp::MoveTo(p));
                    self.at = Some(p);
                } else if placed && (clicks || turns(wheel)) {
                    // The virtual cursor is shared: a finger, the pen or the controller may have
                    // moved it, or taken it away, since this mouse last put it. A click and the
                    // wheel act where this pointer is, so the cursor is put back first. Where it
                    // already is, the move is nothing to the virtual mouse, and no frame waits.
                    out.push(MouseOp::MoveTo(p));
                }
                if action != action::CANCEL {
                    self.buttons(buttons, out);
                }
                push_wheel(wheel, out);
            }
            HostPointerEvent::Relative {
                buttons,
                delta,
                wheel,
            } => {
                if delta[0].is_finite() && delta[1].is_finite() && delta != [0.0, 0.0] {
                    out.push(MouseOp::Motion(delta));
                }
                self.buttons(buttons, out);
                push_wheel(wheel, out);
            }
            HostPointerEvent::PenHover(_) | HostPointerEvent::Keyboard => {}
        }
    }

    /// Let go of every button held (the window lost focus: their releases go elsewhere).
    pub fn release_all(&mut self, out: &mut Vec<MouseOp>) {
        self.buttons(0, out);
    }

    /// Releases first, so a report that swaps one button for another lets go before it presses.
    fn buttons(&mut self, now: u32, out: &mut Vec<MouseOp>) {
        for (bit, b) in BUTTONS {
            if self.held & bit != 0 && now & bit == 0 {
                self.held &= !bit;
                out.push(MouseOp::Up(b));
            }
        }
        for (bit, b) in BUTTONS {
            if self.held & bit == 0 && now & bit != 0 {
                self.held |= bit;
                out.push(MouseOp::Down(b));
            }
        }
    }
}

/// Android also makes a Back key of a mouse's back button. Where the Activity sees that key
/// (`PointerBridge.onKey`), it drops it. With predictive back (Android 16, targetSdk 36), Android
/// hands the key to the back callback before the Activity sees it. In that case the one Back press
/// that follows a release of the button is taken out here, so the button stays BUTTON5 and never
/// becomes the game's Escape as well.
#[derive(Clone, Copy, Debug, Default)]
pub struct MouseBackKeys {
    /// Back presses still expected from the button's releases.
    owed: u32,
    /// Until when (seconds) they are expected.
    until: f64,
}

impl MouseBackKeys {
    /// How long after the button's release its Back press may still arrive (seconds): the press
    /// follows the release on the same thread, a frame or two later at most.
    pub const WINDOW: f64 = 0.5;

    /// The mouse's back button came up at `now`.
    pub fn released(&mut self, now: f64) {
        self.expire(now);
        self.owed = self.owed.saturating_add(1);
        self.until = now + Self::WINDOW;
    }

    /// Of `presses` Back presses seen by `now`, how many are the player's own.
    pub fn filter(&mut self, presses: u32, now: f64) -> u32 {
        self.expire(now);
        let ours = presses.min(self.owed);
        self.owed -= ours;
        presses - ours
    }

    fn expire(&mut self, now: f64) {
        if now > self.until {
            self.owed = 0;
        }
    }
}

/// The [`BUTTONS`] bits together; the others (a stylus's buttons) are not the mouse's.
const KNOWN_BUTTONS: u32 = 1 | 2 | 4 | 8 | 16;

/// The wheel turned (by a number).
fn turns(wheel: f32) -> bool {
    wheel.is_finite() && wheel != 0.0
}

fn push_wheel(wheel: f32, out: &mut Vec<MouseOp>) {
    if turns(wheel) {
        out.push(MouseOp::Wheel(wheel));
    }
}

/// `p` (points) is beyond the window, whose size is `size` (points).
fn outside(p: [f32; 2], size: [f32; 2]) -> bool {
    p[0] < 0.0 || p[1] < 0.0 || p[0] >= size[0] || p[1] >= size[1]
}

/// A cursor image for Android's pointer icon: `ARGB_8888` colours (straight alpha, sRGB), row by
/// row from the top, and the hotspot in its pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PointerImage {
    pub argb: Vec<i32>,
    pub width: u32,
    pub height: u32,
    pub hotspot: [u32; 2],
}

impl PointerImage {
    /// The largest side taken. The 1.12 cursors and a held item's icon are 32 px.
    pub const MAX_SIDE: u32 = 256;

    /// From RGBA8 rows (top first; sRGB, straight alpha, as winit's custom cursor takes them),
    /// mirrored as the cursor asks, with the hotspot mirrored too. `None` when the bytes do not
    /// make the image.
    pub fn from_rgba(
        width: u32,
        height: u32,
        rgba: &[u8],
        hotspot: (u16, u16),
        flip_x: bool,
        flip_y: bool,
    ) -> Option<Self> {
        if width == 0 || height == 0 || width > Self::MAX_SIDE || height > Self::MAX_SIDE {
            return None;
        }
        let (w, h) = (width as usize, height as usize);
        if rgba.len() < w * h * 4 {
            return None;
        }
        let mut argb = Vec::with_capacity(w * h);
        for y in 0..h {
            let sy = if flip_y { h - 1 - y } else { y };
            for x in 0..w {
                let sx = if flip_x { w - 1 - x } else { x };
                let i = (sy * w + sx) * 4;
                let [r, g, b, a] = [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]];
                argb.push(i32::from_be_bytes([a, r, g, b]));
            }
        }
        let hx = u32::from(hotspot.0).min(width - 1);
        let hy = u32::from(hotspot.1).min(height - 1);
        Some(Self {
            argb,
            width,
            height,
            hotspot: [
                if flip_x { width - 1 - hx } else { hx },
                if flip_y { height - 1 - hy } else { hy },
            ],
        })
    }
}

/// Pointer capture on or off: the mouse's raw motion comes as [`HostPointerEvent::Relative`] and
/// the pointer hides, the way a desktop grab works for mouse-look. Android grants it only while
/// the window has focus; the Java side asks again when focus comes back. A no-op off Android.
pub fn set_pointer_capture(on: bool) {
    #[cfg(target_os = "android")]
    host::set_capture(on);
    #[cfg(not(target_os = "android"))]
    let _ = on;
}

/// Show or hide the mouse's pointer (a hidden one is Android's `TYPE_NULL` icon). A no-op off
/// Android.
pub fn set_pointer_visible(visible: bool) {
    #[cfg(target_os = "android")]
    host::set_visible(visible);
    #[cfg(not(target_os = "android"))]
    let _ = visible;
}

/// The pointer's image, or `None` for the system's arrow. A no-op off Android.
pub fn set_pointer_icon(image: Option<&PointerImage>) {
    #[cfg(target_os = "android")]
    host::set_icon(image);
    #[cfg(not(target_os = "android"))]
    let _ = image;
}

#[cfg(target_os = "android")]
mod host {
    use jni::objects::{JObject, JValue};

    use super::PointerImage;
    use crate::android::{bridge, call_bridge_void, with_env};

    pub(super) fn set_capture(on: bool) {
        call_bridge_void("setPointerCapture", "(Z)V", &[JValue::Bool(on.into())]);
    }

    pub(super) fn set_visible(visible: bool) {
        call_bridge_void("setPointerVisible", "(Z)V", &[JValue::Bool(visible.into())]);
    }

    pub(super) fn set_icon(image: Option<&PointerImage>) {
        with_env(|env| {
            let class = bridge(env)?;
            let (pixels, w, h, hx, hy) = match image {
                Some(i) => {
                    let arr = env.new_int_array(i.argb.len() as i32)?;
                    env.set_int_array_region(&arr, 0, &i.argb)?;
                    (
                        JObject::from(arr),
                        i.width as i32,
                        i.height as i32,
                        i.hotspot[0] as i32,
                        i.hotspot[1] as i32,
                    )
                }
                None => (JObject::null(), 0, 0, 0, 0),
            };
            env.call_static_method(
                &class,
                "setPointerIcon",
                "([IIIII)V",
                &[
                    JValue::Object(&pixels),
                    JValue::Int(w),
                    JValue::Int(h),
                    JValue::Int(hx),
                    JValue::Int(hy),
                ],
            )?;
            Ok(())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: [f32; 2] = [960.0, 411.0];

    fn mouse(action: i32, buttons: u32, x: f32, y: f32) -> HostPointerEvent {
        HostPointerEvent::Mouse {
            action,
            buttons,
            pos: [x, y],
            wheel: 0.0,
        }
    }

    fn feed(m: &mut HostMouse, ev: HostPointerEvent) -> Vec<MouseOp> {
        let mut out = Vec::new();
        m.feed(&ev, 2.0, SIZE, &mut out);
        out
    }

    #[test]
    fn a_click_puts_the_cursor_in_points_then_presses_and_releases() {
        let mut m = HostMouse::default();
        assert_eq!(
            feed(&mut m, mouse(action::HOVER_MOVE, 0, 200.0, 100.0)),
            vec![MouseOp::MoveTo([100.0, 50.0])]
        );
        // Android's order: the hover ends where the pointer is, then the button goes down.
        assert!(feed(&mut m, mouse(action::HOVER_EXIT, 0, 200.0, 100.0)).is_empty());
        // Each click puts the cursor where the pointer is first (nothing, where it already is).
        assert_eq!(
            feed(&mut m, mouse(action::DOWN, 1, 200.0, 100.0)),
            vec![
                MouseOp::MoveTo([100.0, 50.0]),
                MouseOp::Down(MouseButton::Left)
            ]
        );
        assert!(feed(&mut m, mouse(action::BUTTON_PRESS, 1, 200.0, 100.0)).is_empty());
        assert!(m.holds_a_button());
        assert_eq!(
            feed(&mut m, mouse(action::UP, 0, 200.0, 100.0)),
            vec![
                MouseOp::MoveTo([100.0, 50.0]),
                MouseOp::Up(MouseButton::Left)
            ]
        );
        assert!(!m.holds_a_button());
    }

    #[test]
    fn a_click_or_the_wheel_puts_the_shared_cursor_back_under_the_pointer() {
        // A finger, the pen or the controller moved the virtual cursor (or took it away) while
        // this mouse stood still: its click and its wheel still act under its pointer.
        let mut m = HostMouse::default();
        feed(&mut m, mouse(action::HOVER_MOVE, 0, 200.0, 100.0));
        assert_eq!(
            feed(&mut m, mouse(action::BUTTON_PRESS, 2, 200.0, 100.0)),
            vec![
                MouseOp::MoveTo([100.0, 50.0]),
                MouseOp::Down(MouseButton::Right)
            ]
        );
        assert_eq!(
            feed(
                &mut m,
                HostPointerEvent::Mouse {
                    action: action::SCROLL,
                    buttons: 2,
                    pos: [200.0, 100.0],
                    wheel: -1.0,
                }
            ),
            vec![MouseOp::MoveTo([100.0, 50.0]), MouseOp::Wheel(-1.0)]
        );
        // A bit that is no mouse button (a stylus's) changes nothing, so it puts nothing.
        assert!(feed(&mut m, mouse(action::MOVE, 2 | 32, 200.0, 100.0)).is_empty());
    }

    #[test]
    fn a_position_that_is_no_number_still_lets_go() {
        let mut m = HostMouse::default();
        feed(&mut m, mouse(action::DOWN, 1, 20.0, 20.0));
        assert_eq!(
            feed(&mut m, mouse(action::UP, 0, f32::NAN, 20.0)),
            vec![MouseOp::Up(MouseButton::Left)]
        );
        assert!(!m.holds_a_button());
        // Nor does it take the cursor away.
        assert!(feed(&mut m, mouse(action::HOVER_EXIT, 0, f32::NAN, f32::NAN)).is_empty());
    }

    #[test]
    fn every_button_is_its_own_and_a_swap_releases_first() {
        let mut m = HostMouse::default();
        feed(&mut m, mouse(action::HOVER_MOVE, 0, 10.0, 10.0));
        assert_eq!(
            feed(&mut m, mouse(action::BUTTON_PRESS, 2 | 4, 10.0, 10.0)),
            vec![
                MouseOp::MoveTo([5.0, 5.0]),
                MouseOp::Down(MouseButton::Right),
                MouseOp::Down(MouseButton::Middle)
            ]
        );
        assert_eq!(
            feed(&mut m, mouse(action::BUTTON_PRESS, 4 | 8 | 16, 10.0, 10.0)),
            vec![
                MouseOp::MoveTo([5.0, 5.0]),
                MouseOp::Up(MouseButton::Right),
                MouseOp::Down(MouseButton::Back),
                MouseOp::Down(MouseButton::Forward)
            ]
        );
        assert_eq!(
            BUTTONS.iter().fold(0, |all, (bit, _)| all | bit),
            KNOWN_BUTTONS
        );
        let mut out = Vec::new();
        m.release_all(&mut out);
        assert_eq!(
            out,
            vec![
                MouseOp::Up(MouseButton::Middle),
                MouseOp::Up(MouseButton::Back),
                MouseOp::Up(MouseButton::Forward)
            ]
        );
    }

    #[test]
    fn a_move_is_motion_in_px_and_the_cursor_in_points() {
        let mut m = HostMouse::default();
        feed(&mut m, mouse(action::HOVER_MOVE, 0, 100.0, 100.0));
        assert_eq!(
            feed(&mut m, mouse(action::HOVER_MOVE, 0, 110.0, 96.0)),
            vec![MouseOp::Motion([10.0, -4.0]), MouseOp::MoveTo([55.0, 48.0])]
        );
        // Standing still is nothing.
        assert!(feed(&mut m, mouse(action::HOVER_MOVE, 0, 110.0, 96.0)).is_empty());
    }

    #[test]
    fn a_cancel_moves_but_never_releases() {
        let mut m = HostMouse::default();
        feed(&mut m, mouse(action::DOWN, 2, 50.0, 50.0));
        assert!(feed(&mut m, mouse(action::CANCEL, 0, 50.0, 50.0)).is_empty());
        assert!(m.holds_a_button());
        // The release arrives through pointer capture.
        assert_eq!(
            feed(
                &mut m,
                HostPointerEvent::Relative {
                    buttons: 0,
                    delta: [0.0, 0.0],
                    wheel: 0.0
                }
            ),
            vec![MouseOp::Up(MouseButton::Right)]
        );
    }

    #[test]
    fn the_captured_mouse_is_raw_motion_and_keeps_the_cursor_where_it_was() {
        let mut m = HostMouse::default();
        feed(&mut m, mouse(action::DOWN, 2, 50.0, 50.0));
        let ops = feed(
            &mut m,
            HostPointerEvent::Relative {
                buttons: 2,
                delta: [7.0, -3.0],
                wheel: 1.0,
            },
        );
        assert_eq!(ops, vec![MouseOp::Motion([7.0, -3.0]), MouseOp::Wheel(1.0)]);
        // Capture ends: the next hover is where Android left the pointer, the motion from there.
        feed(
            &mut m,
            HostPointerEvent::Relative {
                buttons: 0,
                delta: [0.0, 0.0],
                wheel: 0.0,
            },
        );
        assert_eq!(
            feed(&mut m, mouse(action::HOVER_MOVE, 0, 52.0, 50.0)),
            vec![MouseOp::Motion([2.0, 0.0]), MouseOp::MoveTo([26.0, 25.0])]
        );
    }

    #[test]
    fn only_an_exit_across_the_edge_takes_the_cursor_away() {
        let mut m = HostMouse::default();
        feed(&mut m, mouse(action::HOVER_MOVE, 0, 1918.0, 400.0));
        assert!(feed(&mut m, mouse(action::HOVER_EXIT, 0, 1918.0, 400.0)).is_empty());
        assert_eq!(
            feed(&mut m, mouse(action::HOVER_EXIT, 0, 1921.0, 400.0)),
            vec![MouseOp::Leave]
        );
        // Gone once is gone: a second exit says nothing, and the way back is a plain move.
        assert!(feed(&mut m, mouse(action::HOVER_EXIT, 0, -4.0, 400.0)).is_empty());
        assert_eq!(
            feed(&mut m, mouse(action::HOVER_ENTER, 0, 1910.0, 400.0)),
            vec![MouseOp::MoveTo([955.0, 200.0])]
        );
    }

    #[test]
    fn the_wheel_turns_in_notches_and_nonsense_is_dropped() {
        let mut m = HostMouse::default();
        let wheel = |w: f32| HostPointerEvent::Mouse {
            action: action::SCROLL,
            buttons: 0,
            pos: [20.0, 20.0],
            wheel: w,
        };
        assert_eq!(
            feed(&mut m, wheel(-0.5)),
            vec![MouseOp::MoveTo([10.0, 10.0]), MouseOp::Wheel(-0.5)]
        );
        assert!(feed(&mut m, wheel(f32::NAN)).is_empty());
        assert!(feed(&mut m, mouse(action::HOVER_MOVE, 0, f32::INFINITY, 0.0)).is_empty());
        // The pen and the keyboard are not the mouse's business.
        assert!(feed(&mut m, HostPointerEvent::PenHover(Some([1.0, 1.0]))).is_empty());
        assert!(feed(&mut m, HostPointerEvent::Keyboard).is_empty());
    }

    #[test]
    fn what_android_does_on_its_own_is_not_the_player_using_the_mouse() {
        assert!(mouse(action::HOVER_MOVE, 0, 1.0, 1.0).is_use());
        assert!(mouse(action::DOWN, 1, 1.0, 1.0).is_use());
        assert!(HostPointerEvent::Keyboard.is_use());
        assert!(HostPointerEvent::Relative {
            buttons: 0,
            delta: [1.0, 0.0],
            wheel: 0.0
        }
        .is_use());
        // A touch on the screen or capture starting may end the mouse's hover or its stream.
        assert!(!mouse(action::HOVER_EXIT, 0, 1.0, 1.0).is_use());
        assert!(!mouse(action::CANCEL, 1, 1.0, 1.0).is_use());
        assert!(!HostPointerEvent::PenHover(Some([1.0, 1.0])).is_use());
    }

    #[test]
    fn the_back_press_after_the_back_button_is_the_buttons() {
        let mut k = MouseBackKeys::default();
        // Nothing owed: every Back is the player's.
        assert_eq!(k.filter(1, 0.0), 1);
        k.released(1.0);
        // The press may arrive in the same frame or a little later; one is taken, once.
        assert_eq!(k.filter(0, 1.0), 0);
        assert_eq!(k.filter(2, 1.1), 1);
        assert_eq!(k.filter(1, 1.2), 1);
        // Where the Activity dropped the key itself (no predictive back), nothing comes: what is
        // owed expires, and a later Back gesture is the player's.
        k.released(2.0);
        assert_eq!(k.filter(1, 2.0 + MouseBackKeys::WINDOW + 0.01), 1);
        // Two clicks, two presses.
        k.released(5.0);
        k.released(5.1);
        assert_eq!(k.filter(2, 5.2), 0);
    }

    #[test]
    fn the_queue_folds_what_loses_nothing() {
        let mut q = Queue::new();
        q.push(mouse(action::HOVER_MOVE, 0, 1.0, 1.0));
        q.push(mouse(action::HOVER_MOVE, 0, 2.0, 2.0));
        q.push(mouse(action::HOVER_MOVE, 0, 3.0, 3.0));
        assert_eq!(q.len(), 1);
        // A press is kept, and a drag after it starts a new run.
        q.push(mouse(action::DOWN, 1, 3.0, 3.0));
        q.push(mouse(action::MOVE, 1, 4.0, 4.0));
        q.push(mouse(action::MOVE, 1, 5.0, 5.0));
        q.push(HostPointerEvent::Relative {
            buttons: 1,
            delta: [1.0, 2.0],
            wheel: 0.0,
        });
        q.push(HostPointerEvent::Relative {
            buttons: 1,
            delta: [3.0, -1.0],
            wheel: 0.0,
        });
        q.push(HostPointerEvent::Keyboard);
        q.push(HostPointerEvent::Keyboard);
        q.push(HostPointerEvent::PenHover(Some([1.0, 1.0])));
        q.push(HostPointerEvent::PenHover(Some([2.0, 1.0])));
        q.push(HostPointerEvent::PenHover(None));
        assert_eq!(
            q.drain(),
            vec![
                mouse(action::HOVER_MOVE, 0, 3.0, 3.0),
                mouse(action::DOWN, 1, 3.0, 3.0),
                mouse(action::MOVE, 1, 5.0, 5.0),
                HostPointerEvent::Relative {
                    buttons: 1,
                    delta: [4.0, 1.0],
                    wheel: 0.0
                },
                HostPointerEvent::Keyboard,
                HostPointerEvent::PenHover(Some([2.0, 1.0])),
                HostPointerEvent::PenHover(None),
            ]
        );
        assert!(q.is_empty());
    }

    #[test]
    fn the_queue_never_folds_a_wheel_or_a_button_change_and_stays_bounded() {
        let mut q = Queue::new();
        let scroll = HostPointerEvent::Mouse {
            action: action::SCROLL,
            buttons: 0,
            pos: [1.0, 1.0],
            wheel: 1.0,
        };
        q.push(scroll);
        q.push(scroll);
        q.push(mouse(action::MOVE, 1, 1.0, 1.0));
        q.push(mouse(action::MOVE, 3, 1.0, 1.0));
        assert_eq!(q.len(), 4);
        for i in 0..QUEUE_CAP + 10 {
            q.push(HostPointerEvent::Mouse {
                action: action::SCROLL,
                buttons: 0,
                pos: [i as f32, 0.0],
                wheel: 1.0,
            });
        }
        assert_eq!(q.len(), QUEUE_CAP);
        let drained = q.drain();
        assert_eq!(
            drained.last(),
            Some(&HostPointerEvent::Mouse {
                action: action::SCROLL,
                buttons: 0,
                pos: [(QUEUE_CAP + 9) as f32, 0.0],
                wheel: 1.0,
            })
        );
    }

    #[test]
    fn nothing_is_drained_off_android() {
        push(HostPointerEvent::Keyboard);
        let drained = drain();
        if AVAILABLE {
            assert_eq!(drained, vec![HostPointerEvent::Keyboard]);
        } else {
            assert!(drained.is_empty());
        }
    }

    #[test]
    fn a_cursor_image_is_argb_with_its_hotspot_and_mirrors() {
        // 2×2: red, green / blue, half-transparent white.
        let rgba = [
            255, 0, 0, 255, 0, 255, 0, 255, //
            0, 0, 255, 255, 255, 255, 255, 128,
        ];
        let i = PointerImage::from_rgba(2, 2, &rgba, (0, 0), false, false).unwrap();
        assert_eq!(
            i.argb,
            vec![
                0xFFFF0000_u32 as i32,
                0xFF00FF00_u32 as i32,
                0xFF0000FF_u32 as i32,
                0x80FFFFFF_u32 as i32
            ]
        );
        assert_eq!(i.hotspot, [0, 0]);
        let m = PointerImage::from_rgba(2, 2, &rgba, (0, 1), true, true).unwrap();
        assert_eq!(m.argb[0], 0x80FFFFFF_u32 as i32);
        assert_eq!(m.argb[3], 0xFFFF0000_u32 as i32);
        assert_eq!(m.hotspot, [1, 0]);
        // A hotspot past the image is pulled inside it.
        let far = PointerImage::from_rgba(2, 2, &rgba, (9, 9), false, false).unwrap();
        assert_eq!(far.hotspot, [1, 1]);
    }

    #[test]
    fn a_cursor_image_needs_its_bytes_and_a_sane_size() {
        assert!(PointerImage::from_rgba(2, 2, &[0; 15], (0, 0), false, false).is_none());
        assert!(PointerImage::from_rgba(0, 2, &[], (0, 0), false, false).is_none());
        let big = PointerImage::MAX_SIDE + 1;
        let bytes = vec![0; (big * big * 4) as usize];
        assert!(PointerImage::from_rgba(big, big, &bytes, (0, 0), false, false).is_none());
    }
}

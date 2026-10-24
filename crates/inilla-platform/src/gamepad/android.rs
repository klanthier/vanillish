//! Vanillish: **controllers on Android** — a store the Java side's `GamepadBridge` feeds (through
//! `libvanillish`'s JNI shims) and [`poll`] reads once a frame, so the app sees the same
//! "this instant" [`Pad`] list GameController gives it on iOS.
//!
//! Android has no GameController: a pad is an `InputDevice` whose buttons arrive as `KeyEvent`s
//! and whose sticks, triggers and hat arrive as joystick `MotionEvent`s, on the UI thread. The
//! Activity hands every event from a pad to `GamepadBridge` before GameActivity sees it (which
//! would drop the axes and pass the buttons to winit as unidentified keys); the bridge reports
//! connections ([`connect`], [`disconnect`]), key edges ([`key`]) and the latest axis values
//! ([`motion`]) here, and asks for a [`release_all`] when the window loses focus.
//!
//! Everything that turns Android's view of a pad into a [`PadState`] is here, in plain Rust, so
//! the host tests cover it:
//!
//! - **Buttons** ([`KEYS`]): Android's `KEYCODE_BUTTON_*` are already positional (the AOSP `.kl`
//!   files map a DualSense's Cross and a Switch pad's south "B" to `BUTTON_A`), so `A` is south as
//!   on iOS. `BACK` (a cheap pad's Select) is Options, `DPAD_CENTER` is A, the diagonal d-pad keys
//!   press two directions, `MODE` is Home.
//! - **Axes**: which ones carry the right stick and the triggers depends on the device. The layout
//!   is chosen once, at connect, from the axes it declares ([`Layout::from_mask`]). Sticks are
//!   turned `+y` up; the hat is a second d-pad, OR'ed with the keys.
//! - **The hardware dead zone** ("flat"): Android reports it but never applies it. A radial gate
//!   per stick zeroes a stick resting inside it, without rescaling — the mapper's own dead zone
//!   (rescaled, the player's slider) still applies on top, and can go below the hardware's without
//!   letting a worn stick drift.
//! - **The tap latch**: a button pressed and released between two frames still shows as down for
//!   one frame. Polling GameController can miss those; a store fed by events need not.
//! - **Order**: most recently used first, so the app's "first pad" is the one in the player's
//!   hands (a Backbone on the cover screen and a DualSense on the table, say).
//!
//! The writers run on Android's UI thread (JNI), the reader on Bevy's (once a frame); each holds
//! the one lock for microseconds. Nothing here may panic — an `extern "system"` function aborts
//! on one — so the lock is recovered from poisoning, indices are constants and every arithmetic
//! step is total.

use std::sync::{Mutex, MutexGuard, PoisonError};

use inilla_core::pad::{PadButton, PadState};

use super::Pad;

/// The axes `GamepadBridge` sends, in `nativePadMotion`'s order (and bit order in the connect
/// masks): Android's `MotionEvent.AXIS_*` ids in a comment.
pub mod axis {
    /// AXIS_X (0): the left stick, `+` right.
    pub const X: usize = 0;
    /// AXIS_Y (1): the left stick, `+` **down**.
    pub const Y: usize = 1;
    /// AXIS_Z (11): the right stick's x on the standard layout; a trigger on generic HID pads.
    pub const Z: usize = 2;
    /// AXIS_RZ (14): the right stick's y (`+` down) on the standard layout; a trigger on generic
    /// HID pads.
    pub const RZ: usize = 3;
    /// AXIS_RX (12): the right stick's x on generic HID pads.
    pub const RX: usize = 4;
    /// AXIS_RY (13): the right stick's y (`+` down) on generic HID pads.
    pub const RY: usize = 5;
    /// AXIS_HAT_X (15): a d-pad reported as an axis, `-1` left.
    pub const HAT_X: usize = 6;
    /// AXIS_HAT_Y (16): a d-pad reported as an axis, `-1` up.
    pub const HAT_Y: usize = 7;
    /// AXIS_LTRIGGER (17), `0..1`.
    pub const LTRIGGER: usize = 8;
    /// AXIS_RTRIGGER (18), `0..1`.
    pub const RTRIGGER: usize = 9;
    /// AXIS_BRAKE (23): Android mirrors LTRIGGER here; a few pads (the Moga Pro 2) report only it.
    pub const BRAKE: usize = 10;
    /// AXIS_GAS (22): RTRIGGER's mirror, as BRAKE is LTRIGGER's.
    pub const GAS: usize = 11;
    /// How many there are.
    pub const COUNT: usize = 12;
}

/// What an Android keycode does on a pad.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyAction {
    /// Presses these [`PadButton::bit`]s.
    Press(u32),
    /// A digital left trigger (ZL on a Switch pad): `lt = 1` — only on a pad without an analog one.
    LeftTrigger,
    /// A digital right trigger.
    RightTrigger,
}

const fn press(b: PadButton) -> KeyAction {
    KeyAction::Press(b.bit())
}

const fn press2(a: PadButton, b: PadButton) -> KeyAction {
    KeyAction::Press(a.bit() | b.bit())
}

/// Android keycode (`KeyEvent.KEYCODE_*`, values as in the NDK's `AKEYCODE_*`) → what it does.
/// Anything else a pad sends (`BUTTON_C`, `BUTTON_Z`, `BUTTON_1..16`, Xbox Share's `MEDIA_RECORD`)
/// is consumed by the Java side and ignored here. The pad's Home on a Switch Pro is `KEYCODE_HOME`,
/// which Android keeps for itself and never delivers.
pub const KEYS: [(i32, KeyAction); 24] = [
    (96, press(PadButton::A)),        // BUTTON_A: south (Cross, Switch "B")
    (97, press(PadButton::B)),        // BUTTON_B: east
    (99, press(PadButton::X)),        // BUTTON_X: west
    (100, press(PadButton::Y)),       // BUTTON_Y: north
    (102, press(PadButton::LB)),      // BUTTON_L1
    (103, press(PadButton::RB)),      // BUTTON_R1
    (104, KeyAction::LeftTrigger),    // BUTTON_L2
    (105, KeyAction::RightTrigger),   // BUTTON_R2
    (106, press(PadButton::L3)),      // BUTTON_THUMBL
    (107, press(PadButton::R3)),      // BUTTON_THUMBR
    (108, press(PadButton::Menu)),    // BUTTON_START: Xbox ☰, PS Options, Switch +
    (82, press(PadButton::Menu)),     // MENU: some Android-native pads
    (109, press(PadButton::Options)), // BUTTON_SELECT: Xbox View, PS Create, Switch −
    // BACK, from a pad (the Java side only reports a known pad's): what cheap pads send for
    // Select. As SDL does.
    (4, press(PadButton::Options)),
    // BUTTON_MODE: reported (the mapper leaves Home alone); the Java side then lets Android's own
    // fallback take the player home, the console way.
    (110, press(PadButton::Home)),
    (19, press(PadButton::Up)),                       // DPAD_UP
    (20, press(PadButton::Down)),                     // DPAD_DOWN
    (21, press(PadButton::Left)),                     // DPAD_LEFT
    (22, press(PadButton::Right)),                    // DPAD_RIGHT
    (23, press(PadButton::A)),                        // DPAD_CENTER: confirm, as SDL maps it
    (268, press2(PadButton::Up, PadButton::Left)),    // DPAD_UP_LEFT
    (269, press2(PadButton::Down, PadButton::Left)),  // DPAD_DOWN_LEFT
    (270, press2(PadButton::Up, PadButton::Right)),   // DPAD_UP_RIGHT
    (271, press2(PadButton::Down, PadButton::Right)), // DPAD_DOWN_RIGHT
];

// A pad's held keys are a `u32`, one bit per entry.
const _: () = assert!(KEYS.len() <= 32);

/// What an Android keycode does on a pad, if anything.
pub fn key_action(code: i32) -> Option<KeyAction> {
    KEYS.iter().find(|(c, _)| *c == code).map(|&(_, a)| a)
}

/// The glyph family a vendor/product id names, for [`Pad::category`]. Android reports a
/// DualSense as "Sony Interactive Entertainment Wireless Controller" and a Backbone by its
/// marketing name, neither of which `GlyphStyle::from_product` would place; the app reads
/// `"{name} {category}"`, so this is what decides the HUD's button glyphs. The table is
/// `inilla_core::pads`, which the Windows build reads too.
pub use inilla_core::pads::family;

/// Where a pad's right stick comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RightStick {
    /// Z/RZ: every AOSP `.kl` (DualSense, DualShock 4, Xbox, Switch Pro).
    ZRz,
    /// RX/RY: generic HID pads with no `.kl`, which put their triggers on Z/RZ.
    RxRy,
    /// No right stick.
    None,
}

/// Where a pad's triggers come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Triggers {
    /// LTRIGGER/RTRIGGER, max-ed with their BRAKE/GAS mirrors (`0..1`).
    Analog,
    /// Z (left) and RZ (right), on a pad whose right stick is RX/RY. Android normalizes Z and RZ
    /// as *centred* axes, so such a trigger rests at `-1`; each flag says the axis is centred
    /// (`MotionRange.getMin() < 0`) and is read `(v + 1) / 2`.
    ZRz { centered: [bool; 2] },
    /// No analog trigger: the BUTTON_L2/R2 keys (a Switch pad's ZL/ZR), as `0` or `1`.
    Keys,
}

/// A pad's axis layout, chosen once at connect from the axes it declares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub right: RightStick,
    pub triggers: Triggers,
}

impl Layout {
    /// The layout for a pad declaring the axes in `axis_mask` (bit *i* for slot *i* of [`axis`]);
    /// `centered_mask` marks the ones whose range goes below zero.
    ///
    /// - An analog trigger axis (LTRIGGER, RTRIGGER, BRAKE or GAS): the standard Android pad —
    ///   right stick on Z/RZ (RX/RY when it has no Z/RZ), analog triggers.
    /// - No trigger axis but RX/RY: a generic HID pad — right stick RX/RY, triggers on Z/RZ (the
    ///   ordering SDL's `RangeComparator` also assumes), or the L2/R2 keys without Z/RZ.
    /// - Neither: right stick on Z/RZ if there is one, triggers from the L2/R2 keys.
    pub fn from_mask(axis_mask: u32, centered_mask: u32) -> Self {
        let has = |slot: usize| axis_mask & (1 << slot) != 0;
        let z_rz = has(axis::Z) && has(axis::RZ);
        let rx_ry = has(axis::RX) && has(axis::RY);
        let analog = [axis::LTRIGGER, axis::RTRIGGER, axis::BRAKE, axis::GAS]
            .into_iter()
            .any(has);
        if analog {
            let right = if z_rz {
                RightStick::ZRz
            } else if rx_ry {
                RightStick::RxRy
            } else {
                RightStick::None
            };
            Layout {
                right,
                triggers: Triggers::Analog,
            }
        } else if rx_ry {
            let triggers = if z_rz {
                let centered = |slot: usize| centered_mask & (1 << slot) != 0;
                Triggers::ZRz {
                    centered: [centered(axis::Z), centered(axis::RZ)],
                }
            } else {
                Triggers::Keys
            };
            Layout {
                right: RightStick::RxRy,
                triggers,
            }
        } else {
            Layout {
                right: if z_rz {
                    RightStick::ZRz
                } else {
                    RightStick::None
                },
                triggers: Triggers::Keys,
            }
        }
    }

    /// The axis values at rest: everything at 0, except a centred Z/RZ trigger at `-1` — so a
    /// trigger reads 0, not 0.5 (past the layer threshold), before the pad's first event and
    /// after a [`release_all`].
    fn rest(self) -> [f32; axis::COUNT] {
        let mut raw = [0.0; axis::COUNT];
        if let Triggers::ZRz { centered } = self.triggers {
            if centered[0] {
                raw[axis::Z] = -1.0;
            }
            if centered[1] {
                raw[axis::RZ] = -1.0;
            }
        }
        raw
    }
}

/// A stick or trigger past this counts as the player using the pad (for the order). High enough
/// that a pad lying on the table with a drifting stick never takes over from the one in hand.
const IN_USE: f32 = 0.5;

/// A stick `[x, y]` (`+y` up), zeroed while inside the hardware's flat, passed through unscaled
/// outside it.
fn gate(x: f32, y: f32, flat: f32) -> [f32; 2] {
    if x.hypot(y) <= flat {
        [0.0, 0.0]
    } else {
        [x, y]
    }
}

/// The d-pad bits an `AXIS_HAT_X`/`HAT_Y` pair holds (±1 when pressed; ±0.5 is the threshold).
fn hat_bits(x: f32, y: f32) -> u32 {
    let mut b = 0;
    if x <= -0.5 {
        b |= PadButton::Left.bit();
    } else if x >= 0.5 {
        b |= PadButton::Right.bit();
    }
    if y <= -0.5 {
        b |= PadButton::Up.bit();
    } else if y >= 0.5 {
        b |= PadButton::Down.bit();
    }
    b
}

/// One connected pad.
struct Dev {
    /// Android's `InputDevice` id (a reconnect gets a new one).
    id: i32,
    name: String,
    category: &'static str,
    layout: Layout,
    /// The sticks' hardware dead zone, as a fraction of full deflection.
    flat: f32,
    /// The latest value of every [`axis`] slot (sanitized: finite, `-1..=1`).
    raw: [f32; axis::COUNT],
    /// The [`KEYS`] entries held down, bit *i* for entry *i* (so two keys that press the same
    /// button — BUTTON_A and DPAD_CENTER — release it only when both are up).
    held: u32,
    /// The d-pad bits the hat holds.
    hat: u32,
    /// Button bits pressed since the last poll: the tap latch.
    latched: u32,
    /// The store's tick at this pad's last use.
    used: u64,
}

impl Dev {
    fn new(id: i32, name: String, category: &'static str, layout: Layout, flat: f32) -> Self {
        Dev {
            id,
            name,
            category,
            layout,
            flat,
            raw: layout.rest(),
            held: 0,
            hat: 0,
            latched: 0,
            used: 0,
        }
    }

    /// The held keys' actions.
    fn held_actions(&self) -> impl Iterator<Item = KeyAction> + '_ {
        KEYS.iter()
            .enumerate()
            .filter(|(i, _)| self.held & (1 << i) != 0)
            .map(|(_, &(_, a))| a)
    }

    /// The state this instant, the tap latch included.
    fn state(&self) -> PadState {
        let r = &self.raw;
        let left = gate(r[axis::X], -r[axis::Y], self.flat);
        let right = match self.layout.right {
            RightStick::ZRz => gate(r[axis::Z], -r[axis::RZ], self.flat),
            RightStick::RxRy => gate(r[axis::RX], -r[axis::RY], self.flat),
            RightStick::None => [0.0, 0.0],
        };
        let (lt, rt) = match self.layout.triggers {
            Triggers::Analog => (
                r[axis::LTRIGGER].max(r[axis::BRAKE]),
                r[axis::RTRIGGER].max(r[axis::GAS]),
            ),
            Triggers::ZRz { centered } => {
                let norm = |v: f32, c: bool| if c { (v + 1.0) * 0.5 } else { v };
                (
                    norm(r[axis::Z], centered[0]),
                    norm(r[axis::RZ], centered[1]),
                )
            }
            Triggers::Keys => {
                let mut t = (0.0, 0.0);
                for a in self.held_actions() {
                    match a {
                        KeyAction::LeftTrigger => t.0 = 1.0,
                        KeyAction::RightTrigger => t.1 = 1.0,
                        KeyAction::Press(_) => {}
                    }
                }
                t
            }
        };
        let mut buttons = self.hat | self.latched;
        for a in self.held_actions() {
            if let KeyAction::Press(bits) = a {
                buttons |= bits;
            }
        }
        PadState {
            connected: true,
            left,
            right,
            lt: lt.clamp(0.0, 1.0),
            rt: rt.clamp(0.0, 1.0),
            buttons,
        }
    }

    /// Back to rest: nothing held, sticks centred, triggers released.
    fn release(&mut self) {
        self.raw = self.layout.rest();
        self.held = 0;
        self.hat = 0;
        self.latched = 0;
    }
}

/// Every connected pad, and a counter that orders their use.
struct Store {
    devs: Vec<Dev>,
    tick: u64,
}

impl Store {
    const fn new() -> Self {
        Store {
            devs: Vec::new(),
            tick: 0,
        }
    }

    fn dev(&mut self, id: i32) -> Option<&mut Dev> {
        self.devs.iter_mut().find(|d| d.id == id)
    }

    fn next_tick(&mut self) -> u64 {
        self.tick = self.tick.wrapping_add(1);
        self.tick
    }

    /// Add a pad, or refresh a known one's metadata (Android reports a pad again when its
    /// sources change: a Bluetooth pad can gain its joystick late). A known pad keeps its keys
    /// and its place; its axes go back to rest only if the layout changed. A new one goes first:
    /// the player just switched it on. Answers a line for the log when something changed.
    fn connect(
        &mut self,
        id: i32,
        name: String,
        vendor: u16,
        product: u16,
        axis_mask: u32,
        centered_mask: u32,
        flat: f32,
    ) -> Option<String> {
        let name = if name.trim().is_empty() {
            "Controller".to_string()
        } else {
            name
        };
        let category = family(vendor, product);
        let layout = Layout::from_mask(axis_mask, centered_mask);
        // Java caps it at 0.5 already; anything else would gate most of a stick's travel.
        let flat = if flat.is_finite() {
            flat.clamp(0.0, 0.5)
        } else {
            0.0
        };
        let line = format!(
            "vanillish: controller {id}: {name} [{vendor:04x}:{product:04x}] \"{category}\", \
             axes {axis_mask:#05x} centred {centered_mask:#05x}, {layout:?}, flat {flat:.3}"
        );
        if let Some(d) = self.dev(id) {
            let changed = d.layout != layout;
            if changed {
                d.layout = layout;
                d.raw = layout.rest();
                d.hat = 0;
            }
            d.name = name;
            d.category = category;
            d.flat = flat;
            return changed.then_some(line);
        }
        let used = self.next_tick();
        let mut d = Dev::new(id, name, category, layout, flat);
        d.used = used;
        self.devs.push(d);
        Some(line)
    }

    fn disconnect(&mut self, id: i32) -> bool {
        let before = self.devs.len();
        self.devs.retain(|d| d.id != id);
        self.devs.len() != before
    }

    fn key(&mut self, id: i32, code: i32, down: bool) {
        let tick = self.tick.wrapping_add(1);
        let Some(d) = self.dev(id) else {
            return;
        };
        let entry = KEYS
            .iter()
            .enumerate()
            .find(|(_, (c, _))| *c == code)
            .map(|(i, &(_, a))| (1u32 << i, a));
        if down {
            if let Some((bit, action)) = entry {
                // Latch on the edge only: a held key's repeats must not outlive its release.
                if let (0, KeyAction::Press(bits)) = (d.held & bit, action) {
                    d.latched |= bits;
                }
                d.held |= bit;
            }
            // Any key down from a pad (even one that maps to nothing) is the player holding it.
            d.used = tick;
            self.tick = tick;
        } else if let Some((bit, _)) = entry {
            d.held &= !bit;
        }
    }

    fn motion(&mut self, id: i32, axes: [f32; axis::COUNT]) {
        let tick = self.tick.wrapping_add(1);
        let Some(d) = self.dev(id) else {
            return;
        };
        for ((raw, v), rest) in d.raw.iter_mut().zip(axes).zip(d.layout.rest()) {
            *raw = if v.is_finite() {
                v.clamp(-1.0, 1.0)
            } else {
                rest
            };
        }
        let hat = hat_bits(d.raw[axis::HAT_X], d.raw[axis::HAT_Y]);
        let rising = hat & !d.hat;
        d.latched |= rising;
        d.hat = hat;

        let s = d.state();
        let pushed = |v: [f32; 2]| v[0].hypot(v[1]) > IN_USE;
        if rising != 0 || pushed(s.left) || pushed(s.right) || s.lt > IN_USE || s.rt > IN_USE {
            d.used = tick;
            self.tick = tick;
        }
    }

    fn release_all(&mut self) {
        for d in &mut self.devs {
            d.release();
        }
    }

    /// One [`Pad`] per connected pad, most recently used first; clears the tap latches.
    fn poll(&mut self, out: &mut Vec<Pad>) {
        // Stable: pads never used keep their connection order.
        self.devs.sort_by_key(|d| std::cmp::Reverse(d.used));
        for d in &mut self.devs {
            out.push(Pad {
                id: u64::from(d.id as u32),
                name: d.name.clone(),
                category: d.category.to_string(),
                state: d.state(),
            });
            d.latched = 0;
        }
    }
}

static PADS: Mutex<Store> = Mutex::new(Store::new());

/// The store, whatever a panicking holder left behind (none should: nothing here panics).
fn store() -> MutexGuard<'static, Store> {
    PADS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One line to the log (logcat and the iNilla console, through the stdio capture). Written
/// with the store unlocked, and never `eprintln!`, which panics when stderr fails.
fn note(line: &str) {
    #[cfg(not(test))]
    {
        use std::io::Write;
        let _ = writeln!(std::io::stderr(), "{line}");
    }
    #[cfg(test)]
    let _ = line;
}

/// A pad connected, or Android reported a known one again (idempotent: its state is kept).
/// `axis_mask` has bit *i* set when the device has [`axis`] slot *i*, `centered_mask` when that
/// axis's range goes below zero; `flat` is the sticks' hardware dead zone (the largest
/// `MotionRange.getFlat()` among them, as a fraction of full deflection).
pub fn connect(
    id: i32,
    name: String,
    vendor: u16,
    product: u16,
    axis_mask: u32,
    centered_mask: u32,
    flat: f32,
) {
    let line = store().connect(id, name, vendor, product, axis_mask, centered_mask, flat);
    if let Some(line) = line {
        note(&line);
    }
}

/// A pad went away. The app sees another first pad, or none, and the mapper lets go of
/// everything it held.
pub fn disconnect(id: i32) {
    if store().disconnect(id) {
        note(&format!("vanillish: controller {id} disconnected"));
    }
}

/// A pad's key went down or up (`code` is Android's `KEYCODE_*`). Repeats are harmless.
pub fn key(id: i32, code: i32, down: bool) {
    store().key(id, code, down);
}

/// A pad's axes now, in [`axis`] order (an axis the device lacks reads 0).
pub fn motion(id: i32, axes: [f32; axis::COUNT]) {
    store().motion(id, axes);
}

/// Let go of everything on every pad: the window lost focus, and the ups may go elsewhere (the
/// notification shade, a fold to the other panel, Game Booster's panel).
pub fn release_all() {
    store().release_all();
}

/// Every connected pad this instant, most recently used first.
pub fn poll(out: &mut Vec<Pad>) {
    store().poll(out);
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn bits(slots: &[usize]) -> u32 {
        let mut m = 0;
        let mut i = 0;
        while i < slots.len() {
            m |= 1 << slots[i];
            i += 1;
        }
        m
    }

    /// A DualSense through AOSP's `.kl`: sticks X/Y + Z/RZ, the hat, LTRIGGER/RTRIGGER and
    /// their BRAKE/GAS mirrors.
    const DUALSENSE: u32 = bits(&[
        axis::X,
        axis::Y,
        axis::Z,
        axis::RZ,
        axis::HAT_X,
        axis::HAT_Y,
        axis::LTRIGGER,
        axis::RTRIGGER,
        axis::BRAKE,
        axis::GAS,
    ]);
    const DUALSENSE_CENTERED: u32 = bits(&[
        axis::X,
        axis::Y,
        axis::Z,
        axis::RZ,
        axis::HAT_X,
        axis::HAT_Y,
    ]);
    /// A generic HID pad with no `.kl`: six centred axes, the triggers on Z/RZ.
    const GENERIC: u32 = bits(&[axis::X, axis::Y, axis::Z, axis::RZ, axis::RX, axis::RY]);
    /// A Switch Pro: two sticks and a hat, digital ZL/ZR.
    const SWITCH_PRO: u32 = bits(&[
        axis::X,
        axis::Y,
        axis::Z,
        axis::RZ,
        axis::HAT_X,
        axis::HAT_Y,
    ]);

    fn store_with(id: i32, mask: u32, centered: u32, flat: f32) -> Store {
        let mut s = Store::new();
        s.connect(id, "Pad".into(), 0, 0, mask, centered, flat);
        s
    }

    fn poll1(s: &mut Store) -> Pad {
        let mut out = Vec::new();
        s.poll(&mut out);
        assert_eq!(out.len(), 1);
        out.remove(0)
    }

    fn axes(set: &[(usize, f32)]) -> [f32; axis::COUNT] {
        let mut a = [0.0; axis::COUNT];
        for &(slot, v) in set {
            a[slot] = v;
        }
        a
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn keycodes_map_to_positional_buttons() {
        use PadButton::*;
        for (code, b) in [
            (96, A),
            (97, B),
            (99, X),
            (100, Y),
            (102, LB),
            (103, RB),
            (106, L3),
            (107, R3),
            (108, Menu),
            (82, Menu),
            (109, Options),
            (4, Options),
            (110, Home),
            (19, Up),
            (20, Down),
            (21, Left),
            (22, Right),
            (23, A),
        ] {
            assert_eq!(
                key_action(code),
                Some(KeyAction::Press(b.bit())),
                "keycode {code}"
            );
        }
        assert_eq!(key_action(104), Some(KeyAction::LeftTrigger));
        assert_eq!(key_action(105), Some(KeyAction::RightTrigger));
        // BUTTON_C, BUTTON_Z, BUTTON_1, MEDIA_RECORD, HOME: nothing.
        for code in [98, 101, 188, 130, 3] {
            assert_eq!(key_action(code), None, "keycode {code}");
        }
        // Every keycode once.
        for (i, (c, _)) in KEYS.iter().enumerate() {
            assert!(
                KEYS[i + 1..].iter().all(|(d, _)| d != c),
                "keycode {c} twice"
            );
        }
    }

    #[test]
    fn diagonals_press_two_directions() {
        use PadButton::*;
        for (code, a, b) in [
            (268, Up, Left),
            (269, Down, Left),
            (270, Up, Right),
            (271, Down, Right),
        ] {
            let mut s = store_with(1, SWITCH_PRO, 0, 0.0);
            s.key(1, code, true);
            let st = poll1(&mut s).state;
            assert!(st.pressed(a) && st.pressed(b), "keycode {code}");
            assert_eq!(st.buttons.count_ones(), 2);
            s.key(1, code, false);
            assert_eq!(poll1(&mut s).state.buttons, 0);
        }
    }

    #[test]
    fn a_button_held_twice_releases_when_both_are_up() {
        let mut s = store_with(1, SWITCH_PRO, 0, 0.0);
        s.key(1, 96, true); // BUTTON_A
        s.key(1, 23, true); // DPAD_CENTER
        s.key(1, 96, false);
        assert!(poll1(&mut s).state.pressed(PadButton::A));
        s.key(1, 23, false);
        assert!(!poll1(&mut s).state.pressed(PadButton::A));
    }

    #[test]
    fn layout_follows_the_declared_axes() {
        assert_eq!(
            Layout::from_mask(DUALSENSE, DUALSENSE_CENTERED),
            Layout {
                right: RightStick::ZRz,
                triggers: Triggers::Analog
            }
        );
        // The Moga Pro 2 reports only BRAKE/GAS: still analog.
        assert_eq!(
            Layout::from_mask(
                bits(&[axis::X, axis::Y, axis::Z, axis::RZ, axis::BRAKE, axis::GAS]),
                0
            )
            .triggers,
            Triggers::Analog
        );
        // Analog triggers with the right stick on RX/RY.
        assert_eq!(
            Layout::from_mask(
                bits(&[
                    axis::X,
                    axis::Y,
                    axis::RX,
                    axis::RY,
                    axis::LTRIGGER,
                    axis::RTRIGGER
                ]),
                0
            )
            .right,
            RightStick::RxRy
        );
        assert_eq!(
            Layout::from_mask(GENERIC, GENERIC),
            Layout {
                right: RightStick::RxRy,
                triggers: Triggers::ZRz {
                    centered: [true, true]
                }
            }
        );
        assert_eq!(
            Layout::from_mask(SWITCH_PRO, SWITCH_PRO),
            Layout {
                right: RightStick::ZRz,
                triggers: Triggers::Keys
            }
        );
        // RX/RY but no Z/RZ: the L2/R2 keys.
        assert_eq!(
            Layout::from_mask(bits(&[axis::X, axis::Y, axis::RX, axis::RY]), 0).triggers,
            Triggers::Keys
        );
        // A single stick and a hat.
        assert_eq!(
            Layout::from_mask(bits(&[axis::X, axis::Y, axis::HAT_X, axis::HAT_Y]), 0),
            Layout {
                right: RightStick::None,
                triggers: Triggers::Keys
            }
        );
    }

    #[test]
    fn standard_layout_reads_z_rz_and_analog_triggers() {
        let mut s = store_with(1, DUALSENSE, DUALSENSE_CENTERED, 0.0);
        s.motion(
            1,
            axes(&[
                (axis::X, 0.25),
                (axis::Y, -0.75), // pushed up
                (axis::Z, -0.5),
                (axis::RZ, 0.5), // pulled down
                (axis::LTRIGGER, 0.6),
                (axis::RTRIGGER, 0.2),
            ]),
        );
        let st = poll1(&mut s).state;
        assert_eq!(st.left, [0.25, 0.75]);
        assert_eq!(st.right, [-0.5, -0.5]);
        assert!(close(st.lt, 0.6) && close(st.rt, 0.2));
        // LT/RT are left to the mapper's threshold, as on iOS.
        assert_eq!(st.buttons, 0);
        // The DualSense sends L2/R2 keys too: ignored, so the analog threshold keeps working.
        s.key(1, 104, true);
        s.key(1, 105, true);
        let st = poll1(&mut s).state;
        assert!(close(st.lt, 0.6) && close(st.rt, 0.2));
        assert_eq!(st.buttons, 0);
    }

    #[test]
    fn brake_and_gas_are_maxed_in() {
        let mut s = store_with(1, DUALSENSE, DUALSENSE_CENTERED, 0.0);
        s.motion(
            1,
            axes(&[
                (axis::LTRIGGER, 0.2),
                (axis::BRAKE, 0.7),
                (axis::RTRIGGER, 0.9),
                (axis::GAS, 0.4),
            ]),
        );
        let st = poll1(&mut s).state;
        assert!(close(st.lt, 0.7) && close(st.rt, 0.9));
    }

    #[test]
    fn generic_layout_reads_rx_ry_and_z_rz_triggers_at_rest_zero() {
        let mut s = store_with(1, GENERIC, GENERIC, 0.0);
        // Before the first event: triggers rest at 0, not 0.5 (above the layer threshold).
        let st = poll1(&mut s).state;
        assert_eq!((st.lt, st.rt), (0.0, 0.0));
        // At rest, as Android reports it: Z/RZ at -1.
        s.motion(1, axes(&[(axis::Z, -1.0), (axis::RZ, -1.0)]));
        let st = poll1(&mut s).state;
        assert_eq!((st.lt, st.rt), (0.0, 0.0));
        assert_eq!(st.right, [0.0, 0.0]);
        // Half and fully pulled; the right stick on RX/RY, +y up.
        s.motion(
            1,
            axes(&[
                (axis::Z, 0.0),
                (axis::RZ, 1.0),
                (axis::RX, 0.5),
                (axis::RY, 0.5),
            ]),
        );
        let st = poll1(&mut s).state;
        assert!(close(st.lt, 0.5) && close(st.rt, 1.0));
        assert_eq!(st.right, [0.5, -0.5]);
    }

    #[test]
    fn analog_triggers_with_the_right_stick_on_rx_ry() {
        let mut s = store_with(
            1,
            bits(&[
                axis::X,
                axis::Y,
                axis::RX,
                axis::RY,
                axis::LTRIGGER,
                axis::RTRIGGER,
            ]),
            bits(&[axis::X, axis::Y, axis::RX, axis::RY]),
            0.0,
        );
        s.motion(
            1,
            axes(&[
                (axis::RX, 0.75),
                (axis::RY, 0.25), // pulled down
                (axis::Z, 1.0),   // not this pad's: ignored
                (axis::LTRIGGER, 0.5),
            ]),
        );
        let st = poll1(&mut s).state;
        assert_eq!(st.right, [0.75, -0.25]);
        assert!(close(st.lt, 0.5));
        assert_eq!(st.rt, 0.0);
    }

    #[test]
    fn a_pad_without_a_right_stick_or_trigger_axes_reads_neither() {
        let mut s = store_with(
            1,
            bits(&[axis::X, axis::Y, axis::HAT_X, axis::HAT_Y]),
            bits(&[axis::X, axis::Y, axis::HAT_X, axis::HAT_Y]),
            0.0,
        );
        s.motion(
            1,
            axes(&[
                (axis::Z, 1.0),
                (axis::RZ, 1.0),
                (axis::RX, 1.0),
                (axis::RY, 1.0),
                (axis::LTRIGGER, 1.0),
            ]),
        );
        let st = poll1(&mut s).state;
        assert_eq!(st.right, [0.0, 0.0]);
        assert_eq!((st.lt, st.rt), (0.0, 0.0));
        // Its L2/R2 keys are the triggers.
        s.key(1, 105, true);
        assert_eq!(poll1(&mut s).state.rt, 1.0);
    }

    #[test]
    fn uncentered_z_rz_triggers_read_as_they_are() {
        let mut s = store_with(
            1,
            GENERIC,
            bits(&[axis::X, axis::Y, axis::RX, axis::RY]),
            0.0,
        );
        s.motion(1, axes(&[(axis::Z, 0.3), (axis::RZ, -0.2)]));
        let st = poll1(&mut s).state;
        assert!(close(st.lt, 0.3));
        assert_eq!(st.rt, 0.0, "clamped");
    }

    #[test]
    fn digital_triggers_come_from_the_l2_r2_keys() {
        let mut s = store_with(1, SWITCH_PRO, SWITCH_PRO, 0.0);
        s.key(1, 104, true);
        let st = poll1(&mut s).state;
        assert_eq!((st.lt, st.rt), (1.0, 0.0));
        assert_eq!(st.buttons, 0, "never the LT/RT bits");
        s.key(1, 105, true);
        s.key(1, 104, false);
        let st = poll1(&mut s).state;
        assert_eq!((st.lt, st.rt), (0.0, 1.0));
        // Switch Pro's right stick is Z/RZ.
        s.motion(1, axes(&[(axis::Z, 1.0), (axis::RZ, -1.0)]));
        assert_eq!(poll1(&mut s).state.right, [1.0, 1.0]);
    }

    #[test]
    fn y_is_up() {
        let mut s = store_with(1, DUALSENSE, DUALSENSE_CENTERED, 0.0);
        s.motion(1, axes(&[(axis::Y, 1.0), (axis::RZ, -1.0)]));
        let st = poll1(&mut s).state;
        assert_eq!(st.left, [0.0, -1.0]);
        assert_eq!(st.right, [0.0, 1.0]);
    }

    #[test]
    fn the_hat_is_a_dpad_ored_with_the_keys() {
        use PadButton::*;
        let mut s = store_with(1, DUALSENSE, DUALSENSE_CENTERED, 0.0);
        for (x, y, want) in [
            (-1.0, 0.0, Left.bit()),
            (1.0, 0.0, Right.bit()),
            (0.0, -1.0, Up.bit()),
            (0.0, 1.0, Down.bit()),
            (1.0, 1.0, Right.bit() | Down.bit()),
            (0.4, -0.4, 0),
        ] {
            s.motion(1, axes(&[(axis::HAT_X, x), (axis::HAT_Y, y)]));
            assert_eq!(poll1(&mut s).state.buttons, want, "hat {x},{y}");
        }
        s.motion(1, axes(&[(axis::HAT_X, -1.0)]));
        s.key(1, 19, true); // DPAD_UP
        assert_eq!(poll1(&mut s).state.buttons, Left.bit() | Up.bit());
        s.motion(1, axes(&[]));
        assert_eq!(poll1(&mut s).state.buttons, Up.bit());
    }

    #[test]
    fn the_flat_gates_each_stick_without_rescaling() {
        let mut s = store_with(1, DUALSENSE, DUALSENSE_CENTERED, 0.125);
        s.motion(
            1,
            axes(&[
                (axis::X, 0.06),
                (axis::Y, 0.08), // magnitude 0.1
                (axis::Z, 0.2),
            ]),
        );
        let st = poll1(&mut s).state;
        assert_eq!(st.left, [0.0, 0.0]);
        assert_eq!(st.right, [0.2, 0.0], "past the flat: passed through as is");
        // Radial: each axis inside the flat, the stick outside it.
        s.motion(1, axes(&[(axis::X, 0.1), (axis::Y, -0.1)]));
        assert_eq!(poll1(&mut s).state.left, [0.1, 0.1]);
    }

    #[test]
    fn the_flat_is_sanitized() {
        let mut s = store_with(1, DUALSENSE, DUALSENSE_CENTERED, f32::NAN);
        s.motion(1, axes(&[(axis::X, 0.01)]));
        assert_eq!(poll1(&mut s).state.left, [0.01, 0.0]);
        let mut s = store_with(1, DUALSENSE, DUALSENSE_CENTERED, 3.0);
        s.motion(1, axes(&[(axis::X, 0.6)]));
        assert_eq!(poll1(&mut s).state.left, [0.6, 0.0], "capped at 0.5");
    }

    #[test]
    fn a_tap_between_two_polls_shows_for_one_frame() {
        let mut s = store_with(1, DUALSENSE, DUALSENSE_CENTERED, 0.0);
        s.key(1, 96, true);
        s.key(1, 96, false);
        assert!(poll1(&mut s).state.pressed(PadButton::A));
        assert!(!poll1(&mut s).state.pressed(PadButton::A));
        // A hat tap too.
        s.motion(1, axes(&[(axis::HAT_Y, -1.0)]));
        s.motion(1, axes(&[]));
        assert!(poll1(&mut s).state.pressed(PadButton::Up));
        assert!(!poll1(&mut s).state.pressed(PadButton::Up));
    }

    #[test]
    fn a_held_key_stays_down_and_its_repeats_do_not_outlive_the_release() {
        let mut s = store_with(1, DUALSENSE, DUALSENSE_CENTERED, 0.0);
        s.key(1, 97, true);
        for _ in 0..3 {
            assert!(poll1(&mut s).state.pressed(PadButton::B));
        }
        s.key(1, 97, true); // a repeat
        s.key(1, 97, false);
        assert!(!poll1(&mut s).state.pressed(PadButton::B));
    }

    #[test]
    fn release_all_lets_go_and_returns_triggers_to_rest() {
        let mut s = store_with(1, GENERIC, GENERIC, 0.0);
        s.key(1, 96, true);
        s.key(1, 102, true);
        s.motion(
            1,
            axes(&[
                (axis::X, 1.0),
                (axis::RX, -1.0),
                (axis::Z, 1.0),
                (axis::RZ, 1.0),
                (axis::HAT_X, 1.0),
            ]),
        );
        s.release_all();
        let st = poll1(&mut s).state;
        assert_eq!(
            st,
            PadState {
                connected: true,
                ..Default::default()
            }
        );
        // The raw Z/RZ went back to -1, so the next unrelated event does not "pull" them.
        s.key(1, 99, true);
        let st = poll1(&mut s).state;
        assert_eq!((st.lt, st.rt), (0.0, 0.0));
        // Digital triggers too.
        let mut s = store_with(1, SWITCH_PRO, SWITCH_PRO, 0.0);
        s.key(1, 104, true);
        s.release_all();
        assert_eq!(poll1(&mut s).state.lt, 0.0);
    }

    #[test]
    fn poll_reports_id_name_category_and_connected() {
        let mut s = Store::new();
        s.connect(
            7,
            "DualSense Wireless Controller".into(),
            0x054C,
            0x0CE6,
            DUALSENSE,
            0,
            0.0,
        );
        s.connect(9, "  ".into(), 0, 0, SWITCH_PRO, 0, 0.0);
        let mut out = Vec::new();
        s.poll(&mut out);
        let p7 = out.iter().find(|p| p.id == 7).unwrap();
        assert_eq!(p7.name, "DualSense Wireless Controller");
        assert_eq!(p7.category, "DualSense");
        assert!(p7.state.connected);
        let p9 = out.iter().find(|p| p.id == 9).unwrap();
        assert_eq!(p9.name, "Controller");
        assert_eq!(p9.category, "");
    }

    #[test]
    fn the_pad_in_hand_comes_first() {
        let mut s = Store::new();
        s.connect(1, "Backbone One".into(), 0x358A, 0x0201, DUALSENSE, 0, 0.0);
        s.connect(2, "DualSense".into(), 0x054C, 0x0CE6, DUALSENSE, 0, 0.0);
        let order = |s: &mut Store| {
            let mut out = Vec::new();
            s.poll(&mut out);
            out.iter().map(|p| p.id).collect::<Vec<_>>()
        };
        // The one connected last was just switched on.
        assert_eq!(order(&mut s), [2, 1]);
        s.key(1, 96, true);
        assert_eq!(order(&mut s), [1, 2]);
        // A key up, a drifting stick or a half-rested trigger on the other pad changes nothing.
        s.key(2, 96, false);
        s.motion(2, axes(&[(axis::X, 0.2), (axis::LTRIGGER, 0.3)]));
        assert_eq!(order(&mut s), [1, 2]);
        // A real push does.
        s.motion(2, axes(&[(axis::RZ, -0.9)]));
        assert_eq!(order(&mut s), [2, 1]);
        s.motion(1, axes(&[(axis::HAT_X, 1.0)]));
        assert_eq!(order(&mut s), [1, 2]);
        s.motion(2, axes(&[(axis::RTRIGGER, 0.8)]));
        assert_eq!(order(&mut s), [2, 1]);
        // A key that maps to nothing still says which pad is in hand.
        s.key(1, 130, true);
        assert_eq!(order(&mut s), [1, 2]);
    }

    #[test]
    fn a_generic_pad_at_rest_never_takes_over() {
        let mut s = Store::new();
        s.connect(1, "DualSense".into(), 0x054C, 0x0CE6, DUALSENSE, 0, 0.0);
        s.connect(2, "Generic".into(), 0, 0, GENERIC, GENERIC, 0.0);
        let order = |s: &mut Store| {
            let mut out = Vec::new();
            s.poll(&mut out);
            out.iter().map(|p| p.id).collect::<Vec<_>>()
        };
        s.key(1, 96, true);
        assert_eq!(order(&mut s), [1, 2]);
        // The generic pad on the table streams its rest: Z/RZ (its triggers) at -1, which reads
        // as released, and a little stick drift.
        s.motion(
            2,
            axes(&[(axis::Z, -1.0), (axis::RZ, -1.0), (axis::RX, 0.1)]),
        );
        assert_eq!(order(&mut s), [1, 2]);
        // Its left trigger pulled past halfway (raw 0.2 → 0.6) is the player picking it up.
        s.motion(2, axes(&[(axis::Z, 0.2), (axis::RZ, -1.0)]));
        assert_eq!(order(&mut s), [2, 1]);
    }

    #[test]
    fn a_new_layout_lets_go_of_the_hat() {
        let mut s = store_with(1, DUALSENSE, DUALSENSE_CENTERED, 0.0);
        s.motion(1, axes(&[(axis::HAT_X, 1.0)]));
        assert!(poll1(&mut s).state.pressed(PadButton::Right));
        // Android reports the pad again with other axes: the old hat value means nothing now.
        s.connect(1, "Pad".into(), 0, 0, GENERIC, GENERIC, 0.0);
        assert_eq!(poll1(&mut s).state.buttons, 0);
        // The same layout again keeps it.
        s.motion(
            1,
            axes(&[(axis::HAT_X, 1.0), (axis::Z, -1.0), (axis::RZ, -1.0)]),
        );
        s.connect(1, "Pad".into(), 0, 0, GENERIC, GENERIC, 0.0);
        let _ = poll1(&mut s);
        assert!(poll1(&mut s).state.pressed(PadButton::Right));
    }

    #[test]
    fn reconnecting_keeps_the_state_and_the_place() {
        let mut s = Store::new();
        s.connect(1, "A".into(), 0, 0, DUALSENSE, 0, 0.0);
        s.connect(2, "B".into(), 0, 0, DUALSENSE, 0, 0.0);
        s.key(1, 96, true);
        s.motion(1, axes(&[(axis::X, 0.5)]));
        // Android reports pad 2 again: it does not jump ahead.
        assert!(s.connect(2, "B".into(), 0, 0, DUALSENSE, 0, 0.0).is_none());
        // Pad 1 again, renamed: held keys and axes kept.
        assert!(s
            .connect(1, "A2".into(), 0x045E, 0, DUALSENSE, 0, 0.0)
            .is_none());
        let mut out = Vec::new();
        s.poll(&mut out);
        assert_eq!(out[0].id, 1);
        assert_eq!(out[0].name, "A2");
        assert_eq!(out[0].category, "Xbox");
        assert!(out[0].state.pressed(PadButton::A));
        assert_eq!(out[0].state.left, [0.5, 0.0]);
        // Pad 1 gains different axes: the axes go back to the new rest, the keys stay.
        assert!(s
            .connect(1, "A2".into(), 0, 0, GENERIC, GENERIC, 0.0)
            .is_some());
        out.clear();
        s.poll(&mut out);
        let st = out[0].state;
        assert_eq!((st.left, st.lt, st.rt), ([0.0, 0.0], 0.0, 0.0));
        assert!(st.pressed(PadButton::A));
    }

    #[test]
    fn disconnect_and_unknown_ids() {
        let mut s = store_with(1, DUALSENSE, 0, 0.0);
        s.key(5, 96, true);
        s.motion(5, axes(&[(axis::X, 1.0)]));
        assert!(!s.disconnect(5));
        assert_eq!(poll1(&mut s).state.buttons, 0);
        assert!(s.disconnect(1));
        let mut out = Vec::new();
        s.poll(&mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn junk_axis_values_are_tamed() {
        let mut s = store_with(1, GENERIC, GENERIC, 0.0);
        s.motion(
            1,
            axes(&[
                (axis::X, f32::NAN),
                (axis::Y, f32::NEG_INFINITY),
                (axis::RX, 4.0),
                (axis::Z, f32::NAN),
                (axis::RZ, 9.0),
            ]),
        );
        let st = poll1(&mut s).state;
        assert_eq!(st.left, [0.0, 0.0]);
        assert_eq!(st.right, [1.0, 0.0]);
        assert_eq!((st.lt, st.rt), (0.0, 1.0));
        // Negative device ids (never seen, but jint) do not sign-extend into the u64.
        let mut s = store_with(-2, DUALSENSE, 0, 0.0);
        assert_eq!(poll1(&mut s).id, u64::from(u32::MAX - 1));
    }

    #[test]
    fn the_global_store_round_trip() {
        // The only test on the process-wide store (the others own theirs), with an id no other
        // could use.
        const ID: i32 = 0x7EAD_BEEF;
        connect(
            ID,
            "Xbox Wireless Controller".into(),
            0x045E,
            0x0B13,
            DUALSENSE,
            0,
            0.0,
        );
        key(ID, 96, true);
        key(ID, 96, false);
        motion(ID, axes(&[(axis::Y, -1.0)]));
        let mut out = Vec::new();
        poll(&mut out);
        let p = out.iter().find(|p| p.id == ID as u64).unwrap();
        assert_eq!(p.category, "Xbox");
        assert!(p.state.pressed(PadButton::A));
        assert_eq!(p.state.left, [0.0, 1.0]);
        release_all();
        out.clear();
        poll(&mut out);
        let p = out.iter().find(|p| p.id == ID as u64).unwrap();
        assert_eq!(p.state.buttons, 0);
        assert_eq!(p.state.left, [0.0, 0.0]);
        disconnect(ID);
        out.clear();
        poll(&mut out);
        assert!(out.iter().all(|p| p.id != ID as u64));
    }
}

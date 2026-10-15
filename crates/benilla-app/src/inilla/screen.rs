//! **Vanillish: the screen the HUD lays out on** — which kind of screen the game is on, and what
//! follows from it. A foldable has three: the narrow cover screen (a phone: iNilla's layout), the
//! near-square inner screen, and the inner screen half folded with the hinge across, standing on a
//! table (tabletop, Samsung's Flex mode). Each has its own touch layout in its own file
//! (`touch.toml`, `touch-tablet.toml`, `touch-tabletop.toml`), and the right one comes up as the
//! player folds and unfolds.
//!
//! - [`HudScreen`] is the screen every reader of the HUD's geometry uses — the touch engine, the
//!   HUD's drawing, the tooltips, the layout editor, the stock interface's inset and its scale:
//!   the window and its safe area, and on a tabletop in the world the world above the hinge and
//!   the deck below it, where the controls go.
//! - On a tabletop the world is drawn above the hinge only ([`WorldViewport`], which the world's
//!   target and its composite follow) and the stock interface ends there (`platform`'s
//!   `clear_the_island`); below it the HUD has a black deck to itself, where a drag on the empty
//!   middle looks around (the touch engine's look pad).
//! - A class counts once it has held for a moment (`ClassTracker`): a fold passes through other
//!   window sizes, and the hinge reports before the resize. On a switch the outgoing layout is
//!   saved at once, every finger is let go of, and the stock interface is re-anchored.
//! - The orientation: the cover screen plays landscape; the inner screen may follow the device
//!   into portrait (`inner_any_orientation`).
//! - Android's back swipe from a side edge is held off where the thumbs rest
//!   (`fold::edge_exclusions`) while the HUD shows.
//!
//! The model is engine-free, in `inilla_core::fold`; the hinge is the OS's
//! (`inilla_platform::device::fold_in_window`: Jetpack WindowManager on Android). A desktop run
//! rehearses one: `INILLA_TOUCH=1 INILLA_FOLD=half` in a near-square window (`flat` and `book`
//! too). iOS keeps iNilla's single layout: an iPhone is always a phone.

use bevy::math::URect;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use inilla_core::fold::{self, ClassTracker, Fold, ScreenClass};
use inilla_core::touch::Screen;
use inilla_platform::device::{self, Orientation};

use super::mouse::VirtualMouse;
use super::pad::PadHud;
use super::platform::SafeArea;
use super::touch::{hud_visible, TouchState};
use super::ui::Overlay;
use super::{store, Inilla, InputProducers};
use crate::char_select::ClientState;
use crate::world_backdrop::WorldViewport;

/// The screen the HUD lays out on, and the class of screen whose layout is in use.
#[derive(Resource)]
pub(crate) struct HudScreen {
    /// The window in points with its safe area; on a tabletop in the world, split at the hinge
    /// (`Screen::world` above it, `Screen::deck` below). Written only when it moves, so a reader
    /// can ask `is_changed`.
    pub(crate) screen: Screen,
    /// The class whose layout is `Inilla::layout`.
    pub(crate) class: ScreenClass,
    /// The class the screen is settling into.
    tracker: ClassTracker,
}

impl Default for HudScreen {
    fn default() -> Self {
        Self {
            screen: Screen::new(0.0, 0.0),
            class: ScreenClass::Phone,
            tracker: ClassTracker::default(),
        }
    }
}

/// The hinge as last asked of the OS, for a window of `size`, and when to ask again.
#[derive(Default)]
struct Hinge {
    fold: Option<Fold>,
    size: [f32; 2],
    next: f64,
}

/// How often the hinge is asked for while the window keeps its size (a JNI call on Android); a
/// new size asks at once. The class settles over `ClassTracker::SETTLE` anyway.
const HINGE_POLL: f64 = 0.1;

/// Where the system's edge swipes hold off: a band this wide (points) along each side edge…
const EXCLUSION_BAND: f32 = 32.0;
/// …at most this tall per edge, Android's limit (dp).
const EXCLUSION_BUDGET: f32 = 200.0;

pub(crate) struct ScreenPlugin;

impl Plugin for ScreenPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HudScreen>()
            // Before the touch pass, which lays the fingers out on this frame's screen.
            .add_systems(PreUpdate, track_screen.before(InputProducers))
            .add_systems(Update, (follow_orientation, hold_off_edge_swipes));
    }
}

/// Whether the screen's class can be anything but a phone here: on Android, and on a desktop
/// rehearsing a touch device. iOS keeps iNilla's one layout.
fn classes_apply() -> bool {
    !cfg!(target_os = "ios") && (cfg!(target_os = "android") || super::touch_device())
}

/// The window, its safe area and the hinge → [`HudScreen`]; a settled change of class swaps the
/// layout in use. The world's viewport follows the split.
#[allow(clippy::too_many_arguments)]
fn track_screen(
    windows: Query<&Window, With<PrimaryWindow>>,
    safe: Res<SafeArea>,
    client: Res<State<ClientState>>,
    time: Res<Time<Real>>,
    mut hud: ResMut<HudScreen>,
    mut inilla: ResMut<Inilla>,
    mut touch: ResMut<TouchState>,
    mut vm: ResMut<VirtualMouse>,
    viewport: Option<ResMut<WorldViewport>>,
    mut hinge: Local<Hinge>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let now = time.elapsed_secs_f64();
    let size = [window.width(), window.height()];
    if size != hinge.size || now >= hinge.next {
        hinge.fold = device::fold_in_window(size).map(|f| f.to_core());
        hinge.size = size;
        hinge.next = now + HINGE_POLL;
    }
    // The split is the player's to refuse: without it, a half-folded inner screen is the inner
    // screen.
    let hinge_fold = hinge.fold.filter(|_| inilla.settings.tabletop_split);
    let seen = if classes_apply() {
        fold::classify(size, hinge_fold.as_ref())
    } else {
        ScreenClass::Phone
    };

    let first = hud.tracker.current().is_none();
    if let Some(class) = hud.bypass_change_detection().tracker.observe(seen, now) {
        if inilla.layout.class != class {
            swap_layout(&mut inilla, class, !first);
            touch.drop_fingers();
            vm.reset();
            info!(
                "inilla: the screen is {} now ({:.0}x{:.0})",
                class.label(),
                size[0],
                size[1]
            );
        }
        if hud.class != class {
            hud.class = class;
        }
    } else if inilla.layout.class != hud.class {
        // Another screen's layout in the slot: "Reset all" (Settings, About) puts the phone's
        // shipped one there. This screen's shipped one is what was meant.
        let enabled = inilla.layout.enabled;
        inilla.layout = hud.class.defaults();
        inilla.layout.enabled = enabled;
    }

    // Split in the world only (the glue screens are the whole window), and only while the hinge
    // still says tabletop — not for the moment an unfold takes to settle.
    let base = safe.screen(size[0], size[1]);
    let split = *client.get() == ClientState::InWorld
        && hud.class == ScreenClass::Tabletop
        && seen == ScreenClass::Tabletop;
    let screen = if split {
        fold::screen_for(base, ScreenClass::Tabletop, hinge_fold.as_ref())
    } else {
        base
    };
    if hud.screen != screen {
        hud.screen = screen;
    }

    if let Some(mut viewport) = viewport {
        let want = screen
            .world
            .map(|world| world_viewport(window, world.max[1]));
        if viewport.0 != want {
            viewport.0 = want;
        }
    }
}

/// The world's rect in the window's physical pixels: the full width, from the top down to
/// `bottom` points (the hinge).
fn world_viewport(window: &Window, bottom: f32) -> URect {
    let width = window.physical_width().max(1);
    let height = window.physical_height().max(1);
    let bottom = ((bottom * window.scale_factor()).round() as u32).clamp(1, height);
    URect::new(0, 0, width, bottom)
}

/// Put `to`'s layout in use. The one in use is saved first, unless this is the launch's first
/// look at the screen (nothing has changed yet): the debounced save would lose an edit made in
/// the last second. It is kept for when its screen comes back; `to`'s comes from where it was
/// kept, or from its own file.
fn swap_layout(inilla: &mut Inilla, to: ScreenClass, save: bool) {
    if save {
        store::save_layout(&inilla.layout);
    }
    let incoming = inilla
        .other_layouts
        .remove(&to)
        .unwrap_or_else(|| store::load_layout_for(to));
    let outgoing = std::mem::replace(&mut inilla.layout, incoming);
    inilla.other_layouts.insert(outgoing.class, outgoing);
}

/// The orientation the game asks for: landscape, either way up — except that on the inner screen
/// the player may let it follow the device into portrait. A tabletop stays landscape: that is what
/// puts its hinge across.
fn follow_orientation(
    hud: Res<HudScreen>,
    inilla: Res<Inilla>,
    mut last: Local<Option<Orientation>>,
) {
    if !cfg!(target_os = "android") {
        return;
    }
    let want = match hud.class {
        ScreenClass::Tablet if inilla.settings.inner_any_orientation => Orientation::Any,
        _ => Orientation::Landscape,
    };
    if *last != Some(want) {
        *last = Some(want);
        device::set_orientation(want);
    }
}

/// Android's back gesture is a swipe in from either side edge, and a thumb landing on the stick
/// near the left edge would leave the game: while the HUD shows, the edges hold off beside the
/// elements a thumb rests on (at most 200 dp an edge, Android's limit). Hidden HUD: none.
#[allow(clippy::too_many_arguments)]
fn hold_off_edge_swipes(
    hud: Res<HudScreen>,
    inilla: Res<Inilla>,
    client: Res<State<ClientState>>,
    overlay: Res<Overlay>,
    pad: Res<PadHud>,
    touch: Res<TouchState>,
    time: Res<Time<Real>>,
    mut last: Local<Option<Vec<[f32; 4]>>>,
) {
    if !cfg!(target_os = "android") {
        return;
    }
    let now = time.elapsed_secs_f64();
    let pad_active = pad.active(now) && pad.last_active > touch.last_touch;
    let shown = hud_visible(&inilla, client.get(), &overlay, pad_active);
    let rects: Vec<[f32; 4]> = if shown {
        fold::edge_exclusions(
            &inilla.layout,
            &hud.screen,
            EXCLUSION_BAND,
            EXCLUSION_BUDGET,
        )
        .into_iter()
        .map(|r| [r.min[0], r.min[1], r.max[0], r.max[1]])
        .collect()
    } else {
        Vec::new()
    };
    if last.as_ref() != Some(&rects) {
        device::set_gesture_exclusion(&rects);
        *last = Some(rects);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use inilla_core::pad::PadProfile;
    use inilla_core::settings::InillaSettings;

    fn inilla(layout: inilla_core::touch::TouchLayout) -> Inilla {
        Inilla {
            settings: InillaSettings::default(),
            launcher: Default::default(),
            layout,
            other_layouts: Default::default(),
            pad: PadProfile::default(),
        }
    }

    /// A fold swaps the layout in use for the new screen's, keeps the outgoing one for when its
    /// screen comes back, and an unfold brings that one back as it was left.
    #[test]
    fn a_switch_puts_the_screens_layout_in_use_and_keeps_the_others() {
        let mut phone = ScreenClass::Phone.defaults();
        phone.hud_scale = 1.3; // the player's own
        let mut tablet = ScreenClass::Tablet.defaults();
        tablet.opacity = 0.4;
        let mut k = inilla(phone.clone());
        k.other_layouts.insert(ScreenClass::Tablet, tablet.clone());

        swap_layout(&mut k, ScreenClass::Tablet, false);
        assert_eq!(k.layout, tablet);
        assert_eq!(k.other_layouts.get(&ScreenClass::Phone), Some(&phone));
        assert!(!k.other_layouts.contains_key(&ScreenClass::Tablet));

        k.layout.opacity = 0.9; // edited on the inner screen
        swap_layout(&mut k, ScreenClass::Phone, false);
        assert_eq!(k.layout, phone, "the cover's layout as it was left");
        let kept = &k.other_layouts[&ScreenClass::Tablet];
        assert_eq!(kept.opacity, 0.9, "the inner screen's edit is kept");
    }

    /// On a tabletop the world is drawn in the window's physical pixels above the hinge, from the
    /// top left; a rect past the window is cut to it.
    #[test]
    fn the_world_viewport_is_the_window_above_the_hinge_in_pixels() {
        let mut w = Window::default();
        // A Galaxy Z Fold8 Ultra's inner screen: 2504 × 2256 px at density 2.625.
        w.resolution = bevy::window::WindowResolution::new(2504, 2256);
        w.resolution.set_scale_factor(2.625);
        let hinge = w.height() / 2.0;
        let r = world_viewport(&w, hinge);
        assert_eq!(r.min, UVec2::ZERO);
        assert_eq!(r.max, UVec2::new(2504, 1128));
        assert_eq!(world_viewport(&w, 5000.0).max.y, 2256);
        assert_eq!(world_viewport(&w, -3.0).max.y, 1, "never empty");
    }
}

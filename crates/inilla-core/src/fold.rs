//! **Vanillish: screens of more than one shape** — a foldable's narrow cover screen, its
//! near-square inner screen, and the inner screen half-folded on a table: which class a screen is,
//! one touch layout per class, and the split of a half-folded screen into the world (above the
//! hinge) and the deck (below it, where the touch controls go).
//!
//! Nothing here is Android's: the app feeds in the window size and the hinge the OS reports (on
//! Android, Jetpack WindowManager's `FoldingFeature`), and gets back the class, the screen to lay
//! the HUD out on, the interface scale, and where the system's edge swipes must hold off.
//!
//! Positions are logical points (iOS points, Android dp), origin top-left, `+y` down, as in
//! [`crate::touch`].

use serde::{Deserialize, Serialize};

use crate::pad::{PadBinding, Special};
use crate::settings::InillaSettings;
use crate::touch::{
    default_layout, Anchor, Element, ElementKind, Hold, Rect, Screen, TouchFeel, TouchLayout, Vec2,
};

/// The kind of screen a touch layout is made for. Each has its own layout file.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
pub enum ScreenClass {
    /// A phone in landscape, or a foldable's cover screen: wide and short, thumbs in the corners.
    #[default]
    Phone,
    /// A near-square screen held in two hands: a foldable's inner screen, a small tablet.
    Tablet,
    /// A foldable half-folded with the hinge across: the world above it, the controls below.
    Tabletop,
}

impl ScreenClass {
    pub const ALL: [ScreenClass; 3] = [
        ScreenClass::Phone,
        ScreenClass::Tablet,
        ScreenClass::Tabletop,
    ];

    /// The layout's file in `benilla-config/inilla/`. The phone keeps iNilla's `touch.toml`.
    pub fn file_name(self) -> &'static str {
        match self {
            ScreenClass::Phone => "touch.toml",
            ScreenClass::Tablet => "touch-tablet.toml",
            ScreenClass::Tabletop => "touch-tabletop.toml",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ScreenClass::Phone => "Phone / cover screen",
            ScreenClass::Tablet => "Inner screen",
            ScreenClass::Tabletop => "Tabletop (half folded)",
        }
    }

    /// The shipped layout for this class.
    pub fn defaults(self) -> TouchLayout {
        match self {
            ScreenClass::Phone => default_layout(),
            ScreenClass::Tablet => tablet_layout(),
            ScreenClass::Tabletop => tabletop_layout(),
        }
    }
}

/// How far a foldable is folded (Jetpack WindowManager's `FoldingFeature.State`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FoldState {
    Flat,
    HalfOpened,
}

/// The hinge, as the platform reports it, in window points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fold {
    pub state: FoldState,
    /// The hinge runs across the window (left to right): the halves are above and below it.
    pub horizontal: bool,
    /// The hinge's bounds; a fold that hides nothing (a flexible screen) is a zero-height line.
    pub bounds: Rect,
}

/// The shortest side, in points, from which a screen is a tablet (Android's `sw600dp`).
pub const TABLET_MIN_SIDE: f32 = 600.0;
/// Wider than this (long side ÷ short side), a screen is laid out as a phone however big it is.
pub const TABLET_MAX_ASPECT: f32 = 1.6;
/// Each half of a tabletop screen is at least this tall, or the screen is not split.
pub const TABLETOP_MIN_HALF: f32 = 240.0;
/// The deck keeps this far clear of the crease, where a touch is least precise.
pub const DECK_GAP: f32 = 12.0;

/// The class of a window `size` points big, on a device whose hinge (if any) is `fold`.
pub fn classify(size: Vec2, fold: Option<&Fold>) -> ScreenClass {
    if let Some(f) = fold {
        let above = f.bounds.min[1];
        let below = size[1] - f.bounds.max[1];
        if f.state == FoldState::HalfOpened
            && f.horizontal
            && above >= TABLETOP_MIN_HALF
            && below >= TABLETOP_MIN_HALF
        {
            return ScreenClass::Tabletop;
        }
    }
    let short = size[0].min(size[1]);
    let long = size[0].max(size[1]);
    if short >= TABLET_MIN_SIDE && long / short.max(1.0) <= TABLET_MAX_ASPECT {
        ScreenClass::Tablet
    } else {
        ScreenClass::Phone
    }
}

/// The screen the HUD lays out on for `class`: on a tabletop, split at the hinge.
pub fn screen_for(base: Screen, class: ScreenClass, fold: Option<&Fold>) -> Screen {
    match (class, fold) {
        (ScreenClass::Tabletop, Some(f)) => base.split_at_hinge(f.bounds, DECK_GAP),
        _ => base,
    }
}

/// Follows the class through a fold or an unfold: the window passes through sizes on the way and
/// the hinge reports before the resize, so a new class counts once it has held for
/// [`ClassTracker::SETTLE`] seconds. The first class seen counts at once.
#[derive(Clone, Debug, Default)]
pub struct ClassTracker {
    current: Option<ScreenClass>,
    pending: Option<(ScreenClass, f64)>,
}

impl ClassTracker {
    pub const SETTLE: f64 = 0.35;

    pub fn current(&self) -> Option<ScreenClass> {
        self.current
    }

    /// Note what the screen looks like now; `Some(class)` when the class switches.
    pub fn observe(&mut self, class: ScreenClass, now: f64) -> Option<ScreenClass> {
        match self.current {
            None => {
                self.current = Some(class);
                return Some(class);
            }
            Some(c) if c == class => {
                self.pending = None;
                return None;
            }
            Some(_) => {}
        }
        match self.pending {
            Some((p, since)) if p == class => {
                if now - since >= Self::SETTLE {
                    self.pending = None;
                    self.current = Some(class);
                    return Some(class);
                }
            }
            _ => self.pending = Some((class, now)),
        }
        None
    }
}

/// The width of the 1.12 main action bar (`MainMenuBar`, four 256-unit textures) in UI units: an
/// automatic scale keeps it on the screen.
const MAIN_BAR_UNITS: f32 = 1024.0;

/// The smallest `uiScale` the 1.12 client applies (`SetUiScale`'s floor, benilla-ui's
/// `UI_SCALE_FLOOR`): its CVar callback raises a smaller one to it, and its display handler — a
/// resize, a reload — drops a smaller one for the automatic scale (0.9 on the inner screen), so a
/// scale under it would not be the one `UIParent` has.
pub const UI_SCALE_FLOOR: f32 = 0.64;

/// The scale `UIParent` ends up with when the `uiScale` CVar is set to `requested` on a window
/// `window` points big — the 1.12 ladder benilla-ui applies (`ui_scale.rs`): the CVar as written
/// (two decimals), the dial's `0.5..=1.5`, capped at `0.75·w/h` below a 4:3 aspect, then floored
/// at [`UI_SCALE_FLOOR`]. An anchor offset given to `UIParent` is multiplied by it.
pub fn ui_parent_scale(requested: f32, window: Vec2) -> f32 {
    let [w, h] = window;
    let mut s = ((requested * 100.0).round() / 100.0).clamp(0.5, 1.5);
    if w > 0.0 && h > 0.0 && (w / h) * 3.0 < 4.0 {
        s = s.min(w / h * 0.75);
    }
    s.max(UI_SCALE_FLOOR)
}

/// The interface size for `class` on a `window` points big: iNilla's phone rule on a phone; on
/// the inner screen a stock action button at fingertip size (36 points) with a screen to spare; on
/// a tabletop the phone's rule applied to the half above the hinge (`world_h` tall) — `UIParent`
/// then spans that half with about the 600–614 units a phone's screen has, so the quest and vendor
/// windows fit above the crease (no less than [`UI_SCALE_FLOOR`], the smallest scale the client
/// applies).
///
/// `UIParent` is `768·w/h ÷ scale` units wide, and on a near-square screen the height rule alone
/// leaves it narrower than the main action bar (954 units on the inner screen, 859 in portrait):
/// an automatic scale stops where the bar still fits. A phone keeps iNilla's rule exactly — on a
/// 2:1 screen the bar fits far above it anyway.
pub fn ui_scale(settings: &InillaSettings, class: ScreenClass, window: Vec2, world_h: f32) -> f32 {
    let [w, h] = window;
    let auto = settings.ui_scale <= 0.0;
    let bar_fits = if w > 0.0 && h > 0.0 {
        768.0 * w / h / MAIN_BAR_UNITS
    } else {
        f32::INFINITY
    };
    match class {
        ScreenClass::Phone => settings.effective_ui_scale(h),
        ScreenClass::Tablet => {
            if !auto || h <= 0.0 {
                settings.effective_ui_scale(h)
            } else {
                (768.0 / h).min(bar_fits).clamp(0.64, 1.25)
            }
        }
        ScreenClass::Tabletop => {
            if h <= 0.0 || world_h <= 0.0 {
                return settings.effective_ui_scale(h);
            }
            let half = settings.effective_ui_scale(world_h) * world_h / h;
            let s = if auto { half.min(bar_fits) } else { half };
            // On the Z Fold8 Ultra the phone's rule asks 0.625 here: the client would make it
            // 0.64 at once, and the automatic 0.9 at the next resize.
            s.clamp(UI_SCALE_FLOOR, 1.5)
        }
    }
}

/// Where the screen edges must not start a system gesture (Android's back swipe from the side):
/// the elements a thumb rests on near each side edge. Android honours at most 200 dp of height
/// per edge; each edge's rect is cut to `budget`, around the middle of what it covers.
pub fn edge_exclusions(layout: &TouchLayout, screen: &Screen, band: f32, budget: f32) -> Vec<Rect> {
    let w = screen.size[0];
    let rects: Vec<Rect> = layout
        .shown()
        .map(|(_, e)| e.rect(screen, layout.hud_scale))
        .collect();
    let mut out = Vec::new();
    for left in [true, false] {
        let (x0, x1) = if left { (0.0, band) } else { (w - band, w) };
        let mut span: Option<(f32, f32)> = None;
        for r in rects.iter().filter(|r| r.min[0] <= x1 && r.max[0] >= x0) {
            span = Some(span.map_or((r.min[1], r.max[1]), |(a, b)| {
                (a.min(r.min[1]), b.max(r.max[1]))
            }));
        }
        if let Some((a, b)) = span {
            let mid = (a + b) / 2.0;
            let half = ((b - a) / 2.0).min(budget / 2.0);
            out.push(Rect::new([x0, mid - half], [x1, mid + half]));
        }
    }
    out
}

/// `n` points on an arc of `radius` around `center`, from `from` to `to` degrees (screen angles:
/// 0 is right, 90 down).
fn arc(center: Vec2, radius: f32, from: f32, to: f32, n: usize) -> Vec<Vec2> {
    (0..n)
        .map(|i| {
            let t = if n > 1 {
                i as f32 / (n - 1) as f32
            } else {
                0.5
            };
            let a = (from + (to - from) * t).to_radians();
            [center[0] + a.cos() * radius, center[1] + a.sin() * radius]
        })
        .collect()
}

fn el(
    id: &str,
    kind: ElementKind,
    anchor: Anchor,
    offset: Vec2,
    size: f32,
    views: &[&str],
) -> Element {
    Element {
        id: id.into(),
        kind,
        anchor,
        offset,
        size: [size, size],
        opacity: 1.0,
        visible: true,
        label: None,
        views: views.iter().map(|v| v.to_string()).collect(),
        hold: Hold::Tooltip,
        strip: None,
    }
}

fn labelled(mut e: Element, label: &str) -> Element {
    e.label = Some(label.into());
    e
}

const EVERY_VIEW_BUT_MINIMAL: [&str; 3] = ["Combat", "Explore", "Social"];

/// One view's six verbs: `(id, binding, label)`.
type Verbs = [(&'static str, PadBinding, &'static str); 6];

/// The Explore and Social views' six verbs, as the phone has them.
fn verbs() -> [(&'static str, Verbs); 2] {
    [
        (
            "Explore",
            [
                ("autorun", PadBinding::cmd("TOGGLEAUTORUN"), "Run"),
                ("map", PadBinding::cmd("TOGGLEWORLDMAP"), "Map"),
                ("bags", PadBinding::cmd("OPENALLBAGS"), "Bags"),
                ("sit", PadBinding::cmd("SITORSTAND"), "Sit"),
                ("mount", PadBinding::cmd("ACTIONBUTTON12"), "Mount"),
                ("quests", PadBinding::cmd("TOGGLEQUESTLOG"), "Quests"),
            ],
        ),
        (
            "Social",
            [
                ("chat", PadBinding::cmd("OPENCHAT"), "Chat"),
                ("reply", PadBinding::cmd("REPLY"), "Reply"),
                ("wave", PadBinding::Chat("/wave".into()), "Wave"),
                ("sit_social", PadBinding::cmd("SITORSTAND"), "Sit"),
                ("social", PadBinding::cmd("TOGGLESOCIAL"), "Friends"),
                ("follow", PadBinding::cmd("FOLLOWTARGET"), "Follow"),
            ],
        ),
    ]
}

/// Jump with two rings of buttons around it — `inner` (slots 1–6, the views' verbs) and `outer`
/// (slots 7–12, Combat only) — anchored at `anchor`, `jump` from it.
fn cluster(anchor: Anchor, jump: Vec2, inner: &[Vec2], outer: &[Vec2]) -> Vec<Element> {
    let mut v = Vec::new();
    let mut j = labelled(
        el(
            "jump",
            ElementKind::Button(PadBinding::cmd("JUMP")),
            anchor,
            jump,
            84.0,
            &[],
        ),
        "Jump",
    );
    j.hold = Hold::Hub;
    v.push(j);
    for (i, at) in inner.iter().enumerate() {
        let n = i as u32 + 1;
        v.push(el(
            &format!("slot{n}"),
            ElementKind::ActionSlot(n),
            anchor,
            *at,
            58.0,
            &["Combat"],
        ));
    }
    for (i, at) in outer.iter().enumerate() {
        let n = i as u32 + 7;
        v.push(el(
            &format!("slot{n}"),
            ElementKind::ActionSlot(n),
            anchor,
            *at,
            52.0,
            &["Combat"],
        ));
    }
    for (view, list) in verbs() {
        for (i, (id, binding, label)) in list.into_iter().enumerate() {
            v.push(labelled(
                el(
                    id,
                    ElementKind::Button(binding),
                    anchor,
                    inner[i],
                    54.0,
                    &[view],
                ),
                label,
            ));
        }
    }
    v
}

/// The system buttons — the radial, the view, iNilla's menu — in a row `y` under `anchor`.
fn system_row(anchor: Anchor, y: f32) -> Vec<Element> {
    vec![
        labelled(
            el("radial", ElementKind::Radial, anchor, [-52.0, y], 44.0, &[]),
            "☰",
        ),
        labelled(
            el("view", ElementKind::NextView, anchor, [0.0, y], 44.0, &[]),
            "View",
        ),
        labelled(
            el(
                "inilla",
                ElementKind::Button(PadBinding::Special(Special::InillaMenu)),
                anchor,
                [52.0, y],
                44.0,
                &[],
            ),
            "N",
        ),
    ]
}

fn layout(class: ScreenClass, elements: Vec<Element>, feel: TouchFeel) -> TouchLayout {
    TouchLayout {
        class,
        elements,
        feel,
        ..default_layout()
    }
}

/// The inner screen held in two hands (≈ 954 × 859 points on a Galaxy Z Fold8 Ultra, 158 × 143
/// mm). The hands grip the side edges below the middle, so everything anchors to the *sides*
/// (`Left`/`Right`, measured from the vertical middle) rather than to the corners, and survives a
/// taller or shorter window: the stick under the left thumb's rest, jump under the right's with
/// slots 1–6 all round it and slots 7–12 on an outer arc toward the middle (≤ 61 mm from the edge,
/// a thumb's reach); target, interact and autorun over the stick for the left thumb at rest. The
/// system buttons stay top-centre, and the hub (hold jump) reaches them all from the thumb.
pub fn tablet_layout() -> TouchLayout {
    use Anchor::*;
    let jump = [-170.0, 140.0];
    let inner = arc(jump, 106.0, -90.0, 210.0, 6);
    let outer = arc(jump, 184.0, 120.0, 270.0, 6);
    let stick = [170.0, 170.0];
    let mut elements = vec![Element {
        size: [320.0, 340.0],
        ..el("stick", ElementKind::Stick, Left, stick, 0.0, &[])
    }];
    let utility = arc(stick, 250.0, 250.0, 300.0, 3);
    elements.push(labelled(
        el(
            "target",
            ElementKind::Button(PadBinding::cmd("TARGETNEARESTENEMY")),
            Left,
            utility[0],
            56.0,
            &EVERY_VIEW_BUT_MINIMAL,
        ),
        "Tab",
    ));
    elements.push(labelled(
        el(
            "interact",
            ElementKind::Button(PadBinding::Special(Special::Interact)),
            Left,
            utility[1],
            56.0,
            &EVERY_VIEW_BUT_MINIMAL,
        ),
        "Use",
    ));
    elements.push(labelled(
        el(
            "autorun_left",
            ElementKind::Button(PadBinding::cmd("TOGGLEAUTORUN")),
            Left,
            utility[2],
            56.0,
            &EVERY_VIEW_BUT_MINIMAL,
        ),
        "Run",
    ));
    elements.extend(system_row(Top, 30.0));
    elements.extend(cluster(Right, jump, &inner, &outer));
    let feel = TouchFeel {
        stick_radius: 64.0,
        hub_radii: [124.0, 210.0],
        radial_at_thumb: true,
        ..TouchFeel::default()
    };
    layout(ScreenClass::Tablet, elements, feel)
}

/// The inner screen half-folded on a table, hinge across: the world and the stock interface above
/// the hinge, and below it ([`Screen::deck`], ≈ 954 × 418 points on a Z Fold8 Ultra) the controls,
/// played with the fingers of both hands from above like a pad: the stick on the left, jump and
/// twelve slots on the right, target and interact at the bottom middle, the system buttons along
/// the crease. The empty middle is a look pad (a drag looks; nothing there clicks the world).
pub fn tabletop_layout() -> TouchLayout {
    use Anchor::*;
    let jump = [-150.0, 30.0];
    let inner = arc(jump, 100.0, -90.0, 210.0, 6);
    let outer = arc(jump, 190.0, 140.0, 262.0, 6);
    let mut elements = vec![Element {
        size: [360.0, 380.0],
        ..el("stick", ElementKind::Stick, Left, [190.0, 10.0], 0.0, &[])
    }];
    elements.push(labelled(
        el(
            "target",
            ElementKind::Button(PadBinding::cmd("TARGETNEARESTENEMY")),
            Bottom,
            [-44.0, -46.0],
            64.0,
            &EVERY_VIEW_BUT_MINIMAL,
        ),
        "Tab",
    ));
    elements.push(labelled(
        el(
            "interact",
            ElementKind::Button(PadBinding::Special(Special::Interact)),
            Bottom,
            [44.0, -46.0],
            64.0,
            &EVERY_VIEW_BUT_MINIMAL,
        ),
        "Use",
    ));
    elements.extend(system_row(Top, 28.0));
    elements.extend(cluster(Right, jump, &inner, &outer));
    let feel = TouchFeel {
        stick_radius: 72.0,
        hub_radii: [112.0, 180.0],
        radial_at_thumb: true,
        ..TouchFeel::default()
    };
    let mut l = layout(ScreenClass::Tabletop, elements, feel);
    l.opacity = 0.8;
    l
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::touch::{
        MouseButton, MouseOp, Phase, TouchEngine, TouchHub, TouchInput, LAYOUT_VERSION,
    };

    /// A Galaxy Z Fold8 Ultra in landscape at density 2.625: the cover screen (2520 × 1080 px)
    /// and the inner one (2504 × 2256 px), in dp.
    const COVER: Vec2 = [960.0, 411.4];
    const INNER: Vec2 = [953.9, 859.4];

    fn hinge() -> Fold {
        let y = INNER[1] / 2.0;
        Fold {
            state: FoldState::HalfOpened,
            horizontal: true,
            bounds: Rect::new([0.0, y], [INNER[0], y]),
        }
    }

    fn tabletop() -> Screen {
        screen_for(
            Screen::new(INNER[0], INNER[1]),
            ScreenClass::Tabletop,
            Some(&hinge()),
        )
    }

    #[test]
    fn the_cover_is_a_phone_the_inner_screen_a_tablet_and_half_folded_a_tabletop() {
        assert_eq!(classify(COVER, None), ScreenClass::Phone);
        assert_eq!(
            classify([852.0, 393.0], None),
            ScreenClass::Phone,
            "an iPhone"
        );
        assert_eq!(classify(INNER, None), ScreenClass::Tablet);
        assert_eq!(
            classify([1194.0, 834.0], None),
            ScreenClass::Tablet,
            "an iPad"
        );
        assert_eq!(
            classify([1280.0, 720.0], None),
            ScreenClass::Phone,
            "a wide window"
        );
        let flat = Fold {
            state: FoldState::Flat,
            ..hinge()
        };
        assert_eq!(classify(INNER, Some(&flat)), ScreenClass::Tablet);
        assert_eq!(classify(INNER, Some(&hinge())), ScreenClass::Tabletop);
        let book = Fold {
            horizontal: false,
            ..hinge()
        };
        assert_eq!(
            classify(INNER, Some(&book)),
            ScreenClass::Tablet,
            "a vertical hinge"
        );
        let low = Fold {
            bounds: Rect::new([0.0, 700.0], [INNER[0], 700.0]),
            ..hinge()
        };
        assert_eq!(
            classify(INNER, Some(&low)),
            ScreenClass::Tablet,
            "no room for a deck"
        );
    }

    #[test]
    fn a_class_counts_once_it_has_settled() {
        let mut t = ClassTracker::default();
        assert_eq!(t.observe(ScreenClass::Phone, 0.0), Some(ScreenClass::Phone));
        // Unfolding: the window passes through a tablet-sized frame and back for a moment.
        assert_eq!(t.observe(ScreenClass::Tablet, 1.0), None);
        assert_eq!(t.observe(ScreenClass::Phone, 1.1), None);
        assert_eq!(t.observe(ScreenClass::Tablet, 1.2), None);
        assert_eq!(t.observe(ScreenClass::Tablet, 1.4), None);
        assert_eq!(
            t.observe(ScreenClass::Tablet, 1.6),
            Some(ScreenClass::Tablet)
        );
        assert_eq!(t.current(), Some(ScreenClass::Tablet));
        assert_eq!(t.observe(ScreenClass::Tablet, 5.0), None);
    }

    #[test]
    fn a_tabletop_splits_at_the_hinge() {
        let s = tabletop();
        let world = s.world.unwrap();
        let deck = s.deck.unwrap();
        assert_eq!(world.min, [0.0, 0.0]);
        assert!((world.max[1] - INNER[1] / 2.0).abs() < 0.01);
        assert!(deck.min[1] >= INNER[1] / 2.0 + DECK_GAP - 0.01);
        assert_eq!(s.layout_rect(), deck);
        assert_eq!(s.strip, None);
    }

    /// Every shown element of every view is on `screen` (inside `area`), and no two pressable
    /// ones overlap.
    fn check(layout: &TouchLayout, screen: &Screen, area: Rect) {
        for view in &layout.views {
            let mut l = layout.clone();
            l.active_view = view.clone();
            let rects: Vec<(String, Rect)> = l
                .shown()
                .filter(|(_, e)| !matches!(e.kind, ElementKind::Stick))
                .map(|(_, e)| (e.id.clone(), e.rect(screen, l.hud_scale)))
                .collect();
            for (id, r) in &rects {
                assert!(
                    r.min[0] >= area.min[0] - 0.5
                        && r.min[1] >= area.min[1] - 0.5
                        && r.max[0] <= area.max[0] + 0.5
                        && r.max[1] <= area.max[1] + 0.5,
                    "{view}: {id} {r:?} is outside {area:?}"
                );
            }
            for (i, (a, ra)) in rects.iter().enumerate() {
                for (b, rb) in &rects[i + 1..] {
                    let apart = ra.max[0] <= rb.min[0]
                        || rb.max[0] <= ra.min[0]
                        || ra.max[1] <= rb.min[1]
                        || rb.max[1] <= ra.min[1];
                    assert!(apart, "{view}: {a} {ra:?} overlaps {b} {rb:?}");
                }
            }
        }
    }

    #[test]
    fn the_inner_screen_layout_fits_and_stays_within_a_thumb_of_the_sides() {
        let l = tablet_layout();
        for size in [
            INNER,
            [INNER[1], INNER[0]],
            [1194.0, 834.0],
            [1133.0, 744.0],
            [1024.0, 768.0],
        ] {
            let s = Screen::new(size[0], size[1]);
            check(&l, &s, s.safe_rect());
            for (_, e) in l.shown() {
                let c = e.rect(&s, l.hud_scale).center();
                let from_side = c[0].min(size[0] - c[0]);
                // ≈ 61 mm at 160 dp per inch: a thumb gripping the side reaches it.
                assert!(
                    from_side <= 385.0 || e.anchor == Anchor::Top,
                    "{} is out of reach",
                    e.id
                );
            }
        }
        let s = Screen::new(INNER[0], INNER[1]);
        let stick = l.stick_zone(&s).unwrap();
        assert!(
            stick.max[0] < INNER[0] / 2.0,
            "the stick is the left thumb's"
        );
    }

    #[test]
    fn the_tabletop_layout_fits_the_deck() {
        let l = tabletop_layout();
        let s = tabletop();
        check(&l, &s, s.deck.unwrap());
        let stick = l.stick_zone(&s).unwrap();
        assert!(
            stick.min[1] >= s.deck.unwrap().min[1] - 0.5,
            "the stick stays under the hinge"
        );
    }

    #[test]
    fn the_hub_opens_inside_the_screen_it_belongs_to() {
        for (l, s) in [
            (tablet_layout(), Screen::new(INNER[0], INNER[1])),
            (tabletop_layout(), tabletop()),
        ] {
            let jump = l.elements.iter().find(|e| e.id == "jump").unwrap();
            let c = jump.rect(&s, l.hud_scale).center();
            let [a, b] = l.feel.hub_radii;
            let hub = TouchHub::open_sized(c, &s, &l.views, [a, b]);
            let area = s.layout_rect();
            for ring in 0..2 {
                for i in 0..hub.rings[ring].len() {
                    let p = hub.pos(ring, i);
                    assert!(
                        area.inflate(-16.0).contains(p),
                        "{:?} entry {ring}/{i} at {p:?}",
                        l.class
                    );
                    assert_eq!(hub.pick(p), Some((ring, i)));
                }
            }
        }
    }

    fn t(id: u64, phase: Phase, p: Vec2) -> TouchInput {
        TouchInput { id, phase, pos: p }
    }

    #[test]
    fn on_a_tabletop_a_drag_on_the_deck_looks_and_a_tap_there_clicks_nothing() {
        let l = tabletop_layout();
        let s = tabletop();
        let world = s.world.unwrap();
        let no_ui = |_: Vec2| false;
        // The middle of the deck: between the stick and the buttons.
        let p = [INNER[0] / 2.0, s.deck.unwrap().center()[1]];
        let mut e = TouchEngine::default();
        e.input(t(1, Phase::Started, p), &l, &s, 0.0, &no_ui);
        e.input(
            t(1, Phase::Moved, [p[0] + 40.0, p[1]]),
            &l,
            &s,
            0.05,
            &no_ui,
        );
        let f = e.frame(&l, 0.05);
        assert_eq!(
            f.mouse[0],
            MouseOp::MoveTo(world.center()),
            "the look starts over the world"
        );
        assert_eq!(f.mouse[1], MouseOp::Down(MouseButton::Right));
        assert!(matches!(f.mouse[2], MouseOp::Motion([x, _]) if x > 0.0));
        e.input(t(1, Phase::Ended, [p[0] + 40.0, p[1]]), &l, &s, 0.1, &no_ui);
        assert!(e
            .frame(&l, 0.1)
            .mouse
            .contains(&MouseOp::Up(MouseButton::Right)));

        let mut e = TouchEngine::default();
        e.input(t(2, Phase::Started, p), &l, &s, 0.0, &no_ui);
        assert!(
            e.frame(&l, 0.6).mouse.is_empty(),
            "a hold on the deck right-clicks nothing"
        );
        e.input(t(2, Phase::Ended, p), &l, &s, 0.7, &no_ui);
        let f = e.frame(&l, 0.7);
        assert!(
            !f.mouse.iter().any(|m| matches!(m, MouseOp::Down(_))),
            "nor does a tap"
        );

        // Above the hinge the world is the world: a tap selects.
        let mut e = TouchEngine::default();
        let up = [INNER[0] / 2.0, 200.0];
        e.input(t(3, Phase::Started, up), &l, &s, 0.0, &no_ui);
        e.input(t(3, Phase::Ended, up), &l, &s, 0.05, &no_ui);
        assert!(e
            .frame(&l, 0.05)
            .mouse
            .contains(&MouseOp::Down(MouseButton::Left)));
    }

    #[test]
    fn a_look_from_the_deck_starts_where_no_window_covers_the_world() {
        let l = tabletop_layout();
        let s = tabletop();
        let world = s.world.unwrap();
        // The world map covers the middle of the top half.
        let map = Rect::from_center(world.center(), [500.0, 260.0]);
        let ui = move |p: Vec2| map.contains(p);
        let p = [INNER[0] / 2.0, s.deck.unwrap().center()[1]];
        let mut e = TouchEngine::default();
        e.input(t(1, Phase::Started, p), &l, &s, 0.0, &ui);
        e.input(t(1, Phase::Moved, [p[0], p[1] + 30.0]), &l, &s, 0.05, &ui);
        let f = e.frame(&l, 0.05);
        let MouseOp::MoveTo(at) = f.mouse[0] else {
            panic!("{:?}", f.mouse);
        };
        assert!(world.contains(at) && !map.contains(at), "{at:?}");
        assert_eq!(f.mouse[1], MouseOp::Down(MouseButton::Right));
    }

    #[test]
    fn on_the_inner_screen_more_opens_the_radial_by_the_thumb() {
        let l = tablet_layout();
        let s = Screen::new(INNER[0], INNER[1]);
        let jump = l.elements.iter().find(|e| e.id == "jump").unwrap();
        let c = jump.rect(&s, l.hud_scale).center();
        let mut e = TouchEngine::default();
        let no_ui = |_: Vec2| false;
        e.input(t(1, Phase::Started, c), &l, &s, 0.0, &no_ui);
        let hub = e.frame(&l, 0.35).hub.unwrap();
        let more = hub.pos(1, hub.rings[1].len() - 1);
        e.input(t(1, Phase::Moved, more), &l, &s, 0.4, &no_ui);
        e.input(t(1, Phase::Ended, more), &l, &s, 0.5, &no_ui);
        let r = e.frame(&l, 0.5).radial.expect("the radial is up");
        assert!(r.radius <= 170.0);
        assert!(
            INNER[0] - (r.center[0] - r.radius) <= 420.0,
            "{r:?}: the far wedge is a thumb away"
        );
        assert!(r.center[1] + r.radius * 1.35 <= INNER[1], "{r:?} fits");
    }

    #[test]
    fn a_hovering_pen_is_the_mouse_and_explains_a_hud_button() {
        let l = tablet_layout();
        let s = Screen::new(INNER[0], INNER[1]);
        let mut e = TouchEngine::default();
        e.hover(Some([480.0, 300.0]), &l, &s, 0.0);
        assert_eq!(
            e.frame(&l, 0.0).mouse,
            vec![MouseOp::MoveTo([480.0, 300.0])]
        );
        let i = l.elements.iter().position(|e| e.id == "slot2").unwrap();
        let c = l.elements[i].rect(&s, l.hud_scale).center();
        e.hover(Some(c), &l, &s, 1.0);
        let f = e.frame(&l, 1.0);
        assert_eq!(
            f.mouse,
            vec![MouseOp::Leave],
            "over the HUD the cursor goes"
        );
        assert!(f.pressed.is_empty() && f.tooltip.is_none());
        let f = e.frame(&l, 1.6);
        assert_eq!(f.tooltip, Some(i), "hovering explains it");
        assert!(f.pressed.is_empty(), "and presses nothing");
        e.hover(None, &l, &s, 2.0);
        assert_eq!(e.frame(&l, 2.0).tooltip, None);
    }

    #[test]
    fn a_finger_down_outranks_the_hover() {
        let l = tablet_layout();
        let s = Screen::new(INNER[0], INNER[1]);
        let mut e = TouchEngine::default();
        e.input(t(1, Phase::Started, [480.0, 300.0]), &l, &s, 0.0, &|_| {
            false
        });
        e.frame(&l, 0.0);
        e.hover(Some([100.0, 100.0]), &l, &s, 0.1);
        assert!(!e
            .frame(&l, 0.1)
            .mouse
            .contains(&MouseOp::MoveTo([100.0, 100.0])));
    }

    /// The deck below the hinge has no world under it: a pen hovering there moves no cursor (the
    /// stock `WorldFrame` would pick through it what is not drawn), and one coming down from the
    /// world takes the cursor away.
    #[test]
    fn a_pen_over_the_deck_points_at_nothing() {
        let l = tabletop_layout();
        let s = tabletop();
        let mut e = TouchEngine::default();
        let up = [INNER[0] / 2.0, 200.0];
        e.hover(Some(up), &l, &s, 0.0);
        assert_eq!(e.frame(&l, 0.0).mouse, vec![MouseOp::MoveTo(up)]);
        let p = [INNER[0] / 2.0, s.deck.unwrap().center()[1]];
        e.hover(Some(p), &l, &s, 0.1);
        assert_eq!(e.frame(&l, 0.1).mouse, vec![MouseOp::Leave]);
        e.hover(Some([p[0], p[1] + 10.0]), &l, &s, 0.2);
        assert!(e.frame(&l, 0.2).mouse.is_empty());
    }

    #[test]
    fn the_interface_fits_above_the_hinge() {
        let st = InillaSettings::default();
        let window_h = INNER[1];
        let world_h = INNER[1] / 2.0;
        let s = ui_scale(&st, ScreenClass::Tabletop, INNER, world_h);
        // The scale asked for is the one the client applies (not under its floor).
        assert_eq!(ui_parent_scale(s, INNER), s, "{s}");
        // UIParent's height in its own units: the half's share of the 768-unit root ÷ the scale.
        let units = 768.0 * (world_h / window_h) / s;
        assert!(
            (599.5..=640.0).contains(&units),
            "{units} units above the hinge at {s}"
        );
        // A phone keeps iNilla's rule; the inner screen gets bigger stock buttons than it would.
        assert_eq!(ui_scale(&st, ScreenClass::Phone, COVER, COVER[1]), 1.25);
        assert_eq!(
            ui_scale(&st, ScreenClass::Phone, [852.0, 393.0], 393.0),
            st.effective_ui_scale(393.0),
            "an iPhone is untouched"
        );
        let tablet = ui_scale(&st, ScreenClass::Tablet, INNER, window_h);
        assert!(tablet > st.effective_ui_scale(window_h));
        assert!(768.0 / tablet >= 614.0, "and the windows still fit");
        // The main action bar (1024 units) stays on the screen, in landscape and in portrait.
        for size in [INNER, [INNER[1], INNER[0]]] {
            let s = ui_scale(&st, ScreenClass::Tablet, size, size[1]);
            let wide = 768.0 * size[0] / size[1] / s;
            assert!(wide >= 1023.5, "{size:?}: {wide} units wide at {s}");
        }
        let wide = 768.0 * INNER[0] / INNER[1] / s;
        assert!(wide >= 1023.5, "tabletop: {wide} units wide");
        // The player's own scale is theirs — on the half above the hinge, as on a phone.
        let pinned = InillaSettings {
            ui_scale: 1.2,
            ..Default::default()
        };
        assert_eq!(ui_scale(&pinned, ScreenClass::Tablet, INNER, window_h), 1.2);
        let half = ui_scale(&pinned, ScreenClass::Tabletop, INNER, world_h);
        assert_eq!(
            half, UI_SCALE_FLOOR,
            "1.2 on the half is 0.6: the client's floor"
        );
        let pinned = InillaSettings {
            ui_scale: 1.4,
            ..Default::default()
        };
        let half = ui_scale(&pinned, ScreenClass::Tabletop, INNER, world_h);
        assert!((half - 0.7).abs() < 0.01, "{half}");
    }

    /// The scale `UIParent` gets for a `uiScale` (the 1.12 ladder benilla-ui applies), which the
    /// interface's insets are divided by.
    #[test]
    fn uiparent_gets_the_scale_the_client_ladder_makes_of_the_cvar() {
        // A phone, and anything 4:3 or wider: the CVar as written, two decimals.
        assert_eq!(ui_parent_scale(1.25, COVER), 1.25);
        assert_eq!(ui_parent_scale(0.8875, [852.0, 393.0]), 0.89);
        assert_eq!(ui_parent_scale(0.9, [1024.0, 768.0]), 0.9);
        // Never under the floor, never past the dial.
        assert_eq!(ui_parent_scale(0.625, COVER), UI_SCALE_FLOOR);
        assert_eq!(ui_parent_scale(2.0, COVER), 1.5);
        // Narrower than 4:3: capped at 0.75·w/h — the inner screen's, its portrait's.
        let cap = INNER[0] / INNER[1] * 0.75;
        assert!((ui_parent_scale(1.2, INNER) - cap).abs() < 1e-6);
        assert_eq!(ui_parent_scale(0.64, INNER), 0.64);
        assert_eq!(
            ui_parent_scale(1.0, [INNER[1], INNER[0]]),
            INNER[1] / INNER[0] * 0.75
        );
        // A degenerate window keeps the request.
        assert_eq!(ui_parent_scale(1.0, [0.0, 0.0]), 1.0);
        // Every automatic scale is applied as asked (to the CVar's two decimals).
        let st = InillaSettings::default();
        for (class, size, world_h) in [
            (ScreenClass::Phone, COVER, COVER[1]),
            (ScreenClass::Tablet, INNER, INNER[1]),
            (ScreenClass::Tablet, [INNER[1], INNER[0]], INNER[0]),
            (ScreenClass::Tabletop, INNER, INNER[1] / 2.0),
        ] {
            let s = ui_scale(&st, class, size, world_h);
            assert!(
                (ui_parent_scale(s, size) - s).abs() <= 0.005 + 1e-6,
                "{class:?} {size:?}: {s}"
            );
        }
    }

    #[test]
    fn the_back_swipe_is_held_off_where_the_thumbs_rest() {
        let l = tablet_layout();
        let s = Screen::new(INNER[0], INNER[1]);
        let rects = edge_exclusions(&l, &s, 32.0, 200.0);
        for r in &rects {
            assert!(
                r.size()[1] <= 200.0 + 0.01,
                "Android honours 200 dp an edge: {r:?}"
            );
        }
        // The stick zone runs to the left edge; the right cluster keeps clear of the right one.
        assert_eq!(rects.len(), 1, "{rects:?}");
        let stick = l.stick_zone(&s).unwrap().center();
        assert!(
            rects[0].min[0] == 0.0 && rects[0].min[1] <= stick[1] && stick[1] <= rects[0].max[1]
        );
    }

    #[test]
    fn each_class_round_trips_and_upgrades_from_its_own_defaults() {
        for class in ScreenClass::ALL {
            let l = class.defaults();
            assert_eq!(l.class, class);
            let back: TouchLayout = toml::from_str(&toml::to_string(&l).unwrap()).unwrap();
            assert_eq!(l, back);
        }
        // A file iNilla wrote has no class: it is the phone's.
        let mut phone = toml::to_string(&default_layout()).unwrap();
        phone = phone.replace("class = \"Phone\"\n", "");
        let back: TouchLayout = toml::from_str(&phone).unwrap();
        assert_eq!(back.class, ScreenClass::Phone);
        // An older inner-screen layout gains the inner screen's elements, not the phone's.
        let mut old = tablet_layout();
        old.version = LAYOUT_VERSION - 1;
        old.elements.retain(|e| e.id != "slot12");
        old.upgrade();
        let slot12 = old.elements.iter().find(|e| e.id == "slot12").unwrap();
        assert_eq!(slot12.anchor, Anchor::Right);
    }
}

//! Vanillish: the opt-in modern lighting look, one boot-latched switch for every world surface
//! shader. Off, nothing changes: no def is pushed and every pipeline compiles today's WGSL. On,
//! `VANILLISH_MODERN` (with its two strength values) is pushed into the terrain, model, WDL and
//! liquid materials' `specialize` and into the static_gx pipelines, whose WGSL then:
//!
//! - lights the sun (the zone's `Light.dbc` direct and ambient colours, the same sun direction)
//!   per pixel, terrain's Gouraud diffuse and row-9 specular included;
//! - tilts the normal by a relief read from the albedo itself: four extra taps of the texture at
//!   one texel of the main sample's mip, luminance as height, in a tangent frame built from
//!   screen-space derivatives (Schüler's cotangent frame), so no mesh needs tangents or new assets;
//! - darkens the cavities those taps find (cracks between bricks and planks);
//! - adds a soft, warm Blinn-Phong sheen on exterior lit surfaces, gated by the texel's
//!   luminance (terrain keeps its authored `_s` mask);
//! - lights the three point lights each draw already picks per pixel, so a torch throws a round
//!   pool instead of a vertex diamond;
//! - on water, the sun sheen per pixel and a fresnel lift toward the sky colour at grazing views.
//!
//! The shared maths is `shaders/vanillish_modern.wgsl`. Everything stays in the gamma space the
//! frame expects (the UI gamma contract, the FFX combine): no new pass, no new render target.
//!
//! A process global, as [`crate::tex_filter`] is, because a pipeline's defs are fixed when it is
//! specialized, by render-world code that shares no resource with the CVar table. Latched at boot:
//! `benilla_app` publishes it after `config.toml` is folded in (the `modernLighting` CVar);
//! `$VANILLISH_MODERN` overrides the CVar for the session, and the world viewer, which has no CVar
//! table, reads the environment alone. One variant set per process, so the pipeline warm pass,
//! which spawns materials of every lane, compiles the modern pipelines in place of the stock ones.

use std::ops::RangeInclusive;
use std::sync::OnceLock;

use bevy::prelude::*;
use bevy::render::render_resource::RenderPipelineDescriptor;
use bevy::shader::ShaderDefVal;

/// The shader def every world surface shader keys the look on.
pub const MODERN_DEF: &str = "VANILLISH_MODERN";
/// The relief's strength def, a percent of the tuned default.
pub const RELIEF_DEF: &str = "VANILLISH_RELIEF";
/// The sheen's strength def, a percent of the tuned default.
pub const SHEEN_DEF: &str = "VANILLISH_SHEEN";

/// The strengths' range in percent: 0 turns the term off, 400 is four times the tuned default.
pub const PERCENT_RANGE: RangeInclusive<u32> = 0..=400;

/// What a strength lever that is unset or unparseable leaves: the tuned default.
pub const PERCENT_DEFAULT: u32 = 100;

/// The look's switch and its two strengths, as the shaders receive them.
#[derive(Resource, Clone, Copy, PartialEq, Eq, Debug)]
pub struct ModernLighting {
    /// The `modernLighting` CVar, or `$VANILLISH_MODERN`, which overrides it.
    pub on: bool,
    /// `$VANILLISH_RELIEF`: the relief and cavity strength, percent.
    pub relief: u32,
    /// `$VANILLISH_SHEEN`: the sheen strength, percent.
    pub sheen: u32,
}

impl ModernLighting {
    /// The reference's look, which every setting boots at.
    pub const STOCK: Self = Self {
        on: false,
        relief: PERCENT_DEFAULT,
        sheen: PERCENT_DEFAULT,
    };

    /// The session's levers, read from their raw values: `modern` as `$VANILLISH_MODERN` (unset
    /// leaves the CVar's say, `""`, `0`, `off` and `false` are off, anything else on), the two
    /// strengths as percents, clamped to [`PERCENT_RANGE`].
    pub fn from_levers(modern: Option<&str>, relief: Option<&str>, sheen: Option<&str>) -> Self {
        let percent = |v: Option<&str>| {
            v.and_then(|v| v.trim().parse::<u32>().ok())
                .map_or(PERCENT_DEFAULT, |p| {
                    p.clamp(*PERCENT_RANGE.start(), *PERCENT_RANGE.end())
                })
        };
        Self {
            on: modern.is_some_and(lever_flag),
            relief: percent(relief),
            sheen: percent(sheen),
        }
    }

    /// Whether `$VANILLISH_MODERN` decides this session, whatever `config.toml` holds.
    pub fn env_owned() -> bool {
        std::env::var("VANILLISH_MODERN").is_ok()
    }

    /// The defs a world pipeline takes: none when off, so the stock WGSL compiles unchanged.
    pub fn shader_defs(self) -> Vec<ShaderDefVal> {
        if !self.on {
            return Vec::new();
        }
        vec![
            ShaderDefVal::from(MODERN_DEF),
            ShaderDefVal::UInt(RELIEF_DEF.into(), self.relief),
            ShaderDefVal::UInt(SHEEN_DEF.into(), self.sheen),
        ]
    }

    /// Push [`Self::shader_defs`] into both stages of a material pipeline: the vertex stage picks
    /// the point lights for the fragment stage, so the two must agree on the interface.
    pub fn specialize(self, descriptor: &mut RenderPipelineDescriptor) {
        let defs = self.shader_defs();
        if defs.is_empty() {
            return;
        }
        descriptor.vertex.shader_defs.extend(defs.iter().cloned());
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment.shader_defs.extend(defs);
        }
    }
}

impl Default for ModernLighting {
    /// The stock look unless `$VANILLISH_MODERN` asks for the modern one, with the strengths from
    /// `$VANILLISH_RELIEF` and `$VANILLISH_SHEEN`.
    fn default() -> Self {
        let env = |k: &str| std::env::var(k).ok();
        Self::from_levers(
            env("VANILLISH_MODERN").as_deref(),
            env("VANILLISH_RELIEF").as_deref(),
            env("VANILLISH_SHEEN").as_deref(),
        )
    }
}

/// A switch lever's value: `""`, `0`, `off` and `false` (any case) are off, anything else on.
fn lever_flag(v: &str) -> bool {
    let v = v.trim().to_ascii_lowercase();
    !matches!(v.as_str(), "" | "0" | "off" | "false")
}

static POLICY: OnceLock<ModernLighting> = OnceLock::new();

/// Publish the look once, from the CVar load (`benilla_app`, after `CvarLoad`): after
/// `config.toml` is folded in, before the render world specializes any pipeline. A later call
/// changes nothing; the first wins, as the pipelines already compiled with it.
pub fn publish_modern_lighting(setting: ModernLighting) {
    let _ = POLICY.set(setting);
}

/// The look every world pipeline takes; unpublished (the world viewer, a headless test), it is
/// [`ModernLighting::default`], the environment's say.
pub fn modern_lighting() -> ModernLighting {
    *POLICY.get_or_init(ModernLighting::default)
}

#[cfg(test)]
mod compose_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::render::render_resource::{FragmentState, VertexState};

    #[test]
    fn the_stock_look_takes_no_def() {
        assert!(ModernLighting::STOCK.shader_defs().is_empty());
        assert_eq!(
            ModernLighting::from_levers(None, None, None),
            ModernLighting::STOCK,
            "with no lever set the look is the reference's"
        );
    }

    #[test]
    fn the_switch_reads_like_the_other_session_levers() {
        for off in ["", "0", "off", "OFF", "false", " 0 "] {
            assert!(
                !ModernLighting::from_levers(Some(off), None, None).on,
                "{off:?}"
            );
        }
        for on in ["1", "on", "yes", "true", "2"] {
            assert!(
                ModernLighting::from_levers(Some(on), None, None).on,
                "{on:?}"
            );
        }
    }

    #[test]
    fn the_strengths_are_percents_clamped_to_their_range() {
        let s = ModernLighting::from_levers(Some("1"), Some("150"), Some(" 40 "));
        assert_eq!((s.relief, s.sheen), (150, 40));
        let s = ModernLighting::from_levers(Some("1"), Some("9999"), Some("-3"));
        assert_eq!(s.relief, *PERCENT_RANGE.end());
        assert_eq!(
            s.sheen, PERCENT_DEFAULT,
            "a value that is no percent is the default"
        );
        let s = ModernLighting::from_levers(Some("1"), Some("0"), Some("x"));
        assert_eq!((s.relief, s.sheen), (0, PERCENT_DEFAULT));
    }

    #[test]
    fn the_modern_look_pushes_its_defs_into_both_stages() {
        let look = ModernLighting {
            on: true,
            relief: 120,
            sheen: 80,
        };
        let defs = look.shader_defs();
        assert_eq!(
            defs,
            vec![
                ShaderDefVal::Bool(MODERN_DEF.into(), true),
                ShaderDefVal::UInt(RELIEF_DEF.into(), 120),
                ShaderDefVal::UInt(SHEEN_DEF.into(), 80),
            ]
        );
        let mut descriptor = RenderPipelineDescriptor {
            vertex: VertexState {
                shader_defs: vec!["WOW_RIG_SKIN".into()],
                ..default()
            },
            fragment: Some(FragmentState::default()),
            ..default()
        };
        look.specialize(&mut descriptor);
        assert_eq!(
            descriptor.vertex.shader_defs.len(),
            4,
            "appended, not replaced"
        );
        assert_eq!(descriptor.fragment.as_ref().unwrap().shader_defs, defs);
        // Off, the descriptor is untouched: the stock pipelines' cache keys do not move.
        let mut stock = RenderPipelineDescriptor {
            fragment: Some(FragmentState::default()),
            ..default()
        };
        ModernLighting::STOCK.specialize(&mut stock);
        assert!(stock.vertex.shader_defs.is_empty());
        assert!(stock.fragment.unwrap().shader_defs.is_empty());
    }
}

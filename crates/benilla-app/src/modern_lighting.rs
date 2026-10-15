//! Vanillish: the `modernLighting` CVar, the switch for the opt-in modern lighting look
//! ([`benilla_assets::ModernLighting`]): the observer that keeps its knob, and the boot latch that
//! publishes the knob once, after `config.toml` is folded in, before the render world specializes
//! a pipeline. A change applies at the next launch, as `trilinear` and `anisotropic` do.
//! `$VANILLISH_MODERN` owns the row for a session (`cvars::session_values`), so an A/B never sticks
//! in the file; `$VANILLISH_RELIEF` and `$VANILLISH_SHEEN` set the two strengths.

use bevy::prelude::*;

use benilla_assets::ModernLighting;

/// The `modernLighting` change callback: a flag, int-parsed and `!= 0`.
pub(crate) fn on_cvar(ev: On<crate::cvars::CvarChanged>, mut knob: ResMut<ModernLighting>) {
    if ev.is("modernLighting") {
        knob.on = ev.flag();
    }
}

/// Freeze the look for the process and say which it is, so a player's log names the run's look.
fn publish(knob: Res<ModernLighting>) {
    benilla_assets::publish_modern_lighting(*knob);
    let look = benilla_assets::modern_lighting();
    if look.on {
        info!(
            "lighting: modern (relief {} %, sheen {} %){}",
            look.relief,
            look.sheen,
            if ModernLighting::env_owned() {
                " — $VANILLISH_MODERN"
            } else {
                ""
            }
        );
    } else {
        info!("lighting: the reference's");
    }
}

/// The knob, its observer and the boot latch. Before `CvarPlugin`, so the knob exists when
/// `load_config` applies the saved value.
pub(crate) fn plugin(app: &mut App) {
    app.init_resource::<ModernLighting>()
        .add_observer(on_cvar)
        .add_systems(Startup, publish.after(crate::cvars::CvarLoad));
}

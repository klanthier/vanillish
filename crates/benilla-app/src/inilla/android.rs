//! **Vanillish**: the Android host's Bevy side — what needs Bevy's `AndroidApp` or the app's
//! schedules (the soft keyboard's text, the native window's size, the app lifecycle). (Filled in by
//! the input work.)

use bevy::prelude::*;

pub(crate) struct AndroidPlugin;

impl Plugin for AndroidPlugin {
    fn build(&self, _app: &mut App) {}
}

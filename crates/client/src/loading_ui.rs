//! Boot/loading cover: hides the HUD skeleton until the local player exists.
//!
//! Filled in by the loading workstream. The DOM `#boot` block in `index.html` covers the gap
//! before wasm starts; this Bevy overlay takes over once the app is running.

use bevy::prelude::*;

pub struct LoadingUiPlugin;

impl Plugin for LoadingUiPlugin {
    fn build(&self, _app: &mut App) {}
}

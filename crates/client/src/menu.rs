//! In-game menu overlay (Esc): resume, controls help, leave the match.
//!
//! The game does not pause (it is multiplayer); the overlay only blocks pointer input. The
//! session's input stack owns the Esc key and toggles [`MenuOpen`].

use bevy::prelude::*;

/// Whether the Esc menu is open.
#[derive(Resource, Default)]
pub struct MenuOpen(pub bool);

pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MenuOpen>();
    }
}

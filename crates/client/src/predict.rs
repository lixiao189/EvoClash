//! Client-side prediction of the local player, plus smoothing between fixed ticks for rendering.

use bevy::prelude::*;
use evoclash_game::protocol::{Dash, PlayerInput, Pos, Vitals};
use evoclash_game::sim::{self, Arena};
use lightyear::prelude::input::native::ActionState;
use lightyear::prelude::*;

/// The last two simulated positions, so frames between ticks can be interpolated.
#[derive(Component, Default)]
pub struct TickSamples {
    pub prev: Vec2,
    pub curr: Vec2,
}

impl TickSamples {
    pub fn at(&self, overstep: f32) -> Vec2 {
        self.prev.lerp(self.curr, overstep)
    }
}

pub struct PredictPlugin;

impl Plugin for PredictPlugin {
    fn build(&self, app: &mut App) {
        // Runs again for every re-simulated tick during a rollback, using the same rules as the server.
        app.add_systems(FixedUpdate, predicted_movement);
        app.add_systems(FixedLast, sample_ticks);
    }
}

fn predicted_movement(
    arena: Res<Arena>,
    mut players: Query<(&mut Pos, &mut Dash, &ActionState<PlayerInput>, &Vitals), With<Predicted>>,
) {
    for (pos, dash, action, vitals) in &mut players {
        sim::step_movement(pos, dash, &action.0, vitals, &arena);
    }
}

fn sample_ticks(
    mut commands: Commands,
    mut players: Query<(Entity, &Pos, Option<&mut TickSamples>), With<Predicted>>,
) {
    for (entity, pos, samples) in &mut players {
        match samples {
            Some(mut samples) => {
                // Teleports (respawn) shouldn't smear across the map.
                samples.prev = if samples.curr.distance(pos.0) > 200.0 {
                    pos.0
                } else {
                    samples.curr
                };
                samples.curr = pos.0;
            }
            None => {
                commands.entity(entity).insert(TickSamples {
                    prev: pos.0,
                    curr: pos.0,
                });
            }
        }
    }
}

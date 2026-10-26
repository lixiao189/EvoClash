//! Client-side prediction of the local player, plus smoothing between fixed ticks for rendering.

use bevy::prelude::*;
use evoclash_game::evolution::{Evolution, SkillState};
use evoclash_game::protocol::{Dash, MatchState, Phase, PlayerInput, Pos, Prop, Vitals};
use evoclash_game::sim::{self, Arena, Obstacles};
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
        app.add_systems(FixedUpdate, (track_obstacles, predicted_movement).chain());
        app.add_systems(FixedLast, sample_ticks);
    }
}

/// Crates and barrels block movement while they stand, same as on the server.
fn track_obstacles(
    mut obstacles: ResMut<Obstacles>,
    props: Query<(&Pos, &Prop, &Vitals), Without<Predicted>>,
) {
    obstacles.0.clear();
    obstacles.0.extend(
        props
            .iter()
            .filter(|(.., v)| v.alive)
            .map(|(p, prop, _)| (p.0, prop.kind.radius())),
    );
}

fn predicted_movement(
    arena: Res<Arena>,
    obstacles: Res<Obstacles>,
    state: Query<&MatchState>,
    mut players: Query<
        (
            &mut Pos,
            &mut Dash,
            &ActionState<PlayerInput>,
            &Vitals,
            Option<&Evolution>,
            Option<&SkillState>,
        ),
        With<Predicted>,
    >,
) {
    // Starting freezes everyone. Zero the same axes the server clears before `step_movement`
    // (`gameplay/movement.rs`); a copy so the input we send is unchanged.
    let frozen = state.iter().any(|s| s.phase == Phase::Starting);
    // Perks and skills change speed; use the server's numbers or a speed perk would rubber-band.
    for (pos, dash, action, vitals, evo, skills) in &mut players {
        let evo = evo.cloned().unwrap_or_default();
        let skills = skills.copied().unwrap_or_default();
        let mut input = action.0.clone();
        if frozen {
            input.move_x = 0;
            input.move_y = 0;
            input.fire = false;
            input.dash = false;
            input.interact = false;
            input.throw = false;
            input.skill_a = false;
            input.skill_b = false;
        }
        sim::step_movement(
            pos,
            dash,
            &input,
            vitals,
            &evo,
            &skills,
            &arena,
            &obstacles.0,
        );
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

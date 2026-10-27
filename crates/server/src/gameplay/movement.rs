//! Tick counter, human input, movement, position history and cooldowns.

use super::*;
use crate::journal::Injected;

pub(super) fn advance_tick(mut tick: ResMut<SimTick>) {
    tick.0 = tick.0.wrapping_add(1);
}

pub fn read_human_inputs(
    mut humans: Query<
        (
            &mut CurrentInput,
            &mut Rewind,
            &Human,
            &Player,
            Option<&ActionState<PlayerInput>>,
            Option<&ControlledBy>,
        ),
        With<Human>,
    >,
    delays: Query<&InterpolationDelay>,
    specs: Query<&crate::spectate::SpectatorSession>,
    mut injected: Option<ResMut<Injected>>,
) {
    let scripted = injected
        .as_mut()
        .map(|injected| std::mem::take(&mut injected.inputs))
        .unwrap_or_default();
    let scripted: Vec<_> = scripted
        .into_iter()
        .filter(|(id, _)| !crate::spectate::is_spectator(&specs, *id))
        .collect();
    for (mut current, mut rewind, human, player, action, control) in &mut humans {
        if !human.connected() {
            // Disconnected bodies stand still instead of replaying the last held keys.
            current.0 = PlayerInput::default();
            continue;
        }
        if let Some((_, input)) = scripted.iter().find(|(id, _)| *id == player.client_id) {
            current.0 = input.clone();
        } else if let Some(action) = action {
            current.0 = action.0.clone();
        }
        rewind.0 = control
            .and_then(|c| delays.get(c.owner).ok())
            .map(|d| (d.delay.tick_diff() + 1).min(MAX_REWIND_TICKS))
            .unwrap_or(0);
    }
}

/// Attacking gives up spawn protection. Runs after every input (human or bot) is in.
pub(super) fn drop_shield_on_attack(mut players: Query<(&CurrentInput, &mut Vitals)>) {
    for (input, mut vitals) in &mut players {
        if vitals.shield > 0 && (input.0.fire || input.0.throw) {
            vitals.shield = 0;
        }
    }
}

pub(super) fn movement(
    arena: Res<Arena>,
    obstacles: Res<Obstacles>,
    state: Query<&MatchState>,
    mut players: Query<(
        &mut Pos,
        &mut Dash,
        &mut Aim,
        &mut CurrentInput,
        &Vitals,
        &Evolution,
        &SkillState,
    )>,
) {
    let frozen = state.iter().any(|s| s.phase == Phase::Starting);
    for (pos, dash, mut aim, mut input, vitals, evo, skills) in &mut players {
        if frozen {
            // Zeroed before step_movement and before Combat reads the same component.
            input.0.move_x = 0;
            input.0.move_y = 0;
            input.0.fire = false;
            input.0.dash = false;
            input.0.interact = false;
            input.0.throw = false;
            input.0.skill_a = false;
            input.0.skill_b = false;
        }
        sim::step_movement(
            pos,
            dash,
            &input.0,
            vitals,
            evo,
            skills,
            &arena,
            &obstacles.0,
        );
        let dir = input.0.aim.normalize_or_zero();
        if vitals.alive && dir != Vec2::ZERO && dir.distance_squared(aim.0) > 1e-4 {
            aim.0 = dir;
        }
    }
}

pub(super) fn record_history(tick: Res<SimTick>, mut players: Query<(&Pos, &mut PosHistory)>) {
    for (pos, mut history) in &mut players {
        history.0.push_back((tick.0, pos.0));
        if history.0.len() > HISTORY_LEN {
            history.0.pop_front();
        }
    }
}

pub(super) fn tick_cooldowns(mut players: Query<(&mut Cooldowns, &mut Vitals)>) {
    for (mut cd, mut vitals) in &mut players {
        cd.fire = cd.fire.saturating_sub(1);
        cd.interact = cd.interact.saturating_sub(1);
        cd.throw = cd.throw.saturating_sub(1);
        if vitals.emp_slow > 0 {
            vitals.emp_slow -= 1;
        }
        if vitals.shield > 0 {
            vitals.shield -= 1;
        }
        if vitals.stealth > 0 {
            vitals.stealth -= 1;
        }
        if vitals.revealed > 0 {
            vitals.revealed -= 1;
        }
    }
}

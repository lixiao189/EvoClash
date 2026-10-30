//! Seat hold. A dropped human keeps their seat until the results screen after the drop ends.
//!
//! The body leaves the map the moment the link goes and comes back with it. While it is out,
//! nothing can reach it: every targeting, collision, pickup and hazard path already skips a
//! dead combatant, so the seat is simply `Vitals::alive = false`. It stays replicated: clients
//! already hide a dead body (and leave it off the minimap) once it has fallen, while the
//! scoreboard and results still list the seat. No protocol change. Level, experience, gear, bag, score,
//! kills, deaths and streak stay frozen because nothing writes to a dead body, and a seat that
//! was already dead stays dead.

use super::*;
use crate::connections::{ClearedSeats, Human};
use crate::journal::Trace;

/// How many `Results -> Lobby` transitions a drop in `phase` has to survive. The results
/// screen that is already up when the player drops does not count as one of them.
pub fn hold_rounds(phase: Phase) -> u8 {
    if phase == Phase::Results { 2 } else { 1 }
}

/// Out of the map while the seat is held. `body`, `pos` and `dash` are what the seat looked
/// like at the drop, put back when the link returns outside Playing (a Playing reconnect
/// respawns on a spawn point instead, with the usual spawn shield).
#[derive(Component)]
pub struct Withdrawn {
    pub body: Vitals,
    pub pos: Vec2,
    pub dash: Dash,
}

/// Input, after the presence systems of this tick: keep every body in step with its seat.
/// A drop takes the body out of the map at once, a reconnect puts it back.
pub(crate) fn sync_seats(
    mut commands: Commands,
    state: Query<&MatchState>,
    mut seats: Query<(
        Entity,
        &Human,
        &mut Vitals,
        &mut Pos,
        &mut Dash,
        &mut CurrentInput,
        Option<&Withdrawn>,
        Option<&Replicate>,
    )>,
) {
    let phase = state.iter().next().map(|s| s.phase).unwrap_or(Phase::Lobby);
    for (entity, human, mut vitals, mut pos, mut dash, mut input, withdrawn, replicated) in
        &mut seats
    {
        if !human.connected() {
            if withdrawn.is_some() {
                continue;
            }
            input.0 = PlayerInput::default();
            let seat = Withdrawn {
                body: *vitals,
                pos: pos.0,
                dash: *dash,
            };
            vitals.alive = false;
            vitals.respawn_in = 0;
            // Replication stays on: clients hide a dead body after its fall, and the seat must
            // stay on everyone's scoreboard and results table, where it still holds a place.
            commands.entity(entity).insert(seat);
            continue;
        }
        let Some(seat) = withdrawn else {
            continue;
        };
        let mut body = commands.entity(entity);
        body.remove::<Withdrawn>();
        if replicated.is_some() {
            body.insert((
                Replicate::to_clients(NetworkTarget::All),
                InterpolationTarget::to_clients(NetworkTarget::AllExceptSingle(human.peer)),
                PredictionTarget::to_clients(NetworkTarget::Single(human.peer)),
            ));
        }
        if phase == Phase::Playing {
            // `respawn` picks a spawn point and hands out the 2.5s spawn shield.
            vitals.alive = false;
            vitals.respawn_in = 0;
            vitals.deploy_in = 0;
        } else {
            // Any other phase: back where they stood, exactly as they left it.
            *vitals = seat.body;
            pos.0 = seat.pos;
            *dash = seat.dash;
        }
    }
}

/// Input, before the rest of the tick: every `Results -> Lobby` uses up one hold. A seat that
/// reaches 0 is gone, the same way an idle kick is: the roster fills it with a bot on this
/// lobby tick and the board drops its row. Runs in Input so a replay removes the body on the
/// tick the log says, before that tick's systems.
pub(crate) fn expire_seats(
    mut commands: Commands,
    state: Query<&MatchState>,
    mut seats: Query<(Entity, &mut Human, &Player)>,
    mut cleared: Option<ResMut<ClearedSeats>>,
    tick: Res<SimTick>,
    mut outbox: ResMut<Outbox>,
    mut trace: Option<ResMut<Trace>>,
    mut last: Local<Option<Phase>>,
) {
    let phase = state.iter().next().map(|s| s.phase).unwrap_or(Phase::Lobby);
    let previous = last.replace(phase);
    if previous != Some(Phase::Results) || phase != Phase::Lobby {
        return;
    }
    let mut gone = Vec::new();
    for (entity, mut human, player) in &mut seats {
        if human.connected() || human.left {
            continue;
        }
        human.holds = human.holds.saturating_sub(1);
        if human.holds > 0 {
            continue;
        }
        gone.push((entity, player.client_id, player.name.clone()));
    }
    gone.sort_by_key(|(_, id, _)| *id);
    for (entity, id, name) in gone {
        info!("{name} did not come back, the seat is gone");
        outbox.announce(format!("{name} 离开了废土"));
        if let Some(trace) = trace.as_mut() {
            trace.push(format!("leave {id}"));
        }
        if let Some(cleared) = cleared.as_mut() {
            cleared.remember(id, &name, tick.0);
        }
        // A replay may already have applied this seat's `leave` line this tick.
        commands.entity(entity).try_despawn();
    }
}

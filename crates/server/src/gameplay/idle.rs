//! Playing idle watch. A living human whose input does not change is warned, then removed.

use super::*;

/// Unchanged input for this long sends one warning to that human.
pub const IDLE_WARN_TICKS: u32 = 60 * TICKS_PER_SEC;
/// Unchanged input for this long removes them as a Playing departure.
pub const IDLE_KICK_TICKS: u32 = 90 * TICKS_PER_SEC;

const IDLE_WARN: &str = "60 秒内无操作，30 秒后将被移出房间";

/// Server-only. Dead time and a dropped link do not add ticks.
#[derive(Component, Default)]
pub struct Idle {
    last: PlayerInput,
    ticks: u32,
    warned: bool,
}

pub(super) fn watch_idle(
    mut commands: Commands,
    state: Query<&MatchState>,
    mut departed: Query<&mut Departed>,
    mut humans: Query<(
        Entity,
        &mut Human,
        &Player,
        &mut Idle,
        &CurrentInput,
        &Vitals,
        &Stats,
        &Evolution,
        Option<&ControlledBy>,
    )>,
    mut outbox: ResMut<Outbox>,
) {
    let playing = state.iter().any(|s| s.phase == Phase::Playing);
    if !playing {
        for (_, _, _, mut idle, _, _, _, _, _) in &mut humans {
            idle.ticks = 0;
            idle.warned = false;
        }
        return;
    }
    let mut ids: Vec<(u64, Entity)> = humans
        .iter()
        .map(|(entity, _, player, ..)| (player.client_id, entity))
        .collect();
    ids.sort_unstable_by_key(|(id, _)| *id);
    let mut leaving = Vec::new();
    for (_, entity) in ids {
        let Ok((entity, mut human, player, mut idle, input, vitals, stats, evo, control)) =
            humans.get_mut(entity)
        else {
            continue;
        };
        // A corpse is on the death screen. A dropped link is on the reconnect clock.
        if !vitals.alive || !human.connected() {
            continue;
        }
        if input.0 != idle.last {
            idle.last = input.0.clone();
            idle.ticks = 0;
            idle.warned = false;
            continue;
        }
        idle.ticks = idle.ticks.saturating_add(1);
        if idle.ticks == IDLE_WARN_TICKS && !idle.warned {
            idle.warned = true;
            outbox
                .direct
                .push((entity, GameEvent::Announce(IDLE_WARN.to_string())));
        }
        if idle.ticks < IDLE_KICK_TICKS {
            continue;
        }
        let link = control.map(|control| control.owner);
        leaving.push((
            entity,
            link,
            player.name.clone(),
            Departure::from_combatant(player, stats, evo),
        ));
        human.left = true;
    }
    if !leaving.is_empty()
        && let Ok(mut departed) = departed.single_mut()
    {
        for (_, _, _, snap) in &leaving {
            departed.0.push(snap.clone());
        }
    }
    for (entity, link, name, _) in leaving {
        outbox.announce(format!("{name} 因挂机离开了废土"));
        commands.entity(entity).despawn();
        if let Some(link) = link {
            commands.entity(link).despawn();
        }
    }
}

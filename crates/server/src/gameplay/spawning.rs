//! Spawning combatants, pickups and the match state.

use super::*;

/// Spawn a fresh combatant. `peer` is set for humans so they predict themselves.
pub fn spawn_combatant(commands: &mut Commands, player: Player, peer: Option<PeerId>) -> Entity {
    let entity = spawn_plain_combatant(commands, player);
    let interpolate = match peer {
        Some(peer) => NetworkTarget::AllExceptSingle(peer),
        None => NetworkTarget::All,
    };
    let mut entity = commands.entity(entity);
    entity.insert((
        Replicate::to_clients(NetworkTarget::All),
        InterpolationTarget::to_clients(interpolate),
    ));
    if let Some(peer) = peer {
        entity.insert(PredictionTarget::to_clients(NetworkTarget::Single(peer)));
    }
    entity.id()
}

/// Gameplay body only, no replication. Bot-fill tests use it so despawn does not need
/// the netcode visibility registry. Matches spawn through [`spawn_combatant`].
pub fn spawn_plain_combatant(commands: &mut Commands, player: Player) -> Entity {
    commands
        .spawn((
            Name::new(player.name.clone()),
            player,
            Pos(Vec2::ZERO),
            Aim(Vec2::X),
            Dash::default(),
            // Spawned dead with a short timer so `respawn` picks a safe spot.
            Vitals {
                alive: false,
                respawn_in: 1,
                ..default()
            },
            Loadout::default(),
            Stats::default(),
            (
                Evolution::default(),
                SkillState::default(),
                Wallet::default(),
            ),
            (
                CurrentInput::default(),
                Cooldowns::default(),
                PosHistory::default(),
                Rewind::default(),
                LifeLog::default(),
                Recovery::default(),
                HiddenFrom::default(),
                Selecting::default(),
            ),
        ))
        .id()
}

pub fn spawn_pickup(commands: &mut Commands, item: Item, pos: Vec2, ttl: Option<u32>) -> Entity {
    let mut entity = commands.spawn((
        Name::new("Pickup"),
        Pickup { item },
        Pos(pos),
        Replicate::to_clients(NetworkTarget::All),
    ));
    if let Some(ttl) = ttl {
        entity.insert(Expires(ttl));
    }
    entity.id()
}

pub(super) fn spawn_match_state(
    mut commands: Commands,
    arena: Res<Arena>,
    config: Res<ServerConfig>,
) {
    commands.spawn((
        Name::new("Match"),
        MatchState {
            phase: Phase::Lobby,
            ticks_left: 0,
            countdown_active: false,
            match_index: 0,
            vault_in: FIRST_VAULT_TICKS,
            winner: None,
            map_seed: arena.seed,
            boss_eta: 0,
            recover: RecoverWait::default(),
        },
        super::room::settings_from_config(&config),
        Departed::default(),
        LeaderPing::default(),
        Replicate::to_clients(NetworkTarget::All),
    ));
}

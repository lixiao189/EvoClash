//! Loot spawns, match phases, reset and flushing outgoing events.

use super::*;

pub(super) fn loot_spawner(
    mut commands: Commands,
    arena: Res<Arena>,
    mut loot: ResMut<LootState>,
    pickups: Query<(), With<Pickup>>,
) {
    if loot.spots.len() != arena.loot_spots.len() {
        loot.spots = arena
            .loot_spots
            .iter()
            .map(|_| SpotState::default())
            .collect();
    }
    let mut rng = rand::rng();
    for (state, spot) in loot.spots.iter_mut().zip(&arena.loot_spots) {
        if let Some(entity) = state.entity {
            if pickups.contains(entity) {
                continue;
            }
            // Taken: higher tiers take longer to come back.
            state.entity = None;
            state.respawn_in = (10 + 6 * spot.tier as u32) * TICKS_PER_SEC;
        }
        if state.respawn_in > 0 {
            state.respawn_in -= 1;
            continue;
        }
        let item = roll_loot(spot.tier, &mut rng);
        state.entity = Some(spawn_pickup(&mut commands, item, spot.pos, None));
    }
}

pub fn match_flow(
    mut commands: Commands,
    mut state: Single<&mut MatchState>,
    mut arena: ResMut<Arena>,
    players: Query<(&Player, &Stats)>,
    mut outbox: ResMut<Outbox>,
    mut reset: ResMut<ResetRequested>,
    config: Res<ServerConfig>,
    ops: Res<OpsState>,
    mut exit: MessageWriter<AppExit>,
) {
    state.ticks_left = state.ticks_left.saturating_sub(1);
    match state.phase {
        Phase::Warmup => {
            if players.iter().count() < 2 {
                state.ticks_left = WARMUP_TICKS;
            } else if state.ticks_left == 0 {
                state.phase = Phase::Playing;
                state.ticks_left = MATCH_TICKS;
                state.vault_in = FIRST_VAULT_TICKS;
                state.winner = None;
                reset.0 = true;
                info!("match started with {} combatants", players.iter().count());
                outbox.announce(format!("对局开始！先拿到 {SCORE_TO_WIN} 分者获胜"));
            }
        }
        Phase::Playing => {
            state.vault_in = state.vault_in.saturating_sub(1);
            if state.vault_in == 10 * TICKS_PER_SEC {
                outbox.announce("前文明地堡将在 10 秒后开启");
            }
            if state.vault_in == 0 {
                state.vault_in = VAULT_INTERVAL_TICKS;
                let mut rng = rand::rng();
                for offset in [Vec2::new(-45.0, 0.0), Vec2::new(45.0, 0.0)] {
                    let item = roll_loot(4, &mut rng);
                    spawn_pickup(
                        &mut commands,
                        item,
                        VAULT_CENTER + offset,
                        Some(DROP_TTL_TICKS * 2),
                    );
                }
                outbox.announce("地堡开启：尖端科技已出土！");
                outbox.fast.push(GameEvent::Boom {
                    pos: VAULT_CENTER,
                    radius: 220.0,
                    fire: false,
                });
            }
            let leader = players.iter().max_by_key(|(_, s)| s.score);
            let won = leader.is_some_and(|(_, s)| s.score >= SCORE_TO_WIN);
            if won || state.ticks_left == 0 {
                let winner = leader.map(|(p, _)| p.name.clone());
                outbox.announce(match &winner {
                    Some(name) => format!("{name} 赢得了本局！"),
                    None => "本局结束".to_string(),
                });
                info!("match over, winner {winner:?}");
                state.phase = Phase::Intermission;
                state.ticks_left = INTERMISSION_TICKS;
                state.winner = winner;
            }
        }
        Phase::Intermission => {
            if state.ticks_left == 0 {
                if config.single_match || ops.is_draining() {
                    info!("match finished, shutting down");
                    exit.write(AppExit::Success);
                    return;
                }
                state.phase = Phase::Warmup;
                state.ticks_left = WARMUP_TICKS;
                state.winner = None;
                // A new city for the next match; everyone respawns into it.
                state.map_seed = rand::random();
                arena.0 = ArenaMap::generate(state.map_seed);
                reset.0 = true;
                info!("new arena layout {:#x}", state.map_seed);
            }
        }
    }
}

/// Start every combatant over from a stick and clear the arena.
pub fn reset_match(
    mut commands: Commands,
    mut reset: ResMut<ResetRequested>,
    mut players: Query<
        (
            &mut Vitals,
            &mut Loadout,
            &mut Stats,
            &mut Evolution,
            &mut SkillState,
            &mut Evolving,
        ),
        With<Player>,
    >,
    clutter: Query<
        Entity,
        Or<(
            With<Pickup>,
            With<Projectile>,
            With<Monster>,
            With<Prop>,
            With<Puddle>,
            With<Turret>,
            With<Trap>,
        )>,
    >,
    mut loot: ResMut<LootState>,
    mut props: ResMut<PropSlots>,
    mut blasts: ResMut<Blasts>,
) {
    if !std::mem::take(&mut reset.0) {
        return;
    }
    for (mut vitals, mut loadout, mut stats, mut evo, mut skills, mut evolving) in &mut players {
        *vitals = Vitals {
            alive: false,
            respawn_in: 1,
            ..default()
        };
        *loadout = Loadout::default();
        *stats = Stats::default();
        // Evolution only lasts one match.
        *evo = Evolution::default();
        *skills = SkillState::default();
        evolving.last_pick = 0;
    }
    for entity in &clutter {
        commands.entity(entity).despawn();
    }
    loot.spots.clear();
    props.clear();
    blasts.clear();
}

pub(super) fn flush_outbox(
    mut outbox: ResMut<Outbox>,
    mut sender: ServerMultiMessageSender,
    server: Single<&Endpoint, With<Server>>,
    humans: Query<&Human>,
) {
    let server = server.into_inner();
    for (entity, event) in outbox.direct.drain(..) {
        if let Ok(human) = humans.get(entity)
            && human.connected()
        {
            let _ =
                sender.send::<_, FastChannel>(&event, server, &NetworkTarget::Single(human.peer));
        }
    }
    for event in outbox.reliable.drain(..) {
        if let Err(e) = sender.send::<_, ReliableChannel>(&event, server, &NetworkTarget::All) {
            warn!("failed to send event: {e:?}");
        }
    }
    for event in outbox.fast.drain(..) {
        let _ = sender.send::<_, FastChannel>(&event, server, &NetworkTarget::All);
    }
}

//! Loot spawns, match phases, reset and flushing outgoing events.

use evoclash_game::rng::mix;

use super::*;
use crate::rng::{SimRng, Stream};

pub(super) fn loot_spawner(
    mut commands: Commands,
    arena: Res<Arena>,
    mut loot: ResMut<LootState>,
    pickups: Query<(), With<Pickup>>,
    state: Query<&MatchState>,
    rng: Res<SimRng>,
) {
    let phase = state.iter().next().map(|s| s.phase).unwrap_or(Phase::Lobby);
    // Starting holds an empty arena until Playing re-inits the loot stream and the next tick rolls.
    if matches!(phase, Phase::Starting | Phase::Recovering) {
        return;
    }
    if loot.spots.len() != arena.loot_spots.len() {
        loot.spots = arena
            .loot_spots
            .iter()
            .map(|_| SpotState::default())
            .collect();
    }
    let playing = phase == Phase::Playing;
    rng.with(playing, Stream::Loot, |rng| {
        for (state, spot) in loot.spots.iter_mut().zip(&arena.loot_spots) {
            // Tier 3+ caches stay shut in the lobby. Safes are props, not spots.
            if phase == Phase::Lobby && spot.tier >= 3 {
                continue;
            }
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
            let item = roll_loot(spot.tier, rng);
            state.entity = Some(spawn_loot(&mut commands, item, spot.pos, None));
        }
    });
}

fn connected_humans(humans: &Query<&Human>) -> usize {
    humans.iter().filter(|human| human.connected()).count()
}

pub fn match_flow(
    mut commands: Commands,
    mut state: Single<&mut MatchState>,
    mut arena: ResMut<Arena>,
    players: Query<(&Player, &Stats)>,
    humans: Query<&Human>,
    mut vitals: Query<&mut Vitals, With<Player>>,
    mut outbox: ResMut<Outbox>,
    mut reset: ResMut<ResetRequested>,
    mut place: ResMut<PendingStart>,
    config: Res<ServerConfig>,
    ops: Res<OpsState>,
    rng: Res<SimRng>,
    mut exit: MessageWriter<AppExit>,
) {
    match state.phase {
        Phase::Lobby => {
            let humans_n = connected_humans(&humans);
            if humans_n == 0 {
                if state.countdown_active {
                    state.countdown_active = false;
                    state.ticks_left = 0;
                }
            } else if !state.countdown_active && players.iter().count() >= 2 {
                // First human (bots already fill the room) starts it. Later joins do not reset it.
                state.countdown_active = true;
                state.ticks_left = LOBBY_COUNTDOWN_TICKS;
            } else if state.countdown_active {
                state.ticks_left = state.ticks_left.saturating_sub(1);
                if state.ticks_left == 0 {
                    begin_starting(&mut state, &mut reset, &mut place);
                }
            }
        }
        Phase::Starting => {
            state.ticks_left = state.ticks_left.saturating_sub(1);
            if state.ticks_left == 0 {
                begin_playing(
                    &mut state,
                    &mut vitals,
                    &rng,
                    &mut outbox,
                    players.iter().count(),
                );
            }
        }
        Phase::Playing => {
            state.ticks_left = state.ticks_left.saturating_sub(1);
            state.vault_in = state.vault_in.saturating_sub(1);
            if state.vault_in == 10 * TICKS_PER_SEC {
                outbox.announce("前文明地堡将在 10 秒后开启");
            }
            if state.vault_in == 0 {
                state.vault_in = VAULT_INTERVAL_TICKS;
                rng.with(true, Stream::Loot, |rng| {
                    for offset in [Vec2::new(-45.0, 0.0), Vec2::new(45.0, 0.0)] {
                        let item = roll_loot(4, rng);
                        spawn_loot(
                            &mut commands,
                            item,
                            VAULT_CENTER + offset,
                            Some(DROP_TTL_TICKS * 2),
                        );
                    }
                });
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
                state.phase = Phase::Results;
                state.ticks_left = RESULTS_TICKS;
                state.countdown_active = false;
                state.winner = winner;
            }
        }
        Phase::Results => {
            state.ticks_left = state.ticks_left.saturating_sub(1);
            if state.ticks_left == 0 {
                if config.single_match || ops.is_draining() {
                    info!("match finished, shutting down");
                    ops.mark_exiting();
                    exit.write(AppExit::Success);
                    return;
                }
                begin_lobby(
                    &mut state, &mut arena, &config, &rng, &mut reset, &humans, &players,
                );
            }
        }
        Phase::Recovering => {}
    }
}

fn begin_starting(state: &mut MatchState, reset: &mut ResetRequested, place: &mut PendingStart) {
    state.phase = Phase::Starting;
    state.ticks_left = STARTING_TICKS;
    state.countdown_active = false;
    state.winner = None;
    reset.0 = true;
    place.0 = true;
    info!("starting countdown");
}

fn begin_playing(
    state: &mut MatchState,
    vitals: &mut Query<&mut Vitals, With<Player>>,
    rng: &SimRng,
    outbox: &mut Outbox,
    combatants: usize,
) {
    state.phase = Phase::Playing;
    state.ticks_left = MATCH_TICKS;
    state.vault_in = FIRST_VAULT_TICKS;
    state.winner = None;
    state.countdown_active = false;
    // Streams 1-6 restart from this round's seed. Lobby draws stay on stream 0.
    rng.reinit_match(state.map_seed);
    for mut vitals in vitals.iter_mut() {
        if vitals.alive {
            vitals.shield = SPAWN_SHIELD_TICKS;
        }
    }
    info!("match started with {combatants} combatants");
    outbox.announce(format!("对局开始！先拿到 {SCORE_TO_WIN} 分者获胜"));
}

fn begin_lobby(
    state: &mut MatchState,
    arena: &mut Arena,
    config: &ServerConfig,
    rng: &SimRng,
    reset: &mut ResetRequested,
    humans: &Query<&Human>,
    players: &Query<(&Player, &Stats)>,
) {
    state.match_index = state.match_index.saturating_add(1);
    state.map_seed = mix(config.room_seed, state.match_index);
    arena.0 = ArenaMap::generate(state.map_seed);
    rng.reset_all(state.map_seed);
    state.phase = Phase::Lobby;
    state.winner = None;
    state.boss_eta = 0;
    state.vault_in = FIRST_VAULT_TICKS;
    // Whoever is still here starts the next public countdown immediately.
    if connected_humans(humans) > 0 && players.iter().count() >= 2 {
        state.countdown_active = true;
        state.ticks_left = LOBBY_COUNTDOWN_TICKS;
    } else {
        state.countdown_active = false;
        state.ticks_left = 0;
    }
    reset.0 = true;
    info!("new arena layout {:#x}", state.map_seed);
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
            &mut Wallet,
            &mut LifeLog,
        ),
        With<Player>,
    >,
    mut aggressors: Query<&mut Aggressors>,
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
    for (mut vitals, mut loadout, mut stats, mut evo, mut skills, mut wallet, mut life) in
        &mut players
    {
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
        *wallet = Wallet::default();
        *life = LifeLog::default();
    }
    for mut aggressors in &mut aggressors {
        aggressors.0.clear();
    }
    for entity in &clutter {
        commands.entity(entity).despawn();
    }
    loot.spots.clear();
    props.clear();
    blasts.clear();
}

/// Teleport every combatant onto a spawn, alive, with no shield. Movement stays frozen while
/// the phase is Starting.
pub(super) fn place_starters(
    mut pending: ResMut<PendingStart>,
    arena: Res<Arena>,
    mut players: Query<(
        Entity,
        &Player,
        &mut Pos,
        &mut Vitals,
        &mut Dash,
        &mut PosHistory,
    )>,
) {
    if !std::mem::take(&mut pending.0) {
        return;
    }
    let mut roster: Vec<(Entity, u64)> = players
        .iter()
        .map(|(entity, player, ..)| (entity, player.client_id))
        .collect();
    roster.sort_by_key(|(_, id)| *id);
    let spots = &arena.spawn_points;
    for (i, (entity, _)) in roster.iter().enumerate() {
        let Ok((_, _, mut pos, mut vitals, mut dash, mut history)) = players.get_mut(*entity)
        else {
            continue;
        };
        if !spots.is_empty() {
            pos.0 = spots[i % spots.len()];
        }
        *vitals = Vitals::default();
        *dash = Dash::default();
        history.0.clear();
    }
}

/// Public room with nobody in it closes itself. Draining an empty lobby leaves at once.
pub(super) fn empty_close(
    state: Query<&MatchState>,
    humans: Query<&Human>,
    ops: Res<OpsState>,
    mut empty_for: Local<u32>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(state) = state.iter().next() else {
        return;
    };
    if connected_humans(&humans) > 0 {
        *empty_for = 0;
        return;
    }
    if ops.is_draining() && state.phase == Phase::Lobby {
        info!("draining empty lobby, shutting down");
        ops.mark_exiting();
        exit.write(AppExit::Success);
        return;
    }
    let limit = match state.phase {
        Phase::Lobby => EMPTY_LOBBY_TICKS,
        Phase::Playing | Phase::Results => EMPTY_MATCH_TICKS,
        _ => {
            *empty_for = 0;
            return;
        }
    };
    *empty_for += 1;
    if *empty_for >= limit {
        info!("no humans left in {:?}, shutting down", state.phase);
        ops.mark_exiting();
        exit.write(AppExit::Success);
    }
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

//! Loot spawns, match phases, reset and flushing outgoing events.

use std::cmp::Reverse;

use evoclash_game::rng::mix;

use super::*;
use crate::rng::{SimRng, Stream};

/// Final-standings key: score desc, kills desc, deaths asc, last score tick asc, client_id asc.
pub(crate) fn final_rank(
    score: u32,
    kills: u32,
    deaths: u32,
    last_score_tick: u32,
    client_id: u64,
) -> (u32, u32, Reverse<u32>, Reverse<u32>, Reverse<u64>) {
    (
        score,
        kills,
        Reverse(deaths),
        Reverse(last_score_tick),
        Reverse(client_id),
    )
}

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

type Presence<'w, 's> = Query<
    'w,
    's,
    (
        Option<&'static Human>,
        Option<&'static crate::spectate::SpectatorSession>,
    ),
    Or<(With<Human>, With<crate::spectate::SpectatorSession>)>,
>;

fn connected_humans(presence: &Presence) -> usize {
    presence
        .iter()
        .filter(|(human, _)| human.is_some_and(|human| human.connected() && !human.left))
        .count()
}

/// Human combatants still holding a seat, including reconnect grace. Spectators are not
/// `Human`. A full public lobby uses this to skip straight into Starting.
fn seated_humans(presence: &Presence) -> usize {
    presence
        .iter()
        .filter(|(human, _)| human.is_some_and(|human| !human.left))
        .count()
}

/// Watchers still in the room, including their reconnect grace.
fn held_watchers(presence: &Presence) -> usize {
    presence
        .iter()
        .filter(|(_, session)| session.is_some())
        .count()
}

fn connected_watchers(presence: &Presence) -> usize {
    presence
        .iter()
        .filter(|(_, session)| session.is_some_and(|session| session.disconnected_ticks == 0))
        .count()
}

pub fn match_flow(
    mut commands: Commands,
    mut state: Single<&mut MatchState>,
    settings: Single<&RoomSettings>,
    mut arena: ResMut<Arena>,
    players: Query<(&Player, &Stats)>,
    presence: Presence,
    mut vitals: Query<&mut Vitals, With<Player>>,
    mut outbox: ResMut<Outbox>,
    mut reset: ResMut<ResetRequested>,
    mut place: ResMut<PendingStart>,
    config: Res<ServerConfig>,
    ops: Option<Res<OpsState>>,
    rng: Res<SimRng>,
    mut tick: ResMut<SimTick>,
    mut histories: Query<&mut PosHistory, With<Player>>,
    mut exit: MessageWriter<AppExit>,
) {
    let private_room = settings.private_room;
    let auto_paused = settings.auto_paused;
    match state.phase {
        Phase::Lobby => {
            let humans_n = connected_humans(&presence);
            let watching = connected_watchers(&presence);
            // A room of only watchers is still occupied. They take seats on this lobby tick.
            if humans_n == 0 && watching == 0 {
                if state.countdown_active {
                    state.countdown_active = false;
                    state.ticks_left = 0;
                }
            } else if !private_room && seated_humans(&presence) >= usize::from(ROOM_COMBATANTS) {
                // Eight human combatants: skip whatever lobby time is left.
                begin_starting(&mut state, &mut reset, &mut place);
            } else if !private_room && !state.countdown_active && players.iter().count() >= 2 {
                // Public rooms start on the first human. Later joins do not reset it.
                // A private room waits for the host's Start. Every round is 12s + 3s.
                state.countdown_active = true;
                state.ticks_left = LOBBY_COUNTDOWN_TICKS;
            } else if private_room
                && state.match_index > 0
                && !auto_paused
                && !state.countdown_active
                && humans_n > 0
                && players.iter().count() >= 2
            {
                // Watchers promoted this tick (Input) should not sit in a private lobby forever.
                // The first private round still waits for the host.
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
            // Start was valid, then someone left: don't begin a match with one combatant.
            if players.iter().count() < 2 {
                info!("not enough combatants to start, back to the lobby");
                state.phase = Phase::Lobby;
                state.countdown_active = false;
                state.ticks_left = 0;
            } else {
                state.ticks_left = state.ticks_left.saturating_sub(1);
                if state.ticks_left == 0 {
                    begin_playing(
                        &mut state,
                        &mut vitals,
                        &rng,
                        &mut outbox,
                        players.iter().count(),
                        &mut tick,
                        &mut histories,
                    );
                }
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
                        let dropped = spawn_loot(
                            &mut commands,
                            item,
                            VAULT_CENTER + offset,
                            Some(DROP_TTL_TICKS * 2),
                        );
                        commands.entity(dropped).insert(BunkerDrop);
                    }
                });
                outbox.announce("地堡开启：尖端科技已出土！");
                outbox.fast.push(GameEvent::Boom {
                    pos: VAULT_CENTER,
                    radius: 220.0,
                    fire: false,
                });
            }
            // No point target: the match always runs the full 8 minutes, then the final
            // scoreboard decides (score, kills, fewer deaths, earlier final score, client_id).
            // The finale (last two minutes) is `finale::step_finale`, chained right after this.
            if state.ticks_left == 0 {
                let winner = players
                    .iter()
                    .max_by_key(|(player, stats)| {
                        final_rank(
                            stats.score,
                            stats.kills,
                            stats.deaths,
                            stats.last_score_tick,
                            player.client_id,
                        )
                    })
                    .map(|(player, _)| player.name.clone());
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
                if config.single_match || ops.as_ref().is_some_and(|ops| ops.is_draining()) {
                    info!("match finished, shutting down");
                    if let Some(ops) = ops.as_ref() {
                        ops.mark_exiting();
                    }
                    exit.write(AppExit::Success);
                    return;
                }
                begin_lobby(
                    &mut state,
                    &mut arena,
                    &config,
                    &rng,
                    &mut reset,
                    connected_humans(&presence),
                    &players,
                    private_room,
                    settings.auto_paused,
                );
            }
        }
        Phase::Recovering => {
            // Clocks stay frozen. `recovering::step_recovering`, chained right after this,
            // counts the 20s grace and the 3s resume.
        }
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
    tick: &mut SimTick,
    histories: &mut Query<&mut PosHistory, With<Player>>,
) {
    state.phase = Phase::Playing;
    state.ticks_left = MATCH_TICKS;
    state.vault_in = FIRST_VAULT_TICKS;
    state.winner = None;
    state.countdown_active = false;
    // Streams 1-6 restart from this round's seed. Lobby draws stay on stream 0.
    rng.reinit_match(state.map_seed);
    // Match ticks start at 0 so a replay that skips the lobby hits the same schedules
    // (radiation, puddles, monster stride) as the process that played the lobby.
    tick.0 = 0;
    for mut history in histories.iter_mut() {
        history.0.clear();
    }
    for mut vitals in vitals.iter_mut() {
        if vitals.alive {
            vitals.shield = SPAWN_SHIELD_TICKS;
        }
    }
    info!("match started with {combatants} combatants");
    outbox.announce(format!(
        "对局开始！{} 分钟后按积分排名决出胜者",
        MATCH_TICKS / (60 * TICKS_PER_SEC)
    ));
}

fn begin_lobby(
    state: &mut MatchState,
    arena: &mut Arena,
    config: &ServerConfig,
    rng: &SimRng,
    reset: &mut ResetRequested,
    humans_n: usize,
    players: &Query<(&Player, &Stats)>,
    private_room: bool,
    auto_paused: bool,
) {
    state.match_index = state.match_index.saturating_add(1);
    state.map_seed = mix(config.room_seed, state.match_index);
    arena.0 = ArenaMap::generate(state.map_seed);
    rng.reset_all(state.map_seed);
    state.phase = Phase::Lobby;
    state.winner = None;
    state.boss_eta = 0;
    state.vault_in = FIRST_VAULT_TICKS;
    // Public rooms, and a private room from the second round on, open the 15s
    // countdown (12s lobby + 3s Starting) for whoever is still here.
    // The first private round waits for the host. A paused private room does too.
    let private_auto = private_room && state.match_index > 0 && !auto_paused;
    if (private_auto || !private_room) && humans_n > 0 && players.iter().count() >= 2 {
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
    mut departed: Query<&mut Departed>,
    bunker: Query<Entity, With<BunkerBounty>>,
) {
    if !std::mem::take(&mut reset.0) {
        return;
    }
    for entity in &bunker {
        commands.entity(entity).remove::<BunkerBounty>();
    }
    for mut departed in &mut departed {
        departed.0.clear();
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
    mut players: Query<
        (
            Entity,
            &Player,
            &mut Pos,
            &mut Vitals,
            &mut Dash,
            &mut PosHistory,
        ),
        Without<Withdrawn>,
    >,
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

/// A room with nobody in it closes itself. Draining an empty lobby leaves at once.
/// Public lobbies wait 30s; private lobbies wait 120s. Playing and results stay 10s.
pub(super) fn empty_close(
    state: Query<&MatchState>,
    settings: Query<&RoomSettings>,
    presence: Presence,
    config: Res<ServerConfig>,
    ops: Option<Res<OpsState>>,
    catching: Option<Res<crate::journal::CatchingUp>>,
    mut empty_for: Local<u32>,
    mut exit: MessageWriter<AppExit>,
) {
    // Replay has no ops resource and must keep simulating with nobody connected.
    // Catch-up humans have no link yet; do not treat the replay as an empty room.
    let Some(ops) = ops else {
        return;
    };
    if catching.is_some() {
        return;
    };
    let Some(state) = state.iter().next() else {
        return;
    };
    // Watchers count as people in the room. A match with no human combatant still finishes.
    // A seat on the reconnect clock counts too: a refresh or a dropped link must find the room
    // still there. The countdown starts once the grace runs out and the seat is cleared.
    if seated_humans(&presence) > 0 || held_watchers(&presence) > 0 {
        *empty_for = 0;
        return;
    }
    if ops.is_draining() && state.phase == Phase::Lobby {
        info!("draining empty lobby, shutting down");
        ops.mark_exiting();
        exit.write(AppExit::Success);
        return;
    }
    // A StatefulSet pod is a long-lived room: its empty lobby is the warm pool the matchmaker
    // keeps on standby, so only a drain (above) ends it. A local fleet room exits and is
    // relaunched on demand.
    if config.statefulset {
        return;
    }
    let private_room = settings.iter().next().is_some_and(|s| s.private_room);
    let limit = match state.phase {
        Phase::Lobby => {
            if private_room {
                PRIVATE_EMPTY_LOBBY_TICKS
            } else {
                EMPTY_LOBBY_TICKS
            }
        }
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

pub fn flush_outbox(
    mut outbox: ResMut<Outbox>,
    mut sender: If<ServerMultiMessageSender>,
    server: Query<&Endpoint, With<Server>>,
    humans: Query<&Human>,
) {
    let Ok(server) = server.single() else {
        outbox.reliable.clear();
        outbox.fast.clear();
        outbox.direct.clear();
        return;
    };
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

/// Drops anything `flush_outbox` did not send (replay and tests have no endpoint).
pub(super) fn clear_outbox(mut outbox: ResMut<Outbox>) {
    outbox.reliable.clear();
    outbox.fast.clear();
    outbox.direct.clear();
}

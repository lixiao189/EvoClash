//! Spectators watch one combatant and wait for a combat seat.
//!
//! Seats are handed out only in the lobby, before bots fill what is left: stayers first,
//! then this queue in arrival order. A mid-match join stays here. Follow order is the same
//! final rank as the results screen. Stealth and traps use the followed body as the eyes.

use bevy::prelude::*;
use evoclash_game::protocol::{
    MAX_SPECTATORS, MatchState, Phase, Player, RoomCommand, Spectator, Stats, Trap,
};
use lightyear::connection::client::Connected;
use lightyear::prelude::*;

use crate::bots::PlainBots;
use crate::config::ServerConfig;
use crate::connections::{Human, RECONNECT_GRACE_TICKS, color_for};
use crate::gameplay::{
    Idle, Outbox, SimSet, SimTick, final_rank, read_human_inputs, read_room_commands,
    spawn_combatant, spawn_plain_combatant,
};
use crate::journal::{Injected, Trace};

/// Arrival counter. A restored watcher bumps it so a later join sorts after them.
#[derive(Resource, Default)]
pub struct SpectateSeq(pub u64);

/// Spectator room commands, filled by `read_room_commands` and drained each tick.
#[derive(Resource, Default)]
pub struct SpectateInbox {
    pub commands: Vec<(u64, RoomCommand)>,
}

/// `Promote` runs before `Bots`, so a promoted human already holds the seat bots would fill.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum SeatSet {
    Promote,
    Bots,
}

/// Server-only. The replicated [`Spectator`] sits on the same entity.
#[derive(Component, Debug)]
pub struct SpectatorSession {
    pub client_id: u64,
    pub name: String,
    pub peer: PeerId,
    pub link: Entity,
    pub order: u64,
    /// A dropped link starts the reconnect grace. Replay leaves this false and removes the
    /// session only when the log says so.
    pub watched: bool,
    pub disconnected_ticks: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admit {
    Combatant,
    Spectator,
    Reject,
}

/// Who a new connection becomes. `held_humans` includes reconnect grace. `spectators` is the
/// whole queue, grace included. Draining rejects everyone.
pub fn admit(
    phase: Phase,
    draining: bool,
    held_humans: usize,
    spectators: usize,
    max_players: usize,
) -> Admit {
    if draining {
        return Admit::Reject;
    }
    let spec_room = spectators < MAX_SPECTATORS as usize;
    match phase {
        Phase::Lobby => {
            // Someone already queued claims the next open seat. A new arrival takes a seat
            // only when one remains after that queue.
            if held_humans.saturating_add(spectators) < max_players {
                Admit::Combatant
            } else if spec_room {
                Admit::Spectator
            } else {
                Admit::Reject
            }
        }
        Phase::Playing | Phase::Results if spec_room => Admit::Spectator,
        _ => Admit::Reject,
    }
}

pub struct NewSpectator {
    pub client_id: u64,
    pub name: String,
    pub peer: PeerId,
    pub link: Entity,
    /// 0 allocates the next arrival number.
    pub order: u64,
    pub following: u64,
    pub queue_pos: u8,
    pub watched: bool,
    pub replicate: bool,
    pub log: bool,
}

pub fn spawn_spectator(
    commands: &mut Commands,
    seq: &mut SpectateSeq,
    trace: Option<&mut Trace>,
    spec: NewSpectator,
) -> Entity {
    let order = if spec.order == 0 {
        seq.0 = seq.0.saturating_add(1);
        seq.0
    } else {
        if spec.order > seq.0 {
            seq.0 = spec.order;
        }
        spec.order
    };
    if spec.log
        && let Some(trace) = trace
    {
        let name = spec.name.replace(['\n', '\r'], " ");
        trace.push(format!(
            "spectate {} {order} {} {name}",
            spec.client_id, spec.following
        ));
    }
    let mut entity = commands.spawn((
        Spectator {
            following: spec.following,
            queue_pos: spec.queue_pos,
        },
        SpectatorSession {
            client_id: spec.client_id,
            name: spec.name,
            peer: spec.peer,
            link: spec.link,
            order,
            watched: spec.watched,
            disconnected_ticks: 0,
        },
    ));
    if spec.replicate {
        entity.insert(Replicate::to_clients(NetworkTarget::Single(spec.peer)));
    }
    entity.id()
}

/// Best first. Same key as the results screen, including combatants still on 0.
pub fn ranked_ids<'a>(players: impl Iterator<Item = (u64, &'a Stats)>) -> Vec<u64> {
    let mut rows: Vec<(u64, Stats)> = players.map(|(id, stats)| (id, *stats)).collect();
    rows.sort_by(|a, b| {
        let key = |row: &(u64, Stats)| {
            final_rank(
                row.1.score,
                row.1.kills,
                row.1.deaths,
                row.1.last_score_tick,
                row.0,
            )
        };
        key(b).cmp(&key(a))
    });
    rows.into_iter().map(|(id, _)| id).collect()
}

pub fn step_follow(ids: &[u64], current: u64, delta: isize) -> u64 {
    if ids.is_empty() {
        return 0;
    }
    let idx = ids.iter().position(|id| *id == current).unwrap_or(0);
    let len = ids.len() as isize;
    let next = (idx as isize + delta).rem_euclid(len) as usize;
    ids[next]
}

pub fn first_id<'a>(players: impl Iterator<Item = (u64, &'a Stats)>) -> u64 {
    ranked_ids(players).first().copied().unwrap_or(0)
}

pub fn is_spectator(sessions: &Query<&SpectatorSession>, client_id: u64) -> bool {
    sessions
        .iter()
        .any(|session| session.client_id == client_id)
}

pub fn spectator_rows(world: &mut World) -> Vec<(u64, u64, u8)> {
    let mut query = world.query::<(&SpectatorSession, &Spectator)>();
    let mut rows: Vec<(u64, u64, u8)> = query
        .iter(world)
        .map(|(session, spec)| (session.client_id, spec.following, spec.queue_pos))
        .collect();
    rows.sort_unstable();
    rows
}

pub struct SpectatePlugin;

impl Plugin for SpectatePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SpectateSeq>()
            .init_resource::<SpectateInbox>();
        app.configure_sets(FixedUpdate, SeatSet::Promote.before(SeatSet::Bots));
        app.add_systems(
            FixedUpdate,
            (
                apply_spectator_log
                    .in_set(SimSet::Input)
                    .before(read_human_inputs)
                    .before(read_room_commands),
                update_spectators
                    .in_set(SimSet::Input)
                    .in_set(SeatSet::Promote)
                    .after(read_room_commands),
                sync_trap_views.in_set(SimSet::Flow),
            ),
        );
    }
}

fn apply_spectator_log(
    mut commands: Commands,
    mut seq: ResMut<SpectateSeq>,
    mut injected: Option<ResMut<Injected>>,
    sessions: Query<(Entity, &SpectatorSession)>,
) {
    let Some(injected) = injected.as_mut() else {
        return;
    };
    let joins = std::mem::take(&mut injected.spectates);
    let leaves = std::mem::take(&mut injected.unspectates);
    let mut left = Vec::new();
    for id in leaves {
        for (entity, session) in &sessions {
            if session.client_id == id {
                commands.entity(entity).despawn();
                left.push(id);
            }
        }
    }
    for (client_id, order, following, name) in joins {
        if left.contains(&client_id) {
            continue;
        }
        if sessions
            .iter()
            .any(|(_, session)| session.client_id == client_id)
        {
            continue;
        }
        spawn_spectator(
            &mut commands,
            &mut seq,
            None,
            NewSpectator {
                client_id,
                name,
                peer: PeerId::Netcode(client_id),
                link: Entity::PLACEHOLDER,
                order,
                following,
                queue_pos: 0,
                watched: false,
                replicate: false,
                log: false,
            },
        );
    }
}

fn update_spectators(
    mut commands: Commands,
    config: Res<ServerConfig>,
    tick: Res<SimTick>,
    state: Query<&MatchState>,
    players: Query<(&Player, &Stats)>,
    humans: Query<&Human>,
    mut sessions: Query<(Entity, &mut SpectatorSession, &mut Spectator)>,
    live_links: Query<(), With<Connected>>,
    mut inbox: ResMut<SpectateInbox>,
    plain: Option<Res<PlainBots>>,
    mut outbox: ResMut<Outbox>,
    mut trace: Option<ResMut<Trace>>,
) {
    let lobby = state
        .iter()
        .any(|match_state| match_state.phase == Phase::Lobby);
    let ids = ranked_ids(
        players
            .iter()
            .map(|(player, stats)| (player.client_id, stats)),
    );
    let cmds = std::mem::take(&mut inbox.commands);
    let mut ordered: Vec<(u64, u64, Entity)> = sessions
        .iter()
        .map(|(entity, session, _)| (session.order, session.client_id, entity))
        .collect();
    ordered.sort_unstable();

    let mut drop_grace = Vec::new();
    for (_, _, entity) in &ordered {
        let Ok((_, session, _)) = sessions.get(*entity) else {
            continue;
        };
        if !session.watched {
            continue;
        }
        if live_links.contains(session.link) {
            if session.disconnected_ticks != 0
                && let Ok((_, mut session, _)) = sessions.get_mut(*entity)
            {
                session.disconnected_ticks = 0;
            }
            continue;
        }
        let Ok((_, mut session, _)) = sessions.get_mut(*entity) else {
            continue;
        };
        session.disconnected_ticks = session.disconnected_ticks.saturating_add(1);
        if session.disconnected_ticks > RECONNECT_GRACE_TICKS {
            drop_grace.push(*entity);
        }
    }

    let mut drop_leave = Vec::new();
    for (client_id, cmd) in &cmds {
        let Some(entity) = sessions
            .iter()
            .find_map(|(entity, session, _)| (session.client_id == *client_id).then_some(entity))
        else {
            continue;
        };
        match cmd {
            RoomCommand::Leave => drop_leave.push(entity),
            RoomCommand::SpectateNext => {
                if let Ok((_, _, mut spec)) = sessions.get_mut(entity) {
                    spec.following = step_follow(&ids, spec.following, 1);
                }
            }
            RoomCommand::SpectatePrev => {
                if let Ok((_, _, mut spec)) = sessions.get_mut(entity) {
                    spec.following = step_follow(&ids, spec.following, -1);
                }
            }
            _ => {}
        }
    }

    let dropping = [drop_leave.clone(), drop_grace.clone()].concat();
    for (_, _, entity) in &ordered {
        if dropping.contains(entity) {
            continue;
        }
        let Ok((_, _, mut spec)) = sessions.get_mut(*entity) else {
            continue;
        };
        if spec.following != 0 && ids.contains(&spec.following) {
            continue;
        }
        let next = ids.first().copied().unwrap_or(0);
        if spec.following != next {
            spec.following = next;
        }
    }

    for entity in &drop_grace {
        if let Ok((_, session, _)) = sessions.get(*entity)
            && let Some(trace) = trace.as_mut()
        {
            trace.push(format!("unspectate {}", session.client_id));
        }
        commands.entity(*entity).despawn();
    }
    for entity in &drop_leave {
        let link = sessions
            .get(*entity)
            .ok()
            .map(|(_, session, _)| session.link);
        commands.entity(*entity).despawn();
        if let Some(link) = link
            && let Ok(mut link_entity) = commands.get_entity(link)
        {
            link_entity.despawn();
        }
    }

    let held = humans.iter().filter(|human| !human.left).count();
    let free = if lobby {
        config.max_players.saturating_sub(held)
    } else {
        0
    };
    let mut promoted = 0usize;
    let plain = plain.is_some();
    for (_, _, entity) in &ordered {
        if dropping.contains(entity) {
            continue;
        }
        if promoted >= free {
            break;
        }
        let Ok((_, session, _)) = sessions.get(*entity) else {
            continue;
        };
        let player = Player {
            client_id: session.client_id,
            name: session.name.clone(),
            color: color_for(session.client_id),
            bot: false,
        };
        let peer = session.peer;
        let link = session.link;
        let disconnected_ticks = session.disconnected_ticks;
        let watched = session.watched;
        let name = session.name.clone();
        let body = if plain {
            spawn_plain_combatant(&mut commands, player)
        } else {
            spawn_combatant(&mut commands, player, Some(peer))
        };
        let mut body = commands.entity(body);
        body.insert((
            Human {
                peer,
                disconnected_ticks,
                joined_at: tick.0,
                left: false,
            },
            Idle::default(),
        ));
        if watched && disconnected_ticks == 0 && link != Entity::PLACEHOLDER {
            body.insert(ControlledBy {
                owner: link,
                lifetime: Lifetime::Persistent,
            });
        }
        commands.entity(*entity).despawn();
        outbox.announce(format!("{name} 进入了废土"));
        promoted += 1;
    }

    let mut place = 0u8;
    for (_, _, entity) in &ordered {
        if dropping.contains(entity) {
            continue;
        }
        // Promoted entities are despawned at the sync point. Skip the same prefix.
        if (place as usize) < promoted {
            place = place.saturating_add(1);
            continue;
        }
        let Ok((_, _, mut spec)) = sessions.get_mut(*entity) else {
            continue;
        };
        let pos = place.saturating_sub(promoted as u8).saturating_add(1);
        if spec.queue_pos != pos {
            spec.queue_pos = pos;
        }
        place = place.saturating_add(1);
    }
}

/// Peer ids (as bits) a trap was last replicated to.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
struct TrapAudience(Vec<u64>);

fn sync_trap_views(
    mut commands: Commands,
    traps: Query<(Entity, &Trap, Option<&TrapAudience>)>,
    humans: Query<(&Human, &Player)>,
    specs: Query<(&SpectatorSession, &Spectator)>,
) {
    for (entity, trap, audience) in &traps {
        let mut bits: Vec<u64> = humans
            .iter()
            .filter_map(|(human, player)| {
                (player.client_id == trap.owner).then_some(human.peer.to_bits())
            })
            .collect();
        for (session, spec) in &specs {
            if spec.following == trap.owner {
                bits.push(session.peer.to_bits());
            }
        }
        bits.sort_unstable();
        bits.dedup();
        if audience.is_some_and(|audience| audience.0 == bits) {
            continue;
        }
        if bits.is_empty() {
            if audience.is_some() {
                commands
                    .entity(entity)
                    .remove::<(Replicate, TrapAudience)>();
            }
            continue;
        }
        let target: NetworkTarget = bits.iter().copied().map(PeerId::Netcode).collect();
        commands
            .entity(entity)
            .insert((Replicate::to_clients(target), TrapAudience(bits)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bots::{self, Bot};
    use crate::config::ServerConfig;
    use crate::rng::SimRng;
    use evoclash_game::protocol::ROOM_COMBATANTS;

    fn test_config() -> ServerConfig {
        ServerConfig {
            port: 0,
            public_addr: "127.0.0.1:0".parse().unwrap(),
            match_id: "test".into(),
            max_players: ROOM_COMBATANTS as usize,
            min_combatants: ROOM_COMBATANTS as usize,
            matchmakers: Vec::new(),
            single_match: false,
            monsters: false,
            threads: 1,
            agones: false,
            room_seed: 1,
            private: false,
            room_code: String::new(),
            log_dir: std::env::temp_dir(),
            replay: None,
            logging: false,
        }
    }

    #[test]
    fn admit_spectator_rules() {
        let max = ROOM_COMBATANTS as usize;
        assert_eq!(
            admit(Phase::Playing, false, 3, 0, max),
            Admit::Spectator,
            "a mid-match join watches"
        );
        assert_eq!(admit(Phase::Results, false, 8, 3, max), Admit::Spectator);
        assert_eq!(
            admit(Phase::Playing, false, 0, MAX_SPECTATORS as usize, max),
            Admit::Reject,
            "four watchers is the cap"
        );
        assert_eq!(
            admit(Phase::Lobby, false, 7, 0, max),
            Admit::Combatant,
            "an open lobby seat still plays"
        );
        assert_eq!(
            admit(Phase::Lobby, false, 8, 0, max),
            Admit::Spectator,
            "a full lobby queues"
        );
        assert_eq!(
            admit(Phase::Lobby, false, 7, 1, max),
            Admit::Spectator,
            "the queue is ahead of a new arrival"
        );
        assert_eq!(
            admit(Phase::Lobby, false, 8, MAX_SPECTATORS as usize, max),
            Admit::Reject
        );
        assert_eq!(admit(Phase::Starting, false, 0, 0, max), Admit::Reject);
        assert_eq!(admit(Phase::Lobby, true, 0, 0, max), Admit::Reject);
    }

    #[test]
    fn follow_starts_at_first_and_steps_in_rank_order() {
        let stats = |score, kills, deaths, tick| Stats {
            score,
            kills,
            deaths,
            last_score_tick: tick,
            ..default()
        };
        let rows = [
            (1, stats(1, 0, 0, 5)),
            (2, stats(9, 1, 0, 4)),
            (3, stats(9, 1, 2, 4)),
        ];
        let ids = ranked_ids(rows.iter().map(|(id, stats)| (*id, stats)));
        assert_eq!(ids, vec![2, 3, 1]);
        assert_eq!(step_follow(&ids, 2, 1), 3);
        assert_eq!(step_follow(&ids, 2, -1), 1);
        assert_eq!(step_follow(&ids, 1, 1), 2);
        assert_eq!(step_follow(&ids, 99, 1), ids[1]);
    }

    fn seat_app() -> App {
        let mut app = App::new();
        app.insert_resource(PlainBots);
        app.insert_resource(test_config());
        app.insert_resource(SimRng::from_match_seed(1));
        app.init_resource::<SimTick>();
        app.init_resource::<Outbox>();
        app.init_resource::<SpectateSeq>();
        app.init_resource::<SpectateInbox>();
        app.world_mut().spawn((
            MatchState {
                phase: Phase::Lobby,
                ..default()
            },
            evoclash_game::protocol::RoomSettings {
                bots: Some(ROOM_COMBATANTS),
                ..default()
            },
        ));
        app.add_systems(Update, (update_spectators, bots::balance_bots).chain());
        app
    }

    fn add_human(app: &mut App, client_id: u64) {
        app.world_mut().spawn((
            Player {
                client_id,
                name: format!("h{client_id}"),
                color: [1.0; 3],
                bot: false,
            },
            Human {
                peer: PeerId::Netcode(client_id),
                disconnected_ticks: 0,
                joined_at: 0,
                left: false,
            },
            Stats::default(),
        ));
    }

    fn add_watcher(app: &mut App, client_id: u64, order: u64) {
        {
            let mut seq = app.world_mut().resource_mut::<SpectateSeq>();
            if order > seq.0 {
                seq.0 = order;
            }
        }
        app.world_mut().spawn((
            Spectator {
                following: 0,
                queue_pos: 0,
            },
            SpectatorSession {
                client_id,
                name: format!("w{client_id}"),
                peer: PeerId::Netcode(client_id),
                link: Entity::PLACEHOLDER,
                order,
                watched: false,
                disconnected_ticks: 0,
            },
        ));
    }

    fn counts(app: &mut App) -> (usize, usize, Vec<u64>) {
        let world = app.world_mut();
        let humans = world
            .query::<&Human>()
            .iter(world)
            .filter(|human| !human.left)
            .count();
        let bots = world.query::<&Bot>().iter(world).count();
        let mut watchers: Vec<u64> = world
            .query::<&SpectatorSession>()
            .iter(world)
            .map(|session| session.client_id)
            .collect();
        watchers.sort_unstable();
        (humans, bots, watchers)
    }

    #[test]
    fn lobby_seats_stayers_then_watchers_then_bots() {
        let mut app = seat_app();
        add_human(&mut app, 1);
        add_human(&mut app, 2);
        add_watcher(&mut app, 10, 1);
        add_watcher(&mut app, 11, 2);
        app.update();
        let (humans, bots, watchers) = counts(&mut app);
        assert_eq!(humans, 4, "two stayers plus two watchers");
        assert_eq!(bots, ROOM_COMBATANTS as usize - 4);
        assert!(watchers.is_empty());

        let mut full = seat_app();
        for id in 1..=8 {
            add_human(&mut full, id);
        }
        add_watcher(&mut full, 10, 1);
        add_watcher(&mut full, 11, 2);
        full.update();
        let (humans, bots, watchers) = counts(&mut full);
        assert_eq!(humans, 8);
        assert_eq!(bots, 0);
        assert_eq!(watchers, vec![10, 11], "a full room keeps the queue");
        let world = full.world_mut();
        let mut queued = world.query::<(&SpectatorSession, &Spectator)>();
        let mut places: Vec<(u64, u8)> = queued
            .iter(world)
            .map(|(session, spec)| (session.order, spec.queue_pos))
            .collect();
        places.sort_unstable();
        assert_eq!(places, vec![(1, 1), (2, 2)]);

        let gone = world
            .query::<(Entity, &Player)>()
            .iter(world)
            .find_map(|(entity, player)| (player.client_id == 8).then_some(entity))
            .unwrap();
        full.world_mut().despawn(gone);
        full.update();
        let (humans, bots, watchers) = counts(&mut full);
        assert_eq!(humans, 8, "the earliest watcher takes the open seat");
        assert_eq!(bots, 0);
        assert_eq!(watchers, vec![11]);
    }

    #[test]
    fn follow_tracks_first_place_and_a_departure() {
        let mut app = App::new();
        app.insert_resource(test_config());
        app.init_resource::<SimTick>();
        app.init_resource::<Outbox>();
        app.init_resource::<SpectateSeq>();
        app.init_resource::<SpectateInbox>();
        app.world_mut().spawn(MatchState {
            phase: Phase::Playing,
            ..default()
        });
        for (id, score) in [(1u64, 1u32), (2, 9), (3, 4)] {
            app.world_mut().spawn((
                Player {
                    client_id: id,
                    name: format!("p{id}"),
                    color: [1.0; 3],
                    bot: false,
                },
                Stats { score, ..default() },
            ));
        }
        app.world_mut().spawn((
            Spectator::default(),
            SpectatorSession {
                client_id: 50,
                name: "eye".into(),
                peer: PeerId::Netcode(50),
                link: Entity::PLACEHOLDER,
                order: 1,
                watched: false,
                disconnected_ticks: 0,
            },
        ));
        app.add_systems(Update, update_spectators);
        app.update();
        let following = |app: &mut App| {
            app.world_mut()
                .query::<&Spectator>()
                .iter(app.world())
                .next()
                .unwrap()
                .following
        };
        assert_eq!(following(&mut app), 2, "default follow is first place");
        app.world_mut()
            .resource_mut::<SpectateInbox>()
            .commands
            .push((50, RoomCommand::SpectateNext));
        app.update();
        assert_eq!(following(&mut app), 3);
        app.world_mut()
            .resource_mut::<SpectateInbox>()
            .commands
            .push((50, RoomCommand::SpectatePrev));
        app.update();
        assert_eq!(following(&mut app), 2);
        let target = app
            .world_mut()
            .query::<(Entity, &Player)>()
            .iter(app.world())
            .find_map(|(entity, player)| (player.client_id == 2).then_some(entity))
            .unwrap();
        app.world_mut().despawn(target);
        app.update();
        assert_eq!(following(&mut app), 3, "a lost target snaps back to first");
    }

    #[test]
    fn watcher_inputs_are_ignored() {
        use crate::gameplay::{CurrentInput, read_human_inputs, spawn_plain_combatant};
        use crate::journal::Injected;
        use evoclash_game::evolution::{Evolution, Perk};
        use evoclash_game::items::{InventoryMove, WeaponKind};
        use evoclash_game::protocol::PlayerInput;
        use evoclash_game::shop::PlayerCommand;

        let mut app = App::new();
        app.insert_resource(test_config());
        app.insert_resource(Injected {
            inputs: vec![
                (
                    99,
                    PlayerInput {
                        move_x: 1,
                        fire: true,
                        ..default()
                    },
                ),
                (
                    1,
                    PlayerInput {
                        move_x: -1,
                        ..default()
                    },
                ),
            ],
            commands: vec![(99, PlayerCommand::Spend(Perk::Claws))],
            moves: vec![(99, InventoryMove { from: 0, to: 1 })],
            ..default()
        });
        app.world_mut().spawn(MatchState {
            phase: Phase::Playing,
            ..default()
        });
        app.world_mut().spawn((
            Spectator::default(),
            SpectatorSession {
                client_id: 99,
                name: "eye".into(),
                peer: PeerId::Netcode(99),
                link: Entity::PLACEHOLDER,
                order: 1,
                watched: false,
                disconnected_ticks: 0,
            },
        ));
        let human = {
            let mut commands = app.world_mut().commands();
            let entity = spawn_plain_combatant(
                &mut commands,
                Player {
                    client_id: 1,
                    name: "p".into(),
                    color: [1.0; 3],
                    bot: false,
                },
            );
            commands.entity(entity).insert(Human {
                peer: PeerId::Netcode(1),
                disconnected_ticks: 0,
                joined_at: 0,
                left: false,
            });
            entity
        };
        app.world_mut().flush();
        {
            let mut evo = app.world_mut().get_mut::<Evolution>(human).unwrap();
            evo.points = 2;
            app.world_mut()
                .get_mut::<evoclash_game::protocol::Loadout>(human)
                .unwrap()
                .guns[0] = Some(WeaponKind::Rifle);
        }
        app.add_systems(
            Update,
            (
                read_human_inputs,
                crate::evolution::read_commands,
                crate::inventory::apply_moves,
            ),
        );
        app.update();
        let world = app.world();
        assert_eq!(world.get::<CurrentInput>(human).unwrap().0.move_x, -1);
        assert!(!world.get::<CurrentInput>(human).unwrap().0.fire);
        assert_eq!(world.get::<Evolution>(human).unwrap().points, 2);
        assert_eq!(
            world
                .get::<evoclash_game::protocol::Loadout>(human)
                .unwrap()
                .guns[0],
            Some(WeaponKind::Rifle)
        );
    }
}

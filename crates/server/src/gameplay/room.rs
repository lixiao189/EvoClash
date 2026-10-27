//! Private-room host commands, host handoff and the 30 minute idle cap.

use super::*;
use crate::journal::{Injected, Trace, fmt_room};

/// Someone still in the room, for picking the next host.
#[derive(Clone, Debug)]
pub(super) struct PresentHuman {
    pub client_id: u64,
    pub name: String,
    pub joined_at: u32,
    pub connected: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RoomEffect {
    Ignore,
    Updated,
    Started,
    Cancelled,
    Leave,
}

pub(super) fn settings_from_config(config: &ServerConfig) -> RoomSettings {
    RoomSettings {
        bots: Some(ROOM_COMBATANTS),
        monsters: config.monsters,
        host: 0,
        private_room: config.private,
        code: if config.private {
            config.room_code.clone()
        } else {
            String::new()
        },
        auto_paused: false,
    }
}

fn host_command(sender: u64, settings: &RoomSettings) -> bool {
    settings.private_room && settings.host != 0 && sender == settings.host
}

/// Public-style time left until Playing: lobby ticks plus the 3s Starting phase.
fn until_playing(state: &MatchState) -> u32 {
    state.ticks_left.saturating_add(STARTING_TICKS)
}

/// Apply one command. Invalid commands are ignored; nobody is kicked.
pub fn apply_room_command(
    cmd: &RoomCommand,
    sender: u64,
    state: &mut MatchState,
    settings: &mut RoomSettings,
) -> RoomEffect {
    if matches!(cmd, RoomCommand::Leave) {
        return RoomEffect::Leave;
    }
    if !host_command(sender, settings) {
        return RoomEffect::Ignore;
    }
    match cmd {
        RoomCommand::Leave => RoomEffect::Leave,
        RoomCommand::SetSettings { monsters, .. } => {
            // Lobby only, and not once the host has started the countdown.
            if state.phase != Phase::Lobby || state.countdown_active {
                return RoomEffect::Ignore;
            }
            // Every room fields ROOM_COMBATANTS: the host chooses monsters but not the fill.
            settings.bots = Some(ROOM_COMBATANTS);
            settings.monsters = *monsters;
            RoomEffect::Updated
        }
        RoomCommand::Start => {
            if state.phase != Phase::Lobby || state.countdown_active {
                return RoomEffect::Ignore;
            }
            state.countdown_active = true;
            state.ticks_left = PRIVATE_LOBBY_COUNTDOWN_TICKS;
            RoomEffect::Started
        }
        RoomCommand::Cancel => {
            // Only the 2s lobby window. Starting (the last 3s) ignores Cancel.
            if state.phase != Phase::Lobby
                || !state.countdown_active
                || until_playing(state) <= STARTING_TICKS
            {
                return RoomEffect::Ignore;
            }
            state.countdown_active = false;
            state.ticks_left = 0;
            RoomEffect::Cancelled
        }
        RoomCommand::PauseAutoStart => {
            // Results, or a lobby that has not started counting. A running countdown
            // stays up; the host cancels it instead.
            let open = state.phase == Phase::Results
                || (state.phase == Phase::Lobby && !state.countdown_active);
            if !open {
                return RoomEffect::Ignore;
            }
            settings.auto_paused = true;
            RoomEffect::Updated
        }
        RoomCommand::ResumeAutoStart => {
            let open = state.phase == Phase::Results
                || (state.phase == Phase::Lobby && !state.countdown_active);
            if !open {
                return RoomEffect::Ignore;
            }
            settings.auto_paused = false;
            // Does not start the clock. A held lobby stays put until the host sends Start.
            // Cleared during Results, the next begin_lobby auto-starts on its own.
            RoomEffect::Updated
        }
    }
}

/// Connected human who has been here the longest. `leaving` is the host walking out.
pub(super) fn next_host(humans: &[PresentHuman], leaving: Option<u64>) -> Option<(u64, String)> {
    let mut best: Option<(u32, u64, String)> = None;
    for human in humans {
        if !human.connected || Some(human.client_id) == leaving {
            continue;
        }
        let replace = match &best {
            None => true,
            Some((joined, id, _)) => {
                human.joined_at < *joined || (human.joined_at == *joined && human.client_id < *id)
            }
        };
        if replace {
            best = Some((human.joined_at, human.client_id, human.name.clone()));
        }
    }
    best.map(|(_, id, name)| (id, name))
}

pub(super) fn read_room_commands(
    mut commands: Commands,
    mut links: Query<(Entity, &mut MessageReceiver<RoomCommand>), With<ClientOf>>,
    mut humans: Query<(
        Entity,
        &mut Human,
        &Player,
        Option<&ControlledBy>,
        &Stats,
        &Evolution,
    )>,
    mut states: Query<&mut MatchState>,
    mut settings: Query<&mut RoomSettings>,
    mut departed: Query<&mut Departed>,
    mut outbox: ResMut<Outbox>,
    mut injected: Option<ResMut<Injected>>,
    mut trace: Option<ResMut<Trace>>,
) {
    let Ok(mut state) = states.single_mut() else {
        return;
    };
    let Ok(mut settings) = settings.single_mut() else {
        return;
    };
    let mut inbox = Vec::new();
    for (link, mut receiver) in &mut links {
        let cmds: Vec<RoomCommand> = receiver.receive().collect();
        if !cmds.is_empty() {
            inbox.push((link, cmds));
        }
    }
    let scripted = injected
        .as_mut()
        .map(|injected| std::mem::take(&mut injected.rooms))
        .unwrap_or_default();
    let mut leaving = Vec::new();
    for (client_id, cmd) in scripted {
        if let Some(trace) = trace.as_mut() {
            trace.push(format!("room {client_id} {}", fmt_room(&cmd)));
        }
        let Some((entity, name, link, snap)) =
            humans
                .iter()
                .find_map(|(entity, _, player, control, stats, evo)| {
                    (player.client_id == client_id).then(|| {
                        (
                            entity,
                            player.name.clone(),
                            control.map(|control| control.owner),
                            Departure::from_combatant(player, stats, evo),
                        )
                    })
                })
        else {
            continue;
        };
        if apply_room_command(&cmd, client_id, &mut state, &mut settings) != RoomEffect::Leave {
            continue;
        }
        if state.phase == Phase::Playing {
            leaving.push(snap);
        }
        if let Some((_, mut human, ..)) = humans.iter_mut().find(|(e, ..)| *e == entity) {
            human.left = true;
        }
        outbox.announce(format!("{name} 离开了废土"));
        commands.entity(entity).despawn();
        if let Some(link) = link {
            commands.entity(link).despawn();
        }
    }
    for (link, cmds) in inbox {
        let Some((entity, client_id, name, snap)) =
            humans
                .iter()
                .find_map(|(entity, _, player, control, stats, evo)| {
                    (control.is_some_and(|c| c.owner == link)).then(|| {
                        (
                            entity,
                            player.client_id,
                            player.name.clone(),
                            Departure::from_combatant(player, stats, evo),
                        )
                    })
                })
        else {
            continue;
        };
        for cmd in &cmds {
            if let Some(trace) = trace.as_mut() {
                trace.push(format!("room {client_id} {}", fmt_room(cmd)));
            }
            if apply_room_command(cmd, client_id, &mut state, &mut settings) != RoomEffect::Leave {
                continue;
            }
            if state.phase == Phase::Playing {
                leaving.push(snap);
            }
            if let Some((_, mut human, ..)) = humans.iter_mut().find(|(e, ..)| *e == entity) {
                human.left = true;
            }
            outbox.announce(format!("{name} 离开了废土"));
            commands.entity(entity).despawn();
            commands.entity(link).despawn();
            break;
        }
    }
    if state.phase == Phase::Playing
        && !leaving.is_empty()
        && let Ok(mut departed) = departed.single_mut()
    {
        departed.0.extend(leaving);
    }
}

/// First connected human becomes host. When the host entity is gone (Leave or grace),
/// the connected human with the earliest join tick takes over.
pub(super) fn maintain_host(
    mut settings: Query<&mut RoomSettings>,
    humans: Query<(&Human, &Player)>,
    mut outbox: ResMut<Outbox>,
) {
    let Ok(mut settings) = settings.single_mut() else {
        return;
    };
    if !settings.private_room {
        return;
    }
    let host = settings.host;
    if host != 0
        && humans
            .iter()
            .any(|(human, player)| player.client_id == host && !human.left)
    {
        return;
    }
    let present: Vec<PresentHuman> = humans
        .iter()
        .map(|(human, player)| PresentHuman {
            client_id: player.client_id,
            name: player.name.clone(),
            joined_at: human.joined_at,
            connected: human.connected() && !human.left,
        })
        .collect();
    match next_host(&present, None) {
        Some((id, name)) => {
            if host != 0 {
                outbox.announce(format!("{name} 成为房主"));
            }
            settings.host = id;
        }
        None => settings.host = 0,
    }
}

/// A private lobby that goes 28 minutes without Start warns, then exits at 30.
/// Start (countdown running) or any other phase — including a finished match — resets it.
pub(super) fn private_idle_cap(
    state: Query<&MatchState>,
    settings: Query<&RoomSettings>,
    mut idle_for: Local<u32>,
    mut warned: Local<bool>,
    mut outbox: ResMut<Outbox>,
    ops: Option<Res<OpsState>>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(ops) = ops else {
        return;
    };
    let private_room = settings.iter().next().is_some_and(|s| s.private_room);
    let idling = private_room
        && state
            .iter()
            .next()
            .is_some_and(|s| s.phase == Phase::Lobby && !s.countdown_active);
    if !idling {
        *idle_for = 0;
        *warned = false;
        return;
    }
    *idle_for += 1;
    if !*warned && *idle_for >= PRIVATE_IDLE_WARN_TICKS {
        *warned = true;
        outbox.announce("房间将在 2 分钟后关闭");
    }
    if *idle_for >= PRIVATE_IDLE_CLOSE_TICKS {
        info!("private lobby idle for 30 minutes, shutting down");
        ops.mark_exiting();
        exit.write(AppExit::Success);
    }
}

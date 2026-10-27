//! Private-room host commands, host handoff and the 30 minute idle cap.

use super::*;

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
    let bots = match config.min_combatants {
        0 | 1 => None,
        n => Some(n.clamp(2, 8) as u8),
    };
    RoomSettings {
        bots,
        monsters: config.monsters,
        host: 0,
        private_room: config.private,
        code: if config.private {
            config.room_code.clone()
        } else {
            String::new()
        },
    }
}

fn bots_ok(bots: Option<u8>) -> bool {
    match bots {
        None => true,
        Some(n) => (2..=8).contains(&n),
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
pub(super) fn apply_room_command(
    cmd: &RoomCommand,
    sender: u64,
    state: &mut MatchState,
    settings: &mut RoomSettings,
    connected_humans: usize,
) -> RoomEffect {
    if matches!(cmd, RoomCommand::Leave) {
        return RoomEffect::Leave;
    }
    if !host_command(sender, settings) {
        return RoomEffect::Ignore;
    }
    match cmd {
        RoomCommand::Leave => RoomEffect::Leave,
        RoomCommand::SetSettings { bots, monsters } => {
            // Lobby only, and not once the host has started the countdown.
            if state.phase != Phase::Lobby || state.countdown_active || !bots_ok(*bots) {
                return RoomEffect::Ignore;
            }
            settings.bots = *bots;
            settings.monsters = *monsters;
            RoomEffect::Updated
        }
        RoomCommand::Start => {
            if state.phase != Phase::Lobby || state.countdown_active {
                return RoomEffect::Ignore;
            }
            if settings.bots.is_none() && connected_humans < 2 {
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
    mut humans: Query<(Entity, &mut Human, &Player, Option<&ControlledBy>)>,
    mut states: Query<&mut MatchState>,
    mut settings: Query<&mut RoomSettings>,
    mut outbox: ResMut<Outbox>,
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
    let connected = humans
        .iter()
        .filter(|(_, human, _, _)| human.connected() && !human.left)
        .count();
    for (link, cmds) in inbox {
        let Some((entity, client_id, name)) =
            humans.iter().find_map(|(entity, _, player, control)| {
                (control.is_some_and(|c| c.owner == link))
                    .then(|| (entity, player.client_id, player.name.clone()))
            })
        else {
            continue;
        };
        for cmd in &cmds {
            if apply_room_command(cmd, client_id, &mut state, &mut settings, connected)
                != RoomEffect::Leave
            {
                continue;
            }
            if let Some((_, mut human, _, _)) = humans.iter_mut().find(|(e, ..)| *e == entity) {
                human.left = true;
            }
            outbox.announce(format!("{name} 离开了废土"));
            commands.entity(entity).despawn();
            commands.entity(link).despawn();
            break;
        }
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
    ops: Res<OpsState>,
    mut exit: MessageWriter<AppExit>,
) {
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

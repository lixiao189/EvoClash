use super::*;

#[derive(Clone, Debug)]
pub(super) struct Header {
    pub(super) protocol: u64,
    pub(super) build: String,
    pub(super) room_seed: u64,
    pub(super) match_index: u32,
    pub(super) settings: RoomSettings,
    pub(super) roster: Vec<RosterEntry>,
    pub(super) spectators: Vec<SpectatorEntry>,
}

pub(super) enum Event {
    Input(u64, PlayerInput),
    Cmd(u64, PlayerCommand),
    Inv(u64, InventoryMove),
    Room(u64, RoomCommand),
    Connect(u64),
    Disconnect(u64),
    Leave(u64),
    Spectate(u64, u64, u64, String),
    Unspectate(u64),
}

pub(super) struct Parsed {
    pub(super) header: Header,
    pub(super) events: BTreeMap<u32, Vec<Event>>,
    pub(super) digests: BTreeMap<u32, u64>,
    pub(super) scores: Option<Vec<ScoreLine>>,
    pub(super) kills: Option<Vec<KillLine>>,
    pub(super) max_tick: u32,
}

pub(super) fn parse_log(text: &str) -> Result<Parsed, String> {
    let mut lines = text.lines().filter(|line| !line.is_empty() || true);
    let magic = lines.next().unwrap_or("");
    if magic != "EVOCLASH match v1" {
        return Err(format!("not a match log: {magic}"));
    }
    let mut protocol = None;
    let mut build = None;
    let mut room_seed = None;
    let mut match_index = None;
    let mut settings = None;
    let mut roster = Vec::new();
    let mut spectators = Vec::new();
    let mut events: BTreeMap<u32, Vec<Event>> = BTreeMap::new();
    let mut digests = BTreeMap::new();
    let mut scores = None;
    let mut kills = None;
    let mut max_tick = 0;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("protocol ") {
            protocol = rest.trim().parse().ok();
        } else if let Some(rest) = line.strip_prefix("build ") {
            build = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("room_seed ") {
            room_seed = rest.trim().parse().ok();
        } else if let Some(rest) = line.strip_prefix("match_index ") {
            match_index = rest.trim().parse().ok();
        } else if let Some(rest) = line.strip_prefix("settings ") {
            settings = parse_settings(rest);
        } else if let Some(rest) = line.strip_prefix("player ") {
            if let Some(entry) = parse_player(rest) {
                roster.push(entry);
            }
        } else if let Some(rest) = line.strip_prefix("spectator ") {
            if let Some(entry) = parse_spectator(rest) {
                spectators.push(entry);
            }
        } else if let Some(rest) = line.strip_prefix("t ") {
            let mut parts = rest.splitn(3, ' ');
            let tick: u32 = parts
                .next()
                .unwrap_or("0")
                .parse()
                .map_err(|_| format!("bad tick in {line}"))?;
            let kind = parts.next().unwrap_or("");
            let body = parts.next().unwrap_or("");
            max_tick = max_tick.max(tick);
            if kind == "digest" {
                let hash = u64::from_str_radix(body.trim(), 16).map_err(|_| "bad digest")?;
                digests.insert(tick, hash);
            } else if let Some(event) = parse_event(kind, body) {
                events.entry(tick).or_default().push(event);
            }
        } else if let Some(rest) = line.strip_prefix("scores ") {
            scores = Some(parse_scores(rest));
        } else if let Some(rest) = line.strip_prefix("kills ") {
            kills = Some(parse_kills(rest));
        }
    }
    let protocol = protocol.ok_or("missing protocol")?;
    let build = build.ok_or("missing build")?;
    let room_seed = room_seed.ok_or("missing room_seed")?;
    let match_index = match_index.ok_or("missing match_index")?;
    let settings = settings.ok_or("missing settings")?;
    roster.sort_by_key(|e| e.client_id);
    spectators.sort_by_key(|e| (e.order, e.client_id));
    Ok(Parsed {
        header: Header {
            protocol,
            build,
            room_seed,
            match_index,
            settings,
            roster,
            spectators,
        },
        events,
        digests,
        scores,
        kills,
        max_tick,
    })
}

fn parse_settings(rest: &str) -> Option<RoomSettings> {
    let mut parts = rest.split_whitespace();
    let bots = match parts.next()? {
        "off" => None,
        n => Some(n.parse().ok()?),
    };
    let monsters = parts.next()? == "1";
    let host = parts.next()?.parse().ok()?;
    let private_room = parts.next()? == "1";
    let code = parts.next().unwrap_or("-");
    let auto_paused = parts.next().is_some_and(|flag| flag == "1");
    Some(RoomSettings {
        bots,
        monsters,
        host,
        private_room,
        code: if code == "-" {
            String::new()
        } else {
            code.to_string()
        },
        auto_paused,
    })
}

fn parse_spectator(rest: &str) -> Option<SpectatorEntry> {
    let mut parts = rest.splitn(4, ' ');
    Some(SpectatorEntry {
        client_id: parts.next()?.parse().ok()?,
        order: parts.next()?.parse().ok()?,
        following: parts.next()?.parse().ok()?,
        name: parts.next().unwrap_or("").to_string(),
    })
}

fn parse_player(rest: &str) -> Option<RosterEntry> {
    let mut parts = rest.splitn(6, ' ');
    let client_id = parts.next()?.parse().ok()?;
    let bot = parts.next()? == "1";
    let r = f32::from_bits(parts.next()?.parse().ok()?);
    let g = f32::from_bits(parts.next()?.parse().ok()?);
    let b = f32::from_bits(parts.next()?.parse().ok()?);
    let name = parts.next().unwrap_or("").to_string();
    Some(RosterEntry {
        client_id,
        name,
        bot,
        color: [r, g, b],
    })
}

fn parse_event(kind: &str, body: &str) -> Option<Event> {
    match kind {
        "input" => parse_input(body).map(|(id, input)| Event::Input(id, input)),
        "cmd" => parse_cmd(body).map(|(id, cmd)| Event::Cmd(id, cmd)),
        "inv" => {
            let mut parts = body.split_whitespace();
            let id = parts.next()?.parse().ok()?;
            let from = parts.next()?.parse().ok()?;
            let to = parts.next()?.parse().ok()?;
            Some(Event::Inv(id, InventoryMove { from, to }))
        }
        "room" => parse_room(body).map(|(id, cmd)| Event::Room(id, cmd)),
        "connect" => Some(Event::Connect(body.trim().parse().ok()?)),
        "disconnect" => Some(Event::Disconnect(body.trim().parse().ok()?)),
        "leave" => Some(Event::Leave(body.trim().parse().ok()?)),
        "spectate" => {
            let mut parts = body.splitn(4, ' ');
            let id = parts.next()?.parse().ok()?;
            let order = parts.next()?.parse().ok()?;
            let following = parts.next()?.parse().ok()?;
            let name = parts.next().unwrap_or("").to_string();
            Some(Event::Spectate(id, order, following, name))
        }
        "unspectate" => Some(Event::Unspectate(body.trim().parse().ok()?)),
        _ => None,
    }
}

fn parse_input(body: &str) -> Option<(u64, PlayerInput)> {
    let mut p = body.split_whitespace();
    let id = p.next()?.parse().ok()?;
    let move_x = p.next()?.parse().ok()?;
    let move_y = p.next()?.parse().ok()?;
    let aim_x = f32::from_bits(p.next()?.parse().ok()?);
    let aim_y = f32::from_bits(p.next()?.parse().ok()?);
    let mut flag = || p.next().is_some_and(|s| s == "1");
    let fire = flag();
    let dash = flag();
    let interact = flag();
    let throw = flag();
    let skill_a = flag();
    let skill_b = flag();
    let select = p.next()?.parse().ok()?;
    Some((
        id,
        PlayerInput {
            move_x,
            move_y,
            aim: Vec2::new(aim_x, aim_y),
            fire,
            dash,
            interact,
            throw,
            skill_a,
            skill_b,
            select,
        },
    ))
}

fn by_name<T: Copy + std::fmt::Debug>(all: &[T], name: &str) -> Option<T> {
    all.iter().copied().find(|item| format!("{item:?}") == name)
}

fn parse_cmd(body: &str) -> Option<(u64, PlayerCommand)> {
    let mut parts = body.splitn(3, ' ');
    let id = parts.next()?.parse().ok()?;
    let verb = parts.next()?;
    let arg = parts.next().unwrap_or("");
    let cmd = match verb {
        "spend" => PlayerCommand::Spend(by_name(&Perk::ALL, arg)?),
        "refund" => PlayerCommand::Refund(by_name(&Perk::ALL, arg)?),
        "reset" => PlayerCommand::ResetPerks,
        "buy" => PlayerCommand::Buy(by_name(&ShopItem::STOCK, arg).or_else(|| {
            [ShopItem::Medkit, ShopItem::Ammo]
                .into_iter()
                .find(|item| format!("{item:?}") == arg)
        })?),
        "unbuy" => PlayerCommand::Unbuy(by_name(&ShopItem::STOCK, arg)?),
        "deploy" => PlayerCommand::Deploy,
        "deploy_last" => PlayerCommand::DeployLast,
        _ => return None,
    };
    Some((id, cmd))
}

fn parse_room(body: &str) -> Option<(u64, RoomCommand)> {
    let mut parts = body.split_whitespace();
    let id = parts.next()?.parse().ok()?;
    let verb = parts.next()?;
    let cmd = match verb {
        "start" => RoomCommand::Start,
        "cancel" => RoomCommand::Cancel,
        "leave" => RoomCommand::Leave,
        "pause_auto" => RoomCommand::PauseAutoStart,
        "resume_auto" => RoomCommand::ResumeAutoStart,
        "spectate_next" => RoomCommand::SpectateNext,
        "spectate_prev" => RoomCommand::SpectatePrev,
        "settings" => {
            let bots = match parts.next()? {
                "off" => None,
                n => Some(n.parse().ok()?),
            };
            let monsters = parts.next()? == "1";
            RoomCommand::SetSettings { bots, monsters }
        }
        _ => return None,
    };
    Some((id, cmd))
}

pub(super) fn parse_scores(rest: &str) -> Vec<ScoreLine> {
    rest.split_whitespace()
        .filter_map(|part| {
            let mut n = part.split(':');
            Some(ScoreLine {
                client_id: n.next()?.parse().ok()?,
                score: n.next()?.parse().ok()?,
                kills: n.next()?.parse().ok()?,
                deaths: n.next()?.parse().ok()?,
                xp: n.next()?.parse().ok()?,
                last_score_tick: n.next()?.parse().ok()?,
            })
        })
        .collect()
}

fn parse_kills(rest: &str) -> Vec<KillLine> {
    if rest.trim().is_empty() {
        return Vec::new();
    }
    rest.split(';')
        .filter_map(|part| {
            let n = split_esc(part);
            let tick = n.first()?.parse().ok()?;
            Some(KillLine {
                tick,
                killer: unesc(n.get(1)?),
                victim: unesc(n.get(2)?),
                how: unesc(n.get(3).map(|s| s.as_str()).unwrap_or("")),
            })
        })
        .collect()
}

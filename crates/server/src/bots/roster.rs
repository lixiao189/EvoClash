use super::*;

pub(crate) fn balance_bots(
    mut commands: Commands,
    config: Res<ServerConfig>,
    state: Query<&MatchState>,
    settings: Query<&RoomSettings>,
    humans: Query<&Human>,
    bots: Query<(Entity, &Player), With<Bot>>,
    plain: Option<Res<PlainBots>>,
    mut counter: Local<u64>,
    mut cooldown: Local<u32>,
    rng: Res<SimRng>,
) {
    if state.iter().any(|s| s.phase == Phase::Recovering) {
        return;
    }
    let lobby = state.iter().any(|s| s.phase == Phase::Lobby);
    // Outside the lobby, adjust at most once a second so joins/leaves don't thrash.
    // A lobby fills or sheds bots on the same tick a human arrives or leaves.
    if !lobby {
        if *cooldown > 0 {
            *cooldown -= 1;
            return;
        }
        *cooldown = 64;
    }
    let humans_n = humans.iter().filter(|human| !human.left).count();
    let mut bot_count = bots.iter().count();
    // Fill target comes from the replicated settings. Production rooms are always
    // `Some(ROOM_COMBATANTS)`: humans first, bots fill the rest, and a human join over the
    // target is impossible because the room caps humans at the same number. `None` spawns
    // nobody (tests and self-play only).
    let target = settings
        .iter()
        .next()
        .map(|settings| settings.bots.map(|n| n as usize).unwrap_or(0))
        .unwrap_or(config.min_combatants);
    let wanted = target
        .max(humans_n)
        .min(config.max_players)
        .saturating_sub(humans_n);
    let playing = state.iter().any(|s| s.phase == Phase::Playing);
    while bot_count < wanted {
        *counter += 1;
        let n = *counter;
        let name = format!("{}{}", NAMES[(n as usize - 1) % NAMES.len()], n);
        let player = Player {
            // High ids can't collide with matchmaker-issued client ids.
            client_id: u64::MAX - n,
            name,
            color: [0.7, 0.7, 0.7],
            bot: true,
        };
        let (aim_error, path_i) = rng.with(playing, Stream::Bots, |rng| {
            (
                rng.random_range(-0.12..0.12),
                rng.random_range(0..EvoPath::ALL.len()),
            )
        });
        let entity = if plain.is_some() {
            spawn_plain_combatant(&mut commands, player)
        } else {
            spawn_combatant(&mut commands, player, None)
        };
        commands.entity(entity).insert((
            Bot {
                goal: Vec2::ZERO,
                last_pos: Vec2::ZERO,
                stuck: 0,
                strafe: 1.0,
                think_in: 0,
                aim_error,
                target: None,
                locked_for: 0,
                reaction_thinks: REACTION_THINKS,
                skill: BotSkill::Default,
                path: EvoPath::ALL[path_i],
            },
            Aggressors::default(),
        ));
        bot_count += 1;
        if !lobby {
            break;
        }
    }
    if bot_count > wanted {
        // Despawn is deferred, so collect first. Newest bots (smaller ids) leave.
        let mut extras: Vec<(Entity, u64, String)> = bots
            .iter()
            .map(|(entity, player)| (entity, player.client_id, player.name.clone()))
            .collect();
        extras.sort_by_key(|(_, id, _)| *id);
        let remove_n = if lobby { bot_count - wanted } else { 1 };
        for (entity, _, name) in extras.into_iter().take(remove_n) {
            info!("bot {name} leaves to make room");
            commands.entity(entity).despawn();
        }
    }
}

/// Playing start: personalities come from the match bot stream, in client_id order, so lobby
/// wandering cannot leak into the round a replay rebuilds.
pub(super) fn seed_match_once(
    mut armed: Local<bool>,
    state: Query<&MatchState>,
    rng: Res<SimRng>,
    skills: Option<Res<SkillMix>>,
    mut bots: Query<(Entity, &Player, &mut Bot)>,
) {
    let playing = state.iter().any(|s| s.phase == Phase::Playing);
    if !playing {
        *armed = false;
        return;
    }
    if *armed {
        return;
    }
    *armed = true;
    let mut order: Vec<(u64, Entity)> = bots
        .iter()
        .map(|(entity, player, _)| (player.client_id, entity))
        .collect();
    order.sort_unstable_by_key(|(id, _)| *id);
    // Copied out so the RNG closure does not borrow the resource. Empty means online:
    // the aim roll is stored unchanged and the reaction threshold stays [`REACTION_THINKS`].
    let tiers = skills
        .map(|skills| skills.tiers.clone())
        .unwrap_or_default();
    rng.with(true, Stream::Bots, |rng| {
        for (i, (_, entity)) in order.iter().enumerate() {
            let Ok((_, _, mut bot)) = bots.get_mut(*entity) else {
                continue;
            };
            bot.goal = Vec2::ZERO;
            bot.last_pos = Vec2::ZERO;
            bot.stuck = 0;
            bot.strafe = 1.0;
            bot.think_in = 0;
            bot.target = None;
            bot.locked_for = 0;
            let rolled = rng.random_range(-0.12..0.12);
            let path_i = rng.random_range(0..EvoPath::ALL.len());
            if let Some(tier) = tiers.get(i).copied() {
                bot.aim_error = rolled * tier.aim_scale();
                bot.reaction_thinks = tier.reaction_thinks();
                bot.skill = tier;
            } else {
                bot.aim_error = rolled;
                bot.reaction_thinks = REACTION_THINKS;
                bot.skill = BotSkill::Default;
            }
            bot.path = EvoPath::ALL[path_i];
        }
    });
}

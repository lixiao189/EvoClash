//! Picking spots, populating blocks by biome, puddles and monster shots.

use super::*;

pub fn over_building(arena: &Arena, p: Vec2, radius: f32) -> bool {
    arena
        .buildings
        .iter()
        .any(|b| b.rect.inflate(radius).contains(p))
}

/// Whether a `kind` may appear at `p`: clear of walls, props, the vault and living players.
pub fn spot_clear(
    arena: &Arena,
    kind: MonsterKind,
    p: Vec2,
    obstacles: &[(Vec2, f32)],
    players: &[Vec2],
) -> bool {
    let radius = kind.radius();
    arena.resolve_with(p, radius + 12.0, obstacles) == p
        && p.distance(VAULT_CENTER) > SPAWN_CLEAR_OF_VAULT
        && arena.block_at(p).is_some_and(|b| b.biome != Biome::Vault)
        && players
            .iter()
            .all(|q| q.distance(p) > SPAWN_CLEAR_OF_PLAYERS)
        && (!kind.flies() || !over_building(arena, p, radius + 8.0))
}

/// A random clear spot in a block that suits `kind`.
pub fn pick_spot(
    arena: &Arena,
    kind: MonsterKind,
    obstacles: &[(Vec2, f32)],
    players: &[Vec2],
    rng: &mut impl Rng,
) -> Option<(Vec2, Block)> {
    let blocks: Vec<&Block> = arena
        .blocks
        .iter()
        .filter(|b| block_suits(kind, b))
        .collect();
    if blocks.is_empty() {
        return None;
    }
    let inset = MARGIN + kind.radius();
    for _ in 0..24 {
        let block = blocks[rng.random_range(0..blocks.len())];
        let r = block.rect.inflate(-inset);
        let p = Vec2::new(
            rng.random_range(r.min.x..r.max.x),
            rng.random_range(r.min.y..r.max.y),
        );
        if spot_clear(arena, kind, p, obstacles, players) {
            return Some((p, *block));
        }
    }
    None
}

/// Spawn one monster. `max_hp` overrides the kind's (elite) default.
pub fn spawn_monster(
    commands: &mut Commands,
    kind: MonsterKind,
    elite: bool,
    pos: Vec2,
    brain: Brain,
    max_hp: Option<f32>,
) -> Entity {
    let hp = max_hp.unwrap_or(kind.max_hp() * if elite { ELITE_HP_MULT } else { 1.0 });
    let facing = brain.facing;
    commands
        .spawn((
            Name::new(kind.name()),
            Monster { kind, elite },
            MonsterState {
                facing,
                phase: u8::from(kind == MonsterKind::Behemoth),
                ..default()
            },
            Pos(pos),
            Vitals { hp, ..default() },
            PosHistory::default(),
            brain,
            Replicate::to_clients(NetworkTarget::All),
            InterpolationTarget::to_clients(NetworkTarget::All),
        ))
        .id()
}

pub(super) fn populate(
    mut commands: Commands,
    config: Res<ServerConfig>,
    arena: Res<Arena>,
    obstacles: Res<Obstacles>,
    state: Single<&MatchState>,
    mut spawner: ResMut<Spawner>,
    monsters: Query<(&Monster, &Brain)>,
    players: Query<(&Pos, &Vitals), With<Player>>,
) {
    if !config.monsters || state.phase == Phase::Intermission {
        return;
    }
    let mut live = census(monsters.iter().map(|(m, b)| (m.kind, m.elite, b.pack)));
    let warmup = state.phase == Phase::Warmup;
    let mut candidate = None;
    for kind in MonsterKind::ALL {
        if kind == MonsterKind::Behemoth {
            continue;
        }
        let s = spec(kind);
        let timer = &mut spawner.timers[kind_index(kind)];
        if !live.wants(kind) {
            // At the cap: the respawn clock only starts once one dies.
            timer.filled = true;
            timer.wait = s.respawn;
            continue;
        }
        if warmup || !timer.filled {
            timer.wait = timer.wait.min(1);
        }
        timer.wait = timer.wait.saturating_sub(1);
        if timer.wait == 0 && candidate.is_none() {
            candidate = Some(kind);
        }
    }
    // At most one group per tick spreads the cost of a refill.
    let Some(kind) = candidate else {
        return;
    };
    let alive: Vec<Vec2> = players
        .iter()
        .filter(|(_, v)| v.alive)
        .map(|(p, _)| p.0)
        .collect();
    let mut rng = rand::rng();
    let Some((spot, block)) = pick_spot(&arena, kind, &obstacles.0, &alive, &mut rng) else {
        spawner.timers[kind_index(kind)].wait = RETRY_TICKS;
        return;
    };
    let s = spec(kind);
    spawner.timers[kind_index(kind)].wait = s.respawn;
    let pack = if s.group > 1 {
        spawner.next_pack += 1;
        spawner.next_pack
    } else {
        0
    };
    for slot in 0..s.group {
        let pos = if slot == 0 {
            spot
        } else {
            // Followers stand around the leader where there is room.
            (0..8)
                .map(|i| spot + Vec2::from_angle(i as f32 * TAU / 8.0 + slot as f32) * 48.0)
                .find(|p| spot_clear(&arena, kind, *p, &obstacles.0, &alive))
                .unwrap_or(spot)
        };
        let elite = can_be_elite(kind)
            && block.tier >= 2
            && live.elites < MAX_ELITES
            && rng.random_ratio(1, 6);
        live.add(kind, elite, slot == 0);
        let brain = Brain::new(kind, elite, spot, block.rect, pack, slot as u8);
        spawn_monster(&mut commands, kind, elite, pos, brain, None);
    }
}

pub fn spawn_puddle(commands: &mut Commands, kind: PuddleKind, pos: Vec2, owner: Option<Entity>) {
    let (radius, ttl, dps, cause) = match kind {
        PuddleKind::Acid => (ACID_PUDDLE_RADIUS, ACID_PUDDLE_TTL, ACID_DPS, "酸液"),
        PuddleKind::Radiation => (RAD_PUDDLE_RADIUS, RAD_PUDDLE_TTL, RAD_DPS, "辐射"),
    };
    commands.spawn((
        Name::new("Puddle"),
        Puddle { kind, radius, ttl },
        Pos(pos),
        PuddleLife {
            ttl,
            dps,
            owner,
            cause,
        },
        Replicate::to_clients(NetworkTarget::All),
    ));
}

/// Fire a monster projectile from `from` along `dir`.
pub fn shoot(
    commands: &mut Commands,
    kind: ProjectileKind,
    from: Vec2,
    dir: Vec2,
    speed: f32,
    flight: Flight,
) {
    commands.spawn((
        Name::new("MonsterShot"),
        Projectile {
            kind,
            vel: dir * speed,
        },
        Pos(from),
        flight,
        projectile_replication(None),
    ));
}

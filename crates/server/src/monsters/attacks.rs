//! Bites, volleys, ghoul blasts and puddle damage.

use super::*;

#[allow(clippy::type_complexity)]
pub(super) fn attack(
    mut commands: Commands,
    state: Single<&MatchState>,
    arena: Res<Arena>,
    mut monsters: Query<
        (
            Entity,
            &Monster,
            &Pos,
            &mut Brain,
            &mut MonsterState,
            &mut Vitals,
        ),
        Without<Player>,
    >,
    players: Query<(Entity, &Pos, &Vitals), With<Player>>,
    mut damage: ResMut<DamageQueue>,
    mut blasts: ResMut<GhoulBlasts>,
    rng: Res<crate::rng::SimRng>,
) {
    if matches!(
        state.phase,
        Phase::Results | Phase::Recovering | Phase::Starting
    ) {
        return;
    }
    let alive = prey(players.iter());
    let playing = state.phase == Phase::Playing;
    rng.with(playing, crate::rng::Stream::Monsters, |mut rng| {
        let mut order: Vec<(u32, Entity)> = monsters
            .iter()
            .map(|(entity, _, _, brain, _, _)| (brain.serial, entity))
            .collect();
        order.sort_unstable_by_key(|(serial, _)| *serial);
        for (_, entity) in order {
            let Ok((entity, monster, pos, mut brain, mut mstate, mut vitals)) =
                monsters.get_mut(entity)
            else {
                continue;
            };
            let kind = monster.kind;
            if !vitals.alive || kind == MonsterKind::Behemoth {
                continue;
            }
            brain.attack_cd = brain.attack_cd.saturating_sub(1);
            if brain.far {
                continue;
            }
            let mult = brain.damage_mult;
            let at = pos.0;
            let reach = kind.radius() + PLAYER_RADIUS + 4.0;
            let bite = |amount: f32, damage: &mut DamageQueue, cause| {
                let victim = alive.iter().find(|(_, p, _)| p.distance(at) <= reach);
                if let Some((target, ..)) = victim {
                    damage.0.push(Damage {
                        target: *target,
                        attacker: Some(entity),
                        amount: amount * mult,
                        armor_mult: 1.0,
                        weapon: None,
                        emp: false,
                        cause: Some(cause),
                    });
                }
                victim.is_some()
            };
            if kind == MonsterKind::Ghoul && mstate.action == MonsterAction::Arming {
                let fuse = mstate.fuse.saturating_sub(1);
                if fuse == 0 {
                    blasts.0.push(PendingBlast {
                        pos: at,
                        attacker: Some(entity),
                        source: entity,
                        damage: GHOUL_BLAST * mult,
                    });
                    vitals.alive = false;
                    commands.entity(entity).insert(Dying);
                }
                let new = MonsterState { fuse, ..*mstate };
                set_state(&mut mstate, new);
                continue;
            }
            if brain.attack_cd > 0 {
                continue;
            }
            let sees = |range: f32| {
                nearest(&alive, at).filter(|(_, p, d)| *d < range && arena.line_of_sight(at, *p))
            };
            let muzzle = |dir: Vec2| at + dir * (kind.radius() + 6.0);
            match kind {
                MonsterKind::Roach => {
                    if bite(ROACH_BITE, &mut damage, "撕咬") {
                        brain.attack_cd = ROACH_BITE_CD;
                    }
                }
                MonsterKind::Dog => {
                    if bite(DOG_BITE, &mut damage, "撕咬") {
                        brain.attack_cd = DOG_BITE_CD;
                    }
                }
                MonsterKind::Crawler => {
                    if bite(CRAWLER_HIT, &mut damage, "钳击") {
                        brain.attack_cd = CRAWLER_HIT_CD;
                    }
                }
                MonsterKind::Ghoul => {
                    let close = GHOUL_ARM_RANGE + kind.radius();
                    if alive.iter().any(|(_, p, _)| p.distance(at) <= close) {
                        let new = MonsterState {
                            action: MonsterAction::Arming,
                            fuse: GHOUL_FUSE,
                            ..*mstate
                        };
                        set_state(&mut mstate, new);
                    }
                }
                MonsterKind::SporePod => {
                    if sees(POD_WAKE).is_none() {
                        continue;
                    }
                    brain.attack_cd = POD_VOLLEY_EVERY;
                    brain.spiral += 0.23;
                    let step = TAU / POD_GLOBS as f32;
                    for i in 0..POD_GLOBS {
                        let dir = Vec2::from_angle(brain.spiral + i as f32 * step);
                        let flight = Flight::fauna(entity, SPORE_TTL, SPORE_DAMAGE * mult, "孢子");
                        let kind = ProjectileKind::Spore;
                        shoot(&mut commands, kind, muzzle(dir), dir, SPORE_SPEED, flight);
                    }
                }
                MonsterKind::Spitter => {
                    if let Some((_, p, _)) = sees(SPITTER_RANGE) {
                        brain.attack_cd = SPITTER_CD;
                        let dir = (p - at).normalize_or(Vec2::X);
                        let flight = Flight::fauna(entity, ACID_TTL, ACID_DAMAGE * mult, "酸液");
                        let kind = ProjectileKind::Acid;
                        shoot(&mut commands, kind, muzzle(dir), dir, ACID_SPEED, flight);
                    }
                }
                MonsterKind::Drone => {
                    if let Some((_, p, _)) = sees(DRONE_RANGE) {
                        brain.attack_cd = DRONE_CD;
                        let dir = (p - at).normalize_or(Vec2::X);
                        let flight = Flight::fauna(entity, BOLT_TTL, BOLT_DAMAGE * mult, "电弧");
                        let kind = ProjectileKind::Bolt;
                        shoot(&mut commands, kind, muzzle(dir), dir, BOLT_SPEED, flight);
                    }
                }
                MonsterKind::Raider => {
                    if !brain.fire {
                        continue;
                    }
                    let weapon = brain.weapon;
                    // Raiders only ever carry guns.
                    let Some(gun) = weapon.ranged() else {
                        continue;
                    };
                    // Slower trigger finger than a player, or a rifle squad would shred everyone.
                    brain.attack_cd = (gun.cooldown() * 2).max(14);
                    for dir in gun.volley(brain.aim, &mut rng) {
                        let flight = Flight::raider_shot(entity, gun);
                        let kind = ProjectileKind::Shot(weapon);
                        let speed = gun.muzzle_speed();
                        shoot(&mut commands, kind, muzzle(dir), dir, speed, flight);
                    }
                }
                MonsterKind::Behemoth => {}
            }
        }
    });
}

/// Everyone caught in a ghoul blast, once each, with the damage they take.
pub fn blast_hits(
    center: Vec2,
    damage: f32,
    source: Entity,
    targets: impl Iterator<Item = (Entity, Vec2)>,
    line_of_sight: impl Fn(Vec2, Vec2) -> bool,
) -> Vec<(Entity, f32)> {
    let radius = GHOUL_BLAST_RADIUS;
    let mut seen = HashSet::new();
    targets
        .filter(|(e, p)| *e != source && p.distance(center) <= radius && line_of_sight(center, *p))
        .filter(|(e, _)| seen.insert(*e))
        .map(|(e, p)| {
            let falloff = 1.0 - GHOUL_FALLOFF * (p.distance(center) / radius);
            (e, damage * falloff)
        })
        .collect()
}

/// Ghoul blasts hurt players and monsters alike.
#[allow(clippy::type_complexity)]
pub(super) fn ghoul_blasts(
    arena: Res<Arena>,
    mut blasts: ResMut<GhoulBlasts>,
    targets: Query<(Entity, &Pos, &Vitals), Or<(With<Player>, With<Monster>)>>,
    mut damage: ResMut<DamageQueue>,
    mut outbox: ResMut<Outbox>,
) {
    for blast in blasts.0.drain(..) {
        let alive = targets
            .iter()
            .filter(|(_, _, v)| v.alive)
            .map(|(e, p, _)| (e, p.0));
        let hits = blast_hits(blast.pos, blast.damage, blast.source, alive, |a, b| {
            arena.line_of_sight(a, b)
        });
        for (target, amount) in hits {
            damage.0.push(Damage {
                target,
                attacker: blast.attacker,
                amount,
                armor_mult: 1.0,
                weapon: None,
                emp: false,
                cause: Some("尸鬼自爆"),
            });
        }
        outbox.fast.push(GameEvent::Boom {
            pos: blast.pos,
            radius: GHOUL_BLAST_RADIUS,
            fire: true,
        });
    }
}

pub(super) fn puddles(
    mut commands: Commands,
    tick: Res<SimTick>,
    mut puddles: Query<(Entity, &Pos, &Puddle, &mut PuddleLife)>,
    players: Query<(Entity, &Pos, &Vitals), With<Player>>,
    mut damage: ResMut<DamageQueue>,
) {
    let bite = tick.0.is_multiple_of(PUDDLE_EVERY);
    for (entity, pos, puddle, mut life) in &mut puddles {
        life.ttl = life.ttl.saturating_sub(1);
        if life.ttl == 0 {
            commands.entity(entity).despawn();
            continue;
        }
        if !bite {
            continue;
        }
        for (target, p, v) in &players {
            if v.alive && p.0.distance(pos.0) <= puddle.radius + PLAYER_RADIUS * 0.5 {
                damage.0.push(Damage {
                    target,
                    attacker: life.owner,
                    amount: life.dps * PUDDLE_EVERY as f32 / TICKS_PER_SEC as f32,
                    armor_mult: 1.0,
                    weapon: None,
                    emp: false,
                    cause: Some(life.cause),
                });
            }
        }
    }
}

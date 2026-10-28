//! Applying queued damage, death, respawn and dropped loot expiry.

use evoclash_game::sim::PLAYER_RADIUS;

use super::*;
use crate::rng::SimRng;

/// Apply damage through armor. Returns (hp lost, armor lost).
pub(super) fn absorb(vitals: &mut Vitals, amount: f32, armor_mult: f32) -> (f32, f32) {
    let armor_hit = (amount * armor_mult).min(vitals.armor_points);
    vitals.armor_points -= armor_hit;
    let absorbed = armor_hit / armor_mult;
    let hp_hit = (amount - absorbed).max(0.0);
    vitals.hp -= hp_hit;
    if vitals.armor_points <= 0.0 {
        vitals.armor_points = 0.0;
        vitals.armor = ArmorKind::None;
    }
    (hp_hit, armor_hit)
}

#[allow(clippy::type_complexity)]
pub fn resolve_damage(
    mut commands: Commands,
    mut queue: ResMut<DamageQueue>,
    mut outbox: ResMut<Outbox>,
    mut xp: ResMut<XpQueue>,
    mut dealt: ResMut<Dealt>,
    mut players: Query<(
        &Player,
        &Pos,
        &mut Vitals,
        &mut Loadout,
        &mut Stats,
        &Evolution,
        &mut SkillState,
        &mut Recovery,
        &mut Wallet,
        &mut LifeLog,
    )>,
    monsters: Query<&Monster>,
    state: Single<&MatchState>,
    tick: Res<SimTick>,
    mut aggressors: Query<&mut Aggressors>,
    humans: Query<(), With<Human>>,
) {
    if matches!(state.phase, Phase::Results | Phase::Recovering) {
        queue.0.clear();
        return;
    }
    let playing = state.phase == Phase::Playing;
    let elapsed = if playing {
        MATCH_TICKS.saturating_sub(state.ticks_left)
    } else {
        0
    };
    // Snapshot first place before this tick's deaths change kills or deaths.
    let leader = if super::finale::finale_for_kills(&state) {
        super::finale::leader_client_id(
            players
                .iter()
                .map(|(player, _, _, _, stats, ..)| (player.client_id, stats)),
        )
    } else {
        None
    };
    // (victim, killer, how, victim's gear tier and level at the moment of death)
    let mut deaths: Vec<(Entity, Option<Entity>, &'static str, u8, u8)> = Vec::new();
    for dmg in queue.0.drain(..) {
        let attacker = dmg.attacker.and_then(|a| players.get(a).ok());
        let attacker_id = attacker.map_or(0, |(p, ..)| p.client_id);
        let attacker_tier = attacker.map(|(_, _, _, loadout, ..)| loadout.weapon().spec().tier());
        let Ok((player, pos, mut vitals, mut loadout, _, evo, mut skills, mut recovery, ..)) =
            players.get_mut(dmg.target)
        else {
            continue;
        };
        if !vitals.alive || vitals.shield > 0 {
            continue;
        }
        let target_tier = gear_tier(&vitals, &loadout);
        // Catch-up: weaker weapons hit better-geared targets harder (monsters and EMPs excluded).
        let mut amount = match attacker_tier {
            Some(tier) if !dmg.emp => dmg.amount * underdog_mult(tier, target_tier),
            _ => dmg.amount,
        };
        // Ordinary fauna and elites, not the behemoth, from 5:00 of Playing.
        if playing
            && dmg.attacker.is_some_and(|id| {
                monsters
                    .get(id)
                    .is_ok_and(|monster| monster.kind != MonsterKind::Behemoth)
            })
        {
            amount *= sim::fauna_player_mult(elapsed);
        }
        if dmg.emp {
            vitals.armor_points = match vitals.armor {
                ArmorKind::PowerArmor => 0.0,
                _ => vitals.armor_points * 0.5,
            };
            if vitals.armor_points < 1.0 {
                vitals.armor_points = 0.0;
                vitals.armor = ArmorKind::None;
            }
            vitals.emp_slow = 3 * TICKS_PER_SEC as u16;
            // Energy cells short out too.
            loadout.ammo[AmmoKind::Cell.index()] /= 2;
        }
        // 能量护盾 soaks damage before armor does.
        let mut soaked = 0.0;
        if skills.shielded() {
            soaked = amount.min(skills.shield_hp);
            skills.shield_hp -= soaked;
            amount -= soaked;
            if skills.shield_hp <= 0.0 {
                skills.shield_hp = 0.0;
                skills.active_a = 0;
            }
        }
        let hp_before = vitals.hp;
        let (hp_hit, armor_hit) =
            absorb(&mut vitals, amount, dmg.armor_mult * evo.armor_wear_mult());
        recovery.since_hurt = 0;
        if vitals.stealth > 0 {
            vitals.stealth = 0;
            if skills.skill_a == Some(Skill::Stealth) {
                skills.active_a = 0;
            }
        }
        if let Some(by) = dmg.attacker.filter(|a| *a != dmg.target) {
            let melee =
                !dmg.emp && dmg.cause.is_none() && dmg.weapon.is_some_and(|w| w.melee().is_some());
            dealt.0.push(crate::evolution::Hit {
                attacker: by,
                amount: soaked + (amount - hp_hit) + hp_hit.min(hp_before),
                melee,
            });
            // Lobby bots only shoot back at a human who has already hit them.
            if state.phase == Phase::Lobby
                && humans.contains(by)
                && let Ok(mut agg) = aggressors.get_mut(dmg.target)
                && !agg.0.contains(&by)
            {
                agg.0.push(by);
            }
        }
        outbox.fast.push(GameEvent::Hit {
            pos: pos.0,
            amount: hp_hit + armor_hit + soaked,
            armor: armor_hit + soaked > hp_hit,
            attacker: attacker_id,
        });
        if vitals.hp <= 0.0 {
            vitals.hp = 0.0;
            vitals.alive = false;
            vitals.respawn_in = if state.phase == Phase::Lobby {
                LOBBY_RESPAWN_TICKS
            } else {
                RESPAWN_MAX_TICKS
            };
            // Humans get 45s on the death screen, then DeployLast. Bots still leave on their own.
            vitals.deploy_in = if playing && !player.bot {
                AUTO_DEPLOY_TICKS
            } else {
                0
            };
            vitals.stealth = 0;
            // Active skills end with you; cooldowns and perks carry over.
            skills.active_a = 0;
            skills.active_b = 0;
            skills.shield_hp = 0.0;
            let how = dmg
                .cause
                .or(dmg.weapon.map(|w| w.spec().name()))
                .unwrap_or("撕咬");
            deaths.push((dmg.target, dmg.attacker, how, target_tier, evo.level));
        }
    }

    for (victim, attacker, how, victim_tier, victim_level) in deaths {
        let Ok((player, pos, mut vitals, mut loadout, mut stats, ..)) = players.get_mut(victim)
        else {
            continue;
        };
        let victim_name = player.name.clone();
        let victim_id = player.client_id;
        let had_bounty = stats.has_bounty();
        let was_leader = leader == Some(victim_id);
        stats.deaths += 1;
        stats.streak = 0;
        commands.entity(victim).remove::<BunkerBounty>();

        // Everything you carried is up for grabs, in one bag: killing the strong is how the
        // weak catch up. Lobby and Starting deaths leave no bag.
        let drop_at = pos.0;
        if playing {
            let mut belongings = loadout.contents();
            if vitals.armor != ArmorKind::None && vitals.armor_points > 0.0 {
                belongings.push(Item::Armor {
                    kind: vitals.armor,
                    points: vitals.armor_points,
                });
            }
            spawn_bag(&mut commands, belongings, drop_at, DROP_TTL_TICKS);
        }
        *loadout = Loadout::default();
        vitals.armor = ArmorKind::None;
        vitals.armor_points = 0.0;

        let killer = attacker.filter(|a| *a != victim);
        let (killer_name, killer_path, killer_level) =
            match killer.and_then(|k| players.get_mut(k).ok()) {
                Some((
                    killer_player,
                    _,
                    _,
                    mut killer_loadout,
                    mut killer_stats,
                    killer_evo,
                    _,
                    _,
                    _,
                    mut killer_life,
                )) => {
                    // Kills outside Playing grant no score, xp, scrap, streak or bounty.
                    if playing {
                        killer_stats.kills += 1;
                        killer_life.kills += 1;
                        killer_stats.streak += 1;
                        let points = (if had_bounty { 3 } else { 1 })
                            + if was_leader {
                                super::finale::FINALE_LEADER_BONUS
                            } else {
                                0
                            };
                        killer_stats.add_score(points, tick.0);
                        if was_leader {
                            outbox.announce(format!(
                                "{} 击杀了第一名 {victim_name}，+2",
                                killer_player.name
                            ));
                        }
                        let killer_tier = killer_loadout.weapon().spec().tier();
                        let giant_slayer = victim_tier >= killer_tier + GIANT_SLAYER_GAP;
                        xp.0.push(XpAward {
                            to: killer.expect("killer"),
                            base: player_kill_xp(victim_level, had_bounty, giant_slayer),
                            pos: drop_at,
                            loot: false,
                            scrap: player_kill_scrap(had_bounty),
                        });
                        // Every kill salvages a quarter box for the selected gun (or the best one
                        // on the hotbar); 弹药回收 adds shots to it.
                        if let Some(gun) = salvage_target(&killer_loadout)
                            && let Some(kind) = gun.ammo()
                        {
                            let refill = kind.box_rounds() / 4
                                + killer_evo.ammo_per_kill() * gun.ammo_cost();
                            killer_loadout.add_ammo(kind, refill, killer_evo);
                        }
                        if giant_slayer {
                            killer_stats.add_score(1, tick.0);
                            outbox.announce(format!(
                                "{} 以弱胜强击败了 {victim_name}，额外 +1 分！",
                                killer_player.name
                            ));
                        }
                        if killer_stats.streak == sim::BOUNTY_STREAK {
                            outbox.announce(format!(
                                "{} 连杀 {} 人，已被悬赏！",
                                killer_player.name, killer_stats.streak
                            ));
                        }
                    }
                    (
                        killer_player.name.clone(),
                        killer_evo.lead_path(),
                        killer_evo.level,
                    )
                }
                None => (
                    attacker
                        .and_then(|a| monsters.get(a).ok())
                        .map_or_else(|| "废土".to_string(), |m| m.display_name()),
                    None,
                    0,
                ),
            };
        info!("{killer_name} killed {victim_name} ({how})");
        // The death card, and the perks 沿用上次配置 goes back to. Not during practice.
        if let Ok((.., evo, _, _, mut wallet, mut life)) = players.get_mut(victim) {
            if playing {
                wallet.recap = Some(DeathRecap {
                    killer: killer_name.clone(),
                    how: how.to_string(),
                    killer_path,
                    killer_level,
                    alive_secs: tick.0.saturating_sub(life.born) / TICKS_PER_SEC,
                    scrap: life.scrap,
                    kills: life.kills,
                });
                *life = LifeLog {
                    born: tick.0,
                    last_ranks: evo.ranks,
                    last_first_pick: evo.first_pick,
                    ..default()
                };
            } else {
                wallet.recap = None;
            }
        }
        outbox.reliable.push(GameEvent::Kill {
            killer: killer_name,
            victim: victim_name,
            how: how.to_string(),
            bounty: had_bounty,
        });
    }
}

/// The gun whose ammo a kill salvages: the selected one, else the best on the hotbar.
pub(super) fn salvage_target(loadout: &Loadout) -> Option<WeaponKind> {
    loadout
        .slot(loadout.selected)
        .filter(|w| w.ammo().is_some())
        .or_else(|| {
            loadout
                .guns
                .iter()
                .flatten()
                .copied()
                .max_by_key(|w| w.spec().tier())
        })
}

/// First 3 minutes (more than 5:00 left): the old edge spawn, farthest from the living.
const EARLY_UNTIL: u32 = 3 * 60 * TICKS_PER_SEC;
/// 6:00 onward: street points inside this radius of the bunker.
const LATE_FROM: u32 = 6 * 60 * TICKS_PER_SEC;
/// 3:00 to 6:00: a spawn counts only when the nearest living player is at least this far.
const MID_CLEAR: f32 = 480.0;
const LATE_RADIUS: f32 = 480.0;
const LATE_CLEAR: f32 = 300.0;
/// Fixed grid. No RNG, so a replay that respawns on the same tick lands on the same cell.
const LATE_STEP: i32 = 40;

fn separation_sq(p: Vec2, alive: &[Vec2]) -> f32 {
    alive
        .iter()
        .map(|q| p.distance_squared(*q))
        .fold(f32::MAX, f32::min)
}

fn farther(a: Vec2, b: Vec2, alive: &[Vec2]) -> Vec2 {
    match separation_sq(a, alive).total_cmp(&separation_sq(b, alive)) {
        std::cmp::Ordering::Greater => a,
        std::cmp::Ordering::Less => b,
        std::cmp::Ordering::Equal => {
            if (a.x.to_bits(), a.y.to_bits()) <= (b.x.to_bits(), b.y.to_bits()) {
                a
            } else {
                b
            }
        }
    }
}

fn farthest(points: &[Vec2], alive: &[Vec2]) -> Option<Vec2> {
    points.iter().copied().reduce(|a, b| farther(a, b, alive))
}

fn closest_to_center(points: &[Vec2], n: usize) -> Vec<Vec2> {
    let mut idx: Vec<usize> = (0..points.len()).collect();
    idx.sort_by(|&a, &b| {
        points[a]
            .length_squared()
            .total_cmp(&points[b].length_squared())
            .then(a.cmp(&b))
    });
    idx.truncate(n.min(points.len()));
    idx.into_iter().map(|i| points[i]).collect()
}

fn pick_cleared(points: &[Vec2], alive: &[Vec2], clear: f32) -> Option<Vec2> {
    let clear_sq = clear * clear;
    let far: Vec<Vec2> = points
        .iter()
        .copied()
        .filter(|p| separation_sq(*p, alive) >= clear_sq)
        .collect();
    let pool: &[Vec2] = if far.is_empty() { points } else { &far };
    farthest(pool, alive)
}

fn on_street(map: &ArenaMap, p: Vec2, obstacles: &[(Vec2, f32)]) -> bool {
    if map.building_at(p).is_some() || map.in_hazard(p) {
        return false;
    }
    if map.resolve_circle(p, PLAYER_RADIUS) != p {
        return false;
    }
    obstacles.iter().all(|(c, r)| {
        let min = r + PLAYER_RADIUS;
        p.distance_squared(*c) >= min * min
    })
}

fn disk_points() -> Vec<Vec2> {
    let n = (LATE_RADIUS / LATE_STEP as f32) as i32;
    let radius_sq = LATE_RADIUS * LATE_RADIUS;
    let mut points = Vec::new();
    for iy in -n..=n {
        for ix in -n..=n {
            let p = Vec2::new((ix * LATE_STEP) as f32, (iy * LATE_STEP) as f32);
            if p.length_squared() <= radius_sq {
                points.push(p);
            }
        }
    }
    points
}

/// Playing respawn spot. `elapsed_ticks` is time since Playing started.
fn pick_spawn(
    map: &ArenaMap,
    alive: &[Vec2],
    elapsed_ticks: u32,
    obstacles: &[(Vec2, f32)],
) -> Vec2 {
    if elapsed_ticks >= LATE_FROM {
        let disk = disk_points();
        let street: Vec<Vec2> = disk
            .iter()
            .copied()
            .filter(|p| on_street(map, *p, obstacles))
            .collect();
        let clear_sq = LATE_CLEAR * LATE_CLEAR;
        let cleared: Vec<Vec2> = street
            .iter()
            .copied()
            .filter(|p| separation_sq(*p, alive) >= clear_sq)
            .collect();
        if let Some(spot) = farthest(&cleared, alive) {
            return spot;
        }
        // Nothing on the street is 300 clear: farthest grid point inside the disk.
        return farthest(&disk, alive).unwrap_or(Vec2::ZERO);
    }
    if elapsed_ticks < EARLY_UNTIL {
        return farthest(&map.spawn_points, alive).unwrap_or(Vec2::ZERO);
    }
    let inner = closest_to_center(&map.spawn_points, 4);
    pick_cleared(&inner, alive, MID_CLEAR).unwrap_or(Vec2::ZERO)
}

/// Bring the dead back. Playing: only after Deploy / DeployLast (`deploy_asap`); an
/// unconfirmed human sticks at `respawn_in == 1` until `deploy_in` hits 0, which sends
/// them with DeployLast. Bots are unchanged (their `deploy_in` stays 0). Lobby: the 2s
/// timer counts down to 0 on its own. Whatever was bought in the shop is equipped on the spot.
pub fn respawn(
    arena: Res<Arena>,
    obstacles: Option<Res<Obstacles>>,
    tick: Res<SimTick>,
    state: Query<&MatchState>,
    rng: Option<Res<SimRng>>,
    mut players: Query<(
        Entity,
        &mut Pos,
        &mut Vitals,
        &mut Dash,
        &mut PosHistory,
        &mut Loadout,
        &mut Evolution,
        &mut SkillState,
        &mut Wallet,
        &mut LifeLog,
        Option<&Player>,
    )>,
) {
    if state.iter().any(|s| s.phase == Phase::Recovering) {
        return;
    }
    let lobby = state.iter().any(|s| s.phase == Phase::Lobby);
    let playing = state.iter().any(|s| s.phase == Phase::Playing);
    let mut alive: Vec<(Entity, Vec2)> = players
        .iter()
        .filter(|(_, _, v, ..)| v.alive)
        .map(|(e, p, ..)| (e, p.0))
        .collect();
    let mut dead: Vec<(u64, Entity)> = players
        .iter()
        .filter(|(_, _, v, ..)| !v.alive)
        .map(|(entity, _, _, _, _, _, _, _, _, _, player)| {
            (player.map(|p| p.client_id).unwrap_or(u64::MAX), entity)
        })
        .collect();
    dead.sort_unstable_by_key(|(id, _)| *id);
    for (_, entity) in dead {
        let Ok((
            entity,
            mut pos,
            mut vitals,
            mut dash,
            mut history,
            mut loadout,
            mut evo,
            mut skills,
            mut wallet,
            mut life,
            player,
        )) = players.get_mut(entity)
        else {
            continue;
        };
        if vitals.alive {
            continue;
        }
        if playing {
            let human = player.is_some_and(|p| !p.bot);
            if human && vitals.deploy_in > 0 {
                vitals.deploy_in -= 1;
                if vitals.deploy_in == 0 && !life.deploy_asap {
                    let changed = crate::evolution::handle_command(
                        &PlayerCommand::DeployLast,
                        &mut evo,
                        &mut wallet,
                        &mut vitals,
                        &mut life,
                        true,
                    );
                    if changed {
                        crate::evolution::apply_evolution(&evo, &mut skills, &mut vitals);
                    }
                }
            }
            if life.deploy_asap && can_deploy(vitals.respawn_in) {
                vitals.respawn_in = 0;
            } else if vitals.respawn_in > 1 {
                vitals.respawn_in -= 1;
            }
        } else {
            // Lobby (and any other non-playing phase): ignore the confirm flag.
            vitals.respawn_in = vitals.respawn_in.saturating_sub(1);
        }
        if vitals.respawn_in > 0 {
            continue;
        }
        // Lobby: a random spawn. Playing: early edge, then the inner four, then the streets
        // by the bunker. Anything else (Starting) keeps the old farthest edge spawn.
        let living: Vec<Vec2> = alive
            .iter()
            .filter(|(e, _)| *e != entity)
            .map(|(_, q)| *q)
            .collect();
        let spot = if lobby && !arena.spawn_points.is_empty() {
            let index = rng.as_ref().map_or(0, |rng| {
                rng.with(false, crate::rng::Stream::Loot, |rng| {
                    use rand::Rng;
                    rng.random_range(0..arena.spawn_points.len())
                })
            });
            arena.spawn_points[index]
        } else if playing {
            let elapsed = state
                .iter()
                .find(|s| s.phase == Phase::Playing)
                .map_or(0, |s| MATCH_TICKS.saturating_sub(s.ticks_left));
            let blocks = obstacles.as_ref().map(|o| o.0.as_slice()).unwrap_or(&[]);
            pick_spawn(&arena, &living, elapsed, blocks)
        } else {
            farthest(&arena.spawn_points, &living).unwrap_or(Vec2::ZERO)
        };
        pos.0 = spot;
        alive.push((entity, spot));
        history.0.clear();
        *dash = Dash::default();
        *vitals = Vitals {
            hp: evo.max_hp(),
            max_hp: evo.max_hp(),
            shield: SPAWN_SHIELD_TICKS,
            ..default()
        };
        for item in wallet.take_cart() {
            take(item, &mut vitals, &mut loadout, &evo);
        }
        // Death already wrote a fresh log; revival only stamps this life and drops the confirm.
        life.born = tick.0;
        life.deploy_asap = false;
    }
}

pub(super) fn expire_drops(
    state: Query<&MatchState>,
    mut commands: Commands,
    mut drops: Query<(Entity, &mut Expires)>,
) {
    if state.iter().any(|s| s.phase == Phase::Recovering) {
        return;
    }
    for (entity, mut expires) in &mut drops {
        expires.0 = expires.0.saturating_sub(1);
        if expires.0 == 0 {
            commands.entity(entity).despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inner4(map: &ArenaMap) -> Vec<Vec2> {
        let mut idx: Vec<usize> = (0..map.spawn_points.len()).collect();
        idx.sort_by(|&a, &b| {
            map.spawn_points[a]
                .length_squared()
                .total_cmp(&map.spawn_points[b].length_squared())
                .then(a.cmp(&b))
        });
        idx.truncate(4);
        idx.into_iter().map(|i| map.spawn_points[i]).collect()
    }

    #[test]
    fn playing_spawns_change_at_three_and_six_minutes() {
        let map = ArenaMap::generate(1);
        let inner = inner4(&map);
        assert_eq!(inner.len(), 4);
        let open = inner[0];
        let campers = vec![inner[1], inner[2], inner[3]];

        let early = pick_spawn(&map, &campers, EARLY_UNTIL - 1, &[]);
        let mid = pick_spawn(&map, &campers, EARLY_UNTIL, &[]);
        let still_mid = pick_spawn(&map, &campers, LATE_FROM - 1, &[]);
        assert_eq!(mid, open, "3:00 picks the inner spawn clear of the living");
        assert_eq!(still_mid, open);
        assert!(
            map.spawn_points.contains(&early),
            "before 3:00 a respawn is still an edge spawn"
        );
        assert_ne!(early, open);

        let crowded: Vec<Vec2> = inner.iter().map(|p| *p + Vec2::new(100.0, 0.0)).collect();
        let fallback = pick_spawn(&map, &crowded, EARLY_UNTIL, &[]);
        let expected = inner
            .iter()
            .copied()
            .reduce(|a, b| farther(a, b, &crowded))
            .unwrap();
        assert_eq!(fallback, expected);
        assert!(separation_sq(fallback, &crowded) < MID_CLEAR * MID_CLEAR);

        let late_alive = [Vec2::new(2000.0, 1200.0)];
        let late = pick_spawn(&map, &late_alive, LATE_FROM, &[]);
        assert!(late.length_squared() <= LATE_RADIUS * LATE_RADIUS);
        assert!(on_street(&map, late, &[]));
        assert!(separation_sq(late, &late_alive) >= LATE_CLEAR * LATE_CLEAR);
        assert_eq!(
            late,
            pick_spawn(&map, &late_alive, 8 * 60 * TICKS_PER_SEC, &[])
        );
        assert!(
            !map.spawn_points.contains(&late),
            "6:00 leaves the edge spawn list"
        );

        let blocked = pick_spawn(&map, &late_alive, LATE_FROM, &[(late, 30.0)]);
        assert_ne!(blocked, late);
        assert!(on_street(&map, blocked, &[(late, 30.0)]));
        assert!(blocked.length_squared() <= LATE_RADIUS * LATE_RADIUS);
    }

    fn fauna_hp(elapsed: u32, kind: MonsterKind) -> f32 {
        let mut app = App::new();
        app.init_resource::<DamageQueue>()
            .init_resource::<Outbox>()
            .init_resource::<XpQueue>()
            .init_resource::<Dealt>()
            .init_resource::<SimTick>();
        app.world_mut().spawn(MatchState {
            phase: Phase::Playing,
            ticks_left: MATCH_TICKS.saturating_sub(elapsed),
            ..default()
        });
        app.add_systems(Update, resolve_damage);
        let attacker = app
            .world_mut()
            .spawn(Monster {
                kind,
                elite: kind != MonsterKind::Behemoth,
            })
            .id();
        let player = app
            .world_mut()
            .spawn((
                Player {
                    client_id: 1,
                    name: "p".into(),
                    color: [1.0; 3],
                    bot: true,
                },
                Pos(Vec2::ZERO),
                Vitals::default(),
                Loadout::default(),
                Stats::default(),
                Evolution::default(),
                SkillState::default(),
                Recovery::default(),
                Wallet::default(),
                LifeLog::default(),
            ))
            .id();
        app.world_mut()
            .resource_mut::<DamageQueue>()
            .0
            .push(Damage {
                target: player,
                attacker: Some(attacker),
                amount: 40.0,
                armor_mult: 1.0,
                weapon: None,
                emp: false,
                cause: Some("撕咬"),
            });
        app.update();
        app.world().get::<Vitals>(player).unwrap().hp
    }

    #[test]
    fn fauna_damage_drops_after_five_minutes_except_the_behemoth() {
        let before = 5 * 60 * TICKS_PER_SEC - 1;
        let after = 5 * 60 * TICKS_PER_SEC;
        assert_eq!(fauna_hp(before, MonsterKind::Roach), 60.0);
        assert_eq!(fauna_hp(after, MonsterKind::Roach), 76.0);
        assert_eq!(fauna_hp(after, MonsterKind::Crawler), 76.0);
        assert_eq!(fauna_hp(after, MonsterKind::Behemoth), 60.0);
    }
}

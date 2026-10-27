//! Applying queued damage, death, respawn and dropped loot expiry.

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
        let had_bounty = stats.has_bounty();
        stats.deaths += 1;
        stats.streak = 0;

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
                        killer_stats.add_score(if had_bounty { 3 } else { 1 }, tick.0);
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

/// Bring the dead back. Playing: only after Deploy / DeployLast (`deploy_asap`); an
/// unconfirmed human sticks at `respawn_in == 1` until `deploy_in` hits 0, which sends
/// them with DeployLast. Bots are unchanged (their `deploy_in` stays 0). Lobby: the 2s
/// timer counts down to 0 on its own. Whatever was bought in the shop is equipped on the spot.
pub fn respawn(
    arena: Res<Arena>,
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
        // Lobby: a random spawn. A match: as far as possible from everyone else.
        let spot = if lobby && !arena.spawn_points.is_empty() {
            let index = rng.as_ref().map_or(0, |rng| {
                rng.with(false, crate::rng::Stream::Loot, |rng| {
                    use rand::Rng;
                    rng.random_range(0..arena.spawn_points.len())
                })
            });
            arena.spawn_points[index]
        } else {
            arena
                .spawn_points
                .iter()
                .copied()
                .max_by(|a, b| {
                    let score = |p: &Vec2| {
                        alive
                            .iter()
                            .filter(|(e, _)| *e != entity)
                            .map(|(_, q)| p.distance(*q))
                            .fold(f32::MAX, f32::min)
                    };
                    score(a).total_cmp(&score(b))
                })
                .unwrap_or(Vec2::ZERO)
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

pub(super) fn expire_drops(mut commands: Commands, mut drops: Query<(Entity, &mut Expires)>) {
    for (entity, mut expires) in &mut drops {
        expires.0 = expires.0.saturating_sub(1);
        if expires.0 == 0 {
            commands.entity(entity).despawn();
        }
    }
}

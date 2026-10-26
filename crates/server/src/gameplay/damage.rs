//! Applying queued damage, death, respawn and dropped loot expiry.

use super::*;

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
    )>,
    monsters: Query<&Monster>,
    state: Single<&MatchState>,
) {
    if state.phase == Phase::Intermission {
        queue.0.clear();
        return;
    }
    // (victim, killer, how, victim's gear tier and level at the moment of death)
    let mut deaths: Vec<(Entity, Option<Entity>, &'static str, u8, u8)> = Vec::new();
    for dmg in queue.0.drain(..) {
        let attacker = dmg.attacker.and_then(|a| players.get(a).ok());
        let attacker_id = attacker.map_or(0, |(p, ..)| p.client_id);
        let attacker_tier = attacker.map(|(_, _, _, loadout, ..)| loadout.weapon.spec().tier());
        let Ok((_, pos, mut vitals, mut loadout, _, evo, mut skills, mut recovery)) =
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
            // Energy weapons short out too.
            if matches!(loadout.weapon, WeaponKind::Laser | WeaponKind::Railgun) {
                loadout.ammo /= 2;
            }
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
            vitals.respawn_in = RESPAWN_TICKS;
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

        // Everything you carried is up for grabs: killing the strong is how the weak catch up.
        let drop_at = pos.0;
        if loadout.weapon != WeaponKind::Stick && loadout.ammo > 0 {
            let item = Item::Weapon {
                kind: loadout.weapon,
                ammo: loadout.ammo,
            };
            spawn_pickup(
                &mut commands,
                item,
                drop_at + Vec2::new(-14.0, 0.0),
                Some(DROP_TTL_TICKS),
            );
        }
        if vitals.armor != ArmorKind::None && vitals.armor_points > 0.0 {
            let item = Item::Armor {
                kind: vitals.armor,
                points: vitals.armor_points,
            };
            spawn_pickup(
                &mut commands,
                item,
                drop_at + Vec2::new(14.0, 0.0),
                Some(DROP_TTL_TICKS),
            );
        }
        if loadout.emps > 0 {
            spawn_pickup(
                &mut commands,
                Item::Emp,
                drop_at + Vec2::new(0.0, 14.0),
                Some(DROP_TTL_TICKS),
            );
        }
        *loadout = Loadout::default();
        vitals.armor = ArmorKind::None;
        vitals.armor_points = 0.0;

        let killer = attacker.filter(|a| *a != victim);
        let killer_name = match killer.and_then(|k| players.get_mut(k).ok()) {
            Some((killer_player, _, _, mut killer_loadout, mut killer_stats, killer_evo, ..)) => {
                killer_stats.kills += 1;
                killer_stats.streak += 1;
                killer_stats.score += if had_bounty { 3 } else { 1 };
                let killer_tier = killer_loadout.weapon.spec().tier();
                let giant_slayer = victim_tier >= killer_tier + GIANT_SLAYER_GAP;
                xp.0.push(XpAward {
                    to: killer.expect("killer"),
                    base: player_kill_xp(victim_level, had_bounty, giant_slayer),
                    pos: drop_at,
                    loot: false,
                });
                // Every kill salvages a quarter magazine; 弹药回收 adds to it.
                let weapon = killer_loadout.weapon;
                let refill = killer_evo.ammo_per_kill() + weapon.ammo_box() / 2;
                if refill > 0 && weapon != WeaponKind::Stick {
                    killer_loadout.ammo =
                        (killer_loadout.ammo + refill).min(killer_evo.max_ammo(weapon));
                }
                if giant_slayer {
                    killer_stats.score += 1;
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
                killer_player.name.clone()
            }
            None => attacker
                .and_then(|a| monsters.get(a).ok())
                .map_or_else(|| "废土".to_string(), |m| m.display_name()),
        };
        info!("{killer_name} killed {victim_name} ({how})");
        outbox.reliable.push(GameEvent::Kill {
            killer: killer_name,
            victim: victim_name,
            how: how.to_string(),
            bounty: had_bounty,
        });
    }
}

/// Gear handed to someone respawning this many points behind the leader, so a fallen
/// player comes back able to fight instead of with a stick against a railgun.
pub(super) fn catch_up_kit(
    deficit: u32,
    rng: &mut impl Rng,
) -> (Option<WeaponKind>, Option<ArmorKind>) {
    match deficit {
        12.. => (Some(WeaponKind::Rifle), Some(ArmorKind::Kevlar)),
        7.. => (Some(WeaponKind::Rifle), Some(ArmorKind::Scrap)),
        3.. => {
            let weapon = if rng.random_bool(0.5) {
                WeaponKind::Bow
            } else {
                WeaponKind::PipeShotgun
            };
            (Some(weapon), None)
        }
        _ => (None, None),
    }
}

pub(super) fn respawn(
    arena: Res<Arena>,
    mut players: Query<(
        Entity,
        &mut Pos,
        &mut Vitals,
        &mut Dash,
        &mut PosHistory,
        &mut Loadout,
        &Stats,
        &Evolution,
    )>,
) {
    let mut alive: Vec<(Entity, Vec2)> = players
        .iter()
        .filter(|(_, _, v, ..)| v.alive)
        .map(|(e, p, ..)| (e, p.0))
        .collect();
    let leader = players.iter().map(|(.., s, _)| s.score).max().unwrap_or(0);
    let mut rng = rand::rng();
    for (entity, mut pos, mut vitals, mut dash, mut history, mut loadout, stats, evo) in
        &mut players
    {
        if vitals.alive {
            continue;
        }
        vitals.respawn_in = vitals.respawn_in.saturating_sub(1);
        if vitals.respawn_in > 0 {
            continue;
        }
        // Spawn as far as possible from everyone else.
        let spot = arena
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
            .unwrap_or(Vec2::ZERO);
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
        let (weapon, armor) = catch_up_kit(leader.saturating_sub(stats.score), &mut rng);
        if let Some(kind) = weapon {
            *loadout = Loadout {
                weapon: kind,
                ammo: evo.max_ammo(kind),
                ..*loadout
            };
        }
        if let Some(kind) = armor {
            vitals.armor = kind;
            vitals.armor_points = kind.max_points();
        }
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

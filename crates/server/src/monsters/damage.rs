//! Damage taken by monsters, death, loot drops and match reset.

use super::*;

/// Damage multiplier for a hit on a crawler facing `facing` (radians) from `toward_attacker`.
pub fn crawler_mult(facing: f32, toward_attacker: Vec2) -> f32 {
    let dir = toward_attacker.normalize_or_zero();
    if dir != Vec2::ZERO && dir.dot(Vec2::from_angle(facing)) >= CRAWLER_SHELL_COS {
        CRAWLER_SHELL_MULT
    } else {
        1.0
    }
}

pub(super) fn drops(monster: Monster, brain: &Brain, rng: &mut impl Rng) -> Vec<Item> {
    let tier = spec(monster.kind).loot_tier;
    if monster.elite {
        return vec![roll_loot((tier + 1).min(4), rng), Item::Medkit];
    }
    match monster.kind {
        MonsterKind::Roach if rng.random_bool(0.35) => vec![roll_loot(1, rng)],
        MonsterKind::Roach if rng.random_bool(0.3) => vec![Item::Medkit],
        MonsterKind::Dog if rng.random_bool(0.25) => vec![Item::Medkit],
        MonsterKind::SporePod | MonsterKind::Spitter | MonsterKind::Crawler => {
            vec![roll_loot(tier, rng)]
        }
        MonsterKind::Drone if rng.random_bool(0.4) => {
            vec![if rng.random_bool(0.5) {
                Item::Emp
            } else {
                Item::Medkit
            }]
        }
        MonsterKind::Raider => {
            // Whatever it carried, like any fallen fighter.
            let mut items = vec![Item::Weapon {
                kind: brain.weapon,
                ammo: brain.weapon.stats().max_ammo / 2,
            }];
            if rng.random_bool(0.35) {
                items.push(Item::Armor {
                    kind: ArmorKind::Kevlar,
                    points: 40.0,
                });
            }
            items
        }
        _ => Vec::new(),
    }
}

/// Monsters take damage here; everything aimed at a combatant is left for `resolve_damage`.
#[allow(clippy::type_complexity)]
pub(super) fn resolve_monster_damage(
    mut commands: Commands,
    mut xp: ResMut<XpQueue>,
    mut dealt: ResMut<Dealt>,
    mut queue: ResMut<DamageQueue>,
    mut outbox: ResMut<Outbox>,
    mut monsters: Query<(&Monster, &Pos, &mut Vitals, &mut Brain, &MonsterState)>,
    mut players: Query<(Entity, &Player, &mut Stats)>,
    positions: Query<&Pos>,
    mut blasts: ResMut<GhoulBlasts>,
    mut boss: ResMut<BossState>,
) {
    let mut rng = rand::rng();
    queue.0.retain(|dmg| {
        let Ok((monster, pos, mut vitals, mut brain, mstate)) = monsters.get_mut(dmg.target) else {
            return true;
        };
        if !vitals.alive {
            return false;
        }
        let player_attacker = dmg.attacker.filter(|a| players.contains(*a));
        let mut amount = dmg.amount;
        match monster.kind {
            MonsterKind::Crawler => {
                if let Some(from) = dmg.attacker.and_then(|a| positions.get(a).ok()) {
                    amount *= crawler_mult(mstate.facing, from.0 - pos.0);
                }
            }
            MonsterKind::Raider => brain.provoked = RAIDER_PROVOKED,
            MonsterKind::Behemoth => {
                if let Some(attacker) = player_attacker {
                    let mine = players.get(attacker).map_or(0, |(.., s)| s.score);
                    let best_other = players
                        .iter()
                        .filter(|(e, ..)| *e != attacker)
                        .map(|(.., s)| s.score)
                        .max()
                        .unwrap_or(0);
                    amount *= boss::damage_mult(mine, best_other);
                }
            }
            _ => {}
        }
        let amount = amount.min(vitals.hp);
        vitals.hp -= amount;
        if monster.kind == MonsterKind::Behemoth
            && let Some(attacker) = player_attacker
        {
            boss.credit(attacker, amount);
        }
        outbox.fast.push(GameEvent::Hit {
            pos: pos.0,
            amount,
            armor: false,
            attacker: dmg
                .attacker
                .and_then(|a| players.get(a).ok())
                .map_or(0, |(_, p, _)| p.client_id),
        });
        if let Some(by) = dmg.attacker {
            let melee = dmg.cause.is_none() && dmg.weapon.is_some_and(|w| w.stats().melee);
            dealt.0.push(Hit {
                attacker: by,
                amount,
                melee,
            });
        }
        if vitals.hp > 0.0 {
            return false;
        }
        vitals.alive = false;
        commands.entity(dmg.target).despawn();
        if monster.kind == MonsterKind::Behemoth {
            // Rewards are handed out by `boss::reward`.
            boss.slain(pos.0);
            return false;
        }
        if monster.kind == MonsterKind::Ghoul {
            blasts.0.push(PendingBlast {
                pos: pos.0,
                attacker: dmg.attacker,
                source: dmg.target,
                damage: GHOUL_BLAST * brain.damage_mult,
            });
        }
        let items = drops(*monster, &brain, &mut rng);
        let n = items.len();
        for (i, item) in items.into_iter().enumerate() {
            let offset = if n > 1 {
                Vec2::from_angle(i as f32 * TAU / n as f32) * 16.0
            } else {
                Vec2::ZERO
            };
            spawn_pickup(&mut commands, item, pos.0 + offset, Some(LOOT_TTL));
        }
        if let Some(by) = dmg.attacker {
            // Non-players (raiders) are skipped by `grant_xp`.
            xp.0.push(XpAward {
                to: by,
                base: monster.xp(),
                pos: pos.0,
                loot: false,
            });
        }
        if let Some(killer) = player_attacker
            && let Ok((_, player, mut stats)) = players.get_mut(killer)
        {
            let points = monster.score();
            if points > 0 {
                stats.score += points;
                outbox.announce(format!(
                    "{} 击杀了{}，+{points} 分",
                    player.name,
                    monster.display_name()
                ));
            }
        }
        false
    });
}

pub(super) fn reap(mut commands: Commands, dying: Query<Entity, With<Dying>>) {
    for entity in &dying {
        commands.entity(entity).despawn();
    }
}

pub(super) fn reset_monsters(
    reset: Res<ResetRequested>,
    mut spawner: ResMut<Spawner>,
    mut blasts: ResMut<GhoulBlasts>,
) {
    if reset.0 {
        *spawner = Spawner::default();
        blasts.0.clear();
    }
}

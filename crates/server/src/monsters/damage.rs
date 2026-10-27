//! Damage taken by monsters, death, loot drops and match reset.

use evoclash_game::items::zone_ammo;
use evoclash_game::shop::monster_scrap;

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

/// One `roll_ammo` (so the rolls after it stay in order), replaced by a full box of every
/// family the zone stocks.
fn ammo_drop(tier: u8, rng: &mut impl Rng) -> Vec<Item> {
    let _rolled = roll_ammo(tier, rng);
    zone_ammo(tier).iter().copied().map(Item::ammo).collect()
}

pub(super) fn drops(monster: Monster, brain: &Brain, rng: &mut impl Rng) -> Vec<Item> {
    let tier = spec(monster.kind).loot_tier;
    if monster.elite {
        return vec![roll_loot((tier + 1).min(4), rng), Item::Medkit];
    }
    // Normal monsters pay in scrap; gear comes from the shop, crates and other fighters.
    match monster.kind {
        MonsterKind::Roach if rng.random_bool(0.25) => ammo_drop(tier, rng),
        MonsterKind::Roach if rng.random_bool(0.2) => vec![Item::Medkit],
        MonsterKind::Dog if rng.random_bool(0.25) => vec![Item::Medkit],
        MonsterKind::SporePod | MonsterKind::Spitter | MonsterKind::Crawler => {
            let mut items = ammo_drop(tier, rng);
            if rng.random_bool(0.35) {
                items.push(Item::Medkit);
            }
            items
        }
        MonsterKind::Drone if rng.random_bool(0.4) => match rng.random_range(0..3) {
            0 => vec![Item::Emp],
            1 => vec![Item::Medkit],
            _ => ammo_drop(tier, rng),
        },
        MonsterKind::Raider => {
            // Whatever it carried, like any fallen fighter: the gun and its ammo.
            let gun = Item::weapon(brain.weapon);
            let mut items: Vec<Item> = [Some(gun), gun.matching_ammo()]
                .into_iter()
                .flatten()
                .collect();
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
    state: Query<&MatchState>,
    tick: Res<SimTick>,
    rng: Res<crate::rng::SimRng>,
) {
    let playing = state.iter().any(|s| s.phase == Phase::Playing);
    rng.with(playing, crate::rng::Stream::Monsters, |mut rng| {
        queue.0.retain(|dmg| {
            let Ok((monster, pos, mut vitals, mut brain, mstate)) = monsters.get_mut(dmg.target)
            else {
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
                let melee = dmg.cause.is_none() && dmg.weapon.is_some_and(|w| w.melee().is_some());
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
                spawn_loot(&mut commands, item, pos.0 + offset, Some(LOOT_TTL));
            }
            if playing {
                if let Some(by) = dmg.attacker {
                    // Non-players (raiders) are skipped by `grant_xp`.
                    xp.0.push(XpAward {
                        to: by,
                        base: monster.xp(),
                        pos: pos.0,
                        loot: false,
                        scrap: monster_scrap(monster.kind)
                            * if monster.elite { ELITE_XP_MULT } else { 1 },
                    });
                }
                if let Some(killer) = player_attacker
                    && let Ok((_, player, mut stats)) = players.get_mut(killer)
                {
                    let points = monster.score();
                    if points > 0 {
                        stats.add_score(points, tick.0);
                        outbox.announce(format!(
                            "{} 击杀了{}，+{points} 分",
                            player.name,
                            monster.display_name()
                        ));
                    }
                }
            }
            false
        });
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

#[cfg(test)]
mod tests {
    use bevy::prelude::*;
    use evoclash_game::items::{AmmoKind, Item, roll_ammo, zone_ammo};
    use evoclash_game::protocol::{Monster, MonsterKind};
    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};

    use super::drops;
    use crate::monsters::{Brain, spec};

    fn brain(kind: MonsterKind, rng: &mut impl Rng) -> Brain {
        Brain::new(
            kind,
            false,
            Vec2::ZERO,
            Rect::from_center_size(Vec2::ZERO, Vec2::splat(64.0)),
            0,
            0,
            rng,
        )
    }

    fn ammo_kinds(items: &[Item]) -> Vec<(AmmoKind, u16)> {
        items
            .iter()
            .filter_map(|item| match item {
                Item::Ammo { kind, rounds } => Some((*kind, *rounds)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn normal_ammo_drops_are_both_zone_families() {
        let mut rng = StdRng::seed_from_u64(7);
        let roach_brain = brain(MonsterKind::Roach, &mut rng);
        let mut shadow = rng.clone();
        let roach = Monster {
            kind: MonsterKind::Roach,
            elite: false,
        };
        let tier = spec(MonsterKind::Roach).loot_tier;
        let mut saw_pair = false;
        for _ in 0..40 {
            let items = drops(roach, &roach_brain, &mut rng);
            let expected = if shadow.random_bool(0.25) {
                let _rolled = roll_ammo(tier, &mut shadow);
                let boxes: Vec<Item> = zone_ammo(tier).iter().copied().map(Item::ammo).collect();
                assert_eq!(boxes.len(), 2);
                assert_ne!(boxes[0], boxes[1]);
                saw_pair = true;
                boxes
            } else if shadow.random_bool(0.2) {
                vec![Item::Medkit]
            } else {
                Vec::new()
            };
            assert_eq!(items, expected);
        }
        assert!(saw_pair, "roach ammo chance never came up");

        let mut rng = StdRng::seed_from_u64(8);
        let pod_brain = brain(MonsterKind::SporePod, &mut rng);
        let items = drops(
            Monster {
                kind: MonsterKind::SporePod,
                elite: false,
            },
            &pod_brain,
            &mut rng,
        );
        let ammo = ammo_kinds(&items);
        assert_eq!(
            ammo,
            vec![
                (AmmoKind::Bullet, AmmoKind::Bullet.box_rounds()),
                (AmmoKind::Shell, AmmoKind::Shell.box_rounds()),
            ]
        );

        let mut rng = StdRng::seed_from_u64(9);
        let crawler_brain = brain(MonsterKind::Crawler, &mut rng);
        let items = drops(
            Monster {
                kind: MonsterKind::Crawler,
                elite: false,
            },
            &crawler_brain,
            &mut rng,
        );
        assert_eq!(
            ammo_kinds(&items),
            vec![
                (AmmoKind::Cell, AmmoKind::Cell.box_rounds()),
                (AmmoKind::Bullet, AmmoKind::Bullet.box_rounds()),
            ]
        );
    }

    #[test]
    fn elites_and_raiders_do_not_fan_out_ammo() {
        let mut rng = StdRng::seed_from_u64(3);
        let elite_brain = brain(MonsterKind::Crawler, &mut rng);
        let elite = Monster {
            kind: MonsterKind::Crawler,
            elite: true,
        };
        for _ in 0..20 {
            let items = drops(elite, &elite_brain, &mut rng);
            assert_eq!(items.len(), 2);
            assert_eq!(items[1], Item::Medkit);
            assert!(ammo_kinds(&items).len() <= 1);
        }

        let mut rng = StdRng::seed_from_u64(4);
        let raider_brain = brain(MonsterKind::Raider, &mut rng);
        let raider = Monster {
            kind: MonsterKind::Raider,
            elite: false,
        };
        let gun = Item::weapon(raider_brain.weapon);
        let Item::Ammo { kind, rounds } = gun.matching_ammo().expect("raider gun has ammo") else {
            panic!("matching ammo");
        };
        for _ in 0..20 {
            let items = drops(raider, &raider_brain, &mut rng);
            assert!(items.contains(&gun));
            assert_eq!(ammo_kinds(&items), vec![(kind, rounds)]);
        }
    }
}

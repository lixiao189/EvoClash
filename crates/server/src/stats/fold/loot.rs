use evoclash_game::items::Item;
use evoclash_game::map::{Biome, VAULT_CENTER};
use evoclash_game::protocol::*;
use evoclash_game::shop::{VAULT_SAFE_SCRAP, crate_scrap, safe_scrap};
use evoclash_game::sim::Arena;

use super::*;

impl Fold {
    pub(super) fn diff_props(&mut self, props: &HashMap<Entity, PropSnap>, bodies: &[Sample]) {
        let mut opened_safes: Vec<Vec2> = Vec::new();
        for (e, prev) in &self.props {
            let now = props.get(e);
            let opened = match prev.kind {
                PropKind::Crate => now.is_none(),
                PropKind::Safe => now.is_some_and(|n| n.open && !prev.open),
                PropKind::Barrel => false,
            };
            if !opened {
                continue;
            }
            if prev.kind == PropKind::Safe {
                opened_safes.push(prev.pos);
                self.safes_cracked += 1;
            }
            self.scrap_container += match prev.kind {
                PropKind::Crate => crate_scrap(prev.tier),
                PropKind::Safe if prev.vault => VAULT_SAFE_SCRAP,
                PropKind::Safe => safe_scrap(prev.tier),
                PropKind::Barrel => 0,
            };
            if let Some(id) = nearest(bodies, prev.pos)
                && let Some(life) = self.open_life.get_mut(&id)
            {
                life.opens += 1;
            }
        }
        self.tag_safes = opened_safes;
    }

    pub(super) fn diff_picks(
        &mut self,
        stamp: u32,
        picks: HashMap<Entity, Ground>,
        bodies: &[Sample],
    ) {
        let mut next = HashMap::new();
        for (e, g) in &picks {
            if let Some(prev) = self.picks.get(e) {
                next.insert(
                    *e,
                    Ground {
                        source: prev.source,
                        ..*g
                    },
                );
            } else {
                let source = self.fresh_source(g.pos, stamp);
                next.insert(*e, Ground { source, ..*g });
            }
        }
        for (e, prev) in &self.picks {
            if picks.contains_key(e) {
                continue;
            }
            if prev.source == 0 {
                continue;
            }
            let Some(weapon) = prev.weapon else {
                continue;
            };
            if let Some(id) = bodies
                .iter()
                .filter(|b| b.weapon == weapon && b.pos.distance(prev.pos) < 120.0)
                .min_by(|a, b| a.id.cmp(&b.id))
                .map(|b| b.id)
                && let Some(body) = self.roster.get_mut(&id)
            {
                body.sourced = true;
            }
        }
        // Safes that opened this tick tag brand new pickups beside them.
        for g in next.values_mut() {
            if g.source == 0 && self.tag_safes.iter().any(|p| p.distance(g.pos) < 90.0) {
                g.source = SRC_SAFE;
            }
        }
        self.picks = next;
        self.tag_safes.clear();
    }

    fn fresh_source(&self, pos: Vec2, stamp: u32) -> u8 {
        if self.last_vault == Some(stamp) && pos.distance(VAULT_CENTER) < 200.0 {
            return SRC_VAULT;
        }
        if self.boss_boom.is_some_and(|p| p.distance(pos) < 160.0) {
            return SRC_BOSS;
        }
        0
    }
}

pub(super) fn read_props(world: &mut World) -> HashMap<Entity, PropSnap> {
    let mut q = world.query::<(Entity, &Prop, &Pos)>();
    let rows: Vec<(Entity, Prop, Vec2)> = q
        .iter(world)
        .map(|(e, prop, pos)| (e, *prop, pos.0))
        .collect();
    let mut out = HashMap::new();
    for (e, prop, pos) in rows {
        let vault = world
            .resource::<Arena>()
            .block_at(pos)
            .is_some_and(|b| b.biome == Biome::Vault);
        out.insert(
            e,
            PropSnap {
                kind: prop.kind,
                open: prop.open,
                tier: prop.tier,
                pos,
                vault,
            },
        );
    }
    out
}

pub(super) fn read_picks(world: &mut World) -> HashMap<Entity, Ground> {
    let mut q = world.query::<(Entity, &Pickup, &Pos)>();
    q.iter(world)
        .map(|(e, pick, pos)| {
            let weapon = match pick.item {
                Item::Weapon(w) => Some(w),
                _ => None,
            };
            (
                e,
                Ground {
                    source: 0,
                    weapon,
                    pos: pos.0,
                },
            )
        })
        .collect()
}

fn nearest(bodies: &[Sample], pos: Vec2) -> Option<u64> {
    bodies
        .iter()
        .filter(|b| b.alive)
        .min_by(|a, b| {
            a.pos
                .distance_squared(pos)
                .total_cmp(&b.pos.distance_squared(pos))
                .then(a.id.cmp(&b.id))
        })
        .map(|b| b.id)
}

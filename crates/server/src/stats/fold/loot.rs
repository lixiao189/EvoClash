use evoclash_game::items::{Item, WeaponKind};
use evoclash_game::map::{Biome, VAULT_CENTER};
use evoclash_game::protocol::*;
use evoclash_game::shop::{VAULT_SAFE_SCRAP, crate_scrap, safe_scrap};
use evoclash_game::sim::Arena;

use crate::inventory::BagContents;

use super::*;

/// How far a weapon can move in one tick and still be the same gun (pickup, put-down, death bag).
const TRACE_R2: f32 = 120.0 * 120.0;

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

    /// Tag vault / behemoth / safe guns at spawn and follow that gun through hands, the
    /// backpack, a drop and the next combatant who picks it up. The tag is not the player's.
    pub(super) fn diff_picks(
        &mut self,
        stamp: u32,
        picks: HashMap<Entity, Ground>,
        bodies: &[Sample],
    ) {
        let old_picks = std::mem::take(&mut self.picks);
        let old_carry = std::mem::take(&mut self.carry);
        let mut tokens = Vec::new();
        let mut holes = Vec::new();
        let mut next = HashMap::new();
        for (e, raw) in &picks {
            let ground = if let Some(prev) = old_picks.get(e) {
                keep_ground(*e, prev, raw, &mut tokens, &mut holes)
            } else {
                spawn_ground(*e, raw, &mut holes)
            };
            next.insert(*e, ground);
        }
        for (e, prev) in &old_picks {
            if !picks.contains_key(e) {
                release_ground(prev, &mut tokens);
            }
        }
        let mut carry = BTreeMap::new();
        for b in bodies {
            carry.insert(
                b.id,
                trace_player(b, old_carry.get(&b.id), &mut tokens, &mut holes),
            );
        }
        self.assign_sources(stamp, &tokens, &holes, &mut next, &mut carry);
        self.picks = next;
        self.carry = carry;
        self.tag_safes.clear();
    }

    fn assign_sources(
        &self,
        stamp: u32,
        tokens: &[Tok],
        holes: &[Hol],
        next: &mut HashMap<Entity, Ground>,
        carry: &mut BTreeMap<u64, Carry>,
    ) {
        let mut pairs = Vec::new();
        for (ti, tok) in tokens.iter().enumerate() {
            for (hi, hol) in holes.iter().enumerate() {
                if tok.kind != hol.kind || same_slot(tok.from, hol.spot) {
                    continue;
                }
                let d2 = tok.pos.distance_squared(hol.pos);
                if d2 < TRACE_R2 {
                    pairs.push((d2, ti, hi));
                }
            }
        }
        pairs.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        let mut used_t = vec![false; tokens.len()];
        let mut used_h = vec![false; holes.len()];
        for (_, ti, hi) in pairs {
            if used_t[ti] || used_h[hi] {
                continue;
            }
            used_t[ti] = true;
            used_h[hi] = true;
            write_source(holes[hi].spot, tokens[ti].source, next, carry);
        }
        for (hi, hol) in holes.iter().enumerate() {
            if used_h[hi] || hol.kind == WeaponKind::Stick {
                continue;
            }
            let src = match hol.spot {
                Spot::Ground(_) => self.event_source(hol.pos, stamp, 90.0),
                Spot::Slot { .. } => self.event_source(hol.pos, stamp, 120.0),
                Spot::Bag { .. } => 0,
            };
            if src != 0 {
                write_source(hol.spot, src, next, carry);
            }
        }
    }

    /// Where a brand-new gun was generated this tick. `safe_r` is larger for a combatant who
    /// already picked it up than for the pickup still lying there.
    fn event_source(&self, pos: Vec2, stamp: u32, safe_r: f32) -> u8 {
        if self.last_vault == Some(stamp) && pos.distance_squared(VAULT_CENTER) < 200.0 * 200.0 {
            return SRC_VAULT;
        }
        if self
            .boss_boom
            .is_some_and(|p| pos.distance_squared(p) < 160.0 * 160.0)
        {
            return SRC_BOSS;
        }
        if self
            .tag_safes
            .iter()
            .any(|p| pos.distance_squared(*p) < safe_r * safe_r)
        {
            return SRC_SAFE;
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
    let mut q = world.query::<(Entity, &Pickup, &Pos, Option<&BagContents>)>();
    q.iter(world)
        .map(|(e, pick, pos, bag)| {
            let weapon = match pick.item {
                Item::Weapon(w) => Some(w),
                _ => None,
            };
            let bag = bag
                .map(|contents| {
                    contents
                        .0
                        .iter()
                        .filter_map(|item| match item {
                            Item::Weapon(w) => Some((*w, 0)),
                            _ => None,
                        })
                        .collect()
                })
                .unwrap_or_default();
            (
                e,
                Ground {
                    source: 0,
                    weapon,
                    pos: pos.0,
                    bag,
                },
            )
        })
        .collect()
}

/// A gun that left a slot or the ground this tick.
#[derive(Clone, Copy)]
struct Tok {
    kind: WeaponKind,
    source: u8,
    pos: Vec2,
    /// Player slot it left, so a swap doesn't match a gun to the slot it just vacated.
    from: Option<(u64, usize)>,
}

#[derive(Clone, Copy)]
enum Spot {
    Slot { id: u64, slot: usize },
    Ground(Entity),
    Bag { entity: Entity, index: usize },
}

#[derive(Clone, Copy)]
struct Hol {
    kind: WeaponKind,
    pos: Vec2,
    spot: Spot,
}

fn same_slot(from: Option<(u64, usize)>, spot: Spot) -> bool {
    match (from, spot) {
        (Some((pid, ps)), Spot::Slot { id, slot }) => pid == id && ps == slot,
        _ => false,
    }
}

fn keep_ground(
    entity: Entity,
    prev: &Ground,
    raw: &Ground,
    tokens: &mut Vec<Tok>,
    holes: &mut Vec<Hol>,
) -> Ground {
    // Same entity, same single weapon: the tag stays. Bags are diffed as contents.
    if prev.weapon == raw.weapon && prev.bag.is_empty() && raw.bag.is_empty() {
        return Ground {
            source: prev.source,
            weapon: raw.weapon,
            pos: raw.pos,
            bag: Vec::new(),
        };
    }
    release_ground(prev, tokens);
    spawn_ground(entity, raw, holes)
}

fn spawn_ground(entity: Entity, raw: &Ground, holes: &mut Vec<Hol>) -> Ground {
    if let Some(kind) = raw.weapon {
        holes.push(Hol {
            kind,
            pos: raw.pos,
            spot: Spot::Ground(entity),
        });
        return Ground {
            source: 0,
            weapon: Some(kind),
            pos: raw.pos,
            bag: Vec::new(),
        };
    }
    let mut bag = Vec::with_capacity(raw.bag.len());
    for (index, (kind, _)) in raw.bag.iter().enumerate() {
        bag.push((*kind, 0));
        holes.push(Hol {
            kind: *kind,
            pos: raw.pos,
            spot: Spot::Bag { entity, index },
        });
    }
    Ground {
        source: 0,
        weapon: None,
        pos: raw.pos,
        bag,
    }
}

fn release_ground(prev: &Ground, tokens: &mut Vec<Tok>) {
    if let Some(kind) = prev.weapon {
        tokens.push(Tok {
            kind,
            source: prev.source,
            pos: prev.pos,
            from: None,
        });
    }
    for (kind, source) in &prev.bag {
        tokens.push(Tok {
            kind: *kind,
            source: *source,
            pos: prev.pos,
            from: None,
        });
    }
}

fn trace_player(
    body: &Sample,
    old: Option<&Carry>,
    tokens: &mut Vec<Tok>,
    holes: &mut Vec<Hol>,
) -> Carry {
    let mut src = [0u8; CARRY];
    for (i, (src_slot, slot)) in src.iter_mut().zip(body.slots).enumerate() {
        let prev_kind = old.and_then(|c| c.kind[i]);
        let prev_src = old.map(|c| c.src[i]).unwrap_or(0);
        if prev_kind == slot {
            *src_slot = prev_src;
            continue;
        }
        if let Some(kind) = prev_kind {
            tokens.push(Tok {
                kind,
                source: prev_src,
                pos: body.pos,
                from: Some((body.id, i)),
            });
        }
        if let Some(kind) = slot {
            holes.push(Hol {
                kind,
                pos: body.pos,
                spot: Spot::Slot {
                    id: body.id,
                    slot: i,
                },
            });
        }
    }
    Carry {
        kind: body.slots,
        src,
    }
}

fn write_source(
    spot: Spot,
    source: u8,
    next: &mut HashMap<Entity, Ground>,
    carry: &mut BTreeMap<u64, Carry>,
) {
    match spot {
        Spot::Slot { id, slot } => {
            if let Some(c) = carry.get_mut(&id) {
                c.src[slot] = source;
            }
        }
        Spot::Ground(entity) => {
            if let Some(g) = next.get_mut(&entity) {
                g.source = source;
            }
        }
        Spot::Bag { entity, index } => {
            if let Some(g) = next.get_mut(&entity)
                && let Some(item) = g.bag.get_mut(index)
            {
                item.1 = source;
            }
        }
    }
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

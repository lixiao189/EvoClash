//! Ground loot, crates and safes, picked by distance times how much we want them.

use evoclash_game::map::{ArenaMap, PropKind};
use evoclash_game::sim::PLAYER_RADIUS;

use super::*;

/// Extra reach `environment` uses when cracking. Kept here so a bot stands where a tick counts.
const SAFE_REACH: f32 = 40.0;
/// A player or a dangerous monster this close, and in sight, is reason to stop cracking.
pub(super) const SAFE_THREAT: f32 = 350.0;
/// Blast radius is 150; stand off a bit further.
pub(super) const BARREL_KEEP: f32 = 170.0;
/// Only this tier (or a safe) is worth walking into radiation without power armor.
pub(super) const RAD_LOOT: u8 = 3;

pub(super) fn safe_stand() -> f32 {
    PropKind::Safe.radius() + PLAYER_RADIUS + SAFE_REACH
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Act {
    Walk,
    Smash,
    Crack,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Desire {
    pub pos: Vec2,
    pub score: f32,
    pub act: Act,
    pub entity: Option<Entity>,
    pub safe: bool,
}

pub(super) struct GroundLoot {
    pub pos: Vec2,
    pub weight: f32,
    pub medkit: bool,
}

pub(super) struct Container {
    pub entity: Entity,
    pub pos: Vec2,
    pub kind: PropKind,
    pub tier: u8,
}

/// Lower is keener, same scale as pickup interest. Barrels are never worth it.
pub(super) fn container_weight(kind: PropKind, tier: u8, our_tier: u8) -> Option<f32> {
    match kind {
        PropKind::Crate => Some(if tier > our_tier { 0.55 } else { 0.85 }),
        PropKind::Safe => Some(0.5),
        PropKind::Barrel => None,
    }
}

fn pos_key(p: Vec2) -> (u32, u32) {
    (p.x.to_bits(), p.y.to_bits())
}

fn desire_cmp(a: &Desire, b: &Desire) -> std::cmp::Ordering {
    a.score
        .total_cmp(&b.score)
        .then(pos_key(a.pos).cmp(&pos_key(b.pos)))
}

pub(super) fn candidates(
    at: Vec2,
    our_tier: u8,
    smash_range: f32,
    clear_for_safe: bool,
    hurt: bool,
    sticky: Option<Entity>,
    ground: &[GroundLoot],
    containers: &[Container],
) -> Vec<Desire> {
    let mut out = Vec::new();
    let medkits: Vec<&GroundLoot> = ground.iter().filter(|g| g.medkit).collect();
    let ground = if hurt && !medkits.is_empty() {
        medkits
    } else {
        ground.iter().collect()
    };
    for g in ground {
        let weight = if hurt && g.medkit {
            g.weight * 0.5
        } else {
            g.weight
        };
        out.push(Desire {
            pos: g.pos,
            score: at.distance(g.pos) * weight,
            act: Act::Walk,
            entity: None,
            safe: false,
        });
    }
    for c in containers {
        let Some(mut weight) = container_weight(c.kind, c.tier, our_tier) else {
            continue;
        };
        if c.kind == PropKind::Safe && !clear_for_safe {
            continue;
        }
        if sticky == Some(c.entity) {
            weight *= 0.45;
        }
        let dist = at.distance(c.pos);
        let close = match c.kind {
            PropKind::Crate => smash_range,
            PropKind::Safe => safe_stand(),
            PropKind::Barrel => continue,
        };
        let act = if dist <= close {
            match c.kind {
                PropKind::Crate => Act::Smash,
                PropKind::Safe => Act::Crack,
                PropKind::Barrel => Act::Walk,
            }
        } else {
            Act::Walk
        };
        out.push(Desire {
            pos: c.pos,
            score: dist * weight,
            act,
            entity: Some(c.entity),
            safe: c.kind == PropKind::Safe,
        });
    }
    out
}

/// Prefer a spot the threat cannot see, so backing off also breaks the line.
pub(super) fn choose(
    desires: &[Desire],
    hide_from: Option<Vec2>,
    hidden: impl Fn(Vec2) -> bool,
) -> Option<Desire> {
    let pick = |pool: &[&Desire]| {
        pool.iter()
            .copied()
            .min_by(|a, b| desire_cmp(a, b))
            .copied()
    };
    if let Some(enemy) = hide_from {
        let unseen: Vec<&Desire> = desires
            .iter()
            .filter(|d| hidden(d.pos) && d.pos.distance(enemy) > safe_stand())
            .collect();
        if let Some(best) = pick(&unseen) {
            return Some(best);
        }
    }
    let all: Vec<&Desire> = desires.iter().collect();
    pick(&all)
}

/// A new target must score at least this much better (25% keener) to replace the one the bot
/// is already walking to. Without it two equally good pickups trade places every think.
pub(super) const SWITCH_MARGIN: f32 = 0.75;

fn same_target(a: &Desire, b: &Desire) -> bool {
    a.entity == b.entity && pos_key(a.pos) == pos_key(b.pos)
}

/// [`choose`] with stickiness: keep walking to `held` while it is still there and nothing
/// else is clearly better. A target already reached scores near zero, so it is always kept.
pub(super) fn hold_pick(
    desires: &[Desire],
    held: Option<Desire>,
    pick: impl Fn(&[Desire]) -> Option<Desire>,
) -> Option<Desire> {
    let fresh = held.and_then(|h| desires.iter().copied().find(|d| same_target(d, &h)));
    let best = pick(desires)?;
    match fresh {
        Some(fresh) if !same_target(&best, &fresh) && best.score < fresh.score * SWITCH_MARGIN => {
            Some(best)
        }
        Some(fresh) => Some(fresh),
        None => Some(best),
    }
}

pub(super) fn retreat_point(at: Vec2, enemy: Vec2) -> Vec2 {
    let away = (at - enemy).normalize_or(Vec2::Y);
    let bound = ARENA_HALF * 0.9;
    (at + away * 380.0).clamp(-bound, bound)
}

pub(super) struct Spec<'a> {
    pub at: Vec2,
    pub our_tier: u8,
    pub smash_range: f32,
    pub allow_safe: bool,
    pub hurt: bool,
    pub sticky: Option<Entity>,
    pub sealed: bool,
    /// True when `p` is inside radiation.
    pub hazard: &'a dyn Fn(Vec2) -> bool,
    pub barrels: &'a [Vec2],
}

fn bag_of<'a>(bag: &'a Option<&'a BagContents>) -> Option<&'a BagContents> {
    bag.as_ref().copied()
}

fn worth_rad(item: &Item, bag: Option<&BagContents>) -> bool {
    if let Some(bag) = bag {
        return bag.0.iter().any(|i| i.tier() >= RAD_LOOT);
    }
    item.tier() >= RAD_LOOT
}

fn may_enter(spec: &Spec<'_>, pos: Vec2, worth: bool) -> bool {
    spec.sealed || !(spec.hazard)(pos) || worth
}

/// Ground loot, crates and safes inside sight. Radiation only for a safe or tier 3+.
/// Crates with a barrel inside the blast margin are skipped.
pub(super) fn gather(
    spec: &Spec<'_>,
    items: &Query<(&Pos, &Pickup, Option<&BagContents>)>,
    props: &Query<(Entity, &Prop, &Pos, &Vitals)>,
    weight: impl Fn(&Item, Option<&BagContents>) -> Option<f32>,
) -> Vec<Desire> {
    let ground: Vec<GroundLoot> = items
        .iter()
        .filter(|(p, pickup, bag)| {
            p.0.distance(spec.at) < SIGHT * 1.3
                && may_enter(spec, p.0, worth_rad(&pickup.item, bag_of(bag)))
        })
        .filter_map(|(p, pickup, bag)| {
            weight(&pickup.item, bag).map(|w| GroundLoot {
                pos: p.0,
                weight: w,
                medkit: match pickup.item {
                    Item::Medkit => true,
                    Item::Bag => bag.is_some_and(|b| b.0.iter().any(|i| matches!(i, Item::Medkit))),
                    _ => false,
                },
            })
        })
        .collect();
    let containers: Vec<Container> = props
        .iter()
        .filter(|(_, prop, p, v)| {
            if !v.alive || prop.open || p.0.distance(spec.at) >= SIGHT * 1.3 {
                return false;
            }
            let worth = match prop.kind {
                PropKind::Safe => true,
                PropKind::Crate => prop.tier >= RAD_LOOT,
                PropKind::Barrel => return false,
            };
            if !may_enter(spec, p.0, worth) {
                return false;
            }
            if prop.kind == PropKind::Crate && near_barrel(p.0, spec.barrels) {
                return false;
            }
            true
        })
        .map(|(e, prop, p, _)| Container {
            entity: e,
            pos: p.0,
            kind: prop.kind,
            tier: prop.tier,
        })
        .collect();
    candidates(
        spec.at,
        spec.our_tier,
        spec.smash_range,
        spec.allow_safe,
        spec.hurt,
        spec.sticky,
        &ground,
        &containers,
    )
}

pub(super) fn near_barrel(at: Vec2, barrels: &[Vec2]) -> bool {
    barrels.iter().any(|b| b.distance(at) < BARREL_KEEP)
}

/// Nearest live barrel inside the keep-out, if the bot is standing in it.
pub(super) fn hot_barrel(at: Vec2, barrels: &[Vec2]) -> Option<Vec2> {
    barrels
        .iter()
        .filter(|b| at.distance(**b) < BARREL_KEEP)
        .min_by(|a, b| at.distance(**a).total_cmp(&at.distance(**b)))
        .copied()
}

/// A shot from `from` to `to` would pass through a barrel.
pub(super) fn shot_hits_barrel(from: Vec2, to: Vec2, barrels: &[Vec2]) -> bool {
    let radius = PropKind::Barrel.radius();
    barrels.iter().any(|c| ray_hits(*c, from, to, radius))
}

fn ray_hits(center: Vec2, a: Vec2, b: Vec2, radius: f32) -> bool {
    let d = b - a;
    let f = a - center;
    let c = f.length_squared() - radius * radius;
    if c <= 0.0 {
        return true;
    }
    let a_coef = d.length_squared();
    if a_coef < 1e-6 {
        return false;
    }
    let b_coef = 2.0 * f.dot(d);
    let disc = b_coef * b_coef - 4.0 * a_coef * c;
    if disc < 0.0 {
        return false;
    }
    let t = (-b_coef - disc.sqrt()) / (2.0 * a_coef);
    (0.0..=1.0).contains(&t)
}

/// Walk a chosen desire, or roam when there is nothing. Returns the safe we're committed to.
pub(super) fn follow(
    input: &mut PlayerInput,
    nav: &NavGrid,
    arena: &Arena,
    obstacles: &[(Vec2, f32)],
    at: Vec2,
    desire: Option<Desire>,
    hide: Option<Vec2>,
    stuck: u16,
    goal: &mut Vec2,
    safe_spot: impl Fn(Vec2) -> bool,
    heading: &mut (i8, i8),
    rng: &mut impl Rng,
) -> (Option<Entity>, u16) {
    let mut stuck = stuck;
    if let Some(desire) = desire {
        let crack = desire.safe.then(|| desire.entity.expect("safe"));
        let to_goal = desire.pos - at;
        input.aim = to_goal.normalize_or(Vec2::X);
        match desire.act {
            Act::Crack => {
                input.move_x = 0;
                input.move_y = 0;
                input.interact = true;
            }
            Act::Smash if stuck < 6 => {
                input.move_x = 0;
                input.move_y = 0;
                input.fire = true;
            }
            Act::Smash | Act::Walk => {
                input.fire = desire.act == Act::Smash;
                let dir = if to_goal.length() < 180.0 {
                    to_goal
                } else {
                    next_waypoint(nav, arena, at, desire.pos, PLAYER_RADIUS) - at
                };
                let dir = match hide {
                    Some(enemy) => depart_dir(enemy - at, dir),
                    None => dir,
                };
                super::think::steer(input, dir, arena, obstacles, at, heading);
            }
        }
        return (crack, stuck);
    }
    if stuck >= 12 || goal.distance(at) < 40.0 || !safe_spot(*goal) || rng.random_bool(0.01) {
        let bound = ARENA_HALF * 0.9;
        if let Some(enemy) = hide {
            *goal = retreat_point(at, enemy);
        }
        if hide.is_none() || !safe_spot(*goal) {
            for _ in 0..8 {
                *goal = Vec2::new(
                    rng.random_range(-bound.x..bound.x),
                    rng.random_range(-bound.y..bound.y),
                );
                if safe_spot(*goal) {
                    break;
                }
            }
        }
        stuck = 0;
    }
    let dir = next_waypoint(nav, arena, at, *goal, PLAYER_RADIUS) - at;
    let dir = match hide {
        Some(enemy) => depart_dir(enemy - at, dir),
        None => dir,
    };
    super::think::steer(input, dir, arena, obstacles, at, heading);
    input.aim = dir.normalize_or(Vec2::X);
    (None, stuck)
}

/// A nearby corner the shooter cannot see. Straight backing keeps their line.
pub(super) fn cover_point(map: &ArenaMap, at: Vec2, enemy: Vec2) -> Option<Vec2> {
    let mut best: Option<(u32, u32, f32, Vec2)> = None;
    let mut consider = |p: Vec2| {
        let dist = p.distance(at);
        if !(32.0..420.0).contains(&dist) || map.in_hazard(p) || map.line_of_sight(enemy, p) {
            return;
        }
        let key = (p.x.to_bits(), p.y.to_bits(), dist);
        let better = best.is_none_or(|(_, _, bd, bp)| {
            dist.total_cmp(&bd)
                .then((key.0, key.1).cmp(&(bp.x.to_bits(), bp.y.to_bits())))
                .is_lt()
        });
        if better {
            best = Some((key.0, key.1, dist, p));
        }
    };
    for wall in &map.walls {
        if wall.max.x < at.x - 420.0
            || wall.min.x > at.x + 420.0
            || wall.max.y < at.y - 420.0
            || wall.min.y > at.y + 420.0
        {
            continue;
        }
        let pad = 36.0;
        for p in [
            Vec2::new(wall.min.x - pad, wall.min.y - pad),
            Vec2::new(wall.max.x + pad, wall.min.y - pad),
            Vec2::new(wall.min.x - pad, wall.max.y + pad),
            Vec2::new(wall.max.x + pad, wall.max.y + pad),
        ] {
            consider(p);
        }
    }
    best.map(|(_, _, _, p)| p)
}

/// A step just outside the zone `at` is standing in, if any.
pub(super) fn hazard_exit(zones: &[Rect], at: Vec2) -> Option<Vec2> {
    let mut p = at;
    let mut left = false;
    for _ in 0..4 {
        let Some(zone) = zones.iter().find(|z| z.contains(p)) else {
            break;
        };
        p = push_out(zone, p);
        left = true;
    }
    left.then_some(p)
}

fn push_out(zone: &Rect, at: Vec2) -> Vec2 {
    let pad = 28.0;
    let left = at.x - zone.min.x;
    let right = zone.max.x - at.x;
    let down = at.y - zone.min.y;
    let up = zone.max.y - at.y;
    if left <= right && left <= down && left <= up {
        Vec2::new(zone.min.x - pad, at.y)
    } else if right <= down && right <= up {
        Vec2::new(zone.max.x + pad, at.y)
    } else if down <= up {
        Vec2::new(at.x, zone.min.y - pad)
    } else {
        Vec2::new(at.x, zone.max.y + pad)
    }
}

/// Away from `to_enemy`, unless the goal is already behind us.
pub(super) fn depart_dir(to_enemy: Vec2, to_goal: Vec2) -> Vec2 {
    let away = -to_enemy.normalize_or_zero();
    let goal = to_goal.normalize_or_zero();
    if goal == Vec2::ZERO {
        return away;
    }
    if goal.dot(away) < 0.0 {
        away + away.perp()
    } else {
        away * 0.6 + goal
    }
}

//! Three seconds of each bot, sampled every 16 ticks. Headless only.

use bevy::prelude::*;
use evoclash_game::map::PropKind;
use evoclash_game::protocol::{CRAWLER_SHELL_COS, Monster, MonsterKind, MonsterState};

use crate::bots::Bot;

use super::*;

/// One sample of a bot, every 16 ticks, for the 3 seconds before a death.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct TraceSnap {
    pub tick: u32,
    /// 0 scavenge, 1 avoid, 2 retreat, 3 fight.
    pub stance: u8,
    pub target: String,
    pub dist: u16,
    pub hp: u16,
    pub armor: u16,
    pub weapon: String,
    /// `kind:count` pairs inside 350px, sorted.
    pub threats: String,
    pub threat_n: u8,
    /// Standing in a crawler's front arc.
    pub crawler_front: bool,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct Autopsy {
    pub tick: u32,
    pub how: String,
    pub snaps: Vec<TraceSnap>,
}

const EVERY: u32 = 16;
const KEEP: usize = 3 * 64 / EVERY as usize;
const THREAT: f32 = 350.0;
pub(super) const AUTOPSY_CAP: usize = 40;

impl Fold {
    pub(super) fn sample_bots(&mut self, world: &mut World, stamp: u32) {
        if !stamp.is_multiple_of(EVERY) {
            return;
        }
        let mut bots = world.query::<(&Player, &Bot, &Pos, &Vitals, &Loadout)>();
        let rows: Vec<_> = bots
            .iter(world)
            .filter(|(_, _, _, v, _)| v.alive)
            .map(|(p, bot, pos, vitals, loadout)| {
                let (mind, focus, crack) = bot.trace();
                (
                    p.client_id,
                    mind,
                    focus,
                    crack,
                    pos.0,
                    vitals.hp,
                    vitals.armor_points,
                    loadout.weapon().spec().name().to_string(),
                )
            })
            .collect();
        let mut others =
            world.query::<(Entity, &Pos, &Vitals, Option<&Monster>, Option<&Player>)>();
        let crowd: Vec<_> = others
            .iter(world)
            .filter(|(_, _, v, ..)| v.alive)
            .map(|(e, p, _, monster, player)| (e, p.0, monster.copied(), player.is_some()))
            .collect();
        let mut facing = world.query::<(&Monster, &MonsterState, &Pos)>();
        let crawlers: Vec<_> = facing
            .iter(world)
            .filter(|(m, _, _)| m.kind == MonsterKind::Crawler)
            .map(|(m, state, p)| (p.0, state.facing, m.elite))
            .collect();
        let mut props = world.query::<(Entity, &Prop, &Pos)>();
        let props: Vec<_> = props
            .iter(world)
            .map(|(e, prop, p)| (e, prop.kind, p.0))
            .collect();
        for (id, mind, focus, crack, at, hp, armor, weapon) in rows {
            let (target, dist) = label_target(focus.or(crack), at, &crowd, &props);
            let (threats, threat_n) = threats_near(at, &crowd);
            let crawler_front = crawlers.iter().any(|(p, face, _)| {
                let to_bot = at - *p;
                p.distance(at) < THREAT
                    && to_bot != Vec2::ZERO
                    && to_bot.normalize().dot(Vec2::from_angle(*face)) >= CRAWLER_SHELL_COS
            });
            let ring = self.trace.entry(id).or_default();
            if ring.len() == KEEP {
                ring.pop_front();
            }
            ring.push_back(TraceSnap {
                tick: stamp,
                stance: mind,
                target,
                dist,
                hp: hp.round().clamp(0.0, 1000.0) as u16,
                armor: armor.round().clamp(0.0, 1000.0) as u16,
                weapon,
                threats,
                threat_n,
                crawler_front,
            });
        }
    }

    pub(super) fn take_autopsy(&mut self, id: u64, tick: u32, how: &str) {
        if self.autopsies.len() >= AUTOPSY_CAP {
            self.trace.remove(&id);
            return;
        }
        let Some(snaps) = self.trace.remove(&id) else {
            return;
        };
        if snaps.is_empty() {
            return;
        }
        self.autopsies.push(Autopsy {
            tick,
            how: how.to_string(),
            snaps: snaps.into_iter().collect(),
        });
    }
}

fn label_target(
    focus: Option<Entity>,
    at: Vec2,
    crowd: &[(Entity, Vec2, Option<Monster>, bool)],
    props: &[(Entity, PropKind, Vec2)],
) -> (String, u16) {
    let Some(focus) = focus else {
        return ("none".into(), 0);
    };
    if let Some((_, p, monster, player)) = crowd.iter().find(|(e, ..)| *e == focus) {
        let name = if let Some(m) = monster {
            m.kind.name().to_string()
        } else if *player {
            "player".into()
        } else {
            "none".into()
        };
        return (name, p.distance(at).round() as u16);
    }
    if let Some((_, kind, p)) = props.iter().find(|(e, ..)| *e == focus) {
        return (kind.name().to_string(), p.distance(at).round() as u16);
    }
    ("none".into(), 0)
}

fn threats_near(at: Vec2, crowd: &[(Entity, Vec2, Option<Monster>, bool)]) -> (String, u8) {
    let mut counts: Vec<(&str, u8)> = Vec::new();
    let mut n = 0u8;
    for (_, p, monster, player) in crowd {
        if p.distance(at) > THREAT {
            continue;
        }
        let name = if let Some(m) = monster {
            m.kind.name()
        } else if *player {
            "player"
        } else {
            continue;
        };
        // The bot itself is a player at distance 0. Callers pass only others... we didn't.
        // Distance 0 to self is filtered by the query including self. Skip zero.
        if p.distance(at) < 1.0 {
            continue;
        }
        n = n.saturating_add(1);
        if let Some(slot) = counts.iter_mut().find(|(k, _)| *k == name) {
            slot.1 = slot.1.saturating_add(1);
        } else {
            counts.push((name, 1));
        }
    }
    counts.sort_by_key(|(k, _)| *k);
    let text = counts
        .iter()
        .map(|(k, n)| format!("{k}:{n}"))
        .collect::<Vec<_>>()
        .join(",");
    (text, n)
}

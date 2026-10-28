//! Fight, back off, or leave. Pure so the calls can be tested without a world.

use bevy::prelude::{Entity, Vec2};
use evoclash_game::protocol::MonsterKind;

use crate::gameplay::TICKS_PER_SEC;

/// Playing open: scavenge instead of hunting players.
pub(super) const SCAVENGE_OPEN_TICKS: u32 = 60 * TICKS_PER_SEC;
/// A fresh life with only a stick keeps scavenging this long.
pub(super) const SCAVENGE_LIFE_TICKS: u32 = 15 * TICKS_PER_SEC;
/// During that window a player inside this range is close enough to deal with.
pub(super) const NEAR_PLAYER: f32 = 200.0;
/// Close enough that walking away is no longer a real option.
pub(super) const CAUGHT: f32 = 140.0;
/// Below this fraction of max hp, and without the upper hand, leave.
pub(super) const HURT_FRAC: f32 = 0.35;

#[derive(Clone, Copy, Debug)]
pub(super) struct Side {
    pub tier: u8,
    pub hp: f32,
    pub armor: f32,
}

pub(super) fn power(side: Side) -> f32 {
    (side.tier as f32 + 1.0) * (side.hp + side.armor).max(1.0)
}

/// `me` over `them`. Above 1 is an edge.
pub(super) fn power_ratio(me: Side, them: Side) -> f32 {
    power(me) / power(them)
}

pub(super) fn has_advantage(me: Side, them: Side) -> bool {
    power_ratio(me, them) > 1.0
}

/// At least one weapon tier behind, and hp + armor is not ahead.
/// Aggressive bots (1.15 and up) only count a gap of two tiers.
pub(super) fn clearly_worse(me: Side, them: Side, aggression: f32) -> bool {
    let gap = them.tier as i16 - me.tier as i16;
    let min_gap = if aggression >= 1.15 { 2 } else { 1 };
    gap >= min_gap && me.hp + me.armor <= them.hp + them.armor
}

fn clamp_agg(aggression: f32) -> f32 {
    aggression.clamp(0.7, 1.3)
}

/// Timid bots keep the window; aggressive ones shorten it.
pub(super) fn scaled_window(base: u32, aggression: f32) -> u32 {
    (base as f32 / clamp_agg(aggression)) as u32
}

pub(super) fn scavenging_now(
    playing: bool,
    match_ticks: u32,
    life_ticks: u32,
    stick: bool,
    aggression: f32,
) -> bool {
    if !playing {
        return false;
    }
    match_ticks < scaled_window(SCAVENGE_OPEN_TICKS, aggression)
        || (stick && life_ticks < scaled_window(SCAVENGE_LIFE_TICKS, aggression))
}

pub(super) fn near_player(aggression: f32) -> f32 {
    NEAR_PLAYER * clamp_agg(aggression)
}

pub(super) fn caught_dist(aggression: f32) -> f32 {
    CAUGHT * clamp_agg(aggression)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Stance {
    Fight,
    Avoid,
    Retreat,
}

/// Stored on the bot. `0` is idle scavenge, which does not lock.
pub(super) const MIND_AVOID: u8 = 1;
pub(super) const MIND_RETREAT: u8 = 2;
pub(super) const MIND_FIGHT: u8 = 3;
/// Re-decide only after this. Damage in range, or a corner, can cut it short.
pub(super) const HOLD_TICKS: u32 = 2 * TICKS_PER_SEC;
/// Dodging a shooter: if this long passes and they can still see you, turn and fire.
pub(super) const LOS_GIVE_UP: u32 = 3 * TICKS_PER_SEC / 2;
/// Leaving Avoid for a fight takes a real edge, not a ratio that just crossed 1.
pub(super) const AVOID_EXIT_RATIO: f32 = 1.1;

/// Keep `current` until the hold expires. `interrupt` (cornered, or shot at close range,
/// or a dodge that never breaks sight) may return to Fight early.
pub(super) fn commit(
    current: u8,
    proposed: u8,
    until: u32,
    now: u32,
    interrupt: bool,
    ratio: f32,
) -> u8 {
    if interrupt && matches!(current, MIND_AVOID | MIND_RETREAT) {
        return MIND_FIGHT;
    }
    if current != 0 && now < until {
        return current;
    }
    if current == MIND_AVOID && proposed == MIND_FIGHT && ratio < AVOID_EXIT_RATIO {
        return MIND_AVOID;
    }
    proposed
}

/// Roaches, dogs, ghouls and crawlers bite or blow up. Retreat is for guns and players.
pub(super) fn melee_fauna(kind: MonsterKind) -> bool {
    matches!(
        kind,
        MonsterKind::Roach | MonsterKind::Dog | MonsterKind::Ghoul | MonsterKind::Crawler
    )
}

pub(super) fn ranged_fauna(kind: MonsterKind) -> bool {
    matches!(kind, MonsterKind::Raider | MonsterKind::Spitter)
}

pub(super) struct StanceInput {
    pub me: Side,
    pub them: Side,
    pub player: bool,
    pub dist: f32,
    pub aggressor: bool,
    pub dangerous: bool,
    /// A raider with a gun: same gear check as a player, but not the scavenge leash.
    pub armed: bool,
    pub stick: bool,
    pub aggression: f32,
    pub scavenging: bool,
    pub cornered: bool,
    pub hp_frac: f32,
    /// Close-range fauna. They get fought, not fled, or the turn-and-run is just a bite.
    pub melee: bool,
    pub ghoul: bool,
    pub crawler: bool,
}

pub(super) fn stance(s: &StanceInput) -> Stance {
    let caught = s.dist <= caught_dist(s.aggression);
    if s.melee {
        // A stick has to step back from the blast; a gun holds outside it.
        if s.ghoul && s.stick && !caught && !s.cornered {
            return Stance::Avoid;
        }
        // A crawler that has not closed is left alone. Inside the pinch, circle it.
        let bite = 30.0 + 22.0 + 4.0;
        if !(s.crawler && s.stick && s.dist > bite && !s.cornered) {
            return Stance::Fight;
        }
    }
    if s.hp_frac < HURT_FRAC && !has_advantage(s.me, s.them) && !s.cornered {
        return Stance::Retreat;
    }
    if s.player && s.scavenging && !s.aggressor && s.dist > near_player(s.aggression) && !s.cornered
    {
        return Stance::Avoid;
    }
    if s.player
        && clearly_worse(s.me, s.them, s.aggression)
        && !s.aggressor
        && !caught
        && !s.cornered
    {
        return Stance::Avoid;
    }
    if !s.player && s.stick && s.dangerous && !s.aggressor && !caught && !s.cornered {
        return Stance::Avoid;
    }
    if s.armed
        && clearly_worse(s.me, s.them, s.aggression)
        && !s.aggressor
        && !caught
        && !s.cornered
    {
        return Stance::Avoid;
    }
    Stance::Fight
}

/// Stick bots leave these alone until one is on top of them. Elites of any kind count.
/// Raiders shoot, so a stick treats them the same way.
pub(super) fn dangerous_monster(kind: MonsterKind, elite: bool) -> bool {
    elite
        || matches!(
            kind,
            MonsterKind::Ghoul | MonsterKind::Spitter | MonsterKind::Crawler | MonsterKind::Raider
        )
}

/// What a raider is worth in the gear check. Their gun is a rifle more often than a shotgun;
/// the brain that rolls it is not visible from here.
pub(super) const RAIDER_TIER: u8 = 2;

/// Below this fraction of max hp, walk out of radiation even mid-swing.
pub(super) const RAD_LEAVE: f32 = 0.60;

pub(super) struct Seen {
    pub entity: Entity,
    pub pos: Vec2,
    pub armor: f32,
    pub dist: f32,
    pub score: f32,
    pub tier: u8,
    pub hp: f32,
    pub kind: Option<MonsterKind>,
    pub mind: Stance,
}

pub(super) struct Choice {
    pub retreat: Option<(Entity, Vec2, f32)>,
    pub fight: Option<(Entity, Vec2, f32, f32, Option<MonsterKind>)>,
    pub avoid: Option<(Entity, Vec2)>,
}

fn pos_bits(p: Vec2) -> (u32, u32) {
    (p.x.to_bits(), p.y.to_bits())
}

pub(super) struct Notice {
    pub me: Side,
    pub them: Side,
    pub player: bool,
    pub dist: f32,
    pub aggressor: bool,
    pub dangerous: bool,
    pub armed: bool,
    pub stick: bool,
    pub aggression: f32,
    pub scavenging: bool,
    pub cornered: bool,
    pub hp_frac: f32,
    pub playing: bool,
    pub melee: bool,
    pub ghoul: bool,
    pub crawler: bool,
    pub entity: Entity,
    pub pos: Vec2,
    pub armor: f32,
    pub score: f32,
    pub kind: Option<MonsterKind>,
}

pub(super) fn notice(n: Notice) -> Seen {
    let mind = if !n.playing {
        Stance::Fight
    } else {
        stance(&StanceInput {
            me: n.me,
            them: n.them,
            player: n.player,
            dist: n.dist,
            aggressor: n.aggressor,
            dangerous: n.dangerous,
            armed: n.armed,
            stick: n.stick,
            aggression: n.aggression,
            scavenging: n.scavenging,
            cornered: n.cornered,
            hp_frac: n.hp_frac,
            melee: n.melee,
            ghoul: n.ghoul,
            crawler: n.crawler,
        })
    };
    Seen {
        entity: n.entity,
        pos: n.pos,
        armor: n.armor,
        dist: n.dist,
        score: n.score,
        tier: n.them.tier,
        hp: n.them.hp,
        kind: n.kind,
        mind,
    }
}

pub(super) struct Held {
    pub mind: u8,
    pub until: u32,
    pub hide_since: u32,
    pub leaving: Option<(Entity, Vec2, f32)>,
    pub fighting: Option<(Entity, Vec2, f32, f32, Option<MonsterKind>)>,
    pub avoiding: Option<(Entity, Vec2, f32)>,
}

pub(super) fn settle(
    picked: &Choice,
    focus: Option<&Seen>,
    me: Side,
    current: u8,
    until: u32,
    hide_since: u32,
    now: u32,
    just_hit: bool,
    still_seen: bool,
    in_range: bool,
    cornered: bool,
    here: Vec2,
    dog: Option<&Seen>,
) -> Held {
    let proposed = if dog.is_some() && picked.retreat.is_some() {
        MIND_FIGHT
    } else if picked.retreat.is_some() {
        MIND_RETREAT
    } else if picked.fight.is_some() {
        MIND_FIGHT
    } else if picked.avoid.is_some() {
        MIND_AVOID
    } else {
        0
    };
    let ratio = focus
        .map(|s| {
            power_ratio(
                me,
                Side {
                    tier: s.tier,
                    hp: s.hp,
                    armor: s.armor,
                },
            )
        })
        .unwrap_or(2.0);
    let mut hide_since = hide_since;
    if still_seen && matches!(current, MIND_AVOID | MIND_RETREAT) {
        if hide_since == 0 {
            hide_since = now;
        }
    } else if current != MIND_AVOID && current != MIND_RETREAT {
        hide_since = 0;
    }
    let los_stuck = hide_since != 0 && now.saturating_sub(hide_since) >= LOS_GIVE_UP && just_hit;
    let interrupt = cornered || (just_hit && in_range) || los_stuck || dog.is_some();
    let mind = commit(current, proposed, until, now, interrupt, ratio);
    let mut until = until;
    if mind != current {
        until = now.saturating_add(HOLD_TICKS);
        hide_since = 0;
    }
    let focus_pos = focus.map(|s| (s.entity, s.pos, s.dist));
    Held {
        mind,
        until,
        hide_since,
        leaving: if mind == MIND_RETREAT {
            picked.retreat.or(focus_pos)
        } else {
            None
        },
        fighting: if mind == MIND_FIGHT {
            dog.filter(|_| picked.retreat.is_some())
                .map(|s| (s.entity, s.pos, s.armor, s.dist, s.kind))
                .or(picked.fight)
                .or_else(|| focus.map(|s| (s.entity, s.pos, s.armor, s.dist, s.kind)))
        } else {
            None
        },
        avoiding: if mind == MIND_AVOID {
            picked
                .avoid
                .map(|(e, p)| (e, p, focus.map(|s| s.dist).unwrap_or(p.distance(here))))
                .or(focus_pos)
        } else {
            None
        },
    }
}

pub(super) fn choice(seen: &[Seen]) -> Choice {
    let by_dist = |a: &&Seen, b: &&Seen| {
        a.dist
            .total_cmp(&b.dist)
            .then(pos_bits(a.pos).cmp(&pos_bits(b.pos)))
    };
    Choice {
        retreat: seen
            .iter()
            .filter(|s| s.mind == Stance::Retreat)
            .min_by(by_dist)
            .map(|s| (s.entity, s.pos, s.dist)),
        fight: seen
            .iter()
            .filter(|s| s.mind == Stance::Fight)
            .min_by(|a, b| {
                a.score
                    .total_cmp(&b.score)
                    .then(pos_bits(a.pos).cmp(&pos_bits(b.pos)))
            })
            .map(|s| (s.entity, s.pos, s.armor, s.dist, s.kind)),
        avoid: seen
            .iter()
            .filter(|s| s.mind == Stance::Avoid)
            .min_by(by_dist)
            .map(|s| (s.entity, s.pos)),
    }
}

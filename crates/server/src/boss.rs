//! 辐射巨兽, the behemoth: once per match it wakes three minutes in, rises in the radiation
//! block nearest the vault 30 s later, and lumbers off again at 6:30 if nobody brings it down.
//!
//! Its HP is `BEHEMOTH_BASE_HP` plus `HP_PER_HUMAN` for each connected human (bots don't count,
//! so a bot-filled lobby still gets a beatable boss). Three phases by remaining HP:
//! P1 slams (telegraphed) and spits acid; P2 adds a radiation ring and a charge; P3 is faster,
//! summons ghouls and trails radiation.
//!
//! Reward: 3 points to the top damage dealer only, by damage credit that halves every 8 s,
//! so a last-second steal doesn't pay. Whoever leads the scoreboard by 5+ deals less to it,
//! and a trailing player (≤40% of the leader's score) with ≥15% of the damage gets +1.
//! XP is split by total damage share through `XpEarned`.

use std::collections::HashMap;
use std::f32::consts::TAU;

use bevy::prelude::*;
use evoclash_game::items::{ArmorKind, Item, WeaponKind};
use evoclash_game::map::{Biome, MARGIN, VAULT_CENTER};
use evoclash_game::protocol::*;
use evoclash_game::sim::{Arena, DT, Obstacles, PLAYER_RADIUS};
use rand::Rng;

use crate::combat::Flight;
use crate::connections::Human;
use crate::gameplay::{
    Damage, DamageQueue, MATCH_TICKS, Outbox, ResetRequested, SimSet, TICKS_PER_SEC, match_flow,
    reset_match, resolve_damage, spawn_pickup,
};
use crate::monsters::{
    Brain, MAX_LIVE, XpEarned, nearest, over_building, set_state, shoot, spawn_monster,
    spawn_puddle,
};

pub const ANNOUNCE_AT: u32 = 180 * TICKS_PER_SEC;
pub const SPAWN_AT: u32 = 210 * TICKS_PER_SEC;
pub const LEAVE_AT: u32 = 390 * TICKS_PER_SEC;
pub const HP_PER_HUMAN: f32 = 600.0;
const MIN_ALIVE: usize = 2;
const DROP_TTL: u32 = 60 * TICKS_PER_SEC;

const SPEED: f32 = 85.0;
const P3_SPEED_MULT: f32 = 1.3;
const CHASE_RANGE: f32 = 1500.0;
const SLAM_DAMAGE: f32 = 40.0;
const SLAM_CD: u16 = 90;
/// 0.6 s telegraph.
const SLAM_WINDUP: u16 = 38;
const SPIT_GLOBS: u32 = 3;
const SPIT_DAMAGE: f32 = 12.0;
const SPIT_SPREAD: f32 = 0.2;
const SPIT_CD: u16 = 110;
const SPIT_RANGE: f32 = 650.0;
const ACID_SPEED: f32 = 340.0;
const ACID_TTL: u16 = 110;
const RING_GLOBS: u32 = 10;
const RING_DAMAGE: f32 = 14.0;
const RING_CD: u16 = 200;
const RING_SPEED: f32 = 240.0;
const RING_TTL: u16 = 170;
const CHARGE_SPEED: f32 = 300.0;
const CHARGE_DAMAGE: f32 = 45.0;
const CHARGE_WINDUP: u16 = 32;
const CHARGE_TICKS: u16 = 77;
const CHARGE_CD: u16 = 360;
const SUMMON_EVERY: u16 = 8 * TICKS_PER_SEC as u16;
const SUMMONS: usize = 3;
const TRAIL_EVERY: u16 = 2 * TICKS_PER_SEC as u16;

/// Damage credit halves every 8 s (applied once a second).
const CREDIT_DECAY_PER_SEC: f32 = 0.917_004;
const LEADER_GAP: u32 = 5;
const LEADER_DAMAGE_MULT: f32 = 0.7;
const TOP_SCORE: u32 = 3;
const UNDERDOG_RATIO: f32 = 0.4;
const UNDERDOG_SHARE: f32 = 0.15;

pub fn behemoth_hp(humans: usize) -> f32 {
    BEHEMOTH_BASE_HP + HP_PER_HUMAN * humans as f32
}

/// The scoreboard leader (by `LEADER_GAP` or more) hits the boss for less.
pub fn damage_mult(attacker_score: u32, best_other_score: u32) -> f32 {
    if attacker_score >= best_other_score + LEADER_GAP {
        LEADER_DAMAGE_MULT
    } else {
        1.0
    }
}

pub fn phase_of(hp: f32, max: f32) -> u8 {
    let frac = hp / max.max(1.0);
    if frac > 0.6 {
        1
    } else if frac > 0.3 {
        2
    } else {
        3
    }
}

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage {
    #[default]
    Waiting,
    Countdown,
    Alive,
    Done,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cue {
    Announce,
    Spawn,
    /// Too few players left when it was due.
    Cancel,
    /// Its time ran out.
    Leave,
}

#[derive(Default, Clone, Copy, Debug)]
pub struct Credit {
    /// Decaying credit, decides who gets the kill.
    pub recent: f32,
    /// Everything dealt, for XP and the underdog share.
    pub total: f32,
}

#[derive(Resource, Default)]
pub struct BossState {
    pub stage: Stage,
    credit: HashMap<Entity, Credit>,
    slain_at: Option<Vec2>,
}

impl BossState {
    /// Advance the once-per-match schedule. `elapsed` is ticks into the playing phase.
    pub fn step(&mut self, elapsed: u32, alive: usize) -> Option<Cue> {
        match self.stage {
            Stage::Waiting if elapsed >= ANNOUNCE_AT => {
                if alive >= MIN_ALIVE {
                    self.stage = Stage::Countdown;
                    Some(Cue::Announce)
                } else {
                    self.stage = Stage::Done;
                    None
                }
            }
            Stage::Countdown if elapsed >= SPAWN_AT => {
                if alive >= MIN_ALIVE {
                    self.stage = Stage::Alive;
                    Some(Cue::Spawn)
                } else {
                    self.stage = Stage::Done;
                    Some(Cue::Cancel)
                }
            }
            Stage::Alive if elapsed >= LEAVE_AT => {
                self.stage = Stage::Done;
                Some(Cue::Leave)
            }
            _ => None,
        }
    }

    pub fn credit(&mut self, attacker: Entity, amount: f32) {
        let c = self.credit.entry(attacker).or_default();
        c.recent += amount;
        c.total += amount;
    }

    pub fn slain(&mut self, at: Vec2) {
        self.stage = Stage::Done;
        self.slain_at = Some(at);
    }
}

pub struct Contribution {
    pub entity: Entity,
    pub credit: Credit,
    pub score: u32,
}

#[derive(Debug, PartialEq)]
pub struct Award {
    pub entity: Entity,
    pub score: u32,
    pub xp: u32,
}

/// Who gets what for the kill. `leader_score` is the best score of anyone in the match.
pub fn awards(contributions: &[Contribution], leader_score: u32) -> Vec<Award> {
    let sum: f32 = contributions.iter().map(|c| c.credit.total).sum();
    if sum <= 0.0 {
        return Vec::new();
    }
    let top = contributions
        .iter()
        .max_by(|a, b| {
            a.credit
                .recent
                .total_cmp(&b.credit.recent)
                .then(a.credit.total.total_cmp(&b.credit.total))
        })
        .map(|c| c.entity);
    contributions
        .iter()
        .map(|c| {
            let share = c.credit.total / sum;
            let underdog = leader_score > 0
                && c.score as f32 <= leader_score as f32 * UNDERDOG_RATIO
                && share >= UNDERDOG_SHARE;
            // The top dealer's 3 is the most anyone gets.
            let score = if Some(c.entity) == top {
                TOP_SCORE
            } else {
                u32::from(underdog)
            };
            Award {
                entity: c.entity,
                score,
                xp: (MonsterKind::Behemoth.xp() as f32 * share).round() as u32,
            }
        })
        .collect()
}

/// Where it rises: a clear spot in the radiation block nearest the vault, else near the vault.
pub fn lair(arena: &Arena, obstacles: &[(Vec2, f32)]) -> Vec2 {
    let r = MonsterKind::Behemoth.radius() + 8.0;
    let clear = |p: Vec2| arena.resolve_with(p, r, obstacles) == p && !over_building(arena, p, r);
    let block = arena
        .blocks
        .iter()
        .filter(|b| b.biome == Biome::Radiation)
        .min_by(|a, b| {
            let d = |r: &Rect| r.center().distance(VAULT_CENTER);
            d(&a.rect).total_cmp(&d(&b.rect))
        });
    if let Some(block) = block {
        let area = block.rect.inflate(-MARGIN - r);
        let center = block.rect.center();
        let mut spots = Vec::new();
        let mut y = area.min.y;
        while y <= area.max.y {
            let mut x = area.min.x;
            while x <= area.max.x {
                spots.push(Vec2::new(x, y));
                x += 40.0;
            }
            y += 40.0;
        }
        spots.sort_by(|a, b| a.distance(center).total_cmp(&b.distance(center)));
        if let Some(p) = spots.into_iter().find(|p| clear(*p)) {
            return p;
        }
    }
    (4..16)
        .flat_map(|ring| (0..24).map(move |i| (ring as f32 * 50.0, i as f32 * TAU / 24.0)))
        .map(|(dist, angle)| VAULT_CENTER + Vec2::from_angle(angle) * dist)
        .find(|p| clear(*p))
        .unwrap_or(VAULT_CENTER)
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Act {
    Idle,
    Slam(u16),
    Windup(u16),
    Charging(u16),
}

/// Server-only behemoth state.
#[derive(Component)]
pub struct BossBrain {
    max_hp: f32,
    phase: u8,
    act: Act,
    slam_cd: u16,
    spit_cd: u16,
    ring_cd: u16,
    charge_cd: u16,
    summon_cd: u16,
    trail_cd: u16,
    report_cd: u16,
    spiral: f32,
    charge_dir: Vec2,
    charge_hit: Vec<Entity>,
}

impl BossBrain {
    fn new(max_hp: f32) -> Self {
        Self {
            max_hp,
            phase: 1,
            act: Act::Idle,
            slam_cd: 0,
            spit_cd: SPIT_CD,
            ring_cd: RING_CD,
            charge_cd: CHARGE_CD / 2,
            summon_cd: 0,
            trail_cd: 0,
            report_cd: 0,
            spiral: 0.0,
            charge_dir: Vec2::X,
            charge_hit: Vec::new(),
        }
    }
}

pub struct BossPlugin;

impl Plugin for BossPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BossState>();
        app.add_systems(
            FixedUpdate,
            (
                behemoth.in_set(SimSet::Combat),
                reward.in_set(SimSet::Resolve).after(resolve_damage),
                boss_flow
                    .in_set(SimSet::Flow)
                    .after(match_flow)
                    .before(reset_match),
            ),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn boss_flow(
    mut commands: Commands,
    mut state: Single<&mut MatchState>,
    mut boss: ResMut<BossState>,
    reset: Res<ResetRequested>,
    arena: Res<Arena>,
    obstacles: Res<Obstacles>,
    humans: Query<&Human>,
    players: Query<&Vitals, With<Player>>,
    bosses: Query<(Entity, &Vitals, &BossBrain)>,
    mut outbox: ResMut<Outbox>,
    mut decay_in: Local<u32>,
) {
    if reset.0 {
        // The match is starting over; reset_match clears the arena.
        *boss = BossState::default();
    }
    let eta = if state.phase == Phase::Playing && boss.stage == Stage::Countdown {
        SPAWN_AT.saturating_sub(MATCH_TICKS.saturating_sub(state.ticks_left))
    } else {
        0
    };
    if state.boss_eta != eta {
        state.boss_eta = eta;
    }
    if state.phase != Phase::Playing {
        return;
    }
    if boss.stage == Stage::Alive {
        *decay_in = decay_in.saturating_sub(1);
        if *decay_in == 0 {
            *decay_in = TICKS_PER_SEC;
            for credit in boss.credit.values_mut() {
                credit.recent *= CREDIT_DECAY_PER_SEC;
            }
        }
    }
    let elapsed = MATCH_TICKS.saturating_sub(state.ticks_left);
    let alive = players.iter().filter(|v| v.alive).count();
    match boss.step(elapsed, alive) {
        Some(Cue::Announce) => {
            outbox.announce("辐射巨兽苏醒！30 秒后现身");
            state.boss_eta = SPAWN_AT - elapsed;
        }
        Some(Cue::Spawn) => {
            let humans = humans.iter().filter(|h| h.connected()).count();
            let hp = behemoth_hp(humans);
            let at = lair(&arena, &obstacles.0);
            let turf = Rect::from_center_size(at, Vec2::splat(800.0));
            let brain = Brain::new(MonsterKind::Behemoth, false, at, turf, 0, 0);
            let entity = spawn_monster(
                &mut commands,
                MonsterKind::Behemoth,
                false,
                at,
                brain,
                Some(hp),
            );
            commands.entity(entity).insert(BossBrain::new(hp));
            info!("behemoth rises at {at} with {hp} hp");
            outbox.announce("辐射巨兽出现了！击败它可获得 3 分");
            outbox.fast.push(GameEvent::BossHp {
                hp,
                max: hp,
                phase: 1,
            });
        }
        Some(Cue::Cancel) => outbox.announce("辐射巨兽又沉睡了"),
        Some(Cue::Leave) => {
            for (entity, vitals, _) in &bosses {
                if vitals.alive {
                    commands.entity(entity).despawn();
                }
            }
            outbox.announce("辐射巨兽遁入了废土");
            outbox.reliable.push(GameEvent::BossDown { killer: None });
        }
        None => {}
    }
}

/// Hand out the rewards once the behemoth falls.
fn reward(
    mut commands: Commands,
    mut boss: ResMut<BossState>,
    mut players: Query<(Entity, &Player, &mut Stats)>,
    mut outbox: ResMut<Outbox>,
    mut xp: MessageWriter<XpEarned>,
) {
    let Some(at) = boss.slain_at.take() else {
        return;
    };
    let leader = players.iter().map(|(.., s)| s.score).max().unwrap_or(0);
    let contributions: Vec<Contribution> = boss
        .credit
        .iter()
        .filter_map(|(entity, credit)| {
            let (_, _, stats) = players.get(*entity).ok()?;
            Some(Contribution {
                entity: *entity,
                credit: *credit,
                score: stats.score,
            })
        })
        .collect();
    boss.credit.clear();
    let mut killer = None;
    for award in awards(&contributions, leader) {
        let Ok((_, player, mut stats)) = players.get_mut(award.entity) else {
            continue;
        };
        if award.xp > 0 {
            xp.write(XpEarned {
                player: award.entity,
                amount: award.xp,
            });
        }
        if award.score == 0 {
            continue;
        }
        stats.score += award.score;
        if award.score == TOP_SCORE {
            killer = Some(player.name.clone());
            outbox.announce(format!("{} 击倒了辐射巨兽，+{TOP_SCORE} 分！", player.name));
        } else {
            outbox.announce(format!("{} 以弱胜强参与讨伐，额外 +1 分", player.name));
        }
    }
    info!("behemoth slain, credit to {killer:?}");
    outbox.reliable.push(GameEvent::BossDown { killer });
    for (item, offset) in [
        (Item::weapon(WeaponKind::Railgun), Vec2::new(-40.0, 0.0)),
        (Item::armor(ArmorKind::PowerArmor), Vec2::new(40.0, 0.0)),
        (Item::Medkit, Vec2::new(0.0, 40.0)),
    ] {
        spawn_pickup(&mut commands, item, at + offset, Some(DROP_TTL));
    }
    outbox.fast.push(GameEvent::Boom {
        pos: at,
        radius: 260.0,
        fire: false,
    });
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn behemoth(
    mut commands: Commands,
    state: Single<&MatchState>,
    arena: Res<Arena>,
    obstacles: Res<Obstacles>,
    mut bosses: Query<(Entity, &mut Pos, &Vitals, &mut BossBrain, &mut MonsterState)>,
    players: Query<(Entity, &Pos, &Vitals), (With<Player>, Without<BossBrain>)>,
    monsters: Query<(), With<Monster>>,
    mut damage: ResMut<DamageQueue>,
    mut outbox: ResMut<Outbox>,
) {
    if state.phase == Phase::Intermission {
        return;
    }
    let alive: Vec<(Entity, Vec2)> = players
        .iter()
        .filter(|(_, _, v)| v.alive)
        .map(|(e, p, _)| (e, p.0))
        .collect();
    let mut live_monsters = monsters.iter().count();
    let mut rng = rand::rng();
    let radius = MonsterKind::Behemoth.radius();
    for (entity, mut pos, vitals, mut brain, mut mstate) in &mut bosses {
        if !vitals.alive {
            continue;
        }
        let b = &mut *brain;
        for cd in [
            &mut b.slam_cd,
            &mut b.spit_cd,
            &mut b.ring_cd,
            &mut b.charge_cd,
            &mut b.summon_cd,
            &mut b.trail_cd,
            &mut b.report_cd,
        ] {
            *cd = cd.saturating_sub(1);
        }
        let phase = phase_of(vitals.hp, b.max_hp);
        if phase != b.phase || b.report_cd == 0 {
            if phase == 3 && b.phase != 3 {
                outbox.announce("辐射巨兽陷入狂暴！");
            }
            b.phase = phase;
            b.report_cd = TICKS_PER_SEC as u16;
            outbox.fast.push(GameEvent::BossHp {
                hp: vitals.hp,
                max: b.max_hp,
                phase,
            });
        }
        let at = pos.0;
        let hit = |damage: &mut DamageQueue, target: Entity, amount: f32, cause| {
            damage.0.push(Damage {
                target,
                attacker: Some(entity),
                amount,
                armor_mult: 1.0,
                weapon: None,
                emp: false,
                cause: Some(cause),
            });
        };
        let target = nearest(&alive, at).filter(|(_, _, d)| *d < CHASE_RANGE);
        let mut facing = mstate.facing;
        let mut velocity = Vec2::ZERO;
        b.act = match b.act {
            Act::Slam(t) if t > 1 => Act::Slam(t - 1),
            Act::Slam(_) => {
                for (victim, p) in &alive {
                    if p.distance(at) <= BEHEMOTH_SLAM_RADIUS + PLAYER_RADIUS {
                        hit(&mut damage, *victim, SLAM_DAMAGE, "重踏");
                    }
                }
                outbox.fast.push(GameEvent::Boom {
                    pos: at,
                    radius: BEHEMOTH_SLAM_RADIUS,
                    fire: false,
                });
                b.slam_cd = SLAM_CD;
                Act::Idle
            }
            Act::Windup(t) if t > 1 => Act::Windup(t - 1),
            Act::Windup(_) => {
                b.charge_hit.clear();
                Act::Charging(CHARGE_TICKS)
            }
            Act::Charging(t) => {
                velocity = b.charge_dir * CHARGE_SPEED;
                for (victim, p) in &alive {
                    if p.distance(at) <= radius + PLAYER_RADIUS && !b.charge_hit.contains(victim) {
                        b.charge_hit.push(*victim);
                        hit(&mut damage, *victim, CHARGE_DAMAGE, "冲撞");
                    }
                }
                if t > 1 {
                    Act::Charging(t - 1)
                } else {
                    b.charge_cd = CHARGE_CD;
                    Act::Idle
                }
            }
            Act::Idle => {
                let mut next = Act::Idle;
                if let Some((_, p, d)) = target {
                    let to = (p - at).normalize_or(Vec2::X);
                    facing = to.to_angle();
                    let sight = d < SPIT_RANGE && arena.line_of_sight(at, p);
                    if b.slam_cd == 0 && d <= BEHEMOTH_SLAM_RADIUS + PLAYER_RADIUS - 10.0 {
                        next = Act::Slam(SLAM_WINDUP);
                    } else if phase >= 2
                        && b.charge_cd == 0
                        && (250.0..700.0).contains(&d)
                        && arena.line_of_sight(at, p)
                    {
                        b.charge_dir = to;
                        next = Act::Windup(CHARGE_WINDUP);
                    } else {
                        if d > radius + PLAYER_RADIUS {
                            let mult = if phase == 3 { P3_SPEED_MULT } else { 1.0 };
                            velocity = to * SPEED * mult;
                        }
                        if b.spit_cd == 0 && sight {
                            b.spit_cd = SPIT_CD;
                            for i in 0..SPIT_GLOBS {
                                let off = (i as f32 - (SPIT_GLOBS - 1) as f32 / 2.0) * SPIT_SPREAD;
                                let dir = Vec2::from_angle(to.to_angle() + off);
                                let flight = Flight::fauna(entity, ACID_TTL, SPIT_DAMAGE, "酸液");
                                let from = at + dir * (radius + 8.0);
                                shoot(
                                    &mut commands,
                                    ProjectileKind::Acid,
                                    from,
                                    dir,
                                    ACID_SPEED,
                                    flight,
                                );
                            }
                        }
                    }
                }
                next
            }
        };
        if phase >= 2 && b.ring_cd == 0 && target.is_some() {
            b.ring_cd = RING_CD;
            b.spiral += 0.31;
            for i in 0..RING_GLOBS {
                let dir = Vec2::from_angle(b.spiral + i as f32 * TAU / RING_GLOBS as f32);
                let flight = Flight::fauna(entity, RING_TTL, RING_DAMAGE, "辐射");
                let from = at + dir * (radius + 8.0);
                shoot(
                    &mut commands,
                    ProjectileKind::Spore,
                    from,
                    dir,
                    RING_SPEED,
                    flight,
                );
            }
        }
        if phase == 3 {
            if b.trail_cd == 0 {
                b.trail_cd = TRAIL_EVERY;
                spawn_puddle(&mut commands, PuddleKind::Radiation, at, Some(entity));
            }
            if b.summon_cd == 0 && target.is_some() {
                b.summon_cd = SUMMON_EVERY;
                let start = rng.random_range(0.0..TAU);
                for i in 0..SUMMONS {
                    if live_monsters >= MAX_LIVE {
                        break;
                    }
                    let dir = Vec2::from_angle(start + i as f32 * TAU / SUMMONS as f32);
                    let spot = at + dir * (radius + 40.0);
                    let ghoul = MonsterKind::Ghoul;
                    if arena.resolve_with(spot, ghoul.radius(), &obstacles.0) != spot {
                        continue;
                    }
                    let turf = Rect::from_center_size(spot, Vec2::splat(600.0));
                    let brain = Brain::new(ghoul, false, spot, turf, 0, 0);
                    spawn_monster(&mut commands, ghoul, false, spot, brain, None);
                    live_monsters += 1;
                }
            }
        }
        if velocity != Vec2::ZERO {
            let want = at + velocity * DT;
            let next = arena.resolve_with(want, radius, &obstacles.0);
            if let Act::Charging(_) = b.act
                && next.distance(want) > CHARGE_SPEED * DT * 0.5
            {
                // Slammed into a wall: the charge ends.
                b.act = Act::Idle;
                b.charge_cd = CHARGE_CD;
            }
            if next != at {
                pos.0 = next;
            }
            if !matches!(b.act, Act::Charging(_)) {
                facing = velocity.to_angle();
            }
        }
        let (action, fuse) = match b.act {
            Act::Idle => (MonsterAction::Idle, 0),
            Act::Slam(t) => (MonsterAction::Slam, t),
            Act::Windup(t) => (MonsterAction::Charge, t),
            Act::Charging(_) => (MonsterAction::Charge, 0),
        };
        if matches!(b.act, Act::Windup(_) | Act::Charging(_)) {
            facing = b.charge_dir.to_angle();
        }
        let new = MonsterState {
            facing,
            phase,
            action,
            fuse,
        };
        set_state(&mut mstate, new);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evoclash_game::map::ArenaMap;

    fn run(alive_at: impl Fn(u32) -> usize) -> Vec<(u32, Cue)> {
        let mut boss = BossState::default();
        (0..MATCH_TICKS)
            .filter_map(|t| boss.step(t, alive_at(t)).map(|c| (t, c)))
            .collect()
    }

    #[test]
    fn spawns_once_at_three_thirty() {
        let cues = run(|_| 4);
        assert_eq!(
            cues,
            vec![
                (ANNOUNCE_AT, Cue::Announce),
                (SPAWN_AT, Cue::Spawn),
                (LEAVE_AT, Cue::Leave)
            ]
        );
        assert_eq!(SPAWN_AT, (3 * 60 + 30) * TICKS_PER_SEC);
        assert_eq!(LEAVE_AT, (6 * 60 + 30) * TICKS_PER_SEC);
        // Killed early: it never leaves or comes back.
        let mut boss = BossState::default();
        for t in 0..=SPAWN_AT {
            boss.step(t, 4);
        }
        boss.slain(Vec2::ZERO);
        assert!((SPAWN_AT..MATCH_TICKS).all(|t| boss.step(t, 4).is_none()));
    }

    #[test]
    fn needs_two_players() {
        assert!(run(|_| 1).is_empty());
        let cues = run(|t| if t < SPAWN_AT { 3 } else { 1 });
        assert_eq!(
            cues,
            vec![(ANNOUNCE_AT, Cue::Announce), (SPAWN_AT, Cue::Cancel)]
        );
    }

    #[test]
    fn hp_scales_with_humans() {
        assert_eq!(behemoth_hp(0), 1800.0);
        assert_eq!(behemoth_hp(1), 2400.0);
        assert_eq!(behemoth_hp(4), 4200.0);
        assert_eq!(phase_of(1800.0, 1800.0), 1);
        assert_eq!(phase_of(1000.0, 1800.0), 2);
        assert_eq!(phase_of(500.0, 1800.0), 3);
    }

    #[test]
    fn leader_hits_softer() {
        assert_eq!(damage_mult(9, 4), LEADER_DAMAGE_MULT);
        assert_eq!(damage_mult(8, 4), 1.0);
        assert_eq!(damage_mult(0, 0), 1.0);
    }

    #[test]
    fn score_goes_to_top_dealer_capped_at_three() {
        let e = |i| Entity::from_raw_u32(i).unwrap();
        let c = |i, recent, total, score| Contribution {
            entity: e(i),
            credit: Credit { recent, total },
            score,
        };
        // 1 dealt the most overall, but 2 has the freshest credit; 2 is also an underdog.
        let list = [
            c(1, 100.0, 1000.0, 10),
            c(2, 400.0, 600.0, 2),
            c(3, 10.0, 300.0, 4),
            c(4, 5.0, 100.0, 0),
        ];
        let awards = awards(&list, 10);
        let get = |i| awards.iter().find(|a| a.entity == e(i)).unwrap();
        assert_eq!(get(2).score, TOP_SCORE);
        assert_eq!(get(1).score, 0);
        // 3: 4 <= 40% of 10 and 15% of the damage.
        assert_eq!(get(3).score, 1);
        // 4: underdog but only 5% of the damage.
        assert_eq!(get(4).score, 0);
        assert!(awards.iter().all(|a| a.score <= TOP_SCORE));
        let xp: u32 = awards.iter().map(|a| a.xp).sum();
        assert!((599..=601).contains(&xp));
        assert_eq!(get(1).xp, 300);
    }

    #[test]
    fn lair_is_clear() {
        for seed in 0..100 {
            let arena = Arena(ArenaMap::generate(seed));
            let obstacles: Vec<(Vec2, f32)> = arena
                .props
                .iter()
                .map(|p| (p.pos, p.kind.radius()))
                .collect();
            let p = lair(&arena, &obstacles);
            let r = MonsterKind::Behemoth.radius();
            assert_eq!(arena.resolve_circle(p, r), p, "seed {seed}");
            assert!(obstacles.iter().all(|(c, cr)| c.distance(p) >= cr + r));
            assert!(!over_building(&arena, p, r), "seed {seed}");
        }
    }
}

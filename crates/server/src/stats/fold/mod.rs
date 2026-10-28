//! Fold one match's samples into a [`MatchRecord`](super::MatchRecord).

mod autopsy;
mod boss;
mod kills;
mod lives;
mod loot;
mod snapshots;

pub use autopsy::{Autopsy, TraceSnap};

use std::collections::{BTreeMap, HashMap, VecDeque};

use bevy::prelude::*;
use evoclash_game::items::{BAG_SLOTS, GUN_SLOTS, WeaponKind};
use evoclash_game::map::PropKind;
use evoclash_game::protocol::*;
use evoclash_game::sim::Arena;

use crate::boss::{BossState, Stage};
use crate::config::ServerConfig;
use crate::gameplay::TICKS_PER_SEC;

use super::rank_key;
use super::{KillRec, LifeRec, MatchRecord, PlayerRec, SnapRow, StatsBus};

const GAP: u8 = 2;
const FIGHT_GAP: u32 = 2 * TICKS_PER_SEC;
const T4_WINDOW: u32 = 20 * TICKS_PER_SEC;
const FINALE_AT: u32 = 6 * 60 * TICKS_PER_SEC;
const SNAP_2: u32 = 2 * 60 * TICKS_PER_SEC;
const SNAP_5: u32 = 5 * 60 * TICKS_PER_SEC;
const SRC_VAULT: u8 = 1;
const SRC_BOSS: u8 = 2;
const SRC_SAFE: u8 = 3;
/// Guns, then the backpack, then the melee weapon. A source tag rides on the slot.
const CARRY: usize = GUN_SLOTS + BAG_SLOTS + 1;
const MELEE_AT: usize = GUN_SLOTS + BAG_SLOTS;

/// Weapons a combatant is carrying, and where each one came from (0 = ordinary).
#[derive(Clone, Copy)]
struct Carry {
    kind: [Option<WeaponKind>; CARRY],
    src: [u8; CARRY],
}

impl Carry {
    fn bare(kind: [Option<WeaponKind>; CARRY]) -> Self {
        Self {
            kind,
            src: [0; CARRY],
        }
    }
}

struct Body {
    name: String,
    alive: bool,
    gear: u8,
    weapon: WeaponKind,
    /// Hotbar slot actually selected (`Loadout::selected`).
    selected: u8,
    holds_t4: bool,
    score: u32,
    kills: u32,
    deaths: u32,
    last_score_tick: u32,
    level: u8,
    streak: u32,
    skill: String,
    path: String,
}

struct OpenLife {
    start: u32,
    opens: u32,
    first_t1: Option<u32>,
    first_t2: Option<u32>,
    last_combat: u32,
    quiet: u32,
    scrap: u32,
}

struct Mon {
    elite: bool,
    hp: f32,
    first: Option<u32>,
}

struct Ground {
    source: u8,
    weapon: Option<WeaponKind>,
    pos: Vec2,
    /// Weapons inside a death bag, each with its own source. Empty otherwise.
    bag: Vec<(WeaponKind, u8)>,
}

struct PropSnap {
    kind: PropKind,
    open: bool,
    tier: u8,
    pos: Vec2,
    vault: bool,
}

pub struct Fold {
    roster: BTreeMap<u64, Body>,
    open_life: BTreeMap<u64, OpenLife>,
    lives: Vec<LifeRec>,
    fights: HashMap<(u64, u64), (u32, u32)>,
    props: HashMap<Entity, PropSnap>,
    mons: HashMap<Entity, Mon>,
    picks: HashMap<Entity, Ground>,
    carry: BTreeMap<u64, Carry>,
    holds: BTreeMap<u64, u32>,
    had_t4: BTreeMap<u64, bool>,
    bounty_at: BTreeMap<u64, u32>,
    t4_gains: Vec<u32>,
    vaults: Vec<u32>,
    last_vault: Option<u32>,
    boss_boom: Option<Vec2>,
    tag_safes: Vec<Vec2>,
    prev_stage: Stage,
    last_stamp: u32,
    started: bool,
    kills: Vec<KillRec>,
    fight_ticks: Vec<u32>,
    t4_hold_ticks: Vec<u32>,
    monster_ttk: Vec<u32>,
    elite_ttk: Vec<u32>,
    scrap_earned: u32,
    scrap_player: u32,
    scrap_container: u32,
    safes_cracked: u32,
    snap_2m: Option<Vec<SnapRow>>,
    snap_5m: Option<Vec<SnapRow>>,
    snap_6m: Option<Vec<SnapRow>>,
    finale_inner: u32,
    finale_alive: u32,
    bounties: u32,
    bounties_killed_60s: u32,
    boss_appeared: bool,
    boss_killed: bool,
    boss_left: bool,
    boss_appear_tick: Option<u32>,
    boss_end_tick: Option<u32>,
    boss_top_not_leader: Option<bool>,
    max_streak: u32,
    trace: HashMap<u64, VecDeque<TraceSnap>>,
    autopsies: Vec<Autopsy>,
    births: u32,
    births_armed: u32,
}

impl Fold {
    pub fn new() -> Self {
        Self {
            roster: BTreeMap::new(),
            open_life: BTreeMap::new(),
            lives: Vec::new(),
            fights: HashMap::new(),
            props: HashMap::new(),
            mons: HashMap::new(),
            picks: HashMap::new(),
            carry: BTreeMap::new(),
            holds: BTreeMap::new(),
            had_t4: BTreeMap::new(),
            bounty_at: BTreeMap::new(),
            t4_gains: Vec::new(),
            vaults: Vec::new(),
            last_vault: None,
            boss_boom: None,
            tag_safes: Vec::new(),
            prev_stage: Stage::Waiting,
            last_stamp: 0,
            started: false,
            kills: Vec::new(),
            fight_ticks: Vec::new(),
            t4_hold_ticks: Vec::new(),
            monster_ttk: Vec::new(),
            elite_ttk: Vec::new(),
            scrap_earned: 0,
            scrap_player: 0,
            scrap_container: 0,
            safes_cracked: 0,
            snap_2m: None,
            snap_5m: None,
            snap_6m: None,
            finale_inner: 0,
            finale_alive: 0,
            bounties: 0,
            bounties_killed_60s: 0,
            boss_appeared: false,
            boss_killed: false,
            boss_left: false,
            boss_appear_tick: None,
            boss_end_tick: None,
            boss_top_not_leader: None,
            max_streak: 0,
            trace: HashMap::new(),
            autopsies: Vec::new(),
            births: 0,
            births_armed: 0,
        }
    }

    pub fn tick(&mut self, world: &mut World, stamp: Option<u32>, bus: &StatsBus) {
        let Some(stamp) = stamp else {
            return;
        };
        self.last_stamp = stamp;
        if !self.started {
            self.started = true;
            self.capture(world, stamp, true);
            return;
        }
        let combat = self.apply_hits(stamp, &bus.hits);
        self.apply_notes(stamp, &bus.events);
        self.note_quiet(stamp, &combat);
        self.capture(world, stamp, false);
    }

    pub fn finish(mut self, world: &mut World, finished: bool) -> MatchRecord {
        let end = self.last_stamp;
        let ids: Vec<u64> = self.open_life.keys().copied().collect();
        for id in ids {
            self.close_life(id, end);
        }
        let holding: Vec<u64> = self.holds.keys().copied().collect();
        for id in holding {
            if let Some(start) = self.holds.remove(&id) {
                self.t4_hold_ticks.push(end.saturating_sub(start));
            }
        }
        let vault_opens = self.vaults.len() as u32;
        let vault_t4_hits = self
            .vaults
            .iter()
            .filter(|open| {
                self.t4_gains
                    .iter()
                    .any(|g| *g > **open && g.saturating_sub(**open) <= T4_WINDOW)
            })
            .count() as u32;

        let config = world.resource::<ServerConfig>();
        let seed = config.room_seed;
        let monsters = config.monsters;
        let match_index = world
            .query::<&MatchState>()
            .iter(world)
            .next()
            .map(|s| s.match_index)
            .unwrap_or(0);

        let mut players: Vec<PlayerRec> = self
            .roster
            .iter()
            .map(|(id, b)| PlayerRec {
                client_id: *id,
                name: b.name.clone(),
                skill: b.skill.clone(),
                path: b.path.clone(),
                rank: 0,
                score: b.score,
                kills: b.kills,
                deaths: b.deaths,
                level: b.level,
                had_t4: self.had_t4.get(id).copied().unwrap_or(false),
            })
            .collect();
        players.sort_by(|a, b| {
            let tb = tick_of(&self.roster, b.client_id);
            let ta = tick_of(&self.roster, a.client_id);
            rank_key(b.score, b.kills, b.deaths, tb, b.client_id).cmp(&rank_key(
                a.score,
                a.kills,
                a.deaths,
                ta,
                a.client_id,
            ))
        });
        for (i, p) in players.iter_mut().enumerate() {
            p.rank = (i as u8).saturating_add(1);
        }
        let winner_id = players.first().map(|p| p.client_id);

        self.kills.sort_by(|a, b| {
            (a.tick, a.killer_id, a.victim_id).cmp(&(b.tick, b.killer_id, b.victim_id))
        });
        self.fight_ticks.sort_unstable();
        self.t4_hold_ticks.sort_unstable();
        self.monster_ttk.sort_unstable();
        self.elite_ttk.sort_unstable();
        self.lives.sort_by_key(|l| (l.client_id, l.start));

        MatchRecord {
            seed,
            match_index,
            monsters,
            playing_ticks: self.last_stamp,
            finished,
            players,
            kills: self.kills,
            lives: self.lives,
            scrap_earned: self.scrap_earned,
            scrap_player: self.scrap_player,
            scrap_container: self.scrap_container,
            safes_cracked: self.safes_cracked,
            bot_retreats: world
                .get_resource::<crate::bots::BotPulse>()
                .map(|p| p.retreats)
                .unwrap_or(0),
            bot_avoids: world
                .get_resource::<crate::bots::BotPulse>()
                .map(|p| p.avoids)
                .unwrap_or(0),
            snap_2m: self.snap_2m,
            snap_5m: self.snap_5m,
            snap_6m: self.snap_6m,
            finale_inner: self.finale_inner,
            finale_alive: self.finale_alive,
            t4_hold_ticks: self.t4_hold_ticks,
            vault_opens,
            vault_t4_hits,
            bounties: self.bounties,
            bounties_killed_60s: self.bounties_killed_60s,
            boss_appeared: self.boss_appeared,
            boss_killed: self.boss_killed,
            boss_left: self.boss_left,
            boss_appear_tick: self.boss_appear_tick,
            boss_end_tick: self.boss_end_tick,
            boss_top_not_leader: self.boss_top_not_leader,
            max_streak: self.max_streak,
            autopsies: self.autopsies,
            births: self.births,
            births_armed: self.births_armed,
            monster_ttk_ticks: self.monster_ttk,
            elite_ttk_ticks: self.elite_ttk,
            fight_ticks: self.fight_ticks,
            winner_id,
        }
    }

    fn capture(&mut self, world: &mut World, stamp: u32, init: bool) {
        let bodies = lives::read_bodies(world);
        let props = loot::read_props(world);
        let mons = boss::read_mons(world);
        let picks = loot::read_picks(world);
        let stage = world.resource::<BossState>().stage;
        let inner = |pos: Vec2| {
            world
                .resource::<Arena>()
                .block_at(pos)
                .is_some_and(|b| b.tier >= 2)
        };

        if init {
            for b in &bodies {
                self.open_for(b.id, stamp);
            }
            self.props = props;
            self.mons = mons
                .into_iter()
                .map(|(e, m)| {
                    (
                        e,
                        Mon {
                            elite: m.0,
                            hp: m.1,
                            first: None,
                        },
                    )
                })
                .collect();
            self.picks = picks;
            self.carry = bodies
                .iter()
                .map(|b| (b.id, Carry::bare(b.slots)))
                .collect();
            self.store_bodies(bodies);
            self.prev_stage = stage;
            return;
        }

        self.diff_props(&props, &bodies);
        self.diff_mons(stamp, mons);
        self.diff_bodies(stamp, &bodies, inner);
        self.diff_picks(stamp, picks, &bodies);
        self.note_boss(stamp, stage);
        self.sample_bots(world, stamp);
        self.props = props;
        self.store_bodies(bodies);
        self.seal_sourced(stamp);
        self.prev_stage = stage;
        self.boss_boom = None;
    }
}

fn carried(loadout: &Loadout) -> [Option<WeaponKind>; CARRY] {
    let mut slots = [None; CARRY];
    slots[..GUN_SLOTS].copy_from_slice(&loadout.guns);
    slots[GUN_SLOTS..MELEE_AT].copy_from_slice(&loadout.bag);
    slots[MELEE_AT] = Some(loadout.melee);
    slots
}

/// Source tag of the weapon in hand, 0 if that gun was not from the vault, a safe, or the behemoth.
fn hand_source(body: &Body, carry: &Carry) -> u8 {
    let gun = (body.selected as usize) < GUN_SLOTS
        && carry.kind[body.selected as usize] == Some(body.weapon);
    if gun {
        carry.src[body.selected as usize]
    } else if carry.kind[MELEE_AT] == Some(body.weapon) {
        carry.src[MELEE_AT]
    } else {
        0
    }
}

struct Sample {
    id: u64,
    name: String,
    pos: Vec2,
    alive: bool,
    gear: u8,
    weapon: WeaponKind,
    selected: u8,
    slots: [Option<WeaponKind>; CARRY],
    holds_t4: bool,
    score: u32,
    kills: u32,
    deaths: u32,
    last_score_tick: u32,
    level: u8,
    streak: u32,
    life_scrap: u32,
    skill: String,
    path: String,
}

fn tick_of(roster: &BTreeMap<u64, Body>, id: u64) -> u32 {
    roster.get(&id).map(|b| b.last_score_tick).unwrap_or(0)
}

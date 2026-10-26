//! In-match evolution: XP, levels 1-15, perks along three paths and the skills a path grants.
//!
//! Everything here is plain data and arithmetic shared by server (authority) and client (HUD,
//! prediction). Only the movement multiplier feeds the deterministic sim (`sim::speed_mult`).
//! Every level is one point to spend on any perk; the server decides when spending is allowed
//! (any time) and when refunds are (only while dead).

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::items::{AmmoKind, Arm, ArmorKind, WeaponKind};
use crate::sim::MAX_HP;

const SEC: u16 = 64;

pub const MAX_LEVEL: u8 = 15;
pub const MAX_RANK: u8 = 3;
/// Cumulative XP needed to reach levels 2..=15. Paced so most players end a match around
/// level 12 and only the leader gets near the cap.
pub const LEVEL_XP: [u32; 14] = [
    100, 250, 450, 700, 1000, 1360, 1780, 2260, 2800, 3410, 4090, 4840, 5660, 6560,
];
pub const CATCH_UP_PER_LEVEL: f32 = 0.1;
pub const CATCH_UP_MAX: f32 = 1.5;
/// Cap on the combined perk/skill movement multiplier (armor and EMP stack on top).
pub const MOVE_MULT_CAP: f32 = 1.3;
/// Path rank at which the leading path grants its first and second skill.
pub const SKILL_A_RANK: u8 = 2;
pub const SKILL_B_RANK: u8 = 5;
/// The last part of 狂暴 is a slow aftermath.
pub const BERSERK_AFTERMATH_TICKS: u16 = 2 * SEC;
pub const VAULT_SAFE_XP: u32 = 150;
/// Stealthed combatants are seen (by players, bots and turrets) only this close.
pub const STEALTH_SIGHT: f32 = 250.0;
/// Damage 能量护盾 absorbs when raised.
pub const SHIELD_HP: f32 = 60.0;
/// 酸液吐息 reach and cone half-angle (the cone is 70° wide).
pub const ACID_RANGE: f32 = 400.0;
pub const ACID_HALF_ANGLE: f32 = 35.0 * std::f32::consts::PI / 180.0;
/// cos(ACID_HALF_ANGLE), for dot-product tests.
pub const ACID_CONE_COS: f32 = 0.819_152;

/// Whether someone at `pos` with these vitals can be seen from `viewer_pos`: always, unless
/// stealthed and farther than `STEALTH_SIGHT`. Being revealed (sprung a trap) beats stealth.
pub fn seen_from(vitals: &crate::protocol::Vitals, pos: Vec2, viewer_pos: Vec2) -> bool {
    vitals.stealth == 0 || vitals.revealed > 0 || pos.distance(viewer_pos) <= STEALTH_SIGHT
}

/// Cumulative XP needed to reach `level` (0 for level 1).
pub fn xp_to_reach(level: u8) -> u32 {
    match level {
        0 | 1 => 0,
        l => LEVEL_XP[(l.min(MAX_LEVEL) - 2) as usize],
    }
}

pub fn level_for_xp(xp: u32) -> u8 {
    1 + LEVEL_XP.iter().filter(|t| xp >= **t).count() as u8
}

/// Whoever trails the top level earns more, up to ×1.5.
pub fn catch_up_mult(top_level: u8, own_level: u8) -> f32 {
    let gap = top_level.saturating_sub(own_level) as f32;
    (1.0 + CATCH_UP_PER_LEVEL * gap).min(CATCH_UP_MAX)
}

pub fn scaled_xp(base: u32, mult: f32) -> u32 {
    (base as f32 * mult).round() as u32
}

pub fn player_kill_xp(victim_level: u8, bounty: bool, giant_slayer: bool) -> u32 {
    60 + 20 * victim_level as u32 + if bounty { 50 } else { 0 } + if giant_slayer { 40 } else { 0 }
}

pub fn crate_xp(tier: u8) -> u32 {
    10 + 5 * tier as u32
}

pub fn safe_xp(tier: u8) -> u32 {
    60 + 10 * tier as u32
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, Reflect)]
pub enum EvoPath {
    Mutant,
    Cyborg,
    Scavenger,
}

impl EvoPath {
    pub const ALL: [EvoPath; 3] = [EvoPath::Mutant, EvoPath::Cyborg, EvoPath::Scavenger];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn name(self) -> &'static str {
        match self {
            EvoPath::Mutant => "变异体",
            EvoPath::Cyborg => "改造人",
            EvoPath::Scavenger => "拾荒者",
        }
    }

    pub fn perks(self) -> [Perk; 4] {
        let i = self.index() * 4;
        [
            Perk::ALL[i],
            Perk::ALL[i + 1],
            Perk::ALL[i + 2],
            Perk::ALL[i + 3],
        ]
    }

    /// (skill A at path rank `SKILL_A_RANK`, skill B at `SKILL_B_RANK`)
    pub fn skills(self) -> (Skill, Skill) {
        match self {
            EvoPath::Mutant => (Skill::AcidBreath, Skill::Berserk),
            EvoPath::Cyborg => (Skill::EnergyShield, Skill::AutoTurret),
            EvoPath::Scavenger => (Skill::Stealth, Skill::Trap),
        }
    }
}

/// Twelve perks, four per path in `EvoPath::ALL` order. `Evolution::ranks` is indexed by `index()`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, Reflect)]
pub enum Perk {
    // 变异体
    Constitution,
    Regeneration,
    Bloodthirst,
    Claws,
    // 改造人
    Overclock,
    CompositeArmor,
    EnergyFocus,
    SelfRepair,
    // 拾荒者
    LightFoot,
    AmmoRecycler,
    EmpExpert,
    Lockpick,
}

impl Perk {
    pub const ALL: [Perk; 12] = [
        Perk::Constitution,
        Perk::Regeneration,
        Perk::Bloodthirst,
        Perk::Claws,
        Perk::Overclock,
        Perk::CompositeArmor,
        Perk::EnergyFocus,
        Perk::SelfRepair,
        Perk::LightFoot,
        Perk::AmmoRecycler,
        Perk::EmpExpert,
        Perk::Lockpick,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn evo_path(self) -> EvoPath {
        EvoPath::ALL[self.index() / 4]
    }

    pub fn name(self) -> &'static str {
        match self {
            Perk::Constitution => "强化体质",
            Perk::Regeneration => "再生组织",
            Perk::Bloodthirst => "嗜血",
            Perk::Claws => "利爪",
            Perk::Overclock => "超频扳机",
            Perk::CompositeArmor => "复合装甲",
            Perk::EnergyFocus => "能量聚焦",
            Perk::SelfRepair => "自修护甲",
            Perk::LightFoot => "轻足",
            Perk::AmmoRecycler => "弹药回收",
            Perk::EmpExpert => "电磁专家",
            Perk::Lockpick => "撬锁专家",
        }
    }

    /// One-line effect per rank, for the evolution panel.
    pub fn desc(self) -> &'static str {
        match self {
            Perk::Constitution => "最大生命 +12/级",
            Perk::Regeneration => "4 秒未受伤后每秒回复 1.2 生命/级",
            Perk::Bloodthirst => "造成伤害的 8% 转为生命/级（近战 16%）",
            Perk::Claws => "近战伤害 +20%/级，攻击距离 +8/级",
            Perk::Overclock => "射击间隔 -8%/级",
            Perk::CompositeArmor => "护甲损耗 -10%/级，护甲上限 +15/级",
            Perk::EnergyFocus => "激光与轨道炮伤害 +10%/级，破甲 +0.1/级",
            Perk::SelfRepair => "5 秒未受伤后每秒修复 2 护甲/级",
            Perk::LightFoot => "移动速度 +5%/级",
            Perk::AmmoRecycler => "弹药上限 +20%/级，每次击杀回收 2 发/级",
            Perk::EmpExpert => "电磁脉冲雷上限 +1/级，范围 +15%/级",
            Perk::Lockpick => "撬保险柜更快，箱子与保险柜经验 +10%/级",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, Reflect)]
pub enum Skill {
    /// 变异体 A (R).
    AcidBreath,
    /// 变异体 B (T).
    Berserk,
    /// 改造人 A.
    EnergyShield,
    /// 改造人 B.
    AutoTurret,
    /// 拾荒者 A.
    Stealth,
    /// 拾荒者 B.
    Trap,
}

impl Skill {
    pub const ALL: [Skill; 6] = [
        Skill::AcidBreath,
        Skill::Berserk,
        Skill::EnergyShield,
        Skill::AutoTurret,
        Skill::Stealth,
        Skill::Trap,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Skill::AcidBreath => "酸液吐息",
            Skill::Berserk => "狂暴",
            Skill::EnergyShield => "能量护盾",
            Skill::AutoTurret => "自动炮塔",
            Skill::Stealth => "隐身",
            Skill::Trap => "陷阱",
        }
    }

    pub fn desc(self) -> &'static str {
        match self {
            Skill::AcidBreath => "向前喷吐酸液 1.5 秒，大量损耗护甲",
            Skill::Berserk => "8 秒内伤害、速度与吸血提升，之后短暂乏力",
            Skill::EnergyShield => "4 秒内先于护甲吸收 60 点伤害",
            Skill::AutoTurret => "部署一座炮塔，自动射击 12 秒",
            Skill::Stealth => "5 秒内远处的敌人看不见你，开火或受伤即现形",
            Skill::Trap => "布置隐形陷阱，触发后造成伤害、减速并暴露目标",
        }
    }

    pub fn cooldown(self) -> u16 {
        SEC * match self {
            Skill::AcidBreath => 14,
            Skill::Berserk => 40,
            Skill::EnergyShield => 20,
            Skill::AutoTurret => 45,
            Skill::Stealth => 22,
            Skill::Trap => 12,
        }
    }

    /// Ticks the skill stays active (`SkillState::active_*`). 狂暴 includes its 2 s aftermath;
    /// 自动炮塔 is the turret's lifetime; a trap has no active phase.
    pub fn duration(self) -> u16 {
        match self {
            Skill::AcidBreath => SEC * 3 / 2,
            Skill::Berserk => 8 * SEC + BERSERK_AFTERMATH_TICKS,
            Skill::EnergyShield => 4 * SEC,
            Skill::AutoTurret => 12 * SEC,
            Skill::Stealth => 5 * SEC,
            Skill::Trap => 0,
        }
    }
}

/// A combatant's progress this match. Kept through death, reset when a match starts.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Reflect)]
pub struct Evolution {
    pub level: u8,
    pub xp: u32,
    /// Cumulative XP of the next level (equal to the max-level threshold at max level).
    pub xp_next: u32,
    /// Rank 0..=3 of each perk, indexed by `Perk::index()`.
    pub ranks: [u8; 12],
    /// Points earned (one per level) but not spent yet. `points + total_ranks() == level - 1`.
    pub points: u8,
    /// For each path, the number of the point that first put a rank in it (0 = none now).
    /// Breaks ties for the leading path in favour of the one invested in first.
    pub first_pick: [u8; 3],
}

impl Default for Evolution {
    fn default() -> Self {
        Self {
            level: 1,
            xp: 0,
            xp_next: xp_to_reach(2),
            ranks: [0; 12],
            points: 0,
            first_pick: [0; 3],
        }
    }
}

impl Evolution {
    pub fn rank(&self, perk: Perk) -> u8 {
        self.ranks[perk.index()]
    }

    fn r(&self, perk: Perk) -> f32 {
        self.rank(perk) as f32
    }

    pub fn path_rank(&self, path: EvoPath) -> u8 {
        path.perks().iter().map(|p| self.rank(*p)).sum()
    }

    pub fn total_ranks(&self) -> u8 {
        self.ranks.iter().sum()
    }

    /// The highest-rank path (ties go to the one picked first); `None` before any pick.
    pub fn lead_path(&self) -> Option<EvoPath> {
        EvoPath::ALL
            .into_iter()
            .filter(|p| self.path_rank(*p) > 0)
            .min_by_key(|p| {
                (
                    std::cmp::Reverse(self.path_rank(*p)),
                    self.first_pick[p.index()],
                )
            })
    }

    /// Skills granted by the leading path: (A, B).
    pub fn skills(&self) -> (Option<Skill>, Option<Skill>) {
        let Some(path) = self.lead_path() else {
            return (None, None);
        };
        let rank = self.path_rank(path);
        let (a, b) = path.skills();
        (
            (rank >= SKILL_A_RANK).then_some(a),
            (rank >= SKILL_B_RANK).then_some(b),
        )
    }

    /// Add XP and return how many levels were gained (each one is a point).
    pub fn add_xp(&mut self, amount: u32) -> u8 {
        self.xp = self.xp.saturating_add(amount);
        let level = level_for_xp(self.xp);
        let gained = level.saturating_sub(self.level);
        self.level = level;
        self.xp_next = xp_to_reach((level + 1).min(MAX_LEVEL));
        self.points += gained;
        gained
    }

    /// Perks that can still be ranked up.
    pub fn open_perks(&self) -> impl Iterator<Item = Perk> + '_ {
        Perk::ALL.into_iter().filter(|p| self.rank(*p) < MAX_RANK)
    }

    pub fn can_spend(&self, perk: Perk) -> bool {
        self.points > 0 && self.rank(perk) < MAX_RANK
    }

    /// Put one point into `perk`. Allowed any time (alive or dead).
    pub fn spend(&mut self, perk: Perk) -> bool {
        if !self.can_spend(perk) {
            return false;
        }
        self.points -= 1;
        self.ranks[perk.index()] += 1;
        let path = perk.evo_path().index();
        if self.first_pick[path] == 0 {
            self.first_pick[path] = self.total_ranks();
        }
        true
    }

    /// Take one point back out of `perk`. The server only allows this while dead.
    pub fn refund(&mut self, perk: Perk) -> bool {
        if self.rank(perk) == 0 {
            return false;
        }
        self.ranks[perk.index()] -= 1;
        self.points += 1;
        let path = perk.evo_path();
        if self.path_rank(path) == 0 {
            self.first_pick[path.index()] = 0;
        }
        true
    }

    /// Refund every point (full respec; the server only allows this while dead).
    pub fn reset(&mut self) {
        self.points += self.total_ranks();
        self.ranks = [0; 12];
        self.first_pick = [0; 3];
    }

    /// Go back to an earlier allocation (沿用上次配置), keeping the points earned since as
    /// unspent. Returns false (and changes nothing) if it needs more points than there are.
    pub fn restore(&mut self, ranks: [u8; 12], first_pick: [u8; 3]) -> bool {
        let need: u8 = ranks.iter().sum();
        let have = self.points + self.total_ranks();
        if need > have || ranks.iter().any(|r| *r > MAX_RANK) {
            return false;
        }
        self.ranks = ranks;
        self.first_pick = first_pick;
        self.points = have - need;
        true
    }

    pub fn max_hp(&self) -> f32 {
        MAX_HP + 12.0 * self.r(Perk::Constitution)
    }

    pub fn regen_per_sec(&self) -> f32 {
        1.2 * self.r(Perk::Regeneration)
    }

    /// Fraction of damage dealt that heals you.
    pub fn lifesteal(&self, melee: bool) -> f32 {
        let per = if melee { 0.16 } else { 0.08 };
        per * self.r(Perk::Bloodthirst)
    }

    pub fn melee_damage_mult(&self) -> f32 {
        1.0 + 0.2 * self.r(Perk::Claws)
    }

    pub fn melee_reach_bonus(&self) -> f32 {
        8.0 * self.r(Perk::Claws)
    }

    /// Ticks between shots: -8%/rank, never below 4 (or the base, if that is lower).
    pub fn fire_cooldown(&self, base: u16) -> u16 {
        let cd = (base as f32 * (1.0 - 0.08 * self.r(Perk::Overclock))).round() as u16;
        cd.max(4.min(base))
    }

    /// Multiplier on armor points lost when hit.
    pub fn armor_wear_mult(&self) -> f32 {
        1.0 - 0.1 * self.r(Perk::CompositeArmor)
    }

    pub fn armor_cap(&self, kind: ArmorKind) -> f32 {
        if kind == ArmorKind::None {
            return 0.0;
        }
        kind.max_points() + 15.0 * self.r(Perk::CompositeArmor)
    }

    fn energy(weapon: WeaponKind) -> bool {
        weapon.ranged().is_some_and(|w| w.energy())
    }

    pub fn weapon_damage_mult(&self, weapon: WeaponKind) -> f32 {
        match weapon.arm() {
            Arm::Melee(_) => self.melee_damage_mult(),
            Arm::Ranged(w) if w.energy() => 1.0 + 0.1 * self.r(Perk::EnergyFocus),
            Arm::Ranged(_) => 1.0,
        }
    }

    pub fn armor_mult_bonus(&self, weapon: WeaponKind) -> f32 {
        if Self::energy(weapon) {
            0.1 * self.r(Perk::EnergyFocus)
        } else {
            0.0
        }
    }

    pub fn armor_repair_per_sec(&self) -> f32 {
        2.0 * self.r(Perk::SelfRepair)
    }

    pub fn move_mult(&self) -> f32 {
        1.0 + 0.05 * self.r(Perk::LightFoot)
    }

    /// Ammo pouch capacity for one family.
    pub fn ammo_cap(&self, kind: AmmoKind) -> u16 {
        let base = kind.base_cap();
        (base as f32 * (1.0 + 0.2 * self.r(Perk::AmmoRecycler))).round() as u16
    }

    /// Extra trigger pulls of ammo salvaged per kill.
    pub fn ammo_per_kill(&self) -> u16 {
        2 * self.rank(Perk::AmmoRecycler) as u16
    }

    pub fn emp_cap(&self, base: u8) -> u8 {
        base + self.rank(Perk::EmpExpert)
    }

    pub fn emp_radius_mult(&self) -> f32 {
        1.0 + 0.15 * self.r(Perk::EmpExpert)
    }

    pub fn crack_ticks(&self) -> u32 {
        [160, 135, 110, 85][self.rank(Perk::Lockpick).min(MAX_RANK) as usize]
    }

    /// Extra XP from crates and safes.
    pub fn loot_xp_mult(&self) -> f32 {
        1.0 + 0.1 * self.r(Perk::Lockpick)
    }
}

/// Skill slots and timers. Skill A is key R, skill B is key T.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Reflect, Default)]
pub struct SkillState {
    /// Unlocked skills (from `Evolution::skills`).
    pub skill_a: Option<Skill>,
    pub skill_b: Option<Skill>,
    /// Ticks until each skill can be used again (counted from activation).
    pub cd_a: u16,
    pub cd_b: u16,
    /// Ticks each skill stays active.
    pub active_a: u16,
    pub active_b: u16,
    /// Damage 能量护盾 still absorbs.
    pub shield_hp: f32,
}

impl SkillState {
    fn a_is(&self, skill: Skill) -> bool {
        self.skill_a == Some(skill) && self.active_a > 0
    }

    pub fn breathing_acid(&self) -> bool {
        self.a_is(Skill::AcidBreath)
    }

    pub fn shielded(&self) -> bool {
        self.a_is(Skill::EnergyShield) && self.shield_hp > 0.0
    }

    pub fn berserk(&self) -> bool {
        self.skill_b == Some(Skill::Berserk) && self.active_b > BERSERK_AFTERMATH_TICKS
    }

    /// The slow aftermath of 狂暴.
    pub fn exhausted(&self) -> bool {
        self.skill_b == Some(Skill::Berserk)
            && self.active_b > 0
            && self.active_b <= BERSERK_AFTERMATH_TICKS
    }

    pub fn damage_mult(&self) -> f32 {
        if self.berserk() { 1.25 } else { 1.0 }
    }

    pub fn lifesteal_mult(&self) -> f32 {
        if self.berserk() { 2.0 } else { 1.0 }
    }
}

/// Movement multiplier from perks and skills (轻足, 狂暴 and its aftermath, 酸液吐息), capped
/// at ×1.3. Part of the shared sim: client prediction must use the same numbers.
pub fn evolution_speed_mult(evo: &Evolution, skills: &SkillState) -> f32 {
    let mut mult = evo.move_mult();
    if skills.berserk() {
        mult *= 1.15;
    } else if skills.exhausted() {
        mult *= 0.8;
    }
    if skills.breathing_acid() {
        mult *= 0.6;
    }
    mult.min(MOVE_MULT_CAP)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xp_table_maps_to_levels() {
        assert_eq!(level_for_xp(0), 1);
        assert_eq!(level_for_xp(99), 1);
        assert_eq!(level_for_xp(100), 2);
        assert_eq!(level_for_xp(249), 2);
        assert_eq!(level_for_xp(250), 3);
        assert_eq!(level_for_xp(6559), 14);
        assert_eq!(level_for_xp(6560), 15);
        assert_eq!(level_for_xp(u32::MAX), MAX_LEVEL);
        for level in 2..=MAX_LEVEL {
            assert_eq!(level_for_xp(xp_to_reach(level)), level);
            assert_eq!(level_for_xp(xp_to_reach(level) - 1), level - 1);
        }
    }

    #[test]
    fn add_xp_grants_one_point_per_level() {
        let mut evo = Evolution::default();
        assert_eq!(evo.xp_next, 100);
        assert_eq!(evo.add_xp(99), 0);
        assert_eq!(evo.add_xp(200), 2);
        assert_eq!((evo.level, evo.points, evo.xp_next), (3, 2, 450));
        evo.add_xp(100_000);
        assert_eq!((evo.level, evo.points, evo.xp_next), (15, 14, 6560));
    }

    #[test]
    fn catch_up_is_capped() {
        assert_eq!(catch_up_mult(3, 3), 1.0);
        assert_eq!(catch_up_mult(1, 5), 1.0);
        assert!((catch_up_mult(5, 2) - 1.3).abs() < 1e-6);
        assert_eq!(catch_up_mult(10, 1), CATCH_UP_MAX);
        assert_eq!(scaled_xp(100, catch_up_mult(10, 1)), 150);
    }

    #[test]
    fn revealed_beats_stealth() {
        let far = Vec2::new(STEALTH_SIGHT + 1.0, 0.0);
        let mut vitals = crate::protocol::Vitals {
            stealth: 100,
            ..default()
        };
        assert!(!seen_from(&vitals, Vec2::ZERO, far));
        assert!(seen_from(
            &vitals,
            Vec2::ZERO,
            Vec2::new(STEALTH_SIGHT, 0.0)
        ));
        vitals.revealed = 10;
        assert!(seen_from(&vitals, Vec2::ZERO, far));
        assert!((ACID_HALF_ANGLE.cos() - ACID_CONE_COS).abs() < 1e-5);
    }

    #[test]
    fn kill_xp() {
        assert_eq!(player_kill_xp(1, false, false), 80);
        assert_eq!(player_kill_xp(5, true, true), 60 + 100 + 50 + 40);
        assert_eq!(crate_xp(2), 20);
        assert_eq!(safe_xp(3), 90);
        // Monster XP lives on `MonsterKind::xp()`; elites are worth triple.
        use crate::protocol::{Monster, MonsterKind};
        assert_eq!(MonsterKind::Roach.xp(), 25);
        assert_eq!(MonsterKind::SporePod.xp(), 60);
        let elite = Monster {
            kind: MonsterKind::Dog,
            elite: true,
        };
        assert_eq!(elite.xp(), 75);
    }

    fn pick_perk(evo: &mut Evolution, perk: Perk) {
        evo.points += 1;
        assert!(evo.spend(perk));
    }

    #[test]
    fn skills_unlock_at_path_rank_2_and_5() {
        let mut evo = Evolution::default();
        let order = [
            Perk::LightFoot,
            Perk::LightFoot,
            Perk::Lockpick,
            Perk::EmpExpert,
            Perk::EmpExpert,
        ];
        for (i, perk) in order.into_iter().enumerate() {
            pick_perk(&mut evo, perk);
            let rank = i as u8 + 1;
            assert_eq!(evo.path_rank(EvoPath::Scavenger), rank);
            let (a, b) = evo.skills();
            assert_eq!(a.is_some(), rank >= SKILL_A_RANK, "rank {rank}");
            assert_eq!(b.is_some(), rank >= SKILL_B_RANK, "rank {rank}");
        }
        assert_eq!(evo.skills(), (Some(Skill::Stealth), Some(Skill::Trap)));
    }

    #[test]
    fn spending_needs_a_point() {
        let mut evo = Evolution::default();
        assert!(!evo.spend(Perk::Claws));
        evo.add_xp(xp_to_reach(3));
        assert!(evo.spend(Perk::Claws));
        assert!(evo.spend(Perk::Claws));
        assert!(!evo.spend(Perk::Claws));
        assert_eq!((evo.points, evo.rank(Perk::Claws)), (0, 2));
    }

    #[test]
    fn refund_reset_and_restore() {
        let mut evo = Evolution::default();
        evo.add_xp(xp_to_reach(6));
        for perk in [Perk::Overclock, Perk::Claws, Perk::Claws, Perk::Overclock] {
            assert!(evo.spend(perk));
        }
        let (ranks, first) = (evo.ranks, evo.first_pick);
        assert_eq!(evo.lead_path(), Some(EvoPath::Cyborg));
        assert!(evo.refund(Perk::Overclock));
        assert!(evo.refund(Perk::Overclock));
        assert!(!evo.refund(Perk::Overclock));
        assert_eq!(evo.first_pick[EvoPath::Cyborg.index()], 0);
        assert_eq!((evo.points, evo.lead_path()), (3, Some(EvoPath::Mutant)));
        evo.reset();
        assert_eq!(
            (evo.points, evo.total_ranks(), evo.lead_path()),
            (5, 0, None)
        );
        assert!(evo.restore(ranks, first));
        assert_eq!((evo.points, evo.lead_path()), (1, Some(EvoPath::Cyborg)));
        assert!(!evo.restore([MAX_RANK; 12], [0; 3]));
    }

    #[test]
    fn lead_path_tie_goes_to_first_picked() {
        let mut evo = Evolution::default();
        for perk in [Perk::Overclock, Perk::Claws, Perk::Claws, Perk::Overclock] {
            pick_perk(&mut evo, perk);
        }
        assert_eq!(evo.lead_path(), Some(EvoPath::Cyborg));
        pick_perk(&mut evo, Perk::Constitution);
        assert_eq!(evo.lead_path(), Some(EvoPath::Mutant));
    }

    #[test]
    fn cannot_spend_on_a_maxed_perk() {
        let mut evo = Evolution::default();
        for _ in 0..3 {
            pick_perk(&mut evo, Perk::Claws);
        }
        evo.points = 1;
        assert!(!evo.spend(Perk::Claws));
        assert_eq!(evo.points, 1);
        assert!(evo.open_perks().all(|p| p != Perk::Claws));
    }

    #[test]
    fn perk_numbers() {
        let mut evo = Evolution::default();
        for _ in 0..3 {
            pick_perk(&mut evo, Perk::Overclock);
            pick_perk(&mut evo, Perk::AmmoRecycler);
            pick_perk(&mut evo, Perk::Lockpick);
        }
        assert_eq!(evo.fire_cooldown(64), 49);
        assert_eq!(evo.fire_cooldown(7), 5);
        assert_eq!(evo.fire_cooldown(5), 4);
        assert_eq!(evo.fire_cooldown(3), 3);
        assert_eq!(evo.ammo_cap(AmmoKind::Bullet), 216);
        assert_eq!(evo.ammo_per_kill(), 6);
        assert_eq!(evo.crack_ticks(), 85);
    }

    #[test]
    fn speed_mult_is_capped() {
        let mut evo = Evolution::default();
        for _ in 0..3 {
            pick_perk(&mut evo, Perk::LightFoot);
        }
        let mut skills = SkillState {
            skill_b: Some(Skill::Berserk),
            active_b: Skill::Berserk.duration(),
            ..default()
        };
        assert_eq!(evolution_speed_mult(&evo, &skills), MOVE_MULT_CAP);
        skills.active_b = BERSERK_AFTERMATH_TICKS;
        assert!((evolution_speed_mult(&evo, &skills) - 1.15 * 0.8).abs() < 1e-6);
        skills.skill_a = Some(Skill::AcidBreath);
        skills.active_a = 10;
        skills.active_b = 0;
        assert!((evolution_speed_mult(&evo, &skills) - 1.15 * 0.6).abs() < 1e-6);
    }

    /// Every Chinese string of this module must be in the bundled UI font (GB2312 + the
    /// characters used in the sources), or the HUD shows tofu.
    #[test]
    fn names_are_in_the_ui_font() {
        let font = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../client/assets/NotoSansSC-Subset.ttf"
        ))
        .expect("bundled UI font");
        let mut text = String::new();
        for path in EvoPath::ALL {
            text += path.name();
        }
        for perk in Perk::ALL {
            text += perk.name();
            text += perk.desc();
        }
        for skill in Skill::ALL {
            text += skill.name();
            text += skill.desc();
        }
        let missing: String = text
            .chars()
            .filter(|c| !c.is_ascii() && !font_has(&font, *c as u32))
            .collect();
        assert!(missing.is_empty(), "not in the UI font: {missing}");
    }

    fn be16(b: &[u8], at: usize) -> u32 {
        u16::from_be_bytes([b[at], b[at + 1]]) as u32
    }

    fn be32(b: &[u8], at: usize) -> u32 {
        u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
    }

    /// Minimal TrueType `cmap` lookup (formats 4 and 12).
    fn font_has(font: &[u8], c: u32) -> bool {
        let tables = be16(font, 4) as usize;
        let cmap = (0..tables)
            .map(|i| 12 + 16 * i)
            .find(|rec| &font[*rec..*rec + 4] == b"cmap")
            .map(|rec| be32(font, rec + 8) as usize)
            .expect("cmap table");
        let subtables = be16(font, cmap + 2) as usize;
        (0..subtables).any(|i| {
            let rec = cmap + 4 + 8 * i;
            let sub = cmap + be32(font, rec + 4) as usize;
            match be16(font, sub) {
                12 => {
                    let groups = be32(font, sub + 12) as usize;
                    (0..groups).any(|g| {
                        let at = sub + 16 + 12 * g;
                        be32(font, at) <= c && c <= be32(font, at + 4)
                    })
                }
                4 if c <= 0xFFFF => {
                    let segs = be16(font, sub + 6) as usize / 2;
                    let ends = sub + 14;
                    let starts = ends + 2 * segs + 2;
                    let deltas = starts + 2 * segs;
                    let offsets = deltas + 2 * segs;
                    (0..segs).any(|s| {
                        let (start, end) = (be16(font, starts + 2 * s), be16(font, ends + 2 * s));
                        if c < start || c > end {
                            return false;
                        }
                        let delta = be16(font, deltas + 2 * s);
                        let range = be16(font, offsets + 2 * s) as usize;
                        let glyph = if range == 0 {
                            (c + delta) & 0xFFFF
                        } else {
                            let at = offsets + 2 * s + range + 2 * (c - start) as usize;
                            match be16(font, at) {
                                0 => 0,
                                g => (g + delta) & 0xFFFF,
                            }
                        };
                        glyph != 0
                    })
                }
                _ => false,
            }
        })
    }
}

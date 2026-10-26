//! Everything that crosses the wire: inputs, replicated components, messages and channels.

use bevy::curve::Curve;
use bevy::ecs::entity::MapEntities;
use bevy::prelude::*;
use lightyear::prelude::input::native::InputPlugin;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

use crate::evolution::{Evolution, Skill, SkillState};
use crate::items::{ArmorKind, Item, WeaponKind};
use crate::map::PropKind;

/// One tick of player intent. Sent redundantly every tick (UDP, unreliable) so a lost
/// packet never drops a shot; the server replays the newest input it has if one is late.
#[derive(Serialize, Deserialize, Debug, Default, PartialEq, Clone, Reflect)]
pub struct PlayerInput {
    pub move_x: i8,
    pub move_y: i8,
    /// Normalized aim direction in world space.
    pub aim: Vec2,
    pub fire: bool,
    pub dash: bool,
    pub interact: bool,
    pub throw: bool,
    /// Perk pick: 1..=3 chooses that slot of the current offer while held, 0 = none.
    /// The server acts on the change, so holding a key picks once.
    pub pick: u8,
    /// Skill A (R) / skill B (T) held.
    pub skill_a: bool,
    pub skill_b: bool,
}

impl PlayerInput {
    pub fn move_dir(&self) -> Vec2 {
        Vec2::new(self.move_x as f32, self.move_y as f32).normalize_or_zero()
    }
}

impl MapEntities for PlayerInput {
    fn map_entities<M: EntityMapper>(&mut self, _entity_mapper: &mut M) {}
}

/// Identity of a combatant (human or server bot).
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Player {
    pub client_id: u64,
    pub name: String,
    pub color: [f32; 3],
    pub bot: bool,
}

/// World position. Predicted for the local player, interpolated for everyone else.
#[derive(
    Component,
    Serialize,
    Deserialize,
    Clone,
    Copy,
    Debug,
    PartialEq,
    Reflect,
    Deref,
    DerefMut,
    Default,
)]
pub struct Pos(pub Vec2);

impl Ease for Pos {
    fn interpolating_curve_unbounded(start: Self, end: Self) -> impl Curve<Self> {
        FunctionCurve::new(Interval::UNIT, move |t| Pos(start.0.lerp(end.0, t)))
    }
}

/// Lets a misprediction be blended away over a few frames instead of snapping.
impl Diffable for Pos {
    fn base_value() -> Self {
        Pos(Vec2::ZERO)
    }

    fn diff(&self, new: &Self) -> Self {
        Pos(new.0 - self.0)
    }

    fn apply_diff(&mut self, delta: &Self) {
        self.0 += delta.0;
    }
}

/// Facing direction, used for rendering remote players.
#[derive(
    Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Reflect, Deref, Default,
)]
pub struct Aim(pub Vec2);

impl Ease for Aim {
    fn interpolating_curve_unbounded(start: Self, end: Self) -> impl Curve<Self> {
        FunctionCurve::new(Interval::UNIT, move |t| {
            Aim(start.0.lerp(end.0, t).normalize_or(end.0))
        })
    }
}

/// Dash state. Predicted, so the local dash feels instant.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Reflect, Default)]
pub struct Dash {
    pub cooldown: u16,
    pub active: u16,
    pub dir: Vec2,
}

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Reflect)]
pub struct Vitals {
    pub hp: f32,
    /// Grows with 强化体质; use instead of `sim::MAX_HP` for combatants.
    pub max_hp: f32,
    pub armor: ArmorKind,
    pub armor_points: f32,
    pub alive: bool,
    /// Ticks until respawn while dead.
    pub respawn_in: u16,
    /// Ticks of EMP-induced slowdown left.
    pub emp_slow: u16,
    /// Ticks of spawn protection left; ends early when you fire.
    pub shield: u16,
    /// Ticks of 隐身 left: enemies only see you up close.
    pub stealth: u16,
    /// Ticks this combatant is revealed to everyone (sprung a trap).
    pub revealed: u16,
}

impl Default for Vitals {
    fn default() -> Self {
        Self {
            hp: crate::sim::MAX_HP,
            max_hp: crate::sim::MAX_HP,
            armor: ArmorKind::None,
            armor_points: 0.0,
            alive: true,
            respawn_in: 0,
            emp_slow: 0,
            shield: 0,
            stealth: 0,
            revealed: 0,
        }
    }
}

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Reflect, Default)]
pub struct Loadout {
    pub weapon: WeaponKind,
    pub ammo: u16,
    pub emps: u8,
}

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Reflect)]
pub struct Stats {
    pub score: u32,
    pub kills: u32,
    pub deaths: u32,
    pub streak: u32,
    /// Evolution level, mirrored here for the scoreboard.
    pub level: u8,
}

impl Default for Stats {
    fn default() -> Self {
        Self {
            score: 0,
            kills: 0,
            deaths: 0,
            streak: 0,
            level: 1,
        }
    }
}

impl Stats {
    /// A kill streak puts a price on your head: you are revealed and worth more.
    pub fn has_bounty(&self) -> bool {
        self.streak >= crate::sim::BOUNTY_STREAK
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Reflect)]
pub enum ProjectileKind {
    Shot(WeaponKind),
    Emp,
    /// Slow glob from a spore pod; part of a bullet-hell ring. The behemoth's radiation
    /// ring uses it too.
    Spore,
    /// Acid glob from a spitter or the behemoth; leaves an acid [`Puddle`] where it lands.
    Acid,
    /// Energy bolt from a drone.
    Bolt,
}

/// Wasteland fauna. They hurt whoever is closest — humans and bots alike — and drop loot
/// when killed, giving everyone something else to shoot at.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, Reflect)]
pub enum MonsterKind {
    /// Skittering mutant roach: wanders, charges anyone nearby, bites on contact.
    Roach,
    /// Rooted spore pod: never moves, sprays rings of slow globs in a spiral.
    SporePod,
    /// Feral dog, hunts in packs of three that flank their prey.
    Dog,
    /// Bloated ghoul: runs at you, arms ([`MonsterState::fuse`]) and explodes.
    Ghoul,
    /// Keeps its distance and lobs acid globs that leave puddles.
    Spitter,
    /// Slow armored crawler: hits from the front (within [`CRAWLER_SHELL_COS`]) barely hurt.
    Crawler,
    /// Armed human raider (T2 gun) guarding its block in squads of two.
    Raider,
    /// Hovering drone: flies over walls, shoots bolts.
    Drone,
    /// 辐射巨兽: the once-per-match boss.
    Behemoth,
}

impl MonsterKind {
    pub const ALL: [MonsterKind; 9] = [
        MonsterKind::Roach,
        MonsterKind::SporePod,
        MonsterKind::Dog,
        MonsterKind::Ghoul,
        MonsterKind::Spitter,
        MonsterKind::Crawler,
        MonsterKind::Raider,
        MonsterKind::Drone,
        MonsterKind::Behemoth,
    ];

    pub fn name(self) -> &'static str {
        match self {
            MonsterKind::Roach => "辐射蟑螂",
            MonsterKind::SporePod => "孢子囊",
            MonsterKind::Dog => "野狗",
            MonsterKind::Ghoul => "爆裂尸鬼",
            MonsterKind::Spitter => "酸液喷射者",
            MonsterKind::Crawler => "装甲爬行者",
            MonsterKind::Raider => "掠夺者",
            MonsterKind::Drone => "无人机",
            MonsterKind::Behemoth => "辐射巨兽",
        }
    }

    /// Base hit points. Elites have [`ELITE_HP_MULT`] times this; the behemoth scales with
    /// the lobby (see `Monster`'s replicated `Vitals` for the real value).
    pub fn max_hp(self) -> f32 {
        match self {
            MonsterKind::Roach => 45.0,
            MonsterKind::SporePod => 110.0,
            MonsterKind::Dog => 35.0,
            MonsterKind::Ghoul => 60.0,
            MonsterKind::Spitter => 80.0,
            MonsterKind::Crawler => 260.0,
            MonsterKind::Raider => 100.0,
            MonsterKind::Drone => 40.0,
            MonsterKind::Behemoth => BEHEMOTH_BASE_HP,
        }
    }

    pub fn radius(self) -> f32 {
        match self {
            MonsterKind::Roach => 18.0,
            MonsterKind::SporePod => 28.0,
            MonsterKind::Dog => 16.0,
            MonsterKind::Ghoul => 20.0,
            MonsterKind::Spitter => 22.0,
            MonsterKind::Crawler => 30.0,
            MonsterKind::Raider => crate::sim::PLAYER_RADIUS,
            MonsterKind::Drone => 14.0,
            MonsterKind::Behemoth => 70.0,
        }
    }

    /// Experience for the kill (the behemoth's is split by damage share).
    pub fn xp(self) -> u32 {
        match self {
            MonsterKind::Roach => 25,
            MonsterKind::SporePod => 60,
            MonsterKind::Dog => 25,
            MonsterKind::Ghoul => 40,
            MonsterKind::Spitter => 70,
            MonsterKind::Crawler => 110,
            MonsterKind::Raider => 120,
            MonsterKind::Drone => 45,
            MonsterKind::Behemoth => 600,
        }
    }

    /// Scoreboard points for the kill. Only elites (1) and the behemoth (3, to the top
    /// damage dealer) are worth any, so farming fauna can't win a match.
    pub fn score(self) -> u32 {
        match self {
            MonsterKind::Behemoth => 3,
            _ => 0,
        }
    }

    /// Whether this kind flies over walls and obstacles.
    pub fn flies(self) -> bool {
        self == MonsterKind::Drone
    }
}

pub const ELITE_HP_MULT: f32 = 2.2;
pub const ELITE_SPEED_MULT: f32 = 1.15;
pub const ELITE_DAMAGE_MULT: f32 = 1.4;
pub const ELITE_XP_MULT: u32 = 3;
/// Behemoth HP before the per-player bonus.
pub const BEHEMOTH_BASE_HP: f32 = 1800.0;
/// Radius of the behemoth's ground slam (telegraphed by [`MonsterAction::Slam`]).
pub const BEHEMOTH_SLAM_RADIUS: f32 = 140.0;
/// Radius of a ghoul's blast.
pub const GHOUL_BLAST_RADIUS: f32 = 110.0;
/// A crawler's shell covers hits whose direction is within ±70° of its facing.
pub const CRAWLER_SHELL_COS: f32 = 0.342_020_14;

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Reflect)]
pub struct Monster {
    pub kind: MonsterKind,
    /// Elite variant: tougher, faster, hits harder, better loot (draw it bigger/glowing).
    pub elite: bool,
}

impl Monster {
    pub fn xp(&self) -> u32 {
        self.kind.xp() * if self.elite { ELITE_XP_MULT } else { 1 }
    }

    pub fn score(&self) -> u32 {
        if self.elite {
            self.kind.score().max(1)
        } else {
            self.kind.score()
        }
    }

    /// Name for the kill feed.
    pub fn display_name(&self) -> String {
        if self.elite {
            format!("精英{}", self.kind.name())
        } else {
            self.kind.name().to_string()
        }
    }
}

/// What a monster is visibly doing, for telegraphs.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Reflect, Default)]
pub enum MonsterAction {
    #[default]
    Idle,
    /// Behemoth winding up a slam: a ring of [`BEHEMOTH_SLAM_RADIUS`] around it lands when
    /// `fuse` reaches 0.
    Slam,
    /// Behemoth about to charge along `facing` (`fuse` > 0) or charging (`fuse` == 0).
    Charge,
    /// Ghoul armed: explodes ([`GHOUL_BLAST_RADIUS`]) when `fuse` reaches 0.
    Arming,
}

/// Fast-changing, replicated monster state. Only written when it changes.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Reflect, Default)]
pub struct MonsterState {
    /// Facing angle in radians (quantized server-side, so it changes in small steps).
    pub facing: f32,
    /// Behemoth phase 1..=3; 0 for everyone else.
    pub phase: u8,
    pub action: MonsterAction,
    /// Ticks until the current `action` resolves (64 Hz).
    pub fuse: u16,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub enum PuddleKind {
    /// Left by acid globs.
    Acid,
    /// Trailed by the behemoth in its last phase.
    Radiation,
}

/// A damaging ground zone. Never changes after it spawns; the server despawns it.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Reflect)]
pub struct Puddle {
    pub kind: PuddleKind,
    pub radius: f32,
    /// Lifetime in ticks at spawn (count it down locally to fade it out).
    pub ttl: u16,
}

/// A crate, barrel or safe placed by the map generator. Hit points live in its `Vitals`.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Reflect)]
pub struct Prop {
    pub kind: PropKind,
    /// Tier of the loot inside.
    pub tier: u8,
    /// Safe cracking progress, 0..=100.
    pub progress: u8,
    /// A cracked safe stays open (and empty) for a while.
    pub open: bool,
}

/// 自动炮塔: a damageable sentry (hit points in `Vitals`, facing in `Aim`).
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Reflect)]
pub struct Turret {
    /// Client id of the combatant who deployed it.
    pub owner: u64,
}

/// 陷阱: replicated only to its owner (enemies never learn where it is).
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Reflect)]
pub struct Trap {
    pub owner: u64,
}

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Reflect)]
pub struct Projectile {
    pub kind: ProjectileKind,
    pub vel: Vec2,
}

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Reflect)]
pub struct Pickup {
    pub item: Item,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Reflect, Default)]
pub enum Phase {
    #[default]
    Warmup,
    Playing,
    Intermission,
}

/// Singleton describing the match clock.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Default)]
pub struct MatchState {
    pub phase: Phase,
    /// Ticks left in the current phase.
    pub ticks_left: u32,
    /// Ticks until the vault opens and spits out apex tech.
    pub vault_in: u32,
    pub winner: Option<String>,
    /// Seed of this match's arena layout; clients generate the same map from it.
    pub map_seed: u64,
    /// Ticks until the behemoth appears while its countdown runs, otherwise 0.
    pub boss_eta: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum GameEvent {
    Kill {
        killer: String,
        victim: String,
        /// Weapon name, or how the wasteland did it ("油桶爆炸", "辐射", a bite).
        how: String,
        bounty: bool,
    },
    Hit {
        pos: Vec2,
        amount: f32,
        armor: bool,
        /// Client id of the attacker, so they get a hit marker.
        attacker: u64,
    },
    Melee {
        pos: Vec2,
        dir: Vec2,
    },
    Boom {
        pos: Vec2,
        radius: f32,
        /// A fireball (exploding barrel) rather than an electric pulse.
        fire: bool,
    },
    Announce(String),
    /// Behemoth health, about once a second while it is alive (and on phase changes).
    BossHp {
        hp: f32,
        max: f32,
        phase: u8,
    },
    /// The behemoth is gone: `Some(name)` of whoever got the credit, `None` if it left.
    BossDown {
        killer: Option<String>,
    },
    /// Sent only to the combatant who earned it.
    XpGain {
        amount: u32,
        pos: Vec2,
    },
    LevelUp {
        client: u64,
        level: u8,
    },
    SkillUsed {
        client: u64,
        skill: Skill,
        pos: Vec2,
        dir: Vec2,
    },
    /// A trap went off (shown to everyone, since the victim now knows).
    TrapSprung {
        pos: Vec2,
    },
}

/// Kill feed and announcements: must arrive.
pub struct ReliableChannel;
/// Cosmetic feedback (hits, swings, explosions): latest-wins, never retransmitted.
pub struct FastChannel;

pub struct ProtocolPlugin;

impl Plugin for ProtocolPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(InputPlugin::<PlayerInput> {
            config: lightyear::input::config::InputConfig {
                // Send the client's interpolation delay so the server can rewind
                // targets to what the shooter actually saw.
                lag_compensation: true,
                ..default()
            },
        });

        app.component::<Player>().replicate();
        app.component::<Pos>()
            .replicate()
            .predict()
            .add_linear_interpolation()
            .add_correction();
        app.component::<Aim>()
            .replicate()
            .add_linear_interpolation();
        app.component::<Dash>().replicate().predict();
        app.component::<Vitals>().replicate();
        app.component::<Loadout>().replicate();
        app.component::<Stats>().replicate();
        app.component::<Evolution>().replicate();
        app.component::<SkillState>().replicate();
        app.component::<Turret>().replicate();
        app.component::<Trap>().replicate();
        app.component::<Projectile>().replicate();
        app.component::<Pickup>().replicate();
        app.component::<Monster>().replicate();
        app.component::<MonsterState>().replicate();
        app.component::<Puddle>().replicate();
        app.component::<Prop>().replicate();
        app.component::<MatchState>().replicate();

        app.register_message::<GameEvent>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_channel::<ReliableChannel>(ChannelSettings {
            mode: ChannelMode::OrderedReliable(ReliableSettings::default()),
            ..default()
        })
        .add_direction(NetworkDirection::ServerToClient);
        app.add_channel::<FastChannel>(ChannelSettings {
            mode: ChannelMode::UnorderedUnreliable,
            ..default()
        })
        .add_direction(NetworkDirection::ServerToClient);
    }
}

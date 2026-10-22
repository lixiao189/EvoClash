//! Everything that crosses the wire: inputs, replicated components, messages and channels.

use bevy::curve::Curve;
use bevy::ecs::entity::MapEntities;
use bevy::prelude::*;
use lightyear::prelude::input::native::InputPlugin;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

use crate::items::{ArmorKind, Item, WeaponKind};

// ---------------------------------------------------------------- inputs

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
}

impl PlayerInput {
    pub fn move_dir(&self) -> Vec2 {
        Vec2::new(self.move_x as f32, self.move_y as f32).normalize_or_zero()
    }
}

impl MapEntities for PlayerInput {
    fn map_entities<M: EntityMapper>(&mut self, _entity_mapper: &mut M) {}
}

// ---------------------------------------------------------------- components

/// Identity of a combatant (human or server bot).
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Player {
    pub client_id: u64,
    pub name: String,
    pub color: [f32; 3],
    pub bot: bool,
}

/// World position. Predicted for the local player, interpolated for everyone else.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Reflect, Deref, DerefMut, Default)]
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
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Reflect, Deref, Default)]
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
    pub armor: ArmorKind,
    pub armor_points: f32,
    pub alive: bool,
    /// Ticks until respawn while dead.
    pub respawn_in: u16,
    /// Ticks of EMP-induced slowdown left.
    pub emp_slow: u16,
}

impl Default for Vitals {
    fn default() -> Self {
        Self {
            hp: crate::sim::MAX_HP,
            armor: ArmorKind::None,
            armor_points: 0.0,
            alive: true,
            respawn_in: 0,
            emp_slow: 0,
        }
    }
}

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Reflect, Default)]
pub struct Loadout {
    pub weapon: WeaponKind,
    pub ammo: u16,
    pub emps: u8,
}

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Reflect, Default)]
pub struct Stats {
    pub score: u32,
    pub kills: u32,
    pub deaths: u32,
    pub streak: u32,
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
}

// ---------------------------------------------------------------- messages

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum GameEvent {
    Kill {
        killer: String,
        victim: String,
        weapon: WeaponKind,
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
    },
    Announce(String),
}

/// Kill feed and announcements: must arrive.
pub struct ReliableChannel;
/// Cosmetic feedback (hits, swings, explosions): latest-wins, never retransmitted.
pub struct FastChannel;

// ---------------------------------------------------------------- registration

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
        app.component::<Aim>().replicate().add_linear_interpolation();
        app.component::<Dash>().replicate().predict();
        app.component::<Vitals>().replicate();
        app.component::<Loadout>().replicate();
        app.component::<Stats>().replicate();
        app.component::<Projectile>().replicate();
        app.component::<Pickup>().replicate();
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

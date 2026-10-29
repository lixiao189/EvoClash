//! World rendering in a three-quarter view: the ground is seen from above, while people,
//! monsters and walls stand up out of it. Everything upright is depth-sorted by where its
//! feet touch the ground, so walking behind a wall hides you and in front of it doesn't.
//!
//! Collision is unchanged: every position is still the point on the ground under the feet.

use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::image::{ImageLoaderSettings, ImageSampler};
use bevy::prelude::*;
use bevy::render::view::{ColorGrading, ColorGradingGlobal, ColorGradingSection};
use bevy::sprite::{Anchor, SpriteImageMode};
use bevy::window::PrimaryWindow;
use evoclash_game::evolution::{ACID_HALF_ANGLE, ACID_RANGE, Evolution, SHIELD_HP, SkillState};
use evoclash_game::items::{WeaponKind, tier_color};
use evoclash_game::map::{
    ARENA_HALF, ArenaMap, BLOCK, Biome, MARGIN, PropKind, Roof, VAULT_CENTER, WallStyle,
};
use evoclash_game::protocol::*;
use evoclash_game::sim::{Arena, PLAYER_RADIUS};
use lightyear::prelude::input::native::ActionState;
use rand::Rng;

use crate::assets::PendingAssets;
use crate::input::{CursorWorld, GUN_HEIGHT, LocalPlayer, Selection};
use crate::input_mode::SyncInputMode;
use crate::lpc;
use crate::predict::TickSamples;
use crate::textures::{self, Facing, PIXEL, Textures};

mod camera;
mod monsters;
mod overlays;
mod pickups;
mod players;
mod projectiles;
mod props;
mod scenery;
mod skills;
mod smoothing;
#[cfg(test)]
mod tests;

use camera::*;
use lightyear::prelude::InterpolationSystems;
use monsters::*;
use overlays::*;
use pickups::*;
pub use players::*;
pub use projectiles::*;
use props::*;
use scenery::*;
use skills::*;
use smoothing::smooth_remotes;

const Z_FLOOR: f32 = -10.0;
const Z_DECAL: f32 = -8.0;
const Z_PICKUP: f32 = -6.0;
const Z_SHADOW: f32 = -5.0;
const Z_PROJECTILE: f32 = 4.0;
/// Relative to the actor it labels, which sits around z = 2.
const Z_LABEL: f32 = 3.0;
const SPLAT_SECS: f32 = 8.0;
/// How tall brick walls stand, in world pixels. Roofs sit on top of them.
const WALL_HEIGHT: f32 = lpc::WALL_FACE_HEIGHTS[0] as f32;
/// Road width between city blocks.
const STREET: f32 = 2.0 * MARGIN;
/// Roof opacity while you're inside the building.
const ROOF_SEE_THROUGH: f32 = 0.12;
/// Within one actor, parts are layered by tiny z steps so they never cross another actor.
const Z_STEP: f32 = 0.00001;
/// Feet of the survivor sprite sit this far above its texture's bottom edge (world pixels).
const FOOT_INSET: f32 = 2.0;
/// Top of a player's head above the feet.
const SURVIVOR_HEIGHT: f32 = lpc::HEIGHT;
/// Others under 隐身 are drawn this faint (when the server lets us see them at all).
const STEALTH_ALPHA: f32 = 0.15;
/// You see yourself half-faded while hidden.
const STEALTH_SELF_ALPHA: f32 = 0.45;
/// Distance walked per animation frame.
const STRIDE_PX: f32 = 22.0;
/// World pixels per screen pixel; below 1 zooms in.
const CAMERA_SCALE: f32 = 0.8;
/// Relative odds of each decor piece: grass, bush, boulder, skull, tire.
const DECOR_WEIGHTS: [u32; 5] = [36, 20, 26, 6, 12];
/// Elites are drawn this much bigger, in gold.
const ELITE_SCALE: f32 = 1.2;
const ELITE_TINT: Color = Color::srgb(1.0, 0.86, 0.4);
const ELITE_RING: Color = Color::srgb(1.0, 0.78, 0.15);
/// How high a drone's body floats above its shadow.
const DRONE_HOVER: f32 = 26.0;
/// The crawler is seen from above; its center sits this far above its ground point.
const CRAWLER_LIFT: f32 = 22.0;

/// Depth for something standing at ground height `y`: further down the screen is closer.
fn depth(y: f32) -> f32 {
    2.0 - y * 0.001
}

#[derive(Resource, Clone)]
pub struct UiFont(pub Handle<Font>);

impl UiFont {
    pub fn sized(&self, size: f32) -> TextFont {
        TextFont::from_font_size(size).with_font(self.0.clone())
    }
}

/// Decays every frame; effects add to it when something hits hard.
#[derive(Resource, Default)]
pub struct CameraShake(pub f32);

#[derive(Component)]
struct NameLabel;

/// When (`Time::elapsed_secs`) a combatant last fired or swung; drives recoil, the muzzle flash
/// and the swing arc. Set locally on click for us, and from server traffic for everyone else.
#[derive(Component, Clone, Copy)]
pub struct Strike(pub f32);

/// The sprites making up a combatant, animated each frame.
#[derive(Component)]
struct PlayerParts {
    /// The outfit (it changes with the armor worn), and the bandana over it in the player's color.
    body: Entity,
    team: Entity,
    /// A club in front of the body, and the parts of it that swing behind.
    club: Entity,
    club_behind: Entity,
    /// A gun, held at the hip.
    weapon: Entity,
    /// Muzzle flash at the barrel, shown for a moment after each shot.
    flash: Entity,
    shown: Option<WeaponKind>,
    last: Vec2,
    /// Distance walked, in stride frames.
    stride: f32,
    /// When the combatant died, while the fall plays.
    died: Option<f32>,
}

/// LPC map tiles (see `crate::lpc`), drawn at one texture pixel per world pixel.
#[derive(Resource)]
struct LpcMap {
    dirt: Handle<Image>,
    gravel_patch: Handle<Image>,
    sidewalk: Handle<Image>,
    road: Handle<Image>,
    /// Brick, metal and rock wall faces, and the top every wall shows from above.
    wall_faces: [Handle<Image>; 3],
    wall_top: Handle<Image>,
    roof_tar: Handle<Image>,
    roof_concrete: Handle<Image>,
}

/// The LPC sheets players are drawn with (see `crate::lpc`).
#[derive(Resource)]
struct LpcSprites {
    bodies: [Handle<Image>; 4],
    raider: Handle<Image>,
    team: Handle<Image>,
    club: Handle<Image>,
    club_behind: Handle<Image>,
    layout: Handle<TextureAtlasLayout>,
    club_layout: Handle<TextureAtlasLayout>,
    /// Creatures (see `lpc_look`) and their frame grids.
    wolves: [Handle<Image>; 2],
    wolf_layout: Handle<TextureAtlasLayout>,
    ghoul: Handle<Image>,
    ghoul_layout: Handle<TextureAtlasLayout>,
    slime: Handle<Image>,
    slime_layout: Handle<TextureAtlasLayout>,
    golem: Handle<Image>,
    golem_layout: Handle<TextureAtlasLayout>,
    /// Props.
    crate_box: Handle<Image>,
    barrel: Handle<Image>,
    safe: Handle<Image>,
    safe_open: Handle<Image>,
}

#[derive(Component)]
struct PickupIcon {
    phase: f32,
}

#[derive(Component)]
struct MonsterParts {
    body: Entity,
    /// A raider's rifle, held at the hip like a player's gun.
    gun: Option<Entity>,
    /// Distance walked, in stride frames (raiders).
    stride: f32,
    /// Resting height of the body sprite relative to the monster's ground point.
    base_y: f32,
    last: Vec2,
    flip: bool,
    phase: f32,
}

/// Seconds a puddle has existed here; `Puddle::ttl` is only set at spawn.
#[derive(Component, Default)]
struct PuddleAge(f32);

/// Arrow at the screen edge pointing toward the behemoth.
#[derive(Component)]
struct BossArrow;

/// A ground shadow kept on the shadow layer however its parent is depth-sorted.
#[derive(Component)]
struct GroundShadow;

/// Goo left behind by a dead monster; fades out.
#[derive(Component)]
struct Splat {
    age: f32,
}

/// Everything drawn for the current layout; torn down when a new match brings a new one.
#[derive(Component)]
struct Scenery;

/// Roof (or eave) of a building in `Arena::buildings`, and its opacity when you're outside.
#[derive(Component)]
struct RoofOf {
    building: usize,
    alpha: f32,
}

#[derive(Component)]
struct PropBody(Entity);

/// The swivelling gun of a 自动炮塔.
#[derive(Component)]
struct TurretHead(Entity);

/// Whose eyes the roofs use. `building` is the one that combatant is inside, if any.
/// Anyone inside a different building is hidden under its roof.
/// `watch` is the followed combatant's `client_id` while spectating, otherwise `None`.
#[derive(Resource, Default)]
pub struct Indoors {
    pub building: Option<usize>,
    pub watch: Option<u64>,
}

impl Indoors {
    pub fn conceals(&self, arena: &ArenaMap, p: Vec2) -> bool {
        arena
            .building_at(p)
            .is_some_and(|b| Some(b) != self.building)
    }
}

/// Parts are authored at half resolution.
fn pixel_sprite(image: Handle<Image>) -> (Sprite, Transform) {
    (
        Sprite::from_image(image),
        Transform::from_scale(Vec3::new(PIXEL, PIXEL, 1.0)),
    )
}

/// A procedural texture repeated over `size`.
fn tiled(image: Handle<Image>, size: Vec2) -> Sprite {
    tiled_at(image, size, PIXEL)
}

/// `image` repeated over `size`, `scale` world pixels per texture pixel.
fn tiled_at(image: Handle<Image>, size: Vec2, scale: f32) -> Sprite {
    Sprite {
        image,
        custom_size: Some(size),
        image_mode: SpriteImageMode::Tiled {
            tile_x: true,
            tile_y: true,
            stretch_value: scale,
        },
        ..default()
    }
}

fn shadow(textures: &Textures, size: Vec2) -> impl Bundle {
    (
        GroundShadow,
        Sprite {
            image: textures.shadow.clone(),
            custom_size: Some(size),
            color: Color::WHITE.with_alpha(0.6),
            ..default()
        },
        Transform::from_xyz(0.0, 0.0, 0.0),
    )
}

pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ClearColor(Color::srgb(0.1, 0.06, 0.08)));
        app.init_resource::<CameraShake>()
            .init_resource::<Indoors>();
        app.add_systems(PreStartup, (load_font, load_textures, load_lpc));
        app.add_systems(Startup, spawn_camera);
        app.add_observer(dress_player);
        app.add_observer(dress_pickup);
        app.add_observer(dress_monster);
        app.add_observer(dress_prop);
        app.add_observer(dress_projectile);
        app.add_observer(dress_turret);
        app.add_observer(dress_trap);
        app.add_observer(splat_monster);
        app.add_systems(
            Update,
            (
                // After lightyear writes interpolated `Pos`, and before every reader in this
                // chain (players, monsters, overlays, camera). The chain is the ordering.
                smooth_remotes.after(InterpolationSystems::Interpolate),
                adopt_layout,
                build_scenery,
                update_roofs,
                (sync_players, strike_on_shot, pose_players, tint_players)
                    .chain()
                    .after(SyncInputMode),
                sync_props,
                sync_turrets,
                sync_traps,
                sync_pickups,
                bob_pickups,
                sync_monsters,
                pin_shadows,
                sync_projectiles,
                fade_splats,
                age_puddles,
                follow_camera.after(SyncInputMode),
                boss_arrow,
                update_cursor,
                draw_overlays,
            )
                .chain(),
        );
    }
}

/// Text includes Chinese, which the built-in font lacks, and browsers can't hand us system
/// fonts: Noto Sans SC is compiled in, subset to GB2312 plus the sources' characters
/// (see assets/README.md).
fn load_font(mut commands: Commands, mut fonts: ResMut<Assets<Font>>) {
    const NOTO_SANS_SC: &[u8] = include_bytes!("../../assets/NotoSansSC-Subset.ttf");
    let handle = fonts.add(Font::from_bytes(NOTO_SANS_SC.to_vec()));
    commands.insert_resource(UiFont(handle));
}

fn load_textures(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    commands.insert_resource(textures::build(&mut images));
}

/// Start fetching the player sheets and map tiles; the session waits for them (`PendingAssets`).
fn load_lpc(
    mut commands: Commands,
    server: Res<AssetServer>,
    mut layouts: ResMut<Assets<TextureAtlasLayout>>,
    mut pending: ResMut<PendingAssets>,
) {
    let mut load = |path: &'static str| {
        let handle: Handle<Image> = server
            .load_builder()
            .with_settings(|settings: &mut ImageLoaderSettings| {
                settings.sampler = ImageSampler::nearest();
            })
            .load(path);
        pending.add(handle.clone());
        handle
    };
    let club_layout = layouts.add(TextureAtlasLayout::from_grid(
        UVec2::splat(lpc::CLUB_CELL),
        lpc::CLUB_COLUMNS,
        lpc::ROWS,
        None,
        None,
    ));
    // 64 px frames, four facings.
    let mut grid = |columns: u32| {
        layouts.add(TextureAtlasLayout::from_grid(
            UVec2::splat(lpc::CELL),
            columns,
            lpc::ROWS,
            None,
            None,
        ))
    };
    let sprites = LpcSprites {
        bodies: lpc::BODY_FILES.map(&mut load),
        raider: load(lpc::RAIDER_FILE),
        team: load(lpc::TEAM_FILE),
        club: load(lpc::CLUB_FILE),
        club_behind: load(lpc::CLUB_BEHIND_FILE),
        layout: grid(lpc::COLUMNS),
        club_layout,
        wolves: lpc::WOLF_FILES.map(&mut load),
        wolf_layout: grid(lpc::WOLF_COLUMNS),
        ghoul: load(lpc::GHOUL_FILE),
        ghoul_layout: grid(lpc::GHOUL_COLUMNS),
        slime: load(lpc::SLIME_FILE),
        slime_layout: grid(lpc::SLIME_COLUMNS),
        golem: load(lpc::GOLEM_FILE),
        golem_layout: grid(lpc::GOLEM_COLUMNS),
        crate_box: load(lpc::CRATE_FILE),
        barrel: load(lpc::BARREL_FILE),
        safe: load(lpc::SAFE_FILE),
        safe_open: load(lpc::SAFE_OPEN_FILE),
    };
    commands.insert_resource(sprites);
    let map = LpcMap {
        dirt: load(lpc::GROUND_DIRT),
        gravel_patch: load(lpc::GRAVEL_PATCH),
        sidewalk: load(lpc::SIDEWALK),
        road: load(lpc::ROAD),
        wall_faces: lpc::WALL_FACES.map(&mut load),
        wall_top: load(lpc::WALL_TOP),
        roof_tar: load(lpc::ROOF_TAR),
        roof_concrete: load(lpc::ROOF_CONCRETE),
    };
    commands.insert_resource(map);
}

fn spawn_camera(mut commands: Commands) {
    commands.spawn((
        Camera2d,
        // A dusty grade over everything: a little warmer, a little washed out. Color grading
        // runs in the tonemapping pass, which `Tonemapping::None` skips.
        Tonemapping::Linear,
        ColorGrading {
            global: ColorGradingGlobal {
                temperature: 0.12,
                post_saturation: 0.82,
                ..default()
            },
            midtones: ColorGradingSection {
                contrast: 1.06,
                ..default()
            },
            ..default()
        },
        // A little closer than 1:1 so the pixel art reads, while still showing a wide field.
        Projection::Orthographic(OrthographicProjection {
            scale: CAMERA_SCALE,
            ..OrthographicProjection::default_2d()
        }),
        Transform::from_xyz(0.0, 0.0, 100.0),
    ));
}

/// The server picks a new layout every match; regenerate ours from the replicated seed.
fn adopt_layout(state: Query<&MatchState>, mut arena: ResMut<Arena>) {
    let Some(seed) = state.iter().next().map(|s| s.map_seed) else {
        return;
    };
    if arena.seed != seed {
        arena.0 = ArenaMap::generate(seed);
    }
}

/// Shadows are children of depth-sorted actors but belong on the ground layer.
fn pin_shadows(
    parents: Query<&Transform, Without<GroundShadow>>,
    mut shadows: Query<(&ChildOf, &mut Transform), With<GroundShadow>>,
) {
    for (child_of, mut transform) in &mut shadows {
        if let Ok(parent) = parents.get(child_of.parent()) {
            transform.translation.z = Z_SHADOW - parent.translation.z;
        }
    }
}

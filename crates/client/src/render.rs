//! World rendering in a three-quarter view: the ground is seen from above, while people,
//! monsters and walls stand up out of it. Everything upright is depth-sorted by where its
//! feet touch the ground, so walking behind a wall hides you and in front of it doesn't.
//!
//! Collision is unchanged: every position is still the point on the ground under the feet.

use bevy::prelude::*;
use bevy::sprite::{Anchor, SpriteImageMode};
use bevy::window::PrimaryWindow;
use evoclash_game::items::{WeaponKind, tier_color};
use evoclash_game::map::{
    ARENA_HALF, ArenaMap, BLOCK, Biome, MARGIN, PropKind, VAULT_CENTER, WallStyle,
};
use evoclash_game::protocol::*;
use evoclash_game::sim::{Arena, MAX_HP, PLAYER_RADIUS};
use rand::Rng;

use crate::input::{CursorWorld, GUN_HEIGHT, LocalPlayer};
use crate::predict::TickSamples;
use crate::textures::{self, Facing, PIXEL, SURVIVOR, Textures, WallSprites};

const Z_FLOOR: f32 = -10.0;
const Z_DECAL: f32 = -8.0;
const Z_PICKUP: f32 = -6.0;
const Z_SHADOW: f32 = -5.0;
const Z_PROJECTILE: f32 = 4.0;
/// Relative to the actor it labels, which sits around z = 2.
const Z_LABEL: f32 = 3.0;
const SPLAT_SECS: f32 = 8.0;
/// How tall brick walls stand, in world pixels. Roofs sit on top of them.
const WALL_HEIGHT: f32 = 40.0;
/// Road width between city blocks.
const STREET: f32 = 2.0 * MARGIN;
/// Roof opacity while you're inside the building.
const ROOF_SEE_THROUGH: f32 = 0.12;
/// Within one actor, parts are layered by tiny z steps so they never cross another actor.
const Z_STEP: f32 = 0.00001;
/// Feet of the survivor sprite sit this far above its texture's bottom edge (world pixels).
const FOOT_INSET: f32 = 2.0;
const SURVIVOR_HEIGHT: f32 = SURVIVOR.y as f32 * PIXEL;
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

/// The sprites making up a combatant, animated each frame.
#[derive(Component)]
struct PlayerParts {
    jacket: Entity,
    gear: Entity,
    weapon: Entity,
    /// Sleeve from the shoulder to the grip, tinted like the jacket.
    arm: Entity,
    shown: Option<WeaponKind>,
    last: Vec2,
    /// Distance walked, in animation frames.
    stride: f32,
}

#[derive(Component)]
struct PickupIcon {
    phase: f32,
}

#[derive(Component)]
struct MonsterParts {
    body: Entity,
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

/// The building the local player is standing in, if any. Anyone inside a different building
/// is hidden under its roof.
#[derive(Resource, Default)]
pub struct Indoors(pub Option<usize>);

impl Indoors {
    pub fn conceals(&self, arena: &ArenaMap, p: Vec2) -> bool {
        arena.building_at(p).is_some_and(|b| Some(b) != self.0)
    }
}

/// Parts are authored at half resolution.
fn pixel_sprite(image: Handle<Image>) -> (Sprite, Transform) {
    (
        Sprite::from_image(image),
        Transform::from_scale(Vec3::new(PIXEL, PIXEL, 1.0)),
    )
}

fn tiled(image: Handle<Image>, size: Vec2) -> Sprite {
    Sprite {
        image,
        custom_size: Some(size),
        image_mode: SpriteImageMode::Tiled {
            tile_x: true,
            tile_y: true,
            stretch_value: PIXEL,
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
        app.add_systems(PreStartup, (load_font, load_textures));
        app.add_systems(Startup, spawn_camera);
        app.add_observer(dress_player);
        app.add_observer(dress_pickup);
        app.add_observer(dress_monster);
        app.add_observer(dress_prop);
        app.add_observer(dress_projectile);
        app.add_observer(splat_monster);
        app.add_systems(
            Update,
            (
                adopt_layout,
                build_scenery,
                update_roofs,
                sync_players,
                pose_players,
                sync_props,
                sync_pickups,
                bob_pickups,
                sync_monsters,
                pin_shadows,
                sync_projectiles,
                fade_splats,
                age_puddles,
                follow_camera,
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
    const NOTO_SANS_SC: &[u8] = include_bytes!("../assets/NotoSansSC-Subset.ttf");
    let handle = fonts.add(Font::from_bytes(NOTO_SANS_SC.to_vec()));
    commands.insert_resource(UiFont(handle));
}

fn load_textures(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    commands.insert_resource(textures::build(&mut images));
}

fn spawn_camera(mut commands: Commands) {
    commands.spawn((
        Camera2d,
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

/// Ground cover, roads, buildings, walls and clutter for the current layout.
fn build_scenery(
    mut commands: Commands,
    arena: Res<Arena>,
    state: Query<(), With<MatchState>>,
    textures: Res<Textures>,
    old: Query<Entity, With<Scenery>>,
    mut built: Local<Option<u64>>,
) {
    // Wait for the server's layout rather than drawing the placeholder one first.
    if state.is_empty() || *built == Some(arena.seed) {
        return;
    }
    *built = Some(arena.seed);
    for entity in &old {
        commands.entity(entity).despawn();
    }
    let arena = &arena.0;
    let mut floor = |image: Handle<Image>, rect: Rect, z: f32| {
        commands.spawn((
            Scenery,
            tiled(image, rect.size()),
            Transform::from_translation(rect.center().extend(z)),
        ));
    };

    // Dirt everywhere, running a little past the boundary walls.
    floor(
        textures.ground.clone(),
        Rect::from_center_size(Vec2::ZERO, ARENA_HALF * 2.0 + 240.0),
        Z_FLOOR,
    );
    for block in &arena.blocks {
        let cover = match block.biome {
            Biome::Urban | Biome::Vault => Some(textures.pavement.clone()),
            Biome::Industrial => Some(textures.oily_concrete.clone()),
            Biome::Wasteland | Biome::Radiation => None,
        };
        if let Some(image) = cover {
            floor(image, block.rect.inflate(-MARGIN + 12.0), Z_FLOOR + 0.2);
        }
    }
    // Roads between the blocks, with faded center dashes.
    let mut streets_x = Vec::new();
    let mut x = -ARENA_HALF.x + BLOCK.x;
    while x < ARENA_HALF.x - 1.0 {
        streets_x.push(x);
        x += BLOCK.x;
    }
    let mut streets_y = Vec::new();
    let mut y = -ARENA_HALF.y + BLOCK.y;
    while y < ARENA_HALF.y - 1.0 {
        streets_y.push(y);
        y += BLOCK.y;
    }
    for x in &streets_x {
        let rect =
            Rect::from_center_size(Vec2::new(*x, 0.0), Vec2::new(STREET, ARENA_HALF.y * 2.0));
        floor(textures.asphalt.clone(), rect, Z_FLOOR + 0.3);
    }
    for y in &streets_y {
        let rect =
            Rect::from_center_size(Vec2::new(0.0, *y), Vec2::new(ARENA_HALF.x * 2.0, STREET));
        floor(textures.asphalt.clone(), rect, Z_FLOOR + 0.3);
    }
    let dash = Color::srgba(0.85, 0.75, 0.35, 0.5);
    let near = |v: f32, lines: &[f32]| lines.iter().any(|l| (v - l).abs() < STREET);
    let mut t = -ARENA_HALF.y + 40.0;
    while t < ARENA_HALF.y - 40.0 {
        for x in streets_x.iter().filter(|_| !near(t, &streets_y)) {
            commands.spawn((
                Scenery,
                Sprite::from_color(dash, Vec2::new(4.0, 28.0)),
                Transform::from_xyz(*x, t, Z_FLOOR + 0.35),
            ));
        }
        t += 70.0;
    }
    let mut t = -ARENA_HALF.x + 40.0;
    while t < ARENA_HALF.x - 40.0 {
        for y in streets_y.iter().filter(|_| !near(t, &streets_x)) {
            commands.spawn((
                Scenery,
                Sprite::from_color(dash, Vec2::new(28.0, 4.0)),
                Transform::from_xyz(t, *y, Z_FLOOR + 0.35),
            ));
        }
        t += 70.0;
    }
    for hazard in &arena.hazards {
        commands.spawn((
            Scenery,
            tiled(textures.rad_ground.clone(), hazard.size()),
            Transform::from_translation(hazard.center().extend(Z_FLOOR + 0.4)),
        ));
    }
    // The vault: steel plates and a faded radiation sign.
    commands.spawn((
        Scenery,
        tiled(textures.vault_floor.clone(), Vec2::splat(400.0)),
        Transform::from_translation(VAULT_CENTER.extend(Z_FLOOR + 0.5)),
    ));
    commands.spawn((
        Scenery,
        Sprite {
            image: textures.trefoil.clone(),
            custom_size: Some(Vec2::splat(220.0)),
            color: Color::WHITE.with_alpha(0.28),
            ..default()
        },
        Transform::from_translation(VAULT_CENTER.extend(Z_FLOOR + 0.55)),
    ));
    for building in &arena.buildings {
        commands.spawn((
            Scenery,
            tiled(textures.floor(building.floor), building.rect.size()),
            Transform::from_translation(building.rect.center().extend(Z_FLOOR + 0.6)),
        ));
    }

    // Scorch marks and oil stains. Purely cosmetic, so a fresh scatter each time is fine.
    let mut rng = rand::rng();
    let random_spot = |rng: &mut rand::rngs::ThreadRng| {
        Vec2::new(
            rng.random_range(-ARENA_HALF.x..ARENA_HALF.x),
            rng.random_range(-ARENA_HALF.y..ARENA_HALF.y),
        )
    };
    for _ in 0..180 {
        let at = random_spot(&mut rng);
        let image = textures.decals[rng.random_range(0..textures.decals.len())].clone();
        let (sprite, transform) = pixel_sprite(image);
        commands.spawn((
            Scenery,
            sprite,
            transform
                .with_translation(at.extend(Z_DECAL))
                .with_rotation(Quat::from_rotation_z(
                    rng.random_range(0.0..std::f32::consts::TAU),
                )),
        ));
    }
    // Upright clutter, kept out of walls, buildings and the vault so it never looks like cover.
    // Paved blocks get much less of it than the wilds.
    let mut placed = 0;
    let mut tries = 0;
    while placed < 360 && tries < 5000 {
        tries += 1;
        let at = random_spot(&mut rng);
        let paved = arena
            .block_at(at)
            .is_some_and(|b| matches!(b.biome, Biome::Urban | Biome::Vault | Biome::Industrial));
        if (paved && rng.random_bool(0.75))
            || at.distance(VAULT_CENTER) < 260.0
            || arena.resolve_circle(at, 30.0) != at
            || arena
                .buildings
                .iter()
                .any(|b| b.rect.inflate(30.0).contains(at))
            || arena
                .props
                .iter()
                .any(|p| p.pos.distance(at) < p.kind.radius() + 26.0)
        {
            continue;
        }
        placed += 1;
        let mut roll = rng.random_range(0..DECOR_WEIGHTS.iter().sum::<u32>());
        let pick = DECOR_WEIGHTS
            .iter()
            .position(|w| {
                let hit = roll < *w;
                roll = roll.saturating_sub(*w);
                hit
            })
            .unwrap_or(0);
        let image = textures.decor[pick].clone();
        let (mut sprite, transform) = pixel_sprite(image);
        sprite.flip_x = rng.random_bool(0.5);
        commands.spawn((
            Scenery,
            sprite,
            Anchor::BOTTOM_CENTER,
            transform.with_translation(at.extend(depth(at.y))),
        ));
    }

    for style in [
        WallStyle::Brick,
        WallStyle::Metal,
        WallStyle::Rock,
        WallStyle::Sandbag,
    ] {
        let rects: Vec<Rect> = arena
            .walls
            .iter()
            .zip(&arena.wall_styles)
            .filter(|(_, s)| **s == style)
            .map(|(w, _)| *w)
            .collect();
        spawn_walls(&mut commands, textures.wall(style), &rects, style);
    }

    // Roofs sit on the walls, so they're lifted by the wall height and sorted with the front wall:
    // whoever stands in front of the building is drawn over its roof, whoever is inside under it.
    for (i, building) in arena.buildings.iter().enumerate() {
        let r = building.rect;
        let z = depth(r.min.y) + 6.0 * Z_STEP;
        let size = Vec2::new(r.width() + 12.0, r.height() + 6.0);
        commands.spawn((
            Scenery,
            RoofOf {
                building: i,
                alpha: 1.0,
            },
            tiled(textures.roof(building.roof), size),
            Transform::from_translation(Vec3::new(
                r.center().x,
                r.center().y + WALL_HEIGHT - 1.0,
                z,
            )),
        ));
        // Doorways show as dark notches in the roof's edge, since the roof hides the walls.
        for gap in arena.entrances.iter().filter(|e| r.contains(e.center())) {
            let notch = gap.inflate(3.0);
            commands.spawn((
                Scenery,
                RoofOf {
                    building: i,
                    alpha: 0.85,
                },
                Sprite::from_color(Color::srgba(0.06, 0.04, 0.03, 0.85), notch.size()),
                Transform::from_translation(
                    (notch.center() + Vec2::Y * WALL_HEIGHT).extend(z + 2.0 * Z_STEP),
                ),
            ));
        }
        // Dark eave along the front edge so the roof reads as overhanging the wall.
        commands.spawn((
            Scenery,
            RoofOf {
                building: i,
                alpha: 0.7,
            },
            Sprite::from_color(Color::srgba(0.05, 0.04, 0.04, 0.7), Vec2::new(size.x, 4.0)),
            Transform::from_translation(Vec3::new(
                r.center().x,
                r.min.y + WALL_HEIGHT - 2.0,
                z + Z_STEP,
            )),
        ));
    }
}

/// Fade the roof over your head; note which building you're in so others inside stay hidden.
fn update_roofs(
    time: Res<Time>,
    fixed: Res<Time<Fixed>>,
    arena: Res<Arena>,
    me: Query<(&Pos, Option<&TickSamples>, &Vitals), With<LocalPlayer>>,
    mut indoors: ResMut<Indoors>,
    mut roofs: Query<(&RoofOf, &mut Sprite)>,
) {
    let inside = me
        .single()
        .ok()
        .filter(|(.., v)| v.alive)
        .and_then(|(pos, samples, _)| arena.building_at(visual_pos(pos, samples, &fixed)));
    if indoors.0 != inside {
        indoors.0 = inside;
    }
    let k = 1.0 - (-10.0 * time.delta_secs()).exp();
    for (roof, mut sprite) in &mut roofs {
        let now = sprite.color.alpha();
        let goal = if Some(roof.building) == inside {
            roof.alpha * ROOF_SEE_THROUGH
        } else {
            roof.alpha
        };
        let alpha = now + (goal - now) * k;
        if (alpha - now).abs() > 1e-3 {
            sprite.color.set_alpha(alpha);
        }
    }
}

/// A vertical slice of solid wall: footprint `x0..x1` by `y0..y1`, no overlaps with other runs.
#[derive(Clone, Copy, Debug, PartialEq)]
struct WallRun {
    x0: f32,
    x1: f32,
    y0: f32,
    y1: f32,
}

const EPS: f32 = 0.01;

/// Walls overlap at corners. Drawing each rect on its own makes one wall's cap fold over the
/// other's brick face, so render their union instead: cut it into vertical strips at every wall
/// edge, merge the solid spans within each strip, then glue identical neighbouring strips back.
fn wall_runs(walls: &[Rect]) -> Vec<WallRun> {
    let mut xs: Vec<f32> = walls.iter().flat_map(|w| [w.min.x, w.max.x]).collect();
    xs.sort_by(f32::total_cmp);
    xs.dedup_by(|a, b| (*a - *b).abs() < EPS);
    let mut runs: Vec<WallRun> = Vec::new();
    for pair in xs.windows(2) {
        let (x0, x1) = (pair[0], pair[1]);
        let mid = (x0 + x1) / 2.0;
        let mut spans: Vec<(f32, f32)> = walls
            .iter()
            .filter(|w| w.min.x < mid && w.max.x > mid)
            .map(|w| (w.min.y, w.max.y))
            .collect();
        spans.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut merged: Vec<(f32, f32)> = Vec::new();
        for (y0, y1) in spans {
            match merged.last_mut() {
                Some(last) if y0 <= last.1 + EPS => last.1 = last.1.max(y1),
                _ => merged.push((y0, y1)),
            }
        }
        for (y0, y1) in merged {
            // Extend the run to the left if the previous strip ended with the same span.
            let same = |r: &WallRun| {
                (r.x1 - x0).abs() < EPS && (r.y0 - y0).abs() < EPS && (r.y1 - y1).abs() < EPS
            };
            match runs.iter_mut().find(|r| same(r)) {
                Some(run) => run.x1 = x1,
                None => runs.push(WallRun { x0, x1, y0, y1 }),
            }
        }
    }
    runs
}

/// Paint for steel: containers and wrecks come in a few faded colors.
const METAL_TINTS: [Color; 5] = [
    Color::srgb(0.78, 0.42, 0.32),
    Color::srgb(0.45, 0.6, 0.8),
    Color::srgb(0.5, 0.68, 0.45),
    Color::srgb(0.85, 0.85, 0.82),
    Color::srgb(0.9, 0.74, 0.35),
];

fn spawn_walls(commands: &mut Commands, sprites: &WallSprites, walls: &[Rect], style: WallStyle) {
    let runs = wall_runs(walls);
    let dark = Color::srgb(0.08, 0.06, 0.07);
    let wall_height = match style {
        WallStyle::Brick => WALL_HEIGHT,
        WallStyle::Metal => 46.0,
        WallStyle::Rock => 30.0,
        WallStyle::Sandbag => 22.0,
    };
    // Whether another run covers the strip just outside `x` somewhere within `y0..y1`.
    let neighbour = |x: f32, y0: f32, y1: f32| {
        runs.iter()
            .any(|r| (r.x0 - EPS..=r.x1 + EPS).contains(&x) && r.y0 < y1 - EPS && r.y1 > y0 + EPS)
    };
    for run in &runs {
        // The footprint rises WALL_HEIGHT out of the ground: a brick face along its near
        // (bottom) edge, and its cap lifted by the same amount.
        let w = run.x1 - run.x0;
        let cx = (run.x0 + run.x1) / 2.0;
        let depth_len = run.y1 - run.y0;
        let z = depth(run.y0);
        let at = |y: f32, dz: f32| Vec3::new(cx, y, z + dz);
        let tint = match style {
            WallStyle::Metal => {
                METAL_TINTS[((run.x0 * 7.0 + run.y0 * 13.0).abs() as usize) % METAL_TINTS.len()]
            }
            _ => Color::WHITE,
        };
        let face = |image: &Handle<Image>, size: Vec2| {
            let mut sprite = tiled(image.clone(), size);
            sprite.color = tint;
            sprite
        };
        commands.spawn((
            Scenery,
            Sprite::from_color(Color::srgba(0.0, 0.0, 0.0, 0.3), Vec2::new(w, depth_len)),
            Transform::from_translation(Vec3::new(
                cx + 10.0,
                run.y0 + depth_len / 2.0 - 6.0,
                Z_SHADOW,
            )),
        ));
        commands.spawn((
            Scenery,
            face(&sprites.front, Vec2::new(w, wall_height)),
            Transform::from_translation(at(run.y0 + wall_height / 2.0, 0.0)),
        ));
        commands.spawn((
            Scenery,
            face(&sprites.top, Vec2::new(w, depth_len)),
            Transform::from_translation(at(run.y0 + wall_height + depth_len / 2.0, Z_STEP)),
        ));
        // Sunlit lip where the cap meets the face, grime where the face meets the ground,
        // and a dark rim along the cap's far edge.
        let strip = |h: f32| Vec2::new(w, h);
        for (color, size, y) in [
            (
                Color::srgba(1.0, 0.95, 0.85, 0.35),
                strip(2.0),
                run.y0 + wall_height + 1.0,
            ),
            (Color::srgba(0.0, 0.0, 0.0, 0.35), strip(4.0), run.y0 + 2.0),
            (dark, strip(2.0), run.y1 + wall_height + 1.0),
            (dark, strip(2.0), run.y0 - 1.0),
        ] {
            commands.spawn((
                Scenery,
                Sprite::from_color(color, size),
                Transform::from_translation(at(y, 2.0 * Z_STEP)),
            ));
        }
        // Side rims, only where the wall actually ends rather than continuing as another run.
        for (x, outside) in [
            (run.x0 - 1.0, run.x0 - 2.0 * EPS),
            (run.x1 + 1.0, run.x1 + 2.0 * EPS),
        ] {
            if neighbour(outside, run.y0, run.y1) {
                continue;
            }
            let h = depth_len + wall_height + 4.0;
            commands.spawn((
                Scenery,
                Sprite::from_color(dark, Vec2::new(2.0, h)),
                Transform::from_translation(Vec3::new(x, run.y0 - 2.0 + h / 2.0, z + 2.0 * Z_STEP)),
            ));
        }
    }
}

fn player_color(player: &Player) -> Color {
    let [r, g, b] = player.color;
    Color::srgb(r, g, b)
}

fn dress_player(
    trigger: On<Add<Player>>,
    players: Query<&Player>,
    mut commands: Commands,
    textures: Res<Textures>,
    font: Res<UiFont>,
) {
    let Ok(player) = players.get(trigger.entity) else {
        return;
    };
    let frame = textures.survivor(Facing::Down, 0).clone();
    let lift = Vec3::new(0.0, SURVIVOR_HEIGHT / 2.0 - FOOT_INSET, 0.0);
    let (mut jacket, transform) = pixel_sprite(frame.jacket);
    jacket.color = player_color(player);
    let jacket = commands
        .spawn((jacket, transform.with_translation(lift)))
        .id();
    let (gear, transform) = pixel_sprite(frame.gear);
    let gear = commands
        .spawn((gear, transform.with_translation(lift + Vec3::Z * Z_STEP)))
        .id();
    let (weapon, transform) = pixel_sprite(textures.weapon(WeaponKind::Stick).held.clone());
    let weapon = commands.spawn((weapon, transform)).id();
    let (mut arm, transform) = pixel_sprite(textures.arm.clone());
    arm.color = player_color(player);
    let arm = commands.spawn((arm, transform)).id();
    let shadow = commands
        .spawn(shadow(&textures, Vec2::new(46.0, 22.0)))
        .id();
    commands
        .entity(trigger.entity)
        .insert((
            Transform::default(),
            Visibility::Hidden,
            PlayerParts {
                jacket,
                gear,
                weapon,
                arm,
                shown: Some(WeaponKind::Stick),
                last: Vec2::ZERO,
                stride: 0.0,
            },
        ))
        .add_children(&[shadow, jacket, gear, weapon, arm])
        .with_child((
            NameLabel,
            Text2d::new(player.name.clone()),
            font.sized(14.0),
            TextColor(Color::srgba(1.0, 1.0, 1.0, 0.85)),
            Transform::from_xyz(0.0, SURVIVOR_HEIGHT + 16.0, Z_LABEL),
        ));
}

fn dress_pickup(
    trigger: On<Add<Pickup>>,
    pickups: Query<(&Pickup, &Pos)>,
    mut commands: Commands,
    textures: Res<Textures>,
    font: Res<UiFont>,
) {
    let Ok((pickup, pos)) = pickups.get(trigger.entity) else {
        return;
    };
    let apex = pickup.item.tier() >= 4;
    let (icon, icon_transform) = pixel_sprite(textures.item(pickup.item));
    let phase = pos.0.x * 0.013 + pos.0.y * 0.007;
    commands
        .entity(trigger.entity)
        .insert((
            Transform::from_translation(pos.0.extend(Z_PICKUP)),
            Visibility::Visible,
        ))
        .with_child((
            Sprite {
                image: textures.glow.clone(),
                custom_size: Some(Vec2::splat(if apex { 80.0 } else { 56.0 })),
                color: pickup
                    .item
                    .color()
                    .with_alpha(if apex { 0.8 } else { 0.55 }),
                ..default()
            },
            Transform::from_xyz(0.0, 0.0, -0.1),
        ))
        .with_child((
            PickupIcon { phase },
            icon,
            icon_transform.with_scale(Vec3::splat(if apex { PIXEL * 1.3 } else { PIXEL })),
        ))
        .with_child((
            Text2d::new(pickup.item.label()),
            font.sized(12.0),
            TextColor(pickup.item.color().with_alpha(0.9)),
            Transform::from_xyz(0.0, -24.0, 0.1),
        ));
}

/// Rendered position: smoothed prediction for us, interpolated server state for everyone else.
pub fn visual_pos(pos: &Pos, samples: Option<&TickSamples>, fixed: &Time<Fixed>) -> Vec2 {
    samples.map_or(pos.0, |s| s.at(fixed.overstep_fraction()))
}

fn sync_players(
    fixed: Res<Time<Fixed>>,
    arena: Res<Arena>,
    indoors: Res<Indoors>,
    mut players: Query<
        (
            &Pos,
            Option<&TickSamples>,
            &Vitals,
            &mut Transform,
            &mut Visibility,
        ),
        With<Player>,
    >,
) {
    for (pos, samples, vitals, mut transform, mut visibility) in &mut players {
        let p = visual_pos(pos, samples, &fixed);
        transform.translation = p.extend(depth(p.y));
        *visibility = if vitals.alive && !indoors.conceals(&arena, p) {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
}

/// Walk cycle, facing and the weapon swinging around to point at the target.
fn pose_players(
    tex: Res<Textures>,
    cursor: Res<CursorWorld>,
    fixed: Res<Time<Fixed>>,
    time: Res<Time>,
    mut players: Query<(
        &Pos,
        Option<&TickSamples>,
        &Aim,
        &Loadout,
        &mut PlayerParts,
        Has<LocalPlayer>,
    )>,
    mut parts_q: Query<(&mut Sprite, &mut Transform), Without<Player>>,
) {
    for (pos, samples, aim, loadout, mut parts, local) in &mut players {
        let p = visual_pos(pos, samples, &fixed);
        let gun_at = p + Vec2::Y * GUN_HEIGHT;
        // Your own body follows the cursor directly instead of waiting for the server's aim.
        let aim = match (local, cursor.0) {
            (true, Some(c)) => (c - gun_at).normalize_or(aim.0),
            _ => aim.0,
        };
        // Respawn teleports don't count as steps.
        let step = p.distance(parts.last);
        parts.last = p;
        let moving = step > 0.2 && step < 60.0 && step / time.delta_secs().max(1e-3) > 40.0;
        let frame = if moving {
            parts.stride += step / STRIDE_PX;
            parts.stride as usize % textures::WALK_FRAMES
        } else {
            // Settle on a passing pose instead of freezing mid-stride.
            parts.stride = parts.stride.floor()
                + if parts.stride as usize % 2 == 1 {
                    1.0
                } else {
                    0.0
                };
            0
        };
        let (facing, flip) = Facing::from_aim(aim);
        let images = tex.survivor(facing, frame).clone();
        for (entity, image) in [(parts.jacket, images.jacket), (parts.gear, images.gear)] {
            if let Ok((mut sprite, _)) = parts_q.get_mut(entity) {
                sprite.image = image;
                sprite.flip_x = flip;
            }
        }

        // The weapon pivots at the hands; when you face away it's held in front of you, out of sight.
        let behind = facing == Facing::Up;
        let sprite_info = tex.weapon(loadout.weapon);
        let pivot = Vec2::new(
            if facing == Facing::Side {
                if flip { -4.0 } else { 4.0 }
            } else {
                0.0
            },
            GUN_HEIGHT,
        );
        let (center, arm_at) =
            textures::held_layout(pivot, aim, sprite_info.size, sprite_info.grip);
        let z = if behind { -2.0 * Z_STEP } else { 3.0 * Z_STEP };
        if let Ok((mut sprite, mut transform)) = parts_q.get_mut(parts.weapon) {
            if parts.shown != Some(loadout.weapon) {
                sprite.image = sprite_info.held.clone();
                parts.shown = Some(loadout.weapon);
            }
            // Keep the gun upright when it points left.
            sprite.flip_y = aim.x < 0.0;
            transform.translation = center.extend(z);
            transform.rotation = Quat::from_rotation_z(aim.to_angle());
        }
        // Under the weapon, so the fist closes over the cuff.
        if let Ok((mut sprite, mut transform)) = parts_q.get_mut(parts.arm) {
            sprite.flip_y = aim.x < 0.0;
            transform.translation = arm_at.extend(z - Z_STEP);
            transform.rotation = Quat::from_rotation_z(aim.to_angle());
        }
    }
}

fn sync_pickups(mut pickups: Query<(&Pos, &mut Transform), (With<Pickup>, Changed<Pos>)>) {
    for (pos, mut transform) in &mut pickups {
        transform.translation = pos.0.extend(Z_PICKUP);
    }
}

fn bob_pickups(time: Res<Time>, mut icons: Query<(&PickupIcon, &mut Transform)>) {
    let t = time.elapsed_secs();
    for (icon, mut transform) in &mut icons {
        transform.translation.y = 6.0 + 3.0 * (t * 2.6 + icon.phase).sin();
        transform.rotation = Quat::from_rotation_z(0.12 * (t * 1.3 + icon.phase).sin());
    }
}

fn monster_image(kind: MonsterKind, textures: &Textures) -> Handle<Image> {
    match kind {
        MonsterKind::Roach => textures.roach[0].clone(),
        MonsterKind::SporePod => textures.pod.clone(),
        MonsterKind::Dog => textures.dog[0].clone(),
        MonsterKind::Ghoul => textures.ghoul[0].clone(),
        MonsterKind::Spitter => textures.spitter.clone(),
        MonsterKind::Crawler => textures.crawler.clone(),
        MonsterKind::Raider => textures.raider[0].clone(),
        MonsterKind::Drone => textures.drone[0].clone(),
        MonsterKind::Behemoth => textures.behemoth[0].clone(),
    }
}

fn monster_size(kind: MonsterKind, textures: &Textures, images: &Assets<Image>) -> Vec2 {
    images
        .get(&monster_image(kind, textures))
        .map_or(Vec2::splat(32.0), |i| i.size_f32())
        * PIXEL
}

/// Height of the top of a monster's sprite above its ground point.
fn monster_top(monster: &Monster, textures: &Textures, images: &Assets<Image>) -> f32 {
    let size = monster_size(monster.kind, textures, images);
    let top = match monster.kind {
        MonsterKind::Drone => size.y + DRONE_HOVER - FOOT_INSET,
        MonsterKind::Crawler => CRAWLER_LIFT + size.y / 2.0,
        _ => size.y,
    };
    top * if monster.elite { ELITE_SCALE } else { 1.0 }
}

fn dress_monster(
    trigger: On<Add<Monster>>,
    monsters: Query<(&Monster, Option<&Pos>)>,
    mut commands: Commands,
    textures: Res<Textures>,
) {
    // Interpolated entities may not have a position yet; `sync_monsters` reveals them once they do.
    let Ok((monster, pos)) = monsters.get(trigger.entity) else {
        return;
    };
    let pos = pos.map_or(Vec2::ZERO, |p| p.0);
    let (sprite, transform) = pixel_sprite(monster_image(monster.kind, &textures));
    let (anchor, base_y) = match monster.kind {
        MonsterKind::Crawler => (Anchor::CENTER, CRAWLER_LIFT),
        MonsterKind::Drone => (Anchor::BOTTOM_CENTER, DRONE_HOVER - FOOT_INSET),
        _ => (Anchor::BOTTOM_CENTER, -FOOT_INSET),
    };
    let body = commands
        .spawn((sprite, anchor, transform.with_translation(Vec3::Y * base_y)))
        .id();
    let r = monster.kind.radius();
    let shadow = commands
        .spawn(shadow(&textures, Vec2::new(r * 2.6, r * 1.2)))
        .id();
    commands
        .entity(trigger.entity)
        .insert((
            Transform::from_translation(pos.extend(depth(pos.y))),
            Visibility::Hidden,
            MonsterParts {
                body,
                base_y,
                last: pos,
                flip: false,
                phase: pos.x * 0.01,
            },
        ))
        .add_children(&[shadow, body]);
}

fn sync_monsters(
    time: Res<Time>,
    textures: Res<Textures>,
    arena: Res<Arena>,
    indoors: Res<Indoors>,
    mut monsters: Query<(
        &Monster,
        &Pos,
        Option<&MonsterState>,
        &mut MonsterParts,
        &mut Transform,
        &mut Visibility,
    )>,
    mut bodies: Query<(&mut Sprite, &mut Transform), Without<Monster>>,
) {
    let t = time.elapsed_secs();
    for (monster, pos, state, mut parts, mut transform, mut visibility) in &mut monsters {
        transform.translation = pos.0.extend(depth(pos.0.y));
        visibility.set_if_neq(if indoors.conceals(&arena, pos.0) {
            Visibility::Hidden
        } else {
            Visibility::Visible
        });
        let Ok((mut sprite, mut body)) = bodies.get_mut(parts.body) else {
            continue;
        };
        let state = state.copied().unwrap_or_default();
        let moved = pos.0 - parts.last;
        let moving = moved.length_squared() > 0.01;
        parts.last = pos.0;
        // Side-view creatures face where they look (roaches: where they crawl).
        match monster.kind {
            MonsterKind::Roach => {
                if moved.x.abs() > 0.05 {
                    parts.flip = moved.x < 0.0;
                }
            }
            MonsterKind::Dog | MonsterKind::Ghoul | MonsterKind::Spitter | MonsterKind::Raider => {
                let dir = state.facing.cos();
                if dir.abs() > 0.2 {
                    parts.flip = dir < 0.0;
                }
            }
            _ => {}
        }
        sprite.flip_x = parts.flip;
        let walk = |rate: f32, frames: usize| {
            if moving {
                (t * rate + parts.phase) as usize % frames
            } else {
                0
            }
        };
        let mut scale = Vec2::splat(PIXEL * if monster.elite { ELITE_SCALE } else { 1.0 });
        let mut offset = Vec2::new(0.0, parts.base_y);
        let mut tint = if monster.elite {
            ELITE_TINT
        } else {
            Color::WHITE
        };
        match monster.kind {
            MonsterKind::Roach => sprite.image = textures.roach[walk(16.0, 2)].clone(),
            MonsterKind::SporePod => {
                // Breathe: squash and stretch around the planted roots.
                let pulse = 0.05 * (t * 3.0 + parts.phase).sin();
                scale *= Vec2::new(1.0 + pulse, 1.0 - pulse);
            }
            MonsterKind::Dog => sprite.image = textures.dog[walk(18.0, 2)].clone(),
            MonsterKind::Ghoul => {
                sprite.image = textures.ghoul[walk(9.0, 2)].clone();
                if state.action == MonsterAction::Arming {
                    // Blinks faster as the fuse runs down.
                    let rate = 6.0 + 500.0 / (state.fuse as f32 + 24.0);
                    if (t * rate).sin() > 0.0 {
                        tint = Color::srgb(1.0, 0.3, 0.25);
                    }
                    let swell = 1.0 + 0.06 * (t * rate).sin();
                    scale *= Vec2::splat(swell);
                }
            }
            MonsterKind::Spitter => {
                let pulse = 0.04 * (t * 4.0 + parts.phase).sin();
                scale *= Vec2::new(1.0 - pulse, 1.0 + pulse);
            }
            MonsterKind::Crawler => body.rotation = Quat::from_rotation_z(state.facing),
            MonsterKind::Raider => sprite.image = textures.raider[walk(10.0, 4)].clone(),
            MonsterKind::Drone => {
                sprite.image = textures.drone[(t * 30.0) as usize % 2].clone();
                offset.y += 4.0 * (t * 3.0 + parts.phase).sin();
            }
            MonsterKind::Behemoth => {
                sprite.image = textures.behemoth[walk(3.0, 2)].clone();
                tint = match state.phase {
                    0 | 1 => tint,
                    2 => Color::srgb(1.0, 0.82, 0.7),
                    _ => Color::srgb(1.0, 0.5 + 0.12 * (t * 8.0).sin(), 0.42),
                };
                if state.action == MonsterAction::Charge && state.fuse > 0 {
                    // Rearing up and trembling before the rush.
                    offset.x = 3.0 * (t * 70.0).sin();
                    scale *= Vec2::new(1.0 - 0.03, 1.0 + 0.05);
                } else if state.action == MonsterAction::Slam && state.fuse > 0 {
                    scale *= Vec2::new(1.0 + 0.04, 1.0 + 0.1);
                }
            }
        }
        sprite.color = tint;
        body.scale = scale.extend(1.0);
        body.translation.x = offset.x;
        body.translation.y = offset.y;
    }
}

fn prop_image(prop: &Prop, textures: &Textures) -> Handle<Image> {
    match prop.kind {
        PropKind::Crate => textures.crate_box.clone(),
        PropKind::Barrel => textures.barrel.clone(),
        PropKind::Safe if prop.open => textures.safe_open.clone(),
        PropKind::Safe => textures.safe.clone(),
    }
}

fn dress_prop(
    trigger: On<Add<Prop>>,
    props: Query<(&Prop, &Pos)>,
    mut commands: Commands,
    textures: Res<Textures>,
) {
    let Ok((prop, pos)) = props.get(trigger.entity) else {
        return;
    };
    let (sprite, transform) = pixel_sprite(prop_image(prop, &textures));
    let body = commands
        .spawn((
            sprite,
            Anchor::BOTTOM_CENTER,
            transform.with_translation(Vec3::Y * -FOOT_INSET),
        ))
        .id();
    let r = prop.kind.radius();
    let shadow = commands
        .spawn(shadow(&textures, Vec2::new(r * 2.4, r * 1.1)))
        .id();
    commands
        .entity(trigger.entity)
        .insert((
            Transform::from_translation(pos.0.extend(depth(pos.0.y))),
            Visibility::Visible,
            PropBody(body),
        ))
        .add_children(&[shadow, body]);
}

/// Safes swing open; crates and barrels darken as they take a beating.
fn sync_props(
    textures: Res<Textures>,
    props: Query<(&Prop, &Vitals, &PropBody), Or<(Changed<Prop>, Changed<Vitals>)>>,
    mut bodies: Query<&mut Sprite>,
) {
    for (prop, vitals, body) in &props {
        let Ok(mut sprite) = bodies.get_mut(body.0) else {
            continue;
        };
        sprite.image = prop_image(prop, &textures);
        let worn = 1.0 - (vitals.hp / prop.kind.max_hp()).clamp(0.0, 1.0);
        sprite.color = Color::WHITE.mix(&Color::srgb(0.45, 0.35, 0.3), worn * 0.7);
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

fn splat_monster(
    trigger: On<Remove<Monster>>,
    monsters: Query<&Pos>,
    mut commands: Commands,
    textures: Res<Textures>,
) {
    let Ok(pos) = monsters.get(trigger.entity) else {
        return;
    };
    let (sprite, transform) = pixel_sprite(textures.splat.clone());
    commands.spawn((
        Splat { age: 0.0 },
        sprite,
        transform
            .with_translation(pos.0.extend(Z_DECAL + 0.5))
            .with_rotation(Quat::from_rotation_z(
                rand::rng().random_range(0.0..std::f32::consts::TAU),
            )),
    ));
}

fn fade_splats(
    mut commands: Commands,
    time: Res<Time>,
    mut splats: Query<(Entity, &mut Splat, &mut Sprite)>,
) {
    for (entity, mut splat, mut sprite) in &mut splats {
        splat.age += time.delta_secs();
        if splat.age > SPLAT_SECS {
            commands.entity(entity).despawn();
        } else {
            let fade =
                (1.0 - (splat.age - SPLAT_SECS * 0.6).max(0.0) / (SPLAT_SECS * 0.4)).max(0.0);
            sprite.color = Color::WHITE.with_alpha(fade);
        }
    }
}

/// Every projectile is a sprite: spores and EMP bolts glow, bullets are tinted streaks.
fn dress_projectile(
    trigger: On<Add<Projectile>>,
    projectiles: Query<&Projectile>,
    mut commands: Commands,
    textures: Res<Textures>,
) {
    let Ok(projectile) = projectiles.get(trigger.entity) else {
        return;
    };
    let (sprite, anchor) = match projectile.kind {
        ProjectileKind::Spore => (
            Sprite {
                image: textures.spore.clone(),
                custom_size: Some(Vec2::splat(22.0)),
                ..default()
            },
            Anchor::CENTER,
        ),
        ProjectileKind::Acid => (
            Sprite {
                image: textures.acid.clone(),
                custom_size: Some(Vec2::splat(26.0)),
                ..default()
            },
            Anchor::CENTER,
        ),
        ProjectileKind::Bolt => (
            Sprite {
                image: textures.bolt.clone(),
                color: Color::srgb(0.45, 0.95, 1.0),
                custom_size: Some(Vec2::new(26.0, 7.0)),
                ..default()
            },
            Anchor::CENTER_RIGHT,
        ),
        ProjectileKind::Emp => (
            Sprite {
                image: textures.glow.clone(),
                color: Color::srgb(0.4, 0.8, 1.0),
                custom_size: Some(Vec2::splat(30.0)),
                ..default()
            },
            Anchor::CENTER,
        ),
        ProjectileKind::Shot(weapon) => (
            Sprite {
                image: textures.bolt.clone(),
                color: weapon.stats().color,
                custom_size: Some(tracer_size(weapon)),
                ..default()
            },
            Anchor::CENTER_RIGHT,
        ),
    };
    commands.entity(trigger.entity).insert((
        sprite,
        anchor,
        Transform::from_xyz(0.0, 0.0, Z_PROJECTILE),
        Visibility::Hidden,
    ));
}

fn sync_projectiles(
    mut projectiles: Query<(&Pos, &Projectile, &mut Transform, &mut Visibility), With<Sprite>>,
) {
    for (pos, projectile, mut transform, mut visibility) in &mut projectiles {
        transform.translation = (pos.0 + Vec2::Y * GUN_HEIGHT).extend(Z_PROJECTILE);
        if matches!(
            projectile.kind,
            ProjectileKind::Shot(_) | ProjectileKind::Bolt
        ) {
            transform.rotation = Quat::from_rotation_z(projectile.vel.to_angle());
        }
        visibility.set_if_neq(Visibility::Visible);
    }
}

fn age_puddles(
    mut commands: Commands,
    time: Res<Time>,
    new: Query<Entity, (With<Puddle>, Without<PuddleAge>)>,
    mut ages: Query<&mut PuddleAge>,
) {
    for entity in &new {
        commands.entity(entity).insert(PuddleAge::default());
    }
    for mut age in &mut ages {
        age.0 += time.delta_secs();
    }
}

/// While the behemoth is out of view, an arrow at the screen edge points the way.
fn boss_arrow(
    mut commands: Commands,
    time: Res<Time>,
    textures: Res<Textures>,
    window: Query<&Window, With<PrimaryWindow>>,
    camera: Query<&Transform, (With<Camera2d>, Without<BossArrow>)>,
    bosses: Query<(&Pos, &Monster, &Vitals)>,
    mut arrow: Query<(&mut Transform, &mut Visibility, &mut Sprite), With<BossArrow>>,
) {
    let Ok((mut transform, mut visibility, mut sprite)) = arrow.single_mut() else {
        commands.spawn((
            BossArrow,
            Sprite {
                image: textures.arrow.clone(),
                custom_size: Some(Vec2::splat(34.0)),
                ..default()
            },
            Transform::from_xyz(0.0, 0.0, Z_OVERLAY + 10.0),
            Visibility::Hidden,
        ));
        return;
    };
    let boss = bosses
        .iter()
        .find(|(_, m, v)| m.kind == MonsterKind::Behemoth && v.alive);
    let (Some((boss, ..)), Ok(window), Ok(cam)) = (boss, window.single(), camera.single()) else {
        visibility.set_if_neq(Visibility::Hidden);
        return;
    };
    let center = cam.translation.truncate();
    let half = Vec2::new(window.width(), window.height()) * 0.5 * CAMERA_SCALE;
    let to = boss.0 - center;
    if to.x.abs() < half.x && to.y.abs() < half.y {
        visibility.set_if_neq(Visibility::Hidden);
        return;
    }
    let inner = half - Vec2::splat(40.0 * CAMERA_SCALE);
    let k = (inner.x / to.x.abs().max(1.0)).min(inner.y / to.y.abs().max(1.0));
    let at = center + to * k.min(1.0);
    transform.translation = at.extend(Z_OVERLAY + 10.0);
    transform.rotation = Quat::from_rotation_z(to.to_angle());
    sprite.color =
        Color::srgb(1.0, 0.35, 0.2).with_alpha(0.7 + 0.3 * (time.elapsed_secs() * 6.0).sin());
    visibility.set_if_neq(Visibility::Visible);
}

fn follow_camera(
    time: Res<Time>,
    fixed: Res<Time<Fixed>>,
    cursor: Res<CursorWorld>,
    mut shake: ResMut<CameraShake>,
    me: Query<(&Pos, Option<&TickSamples>), With<LocalPlayer>>,
    mut camera: Query<&mut Transform, With<Camera2d>>,
) {
    let Ok(mut cam) = camera.single_mut() else {
        return;
    };
    let Ok((pos, samples)) = me.single() else {
        return;
    };
    let me = visual_pos(pos, samples, &fixed) + Vec2::Y * GUN_HEIGHT;
    // Lean the camera toward where you're aiming so you see threats before they see you.
    let lean = cursor
        .0
        .map(|c| ((c - me) * 0.22).clamp_length_max(160.0))
        .unwrap_or_default();
    let target = me + lean;
    let current = cam.translation.truncate();
    let k = 1.0 - (-12.0 * time.delta_secs()).exp();
    let mut next = current.lerp(target, k);
    if shake.0 > 0.01 {
        let t = time.elapsed_secs() * 60.0;
        next += Vec2::new(t.sin(), (t * 1.3).cos()) * shake.0 * 9.0;
        shake.0 *= (-8.0 * time.delta_secs()).exp();
    }
    cam.translation = next.extend(cam.translation.z);
}

fn update_cursor(
    mut cursor: ResMut<CursorWorld>,
    window: Query<&Window, With<PrimaryWindow>>,
    camera: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
) {
    let (Ok(window), Ok((camera, transform))) = (window.single(), camera.single()) else {
        return;
    };
    if let Some(screen) = window.cursor_position() {
        cursor.0 = camera.viewport_to_world_2d(transform, screen).ok();
    }
}

/// Marks one frame's worth of overlay sprites (bars, rings); redrawn from scratch every frame.
#[derive(Component)]
struct Overlay;

const Z_OVERLAY: f32 = 6.0;

struct Painter<'a, 'w, 's> {
    commands: &'a mut Commands<'w, 's>,
    textures: &'a Textures,
}

impl Painter<'_, '_, '_> {
    /// An ellipse of the ring texture, centered on `center`.
    fn oval(&mut self, center: Vec2, radii: Vec2, color: Color) {
        // The painted ring peaks at 85% of the texture's half-size.
        self.commands.spawn((
            Overlay,
            Sprite {
                image: self.textures.shock.clone(),
                color,
                custom_size: Some(radii * 2.0 / 0.85),
                ..default()
            },
            Transform::from_translation(center.extend(Z_OVERLAY)),
        ));
    }

    /// A circle lying flat on the ground around someone's feet.
    fn ring(&mut self, at: Vec2, r: f32, color: Color) {
        self.oval(at, Vec2::new(r, r * 0.5), color);
    }

    /// A true-radius circle on the ground (world distances are not foreshortened).
    fn zone(&mut self, at: Vec2, r: f32, color: Color) {
        self.oval(at, Vec2::splat(r), color);
    }

    /// A straight stripe from `from` toward `angle`.
    fn line(&mut self, from: Vec2, angle: f32, length: f32, width: f32, color: Color) {
        self.commands.spawn((
            Overlay,
            Sprite {
                image: self.textures.bar.clone(),
                color,
                custom_size: Some(Vec2::new(length, width)),
                ..default()
            },
            Anchor::CENTER_LEFT,
            Transform::from_translation(from.extend(Z_OVERLAY - 0.4))
                .with_rotation(Quat::from_rotation_z(angle)),
        ));
    }

    fn disc(&mut self, at: Vec2, r: f32, color: Color) {
        self.oval(at, Vec2::splat(r), color);
    }

    /// A bar growing rightward from `left`.
    fn bar(&mut self, left: Vec2, width: f32, height: f32, color: Color) {
        if width < 1.0 {
            return;
        }
        self.commands.spawn((
            Overlay,
            Sprite {
                image: self.textures.bar.clone(),
                color,
                custom_size: Some(Vec2::new(width, height)),
                ..default()
            },
            Anchor::CENTER_LEFT,
            Transform::from_translation(left.extend(Z_OVERLAY + 0.01)),
        ));
    }

    fn gauge(&mut self, left: Vec2, width: f32, height: f32, frac: f32, color: Color) {
        self.bar(left, width, height, Color::srgba(0.0, 0.0, 0.0, 0.6));
        self.bar(left, width * frac.clamp(0.0, 1.0), height, color);
    }
}

#[allow(clippy::type_complexity)]
fn draw_overlays(
    mut commands: Commands,
    old: Query<Entity, With<Overlay>>,
    time: Res<Time>,
    fixed: Res<Time<Fixed>>,
    players: Query<(
        &Pos,
        Option<&TickSamples>,
        &Vitals,
        &Stats,
        Has<LocalPlayer>,
    )>,
    monsters: Query<(&Pos, &Vitals, &Monster, Option<&MonsterState>)>,
    puddles: Query<(&Pos, &Puddle, &PuddleAge)>,
    props: Query<(&Pos, &Vitals, &Prop)>,
    pickups: Query<&Pos, With<Pickup>>,
    textures: Res<Textures>,
    images: Res<Assets<Image>>,
    arena: Res<Arena>,
    indoors: Res<Indoors>,
) {
    for entity in &old {
        commands.entity(entity).despawn();
    }
    let mut paint = Painter {
        commands: &mut commands,
        textures: &textures,
    };
    let t = time.elapsed_secs();
    // Radiation zones shimmer at the edges.
    for hazard in &arena.hazards {
        let glow = 0.25 + 0.12 * (t * 2.0).sin();
        paint.commands.spawn((
            Overlay,
            Sprite::from_color(
                Color::srgb(0.6, 1.0, 0.3).with_alpha(glow * 0.35),
                hazard.size(),
            ),
            Transform::from_translation(hazard.center().extend(Z_OVERLAY - 0.5)),
        ));
    }

    // Puddles lie on the ground under everyone, fading out over their last seconds.
    for (pos, puddle, age) in &puddles {
        let life = puddle.ttl as f32 / 64.0;
        let fade = ((life - age.0) / 2.0).clamp(0.0, 1.0) * (age.0 / 0.25).clamp(0.0, 1.0);
        let (fill, edge) = match puddle.kind {
            PuddleKind::Acid => (Color::srgb(0.45, 0.85, 0.15), Color::srgb(0.7, 1.0, 0.3)),
            PuddleKind::Radiation => (Color::srgb(0.7, 0.95, 0.2), Color::srgb(1.0, 1.0, 0.4)),
        };
        let bubble = 0.75 + 0.25 * (t * 4.0 + pos.0.x).sin();
        paint.commands.spawn((
            Overlay,
            Sprite {
                image: textures.glow.clone(),
                color: fill.with_alpha(0.85 * fade * bubble),
                custom_size: Some(Vec2::splat(puddle.radius * 2.3)),
                ..default()
            },
            Transform::from_translation(pos.0.extend(Z_DECAL + 0.7)),
        ));
        paint.zone(pos.0, puddle.radius, edge.with_alpha(0.55 * fade));
    }

    for (pos, vitals, monster, state) in &monsters {
        if indoors.conceals(&arena, pos.0) {
            continue;
        }
        let state = state.copied().unwrap_or_default();
        let r = monster.kind.radius();
        if monster.elite {
            let pulse = 0.55 + 0.25 * (t * 4.0 + pos.0.x).sin();
            paint.ring(pos.0, r + 6.0, ELITE_RING.with_alpha(pulse));
        }
        // Telegraphs: where the blow will land, and how soon.
        match (monster.kind, state.action) {
            (MonsterKind::Ghoul, MonsterAction::Arming) => {
                let blink = 0.5 + 0.5 * (t * (8.0 + 400.0 / (state.fuse as f32 + 30.0))).sin();
                paint.zone(
                    pos.0,
                    GHOUL_BLAST_RADIUS,
                    Color::srgb(1.0, 0.3, 0.15).with_alpha(0.18 + 0.2 * blink),
                );
            }
            (MonsterKind::Behemoth, MonsterAction::Slam) if state.fuse > 0 => {
                let urgency = 1.0 - (state.fuse as f32 / 96.0).min(1.0);
                let color = Color::srgb(1.0, 0.25, 0.1);
                paint.zone(
                    pos.0,
                    BEHEMOTH_SLAM_RADIUS,
                    color.with_alpha(0.35 + 0.45 * urgency),
                );
                paint.zone(
                    pos.0,
                    BEHEMOTH_SLAM_RADIUS * (0.2 + 0.8 * urgency),
                    color.with_alpha(0.3),
                );
            }
            (MonsterKind::Behemoth, MonsterAction::Charge) if state.fuse > 0 => {
                let blink = 0.35 + 0.3 * (t * 14.0).sin().abs();
                paint.line(
                    pos.0,
                    state.facing,
                    900.0,
                    2.0 * r,
                    Color::srgb(1.0, 0.25, 0.1).with_alpha(blink * 0.5),
                );
            }
            _ => {}
        }
        // The boss has its own bar at the top of the screen.
        if monster.kind == MonsterKind::Behemoth {
            continue;
        }
        // Monsters only show a health bar once they've been hurt (elites always do).
        let max = monster.kind.max_hp() * if monster.elite { ELITE_HP_MULT } else { 1.0 };
        if vitals.hp >= max && !monster.elite {
            continue;
        }
        let w = r * 2.0;
        let top = monster_top(monster, &textures, &images);
        let left = pos.0 + Vec2::new(-w / 2.0, top + 4.0);
        let frac = vitals.hp / max;
        paint.gauge(left, w, 4.0, frac, Color::srgb(0.6, 0.95, 0.3));
        if monster.elite {
            // Gold pip marks the bar.
            paint.bar(left + Vec2::new(w + 3.0, 0.0), 7.0, 7.0, ELITE_RING);
        }
    }

    // Crates show their damage; safes show how far along the cracking is.
    for (pos, vitals, prop) in &props {
        if indoors.conceals(&arena, pos.0) {
            continue;
        }
        let w = prop.kind.radius() * 2.0;
        let left = pos.0 + Vec2::new(-w / 2.0, prop.kind.radius() * 2.4 + 6.0);
        let (frac, color) = match prop.kind {
            PropKind::Safe if prop.progress > 0 => {
                (prop.progress as f32 / 100.0, Color::srgb(1.0, 0.85, 0.3))
            }
            PropKind::Crate | PropKind::Barrel if vitals.hp < prop.kind.max_hp() => {
                (vitals.hp / prop.kind.max_hp(), Color::srgb(0.95, 0.6, 0.3))
            }
            _ => continue,
        };
        paint.gauge(left, w, 4.0, frac, color);
    }

    for (pos, samples, vitals, stats, local) in &players {
        if !vitals.alive {
            continue;
        }
        let p = visual_pos(pos, samples, &fixed);
        if !local && indoors.conceals(&arena, p) {
            continue;
        }

        // Armor shell: thicker and brighter the better the tech.
        if vitals.armor_points > 0.0 {
            let color = tier_color(vitals.armor.tier());
            paint.ring(p, PLAYER_RADIUS + 3.0, color);
            if vitals.armor.tier() >= 4 {
                paint.ring(p, PLAYER_RADIUS + 6.0, color.with_alpha(0.6));
            }
        }
        if vitals.emp_slow > 0 {
            let r = PLAYER_RADIUS + 9.0 + (t * 20.0).sin() * 2.0;
            paint.ring(p, r, Color::srgb(0.4, 0.8, 1.0).with_alpha(0.7));
        }
        if vitals.shield > 0 {
            // Spawn protection: a shimmering bubble that flickers faster as it runs out.
            let left = vitals.shield as f32 / 160.0;
            let flicker = 0.45 + 0.35 * (t * (8.0 + 16.0 * (1.0 - left))).sin();
            let bubble = Color::srgb(0.55, 0.85, 1.0).with_alpha(flicker);
            let center = p + Vec2::Y * (SURVIVOR_HEIGHT / 2.0 - FOOT_INSET);
            paint.oval(center, Vec2::new(30.0, SURVIVOR_HEIGHT / 2.0 + 4.0), bubble);
            paint.ring(p, PLAYER_RADIUS + 6.0, bubble);
        }
        if stats.has_bounty() {
            let pulse = 0.6 + 0.4 * (t * 6.0).sin();
            paint.ring(
                p,
                PLAYER_RADIUS + 14.0,
                Color::srgb(1.0, 0.8, 0.1).with_alpha(pulse),
            );
        }

        // Health and armor bars.
        let bar_w = 44.0;
        let left = p + Vec2::new(-bar_w / 2.0, SURVIVOR_HEIGHT + 4.0);
        let hp = (vitals.hp / MAX_HP).clamp(0.0, 1.0);
        let hp_color = Color::srgb(1.0 - hp, 0.25 + 0.7 * hp, 0.25);
        paint.gauge(left, bar_w, 5.0, hp, hp_color);
        if vitals.armor_points > 0.0 {
            let ap = (vitals.armor_points / vitals.armor.max_points().max(1.0)).clamp(0.0, 1.0);
            let left = left + Vec2::Y * 4.0;
            paint.bar(left, bar_w * ap, 3.0, tier_color(vitals.armor.tier()));
        }

        if local {
            // A safe within reach pulses so you know you can crack it.
            for (prop_pos, _, prop) in &props {
                let reach = prop.kind.radius() + PLAYER_RADIUS + 40.0;
                if prop.kind == PropKind::Safe && !prop.open && prop_pos.0.distance(p) < reach {
                    paint.ring(
                        prop_pos.0,
                        prop.kind.radius() + 8.0,
                        Color::srgb(1.0, 0.85, 0.3).with_alpha(0.5 + 0.4 * (t * 6.0).sin()),
                    );
                }
            }
            // Show what's in reach.
            for pickup in &pickups {
                if pickup.0.distance(p) < 46.0 {
                    paint.disc(
                        pickup.0,
                        20.0,
                        Color::WHITE.with_alpha(0.5 + 0.5 * (t * 8.0).sin()),
                    );
                }
            }
        }
    }
}

/// Streak sprite size: the bright head sits at the right end, on the bullet.
pub fn tracer_size(weapon: WeaponKind) -> Vec2 {
    let (length, thickness) = match weapon {
        WeaponKind::Railgun => (170.0, 12.0),
        WeaponKind::Laser => (80.0, 8.0),
        WeaponKind::Rifle => (38.0, 7.0),
        WeaponKind::Bow => (32.0, 6.0),
        _ => (22.0, 6.0),
    };
    Vec2::new(length, thickness)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(runs: &[WallRun]) -> f32 {
        runs.iter().map(|r| (r.x1 - r.x0) * (r.y1 - r.y0)).sum()
    }

    #[test]
    fn overlapping_corner_becomes_disjoint_runs() {
        // An L: a horizontal bar whose end overlaps a vertical one.
        let walls = [
            Rect::new(-20.0, 0.0, 100.0, 10.0),
            Rect::new(-30.0, -100.0, -10.0, 5.0),
        ];
        let runs = wall_runs(&walls);
        // Union area: 120*10 + 20*105 - overlap(10*5).
        assert!(
            (area(&runs) - (1200.0 + 2100.0 - 50.0)).abs() < 0.1,
            "{runs:?}"
        );
        for (i, a) in runs.iter().enumerate() {
            for b in &runs[i + 1..] {
                let overlap_x = a.x0 < b.x1 - EPS && b.x0 < a.x1 - EPS;
                let overlap_y = a.y0 < b.y1 - EPS && b.y0 < a.y1 - EPS;
                assert!(!(overlap_x && overlap_y), "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn separate_walls_stay_whole() {
        let walls = [
            Rect::new(0.0, 0.0, 50.0, 10.0),
            Rect::new(100.0, 0.0, 150.0, 10.0),
        ];
        assert_eq!(wall_runs(&walls).len(), 2);
    }
}

//! World rendering in a three-quarter view: the ground is seen from above, while people,
//! monsters and walls stand up out of it. Everything upright is depth-sorted by where its
//! feet touch the ground, so walking behind a wall hides you and in front of it doesn't.
//!
//! Collision is unchanged: every position is still the point on the ground under the feet.

use bevy::prelude::*;
use bevy::sprite::{Anchor, SpriteImageMode};
use bevy::window::PrimaryWindow;
use evoclash_game::items::{WeaponKind, tier_color};
use evoclash_game::map::{ARENA_HALF, VAULT_CENTER};
use evoclash_game::protocol::*;
use evoclash_game::sim::{Arena, MAX_HP, PLAYER_RADIUS};
use rand::Rng;

use crate::input::{CursorWorld, GUN_HEIGHT, LocalPlayer};
use crate::predict::TickSamples;
use crate::textures::{self, Facing, PIXEL, SURVIVOR, Textures};

const Z_FLOOR: f32 = -10.0;
const Z_DECAL: f32 = -8.0;
const Z_PICKUP: f32 = -6.0;
const Z_SHADOW: f32 = -5.0;
const Z_PROJECTILE: f32 = 4.0;
/// Relative to the actor it labels, which sits around z = 2.
const Z_LABEL: f32 = 3.0;
const SPLAT_SECS: f32 = 8.0;
/// How tall walls stand, in world pixels.
const WALL_HEIGHT: f32 = 40.0;
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
    last: Vec2,
    flip: bool,
    phase: f32,
}

/// A ground shadow kept on the shadow layer however its parent is depth-sorted.
#[derive(Component)]
struct GroundShadow;

/// Goo left behind by a dead monster; fades out.
#[derive(Component)]
struct Splat {
    age: f32,
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
        app.init_resource::<CameraShake>();
        app.add_systems(PreStartup, (load_font, load_textures));
        app.add_systems(Startup, (spawn_camera, spawn_arena));
        app.add_observer(dress_player);
        app.add_observer(dress_pickup);
        app.add_observer(dress_monster);
        app.add_observer(dress_projectile);
        app.add_observer(splat_monster);
        app.add_systems(
            Update,
            (
                sync_players,
                pose_players,
                sync_pickups,
                bob_pickups,
                sync_monsters,
                pin_shadows,
                sync_projectiles,
                fade_splats,
                follow_camera,
                update_cursor,
                draw_overlays,
                draw_projectiles,
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

fn spawn_arena(mut commands: Commands, arena: Res<Arena>, textures: Res<Textures>) {
    commands.spawn((
        tiled(textures.ground.clone(), ARENA_HALF * 2.0),
        Transform::from_xyz(0.0, 0.0, Z_FLOOR),
    ));
    // Scorch marks and oil stains. Purely cosmetic, so a fresh layout each launch is fine.
    let mut rng = rand::rng();
    let random_spot = |rng: &mut rand::rngs::ThreadRng| {
        Vec2::new(
            rng.random_range(-ARENA_HALF.x..ARENA_HALF.x),
            rng.random_range(-ARENA_HALF.y..ARENA_HALF.y),
        )
    };
    for _ in 0..60 {
        let at = random_spot(&mut rng);
        let image = textures.decals[rng.random_range(0..textures.decals.len())].clone();
        let (sprite, transform) = pixel_sprite(image);
        commands.spawn((
            sprite,
            transform
                .with_translation(at.extend(Z_DECAL))
                .with_rotation(Quat::from_rotation_z(
                    rng.random_range(0.0..std::f32::consts::TAU),
                )),
        ));
    }
    // Upright clutter, kept out of walls and the vault so it never looks like cover.
    let mut placed = 0;
    while placed < 140 {
        let at = random_spot(&mut rng);
        if at.distance(VAULT_CENTER) < 260.0 || arena.resolve_circle(at, 30.0) != at {
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
            sprite,
            Anchor::BOTTOM_CENTER,
            transform.with_translation(at.extend(depth(at.y))),
        ));
    }
    // The vault: steel plates and a faded radiation sign.
    commands.spawn((
        tiled(textures.vault_floor.clone(), Vec2::splat(340.0)),
        Transform::from_translation(VAULT_CENTER.extend(Z_FLOOR + 1.0)),
    ));
    commands.spawn((
        Sprite {
            image: textures.trefoil.clone(),
            custom_size: Some(Vec2::splat(200.0)),
            color: Color::WHITE.with_alpha(0.28),
            ..default()
        },
        Transform::from_translation(VAULT_CENTER.extend(Z_FLOOR + 1.5)),
    ));
    spawn_walls(&mut commands, &textures, &arena.walls);
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

fn spawn_walls(commands: &mut Commands, textures: &Textures, walls: &[Rect]) {
    let runs = wall_runs(walls);
    let dark = Color::srgb(0.08, 0.06, 0.07);
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
        commands.spawn((
            Sprite::from_color(Color::srgba(0.0, 0.0, 0.0, 0.3), Vec2::new(w, depth_len)),
            Transform::from_translation(Vec3::new(
                cx + 10.0,
                run.y0 + depth_len / 2.0 - 6.0,
                Z_SHADOW,
            )),
        ));
        commands.spawn((
            tiled(textures.wall_front.clone(), Vec2::new(w, WALL_HEIGHT)),
            Transform::from_translation(at(run.y0 + WALL_HEIGHT / 2.0, 0.0)),
        ));
        commands.spawn((
            tiled(textures.wall_top.clone(), Vec2::new(w, depth_len)),
            Transform::from_translation(at(run.y0 + WALL_HEIGHT + depth_len / 2.0, Z_STEP)),
        ));
        // Sunlit lip where the cap meets the face, grime where the face meets the ground,
        // and a dark rim along the cap's far edge.
        let strip = |h: f32| Vec2::new(w, h);
        for (color, size, y) in [
            (
                Color::srgba(1.0, 0.95, 0.85, 0.35),
                strip(2.0),
                run.y0 + WALL_HEIGHT + 1.0,
            ),
            (Color::srgba(0.0, 0.0, 0.0, 0.35), strip(4.0), run.y0 + 2.0),
            (dark, strip(2.0), run.y1 + WALL_HEIGHT + 1.0),
            (dark, strip(2.0), run.y0 - 1.0),
        ] {
            commands.spawn((
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
            let h = depth_len + WALL_HEIGHT + 4.0;
            commands.spawn((
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
        *visibility = if vitals.alive {
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

fn monster_size(kind: MonsterKind, textures: &Textures, images: &Assets<Image>) -> Vec2 {
    let handle = match kind {
        MonsterKind::Roach => &textures.roach[0],
        MonsterKind::SporePod => &textures.pod,
    };
    images
        .get(handle)
        .map_or(Vec2::splat(32.0), |i| i.size_f32())
        * PIXEL
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
    let image = match monster.kind {
        MonsterKind::Roach => textures.roach[0].clone(),
        MonsterKind::SporePod => textures.pod.clone(),
    };
    let (sprite, transform) = pixel_sprite(image);
    let body = commands
        .spawn((
            sprite,
            Anchor::BOTTOM_CENTER,
            transform.with_translation(Vec3::Y * -FOOT_INSET),
        ))
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
    mut monsters: Query<(
        &Monster,
        &Pos,
        &mut MonsterParts,
        &mut Transform,
        &mut Visibility,
    )>,
    mut bodies: Query<(&mut Sprite, &mut Transform), Without<Monster>>,
) {
    let t = time.elapsed_secs();
    for (monster, pos, mut parts, mut transform, mut visibility) in &mut monsters {
        transform.translation = pos.0.extend(depth(pos.0.y));
        visibility.set_if_neq(Visibility::Visible);
        let Ok((mut sprite, mut body)) = bodies.get_mut(parts.body) else {
            continue;
        };
        match monster.kind {
            MonsterKind::Roach => {
                let moved = pos.0 - parts.last;
                let moving = moved.length_squared() > 0.01;
                parts.last = pos.0;
                if moved.x.abs() > 0.05 {
                    parts.flip = moved.x < 0.0;
                }
                sprite.flip_x = parts.flip;
                let frame = if moving {
                    (t * 16.0 + parts.phase) as usize % 2
                } else {
                    0
                };
                sprite.image = textures.roach[frame].clone();
            }
            MonsterKind::SporePod => {
                // Breathe: squash and stretch around the planted roots.
                let pulse = 0.05 * (t * 3.0 + parts.phase).sin();
                body.scale = Vec3::new(PIXEL * (1.0 + pulse), PIXEL * (1.0 - pulse), 1.0);
            }
        }
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

/// Spores get a glowing sprite; bullets stay as crisp gizmo tracers.
fn dress_projectile(
    trigger: On<Add<Projectile>>,
    projectiles: Query<&Projectile>,
    mut commands: Commands,
    textures: Res<Textures>,
) {
    let Ok(projectile) = projectiles.get(trigger.entity) else {
        return;
    };
    if projectile.kind == ProjectileKind::Spore {
        commands.entity(trigger.entity).insert((
            Sprite {
                image: textures.spore.clone(),
                custom_size: Some(Vec2::splat(22.0)),
                ..default()
            },
            Transform::from_xyz(0.0, 0.0, Z_PROJECTILE),
            Visibility::Hidden,
        ));
    }
}

fn sync_projectiles(
    mut projectiles: Query<
        (&Pos, &mut Transform, &mut Visibility),
        (With<Projectile>, With<Sprite>),
    >,
) {
    for (pos, mut transform, mut visibility) in &mut projectiles {
        transform.translation = (pos.0 + Vec2::Y * GUN_HEIGHT).extend(Z_PROJECTILE);
        visibility.set_if_neq(Visibility::Visible);
    }
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

#[allow(clippy::type_complexity)]
fn draw_overlays(
    mut gizmos: Gizmos,
    time: Res<Time>,
    fixed: Res<Time<Fixed>>,
    players: Query<(
        &Pos,
        Option<&TickSamples>,
        &Vitals,
        &Stats,
        Has<LocalPlayer>,
    )>,
    monsters: Query<(&Pos, &Vitals, &Monster)>,
    pickups: Query<&Pos, With<Pickup>>,
    textures: Res<Textures>,
    images: Res<Assets<Image>>,
) {
    let t = time.elapsed_secs();
    // Zone hints: the closer to the vault, the better the loot.
    gizmos.circle_2d(VAULT_CENTER, 620.0, tier_color(2).with_alpha(0.06));
    gizmos.circle_2d(VAULT_CENTER, 340.0, tier_color(3).with_alpha(0.08));

    // Monsters only show a health bar once they've been hurt.
    for (pos, vitals, monster) in &monsters {
        let max = monster.kind.max_hp();
        if vitals.hp >= max {
            continue;
        }
        let w = monster.kind.radius() * 2.0;
        let top = monster_size(monster.kind, &textures, &images).y;
        let left = pos.0 + Vec2::new(-w / 2.0, top + 4.0);
        gizmos.line_2d(left, left + Vec2::X * w, Color::srgba(0.0, 0.0, 0.0, 0.6));
        let frac = (vitals.hp / max).clamp(0.0, 1.0);
        gizmos.line_2d(left, left + Vec2::X * w * frac, Color::srgb(0.6, 0.95, 0.3));
    }

    for (pos, samples, vitals, stats, local) in &players {
        if !vitals.alive {
            continue;
        }
        let p = visual_pos(pos, samples, &fixed);

        // Armor shell: thicker and brighter the better the tech.
        if vitals.armor_points > 0.0 {
            let color = tier_color(vitals.armor.tier());
            ring(&mut gizmos, p, PLAYER_RADIUS + 3.0, color);
            if vitals.armor.tier() >= 4 {
                ring(&mut gizmos, p, PLAYER_RADIUS + 6.0, color.with_alpha(0.6));
            }
        }
        if vitals.emp_slow > 0 {
            let r = PLAYER_RADIUS + 9.0 + (t * 20.0).sin() * 2.0;
            ring(
                &mut gizmos,
                p,
                r,
                Color::srgb(0.4, 0.8, 1.0).with_alpha(0.7),
            );
        }
        if vitals.shield > 0 {
            // Spawn protection: a shimmering bubble that flickers faster as it runs out.
            let left = vitals.shield as f32 / 160.0;
            let flicker = 0.45 + 0.35 * (t * (8.0 + 16.0 * (1.0 - left))).sin();
            let bubble = Color::srgb(0.55, 0.85, 1.0).with_alpha(flicker);
            let center = p + Vec2::Y * (SURVIVOR_HEIGHT / 2.0 - FOOT_INSET);
            gizmos.ellipse_2d(
                Isometry2d::from_translation(center),
                Vec2::new(30.0, SURVIVOR_HEIGHT / 2.0 + 4.0),
                bubble,
            );
            ring(&mut gizmos, p, PLAYER_RADIUS + 6.0, bubble);
        }
        if stats.has_bounty() {
            let pulse = 0.6 + 0.4 * (t * 6.0).sin();
            ring(
                &mut gizmos,
                p,
                PLAYER_RADIUS + 14.0,
                Color::srgb(1.0, 0.8, 0.1).with_alpha(pulse),
            );
        }

        // Health and armor bars.
        let bar_w = 44.0;
        let left = p + Vec2::new(-bar_w / 2.0, SURVIVOR_HEIGHT + 4.0);
        gizmos.line_2d(
            left,
            left + Vec2::X * bar_w,
            Color::srgba(0.0, 0.0, 0.0, 0.6),
        );
        let hp = (vitals.hp / MAX_HP).clamp(0.0, 1.0);
        let hp_color = Color::srgb(1.0 - hp, 0.25 + 0.7 * hp, 0.25);
        gizmos.line_2d(left, left + Vec2::X * bar_w * hp, hp_color);
        if vitals.armor_points > 0.0 {
            let ap = (vitals.armor_points / vitals.armor.max_points().max(1.0)).clamp(0.0, 1.0);
            let left = left + Vec2::Y * 4.0;
            gizmos.line_2d(
                left,
                left + Vec2::X * bar_w * ap,
                tier_color(vitals.armor.tier()),
            );
        }

        if local {
            // Show what's in reach.
            for pickup in &pickups {
                if pickup.0.distance(p) < 46.0 {
                    gizmos.circle_2d(
                        pickup.0,
                        20.0,
                        Color::WHITE.with_alpha(0.5 + 0.5 * (t * 8.0).sin()),
                    );
                }
            }
        }
    }
}

/// Shots fly at chest height: drawn lifted, resolved on the ground below.
fn draw_projectiles(mut gizmos: Gizmos, projectiles: Query<(&Pos, &Projectile)>) {
    for (pos, projectile) in &projectiles {
        let at = pos.0 + Vec2::Y * GUN_HEIGHT;
        match projectile.kind {
            ProjectileKind::Shot(weapon) => {
                let tail = at - projectile.vel.normalize_or_zero() * tracer_length(weapon);
                gizmos.line_2d(tail, at, weapon.stats().color);
            }
            ProjectileKind::Emp => {
                gizmos.circle_2d(at, 8.0, Color::srgb(0.4, 0.8, 1.0));
                ring(&mut gizmos, pos.0, 6.0, Color::BLACK.with_alpha(0.4));
            }
            ProjectileKind::Spore => {}
        }
    }
}

/// A ring around someone's feet, flattened so it lies on the ground.
fn ring(gizmos: &mut Gizmos, at: Vec2, r: f32, color: Color) {
    gizmos.ellipse_2d(
        Isometry2d::from_translation(at),
        Vec2::new(r, r * 0.5),
        color,
    );
}

pub fn tracer_length(weapon: WeaponKind) -> f32 {
    match weapon {
        WeaponKind::Railgun => 140.0,
        WeaponKind::Laser => 60.0,
        WeaponKind::Rifle => 26.0,
        WeaponKind::Bow => 22.0,
        _ => 12.0,
    }
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

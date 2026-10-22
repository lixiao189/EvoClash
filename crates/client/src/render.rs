//! World rendering: arena, combatants, loot and projectiles.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use evoclash_game::items::{WeaponKind, tier_color};
use evoclash_game::map::{ARENA_HALF, VAULT_CENTER};
use evoclash_game::protocol::*;
use evoclash_game::sim::{Arena, MAX_HP, PLAYER_RADIUS};

use crate::input::{CursorWorld, LocalPlayer};
use crate::predict::TickSamples;

const Z_FLOOR: f32 = -10.0;
const Z_WALL: f32 = 0.0;
const Z_PICKUP: f32 = 1.0;
const Z_PLAYER: f32 = 2.0;
const Z_LABEL: f32 = 5.0;

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

pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ClearColor(Color::srgb(0.07, 0.065, 0.06)));
        app.init_resource::<CameraShake>();
        app.add_systems(PreStartup, load_font);
        app.add_systems(Startup, (spawn_camera, spawn_arena));
        app.add_observer(dress_player);
        app.add_observer(dress_pickup);
        app.add_systems(
            Update,
            (
                sync_players,
                sync_pickups,
                follow_camera,
                update_cursor,
                draw_overlays,
                draw_projectiles,
            )
                .chain(),
        );
    }
}

/// Text includes Chinese, which the built-in font lacks: load a CJK-capable system font.
fn load_font(mut commands: Commands, mut fonts: ResMut<Assets<Font>>) {
    const CANDIDATES: [&str; 7] = [
        "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        "/System/Library/Fonts/STHeiti Medium.ttc",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        "C:\\Windows\\Fonts\\msyh.ttc",
        "C:\\Windows\\Fonts\\simhei.ttf",
    ];
    let handle = CANDIDATES
        .iter()
        .find_map(|path| std::fs::read(path).ok().map(|bytes| (path, bytes)))
        .map(|(path, bytes)| {
            info!("using font {path}");
            fonts.add(Font::from_bytes(bytes))
        })
        .unwrap_or_else(|| {
            warn!("no CJK system font found; Chinese text will not render");
            Handle::default()
        });
    commands.insert_resource(UiFont(handle));
}

fn spawn_camera(mut commands: Commands) {
    commands.spawn((Camera2d, Transform::from_xyz(0.0, 0.0, 100.0)));
}

fn spawn_arena(mut commands: Commands, arena: Res<Arena>) {
    commands.spawn((
        Sprite::from_color(Color::srgb(0.16, 0.145, 0.12), ARENA_HALF * 2.0),
        Transform::from_xyz(0.0, 0.0, Z_FLOOR),
    ));
    // Irradiated vault floor.
    commands.spawn((
        Sprite::from_color(Color::srgb(0.2, 0.17, 0.09), Vec2::splat(340.0)),
        Transform::from_translation(VAULT_CENTER.extend(Z_FLOOR + 1.0)),
    ));
    for wall in &arena.walls {
        commands.spawn((
            Sprite::from_color(Color::srgb(0.36, 0.33, 0.3), wall.size()),
            Transform::from_translation(wall.center().extend(Z_WALL)),
        ));
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
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    font: Res<UiFont>,
) {
    let Ok(player) = players.get(trigger.entity) else {
        return;
    };
    commands
        .entity(trigger.entity)
        .insert((
            Transform::from_xyz(0.0, 0.0, Z_PLAYER),
            Visibility::Hidden,
            Mesh2d(meshes.add(Circle::new(PLAYER_RADIUS))),
            MeshMaterial2d(materials.add(ColorMaterial::from_color(player_color(player)))),
        ))
        .with_child((
            NameLabel,
            Text2d::new(player.name.clone()),
            font.sized(14.0),
            TextColor(Color::srgba(1.0, 1.0, 1.0, 0.85)),
            Transform::from_xyz(0.0, PLAYER_RADIUS + 34.0, Z_LABEL),
        ));
}

fn dress_pickup(
    trigger: On<Add<Pickup>>,
    pickups: Query<(&Pickup, &Pos)>,
    mut commands: Commands,
    font: Res<UiFont>,
) {
    let Ok((pickup, pos)) = pickups.get(trigger.entity) else {
        return;
    };
    let size = if pickup.item.tier() >= 4 { 30.0 } else { 20.0 };
    commands
        .entity(trigger.entity)
        .insert((
            Sprite::from_color(pickup.item.color(), Vec2::splat(size)),
            Transform::from_translation(pos.0.extend(Z_PICKUP)),
            Visibility::Visible,
        ))
        .with_child((
            Text2d::new(pickup.item.label()),
            font.sized(12.0),
            TextColor(pickup.item.color().with_alpha(0.9)),
            Transform::from_xyz(0.0, -24.0, Z_LABEL),
        ));
}

/// Rendered position: smoothed prediction for us, interpolated server state for everyone else.
pub fn visual_pos(pos: &Pos, samples: Option<&TickSamples>, fixed: &Time<Fixed>) -> Vec2 {
    samples.map_or(pos.0, |s| s.at(fixed.overstep_fraction()))
}

fn sync_players(
    fixed: Res<Time<Fixed>>,
    mut players: Query<(&Pos, Option<&TickSamples>, &Vitals, &mut Transform, &mut Visibility), With<Player>>,
) {
    for (pos, samples, vitals, mut transform, mut visibility) in &mut players {
        let p = visual_pos(pos, samples, &fixed);
        transform.translation = p.extend(Z_PLAYER);
        *visibility = if vitals.alive {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
}

fn sync_pickups(mut pickups: Query<(&Pos, &mut Transform), (With<Pickup>, Changed<Pos>)>) {
    for (pos, mut transform) in &mut pickups {
        transform.translation = pos.0.extend(Z_PICKUP);
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
    let me = visual_pos(pos, samples, &fixed);
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

fn weapon_length(weapon: WeaponKind) -> f32 {
    match weapon {
        WeaponKind::Stick => 26.0,
        WeaponKind::Bow => 20.0,
        WeaponKind::PipeShotgun => 24.0,
        WeaponKind::Rifle => 32.0,
        WeaponKind::Laser => 30.0,
        WeaponKind::Railgun => 44.0,
    }
}

#[allow(clippy::type_complexity)]
fn draw_overlays(
    mut gizmos: Gizmos,
    time: Res<Time>,
    fixed: Res<Time<Fixed>>,
    cursor: Res<CursorWorld>,
    players: Query<(
        &Pos,
        Option<&TickSamples>,
        &Aim,
        &Vitals,
        &Loadout,
        &Stats,
        Has<LocalPlayer>,
    )>,
    pickups: Query<&Pos, With<Pickup>>,
) {
    let t = time.elapsed_secs();
    // Zone hints: the closer to the vault, the better the loot.
    gizmos.circle_2d(VAULT_CENTER, 620.0, tier_color(2).with_alpha(0.06));
    gizmos.circle_2d(VAULT_CENTER, 340.0, tier_color(3).with_alpha(0.08));

    for (pos, samples, aim, vitals, loadout, stats, local) in &players {
        if !vitals.alive {
            continue;
        }
        let p = visual_pos(pos, samples, &fixed);
        let aim = if local {
            cursor.0.map(|c| (c - p).normalize_or(aim.0)).unwrap_or(aim.0)
        } else {
            aim.0
        };
        let stats_w = loadout.weapon.stats();
        let tip = p + aim * (PLAYER_RADIUS + weapon_length(loadout.weapon));
        gizmos.line_2d(p + aim * PLAYER_RADIUS * 0.6, tip, stats_w.color);

        // Armor shell: thicker and brighter the better the tech.
        if vitals.armor_points > 0.0 {
            let color = tier_color(vitals.armor.tier());
            gizmos.circle_2d(p, PLAYER_RADIUS + 3.0, color);
            if vitals.armor.tier() >= 4 {
                gizmos.circle_2d(p, PLAYER_RADIUS + 6.0, color.with_alpha(0.6));
            }
        }
        if vitals.emp_slow > 0 {
            let r = PLAYER_RADIUS + 9.0 + (t * 20.0).sin() * 2.0;
            gizmos.circle_2d(p, r, Color::srgb(0.4, 0.8, 1.0).with_alpha(0.7));
        }
        if stats.has_bounty() {
            let pulse = 0.6 + 0.4 * (t * 6.0).sin();
            gizmos.circle_2d(p, PLAYER_RADIUS + 14.0, Color::srgb(1.0, 0.8, 0.1).with_alpha(pulse));
        }

        // Health and armor bars.
        let bar_w = 44.0;
        let left = p + Vec2::new(-bar_w / 2.0, PLAYER_RADIUS + 10.0);
        gizmos.line_2d(left, left + Vec2::X * bar_w, Color::srgba(0.0, 0.0, 0.0, 0.6));
        let hp = (vitals.hp / MAX_HP).clamp(0.0, 1.0);
        let hp_color = Color::srgb(1.0 - hp, 0.25 + 0.7 * hp, 0.25);
        gizmos.line_2d(left, left + Vec2::X * bar_w * hp, hp_color);
        if vitals.armor_points > 0.0 {
            let ap = (vitals.armor_points / vitals.armor.max_points().max(1.0)).clamp(0.0, 1.0);
            let left = left + Vec2::Y * 4.0;
            gizmos.line_2d(left, left + Vec2::X * bar_w * ap, tier_color(vitals.armor.tier()));
        }

        if local {
            // Show what's in reach.
            for pickup in &pickups {
                if pickup.0.distance(p) < 46.0 {
                    gizmos.circle_2d(pickup.0, 20.0, Color::WHITE.with_alpha(0.5 + 0.5 * (t * 8.0).sin()));
                }
            }
        }
    }
}

fn draw_projectiles(mut gizmos: Gizmos, projectiles: Query<(&Pos, &Projectile)>) {
    for (pos, projectile) in &projectiles {
        match projectile.kind {
            ProjectileKind::Shot(weapon) => {
                let tail = pos.0 - projectile.vel.normalize_or_zero() * tracer_length(weapon);
                gizmos.line_2d(tail, pos.0, weapon.stats().color);
            }
            ProjectileKind::Emp => {
                gizmos.circle_2d(pos.0, 8.0, Color::srgb(0.4, 0.8, 1.0));
            }
        }
    }
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

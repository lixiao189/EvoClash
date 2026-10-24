//! Juice: instant local tracers, swings, explosions, damage numbers, hit markers and the kill feed.
//! Everything is a sprite; nothing here is drawn with gizmo lines.
//!
//! Your own shots are drawn the moment you click (the server's copy is never sent back to you),
//! so firing feels zero-latency even though hits are decided by the server.

use bevy::prelude::*;
use bevy::sprite::Anchor;
use evoclash_game::evolution::{Evolution, Skill};
use evoclash_game::items::WeaponKind;
use evoclash_game::protocol::*;
use evoclash_game::sim::{Arena, PLAYER_RADIUS};
use lightyear::prelude::input::native::ActionState;
use lightyear::prelude::*;
use rand::Rng;

use crate::hud::path_color;
use crate::input::{GUN_HEIGHT, LocalPlayer};
use crate::render::{CameraShake, UiFont, tracer_size};
use crate::textures::Textures;

const Z_FX: f32 = 7.0;

pub struct EffectsPlugin;

impl Plugin for EffectsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LocalGun>()
            .init_resource::<KillFeed>()
            .init_resource::<Banner>()
            .init_resource::<HitMarker>();
        app.add_systems(
            Update,
            (
                local_fire,
                step_tracers,
                receive_events,
                shake_on_damage,
                age_effects,
                update_reticle,
            )
                .chain(),
        );
    }
}

#[derive(Resource, Default)]
struct LocalGun {
    cooldown: f32,
}

#[derive(Resource, Default)]
pub struct KillFeed(pub Vec<(String, f32)>);

#[derive(Resource, Default)]
pub struct Banner(pub Option<(String, f32)>);

/// Seconds since our last confirmed hit (drawn as a crosshair flash).
#[derive(Resource, Default)]
pub struct HitMarker(pub f32);

#[derive(Component)]
struct Tracer {
    pos: Vec2,
    vel: Vec2,
    ttl: f32,
    weapon: WeaponKind,
}

#[derive(Component)]
struct Fx {
    kind: FxKind,
    pos: Vec2,
    tint: Color,
    age: f32,
    life: f32,
}

enum FxKind {
    /// Crescent sweeping around the swinger.
    Swing { dir: Vec2 },
    /// Expanding shock ring.
    Ring { radius: f32 },
    /// Billowing fireball that darkens as it fades.
    Puff { radius: f32 },
    /// Short white-hot flash at the center.
    Flash { radius: f32 },
    /// Ember flung outward.
    Spark { vel: Vec2 },
}

#[derive(Component)]
struct Reticle;

fn spawn_fx(
    commands: &mut Commands,
    textures: &Textures,
    kind: FxKind,
    pos: Vec2,
    tint: Color,
    life: f32,
) {
    let (image, z) = match kind {
        FxKind::Swing { .. } => (&textures.slash, Z_FX),
        FxKind::Ring { .. } => (&textures.shock, Z_FX - 0.2),
        FxKind::Puff { .. } => (&textures.blast, Z_FX - 0.1),
        FxKind::Flash { .. } => (&textures.glow, Z_FX),
        FxKind::Spark { .. } => (&textures.glow, Z_FX + 0.1),
    };
    commands.spawn((
        Fx {
            kind,
            pos,
            tint,
            age: 0.0,
            life,
        },
        Sprite {
            image: image.clone(),
            color: tint,
            custom_size: Some(Vec2::ZERO),
            ..default()
        },
        Transform::from_translation(pos.extend(z)),
    ));
}

fn spawn_swing(commands: &mut Commands, textures: &Textures, pos: Vec2, dir: Vec2) {
    spawn_fx(
        commands,
        textures,
        FxKind::Swing { dir },
        pos + Vec2::Y * GUN_HEIGHT,
        Color::srgb(1.0, 0.92, 0.75),
        0.15,
    );
}

fn spawn_boom(commands: &mut Commands, textures: &Textures, pos: Vec2, radius: f32, fire: bool) {
    let mut rng = rand::rng();
    let (shell, body, core, ember) = if fire {
        (
            Color::srgb(1.0, 0.6, 0.2),
            Color::srgb(1.0, 0.5, 0.12),
            Color::srgb(1.0, 0.95, 0.6),
            Color::srgb(1.0, 0.75, 0.25),
        )
    } else {
        (
            Color::srgb(0.4, 0.8, 1.0),
            Color::srgb(0.3, 0.65, 1.0),
            Color::srgb(0.85, 0.97, 1.0),
            Color::srgb(0.6, 0.9, 1.0),
        )
    };
    spawn_fx(
        commands,
        textures,
        FxKind::Ring { radius },
        pos,
        shell,
        0.45,
    );
    spawn_fx(
        commands,
        textures,
        FxKind::Puff { radius },
        pos,
        body,
        if fire { 0.5 } else { 0.35 },
    );
    spawn_fx(
        commands,
        textures,
        FxKind::Flash {
            radius: radius * 0.9,
        },
        pos,
        core,
        0.25,
    );
    for _ in 0..if fire { 14 } else { 9 } {
        let vel = Vec2::from_angle(rng.random_range(0.0..std::f32::consts::TAU))
            * radius
            * rng.random_range(1.0..3.0);
        spawn_fx(
            commands,
            textures,
            FxKind::Spark { vel },
            pos,
            ember,
            rng.random_range(0.3..0.55),
        );
    }
}

/// Damage numbers and "+N XP": drift up and fade out over `life` seconds.
#[derive(Component)]
struct FloatText {
    age: f32,
    life: f32,
}

fn spawn_float_text(
    commands: &mut Commands,
    font: &UiFont,
    text: String,
    size: f32,
    color: Color,
    at: Vec2,
    life: f32,
) {
    commands.spawn((
        FloatText { age: 0.0, life },
        Text2d::new(text),
        font.sized(size),
        TextColor(color),
        Transform::from_translation(at.extend(8.0)),
    ));
}

/// Golden column of light and sparks when anyone levels up.
fn spawn_level_burst(commands: &mut Commands, textures: &Textures, pos: Vec2) {
    let gold = Color::srgb(1.0, 0.85, 0.35);
    spawn_fx(
        commands,
        textures,
        FxKind::Ring { radius: 70.0 },
        pos,
        gold,
        0.7,
    );
    let chest = pos + Vec2::Y * GUN_HEIGHT;
    spawn_fx(
        commands,
        textures,
        FxKind::Flash { radius: 46.0 },
        chest,
        Color::srgb(1.0, 0.95, 0.7),
        0.45,
    );
    let mut rng = rand::rng();
    for _ in 0..14 {
        let vel = Vec2::new(rng.random_range(-60.0..60.0), rng.random_range(80.0..220.0));
        spawn_fx(
            commands,
            textures,
            FxKind::Spark { vel },
            pos + Vec2::new(rng.random_range(-16.0..16.0), rng.random_range(0.0..40.0)),
            gold,
            rng.random_range(0.5..0.9),
        );
    }
}

/// Snap of a sprung trap: a red flash with iron sparks.
fn spawn_trap_snap(commands: &mut Commands, textures: &Textures, pos: Vec2) {
    let red = Color::srgb(1.0, 0.3, 0.2);
    spawn_fx(
        commands,
        textures,
        FxKind::Ring { radius: 55.0 },
        pos,
        red,
        0.4,
    );
    spawn_fx(
        commands,
        textures,
        FxKind::Flash { radius: 36.0 },
        pos,
        Color::srgb(1.0, 0.75, 0.6),
        0.2,
    );
    let mut rng = rand::rng();
    for _ in 0..10 {
        let vel = Vec2::from_angle(rng.random_range(0.0..std::f32::consts::TAU))
            * rng.random_range(80.0..200.0);
        spawn_fx(
            commands,
            textures,
            FxKind::Spark { vel },
            pos,
            Color::srgb(0.85, 0.8, 0.7),
            rng.random_range(0.25..0.45),
        );
    }
}

fn local_fire(
    mut commands: Commands,
    textures: Res<Textures>,
    time: Res<Time>,
    mut gun: ResMut<LocalGun>,
    state: Query<&MatchState>,
    me: Query<
        (
            &Pos,
            &Vitals,
            &Loadout,
            &ActionState<PlayerInput>,
            Option<&Evolution>,
        ),
        With<LocalPlayer>,
    >,
) {
    gun.cooldown -= time.delta_secs();
    let Ok((pos, vitals, loadout, action, evo)) = me.single() else {
        return;
    };
    let input = &action.0;
    let intermission = state.iter().any(|s| s.phase == Phase::Intermission);
    if !input.fire || !vitals.alive || intermission || gun.cooldown > 0.0 {
        return;
    }
    let stats = loadout.weapon.stats();
    // 超频扳机 shortens the interval on the server; match it so tracers don't lag behind.
    let cooldown = evo.map_or(stats.cooldown, |e| e.fire_cooldown(stats.cooldown));
    gun.cooldown = cooldown as f32 / 64.0;
    let aim = input.aim.normalize_or(Vec2::X);
    if stats.melee {
        spawn_swing(&mut commands, &textures, pos.0, aim);
        return;
    }
    let mut rng = rand::rng();
    let base = aim.to_angle();
    for i in 0..stats.pellets {
        let fan = if stats.pellets > 1 {
            (i as f32 / (stats.pellets - 1) as f32 - 0.5) * stats.spread
        } else {
            0.0
        };
        let bloom = if stats.spread > 0.0 && stats.pellets == 1 {
            rng.random_range(-stats.spread..stats.spread)
        } else {
            0.0
        };
        let dir = Vec2::from_angle(base + fan + bloom);
        let start = pos.0 + dir * (PLAYER_RADIUS + 4.0);
        commands.spawn((
            Tracer {
                pos: start,
                vel: dir * stats.range_or_speed,
                ttl: stats.lifetime as f32 / 64.0,
                weapon: loadout.weapon,
            },
            Sprite {
                image: textures.bolt.clone(),
                color: stats.color,
                custom_size: Some(tracer_size(loadout.weapon)),
                ..default()
            },
            Anchor::CENTER_RIGHT,
            Transform::from_translation((start + Vec2::Y * GUN_HEIGHT).extend(Z_FX))
                .with_rotation(Quat::from_rotation_z(dir.to_angle())),
        ));
    }
}

fn step_tracers(
    mut commands: Commands,
    time: Res<Time>,
    arena: Res<Arena>,
    mut tracers: Query<(Entity, &mut Tracer, &mut Transform)>,
    me: Query<&Player, With<LocalPlayer>>,
    others: Query<
        (&Pos, &Vitals, Option<&Turret>),
        (
            Or<(With<Player>, With<Monster>, With<Prop>, With<Turret>)>,
            Without<LocalPlayer>,
        ),
    >,
) {
    let dt = time.delta_secs();
    let my_id = me.single().ok().map(|p| p.client_id);
    for (entity, mut tracer, mut transform) in &mut tracers {
        let from = tracer.pos;
        let to = from + tracer.vel * dt;
        tracer.ttl -= dt;
        let piercing = tracer.weapon.stats().piercing;
        let blocked = !piercing
            && (arena.segment_hits_wall(from, to).is_some()
                || others.iter().any(|(p, v, turret)| {
                    // Our own turret never takes our fire.
                    v.alive
                        && turret.is_none_or(|t| Some(t.owner) != my_id)
                        && evoclash_game::map::segment_circle(from, to, p.0, PLAYER_RADIUS)
                            .is_some()
                }));
        if blocked || tracer.ttl <= 0.0 {
            commands.entity(entity).despawn();
        } else {
            tracer.pos = to;
            transform.translation = (to + Vec2::Y * GUN_HEIGHT).extend(Z_FX);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn receive_events(
    mut commands: Commands,
    textures: Res<Textures>,
    time: Res<Time>,
    font: Res<UiFont>,
    mut receivers: Query<&mut MessageReceiver<GameEvent>, With<Client>>,
    me: Query<(&Player, &Pos), With<LocalPlayer>>,
    players: Query<(&Player, &Pos)>,
    mut feed: ResMut<KillFeed>,
    mut banner: ResMut<Banner>,
    mut marker: ResMut<HitMarker>,
    mut shake: ResMut<CameraShake>,
) {
    let now = time.elapsed_secs();
    let me = me.single().ok();
    for mut receiver in &mut receivers {
        for event in receiver.receive() {
            match event {
                GameEvent::Kill {
                    killer,
                    victim,
                    how,
                    bounty,
                } => {
                    let tag = if bounty { " ★悬赏" } else { "" };
                    feed.0
                        .push((format!("{killer} [{how}] {victim}{tag}"), now));
                    if feed.0.len() > 6 {
                        feed.0.remove(0);
                    }
                }
                GameEvent::Announce(text) => banner.0 = Some((text, now)),
                GameEvent::Hit {
                    pos,
                    amount,
                    armor,
                    attacker,
                } => {
                    if me.is_some_and(|(p, _)| p.client_id == attacker) {
                        marker.0 = 0.0;
                    }
                    let color = if armor {
                        Color::srgb(0.5, 0.8, 1.0)
                    } else {
                        Color::srgb(1.0, 0.85, 0.3)
                    };
                    let jitter = Vec2::new(rand::rng().random_range(-14.0..14.0), 0.0);
                    spawn_float_text(
                        &mut commands,
                        &font,
                        format!("{amount:.0}"),
                        16.0,
                        color,
                        pos + jitter + Vec2::Y * 64.0,
                        0.8,
                    );
                }
                GameEvent::Melee { pos, dir } => {
                    // Our own swing was already drawn locally.
                    if me.is_some_and(|(_, p)| p.0.distance(pos) < 40.0) {
                        continue;
                    }
                    spawn_swing(&mut commands, &textures, pos, dir);
                }
                GameEvent::Boom { pos, radius, fire } => {
                    if me.is_some_and(|(_, p)| p.0.distance(pos) < radius * 1.5) {
                        shake.0 = shake.0.max(if fire { 2.0 } else { 1.2 });
                    }
                    spawn_boom(&mut commands, &textures, pos, radius, fire);
                }
                // Only ever sent to whoever earned it.
                GameEvent::XpGain { amount, pos } => spawn_float_text(
                    &mut commands,
                    &font,
                    format!("+{amount} XP"),
                    15.0,
                    Color::srgb(0.55, 1.0, 0.65),
                    pos + Vec2::Y * 84.0,
                    1.3,
                ),
                GameEvent::LevelUp { client, level } => {
                    let Some((_, pos)) = players.iter().find(|(p, _)| p.client_id == client) else {
                        continue;
                    };
                    spawn_level_burst(&mut commands, &textures, pos.0);
                    spawn_float_text(
                        &mut commands,
                        &font,
                        format!("升级 Lv{level}"),
                        18.0,
                        Color::srgb(1.0, 0.85, 0.35),
                        pos.0 + Vec2::Y * 100.0,
                        1.6,
                    );
                    if me.is_some_and(|(p, _)| p.client_id == client) {
                        banner.0 = Some((format!("升到 {level} 级 — 按 1/2/3 选择进化"), now));
                    }
                }
                GameEvent::SkillUsed {
                    client,
                    skill,
                    pos,
                    dir,
                } => {
                    let mine = me.is_some_and(|(p, _)| p.client_id == client);
                    // Never give away where someone else laid a trap.
                    if skill == Skill::Trap && !mine {
                        continue;
                    }
                    let color = skill_color(skill);
                    let at = match skill {
                        Skill::AcidBreath => pos + dir * 30.0 + Vec2::Y * GUN_HEIGHT,
                        Skill::Berserk | Skill::EnergyShield | Skill::Stealth => {
                            pos + Vec2::Y * GUN_HEIGHT
                        }
                        Skill::AutoTurret | Skill::Trap => pos,
                    };
                    spawn_fx(
                        &mut commands,
                        &textures,
                        FxKind::Ring { radius: 48.0 },
                        pos,
                        color,
                        0.4,
                    );
                    spawn_fx(
                        &mut commands,
                        &textures,
                        FxKind::Flash { radius: 30.0 },
                        at,
                        color,
                        0.25,
                    );
                }
                GameEvent::TrapSprung { pos } => {
                    if me.is_some_and(|(_, p)| p.0.distance(pos) < 80.0) {
                        shake.0 = shake.0.max(1.0);
                    }
                    spawn_trap_snap(&mut commands, &textures, pos);
                }
            }
        }
    }
}

/// The skill's path color.
fn skill_color(skill: Skill) -> Color {
    let path = evoclash_game::evolution::EvoPath::ALL
        .into_iter()
        .find(|p| {
            let (a, b) = p.skills();
            a == skill || b == skill
        })
        .unwrap_or(evoclash_game::evolution::EvoPath::Mutant);
    path_color(path)
}

fn shake_on_damage(
    me: Query<&Vitals, With<LocalPlayer>>,
    mut last: Local<Option<f32>>,
    mut shake: ResMut<CameraShake>,
) {
    let Ok(vitals) = me.single() else {
        return;
    };
    let total = vitals.hp + vitals.armor_points;
    if let Some(prev) = *last
        && total < prev - 0.5
    {
        shake.0 = shake.0.max(((prev - total) / 25.0).clamp(0.3, 2.0));
    }
    *last = Some(total);
}

fn age_effects(
    mut commands: Commands,
    time: Res<Time>,
    mut fx: Query<(Entity, &mut Fx, &mut Sprite, &mut Transform)>,
    mut numbers: Query<(Entity, &mut FloatText, &mut Transform, &mut TextColor), Without<Fx>>,
    mut marker: ResMut<HitMarker>,
) {
    let dt = time.delta_secs();
    marker.0 += dt;
    for (entity, mut fx, mut sprite, mut transform) in &mut fx {
        let fx = &mut *fx;
        fx.age += dt;
        if fx.age > fx.life {
            commands.entity(entity).despawn();
            continue;
        }
        let t = fx.age / fx.life;
        let fade = 1.0 - t;
        let tint = fx.tint.to_srgba();
        let (size, alpha, tint) = match fx.kind {
            FxKind::Swing { dir } => {
                let reach = WeaponKind::Stick.stats().range_or_speed + PLAYER_RADIUS;
                transform.rotation = Quat::from_rotation_z(dir.to_angle() - 1.0 + 2.0 * t);
                (2.0 * reach, fade, tint)
            }
            FxKind::Ring { radius } => (2.0 * radius * (0.3 + 0.7 * t) / 0.85, fade, tint),
            FxKind::Puff { radius } => {
                let grow = 1.0 - fade * fade;
                // Cool from the base color toward soot.
                let soot = Srgba::new(0.12, 0.08, 0.07, 1.0);
                (
                    2.0 * radius * (0.45 + 0.55 * grow) / 0.72,
                    fade.powf(1.3),
                    Srgba::new(
                        tint.red + (soot.red - tint.red) * t,
                        tint.green + (soot.green - tint.green) * t,
                        tint.blue + (soot.blue - tint.blue) * t,
                        1.0,
                    ),
                )
            }
            FxKind::Flash { radius } => (2.0 * radius * (1.0 - 0.4 * t), fade * fade, tint),
            FxKind::Spark { ref mut vel } => {
                fx.pos += *vel * dt;
                *vel *= 0.93;
                (3.0 + 9.0 * fade, fade, tint)
            }
        };
        sprite.custom_size = Some(Vec2::splat(size));
        sprite.color = tint.with_alpha(alpha).into();
        transform.translation.x = fx.pos.x;
        transform.translation.y = fx.pos.y;
    }
    for (entity, mut number, mut transform, mut color) in &mut numbers {
        number.age += dt;
        transform.translation.y += 50.0 * dt;
        color.0 = color
            .0
            .with_alpha((1.0 - number.age / number.life).max(0.0));
        if number.age > number.life {
            commands.entity(entity).despawn();
        }
    }
}

fn update_reticle(
    mut commands: Commands,
    textures: Res<Textures>,
    marker: Res<HitMarker>,
    cursor: Res<crate::input::CursorWorld>,
    mut reticle: Query<(&mut Sprite, &mut Transform, &mut Visibility), With<Reticle>>,
) {
    let Ok((mut sprite, mut transform, mut visibility)) = reticle.single_mut() else {
        commands.spawn((
            Reticle,
            Sprite::from_image(textures.reticle.clone()),
            Transform::from_xyz(0.0, 0.0, Z_FX + 1.0),
            Visibility::Hidden,
        ));
        return;
    };
    let Some(c) = cursor.0 else {
        visibility.set_if_neq(Visibility::Hidden);
        return;
    };
    visibility.set_if_neq(Visibility::Visible);
    let flash = marker.0 < 0.12;
    sprite.color = if flash {
        Color::srgb(1.0, 0.3, 0.2)
    } else {
        Color::WHITE.with_alpha(0.8)
    };
    transform.scale = Vec3::splat(if flash { 1.4 } else { 1.0 });
    transform.translation = c.extend(Z_FX + 1.0);
}

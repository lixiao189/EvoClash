//! Juice: instant local tracers, swings, explosions, damage numbers, hit markers and the kill feed.
//!
//! Your own shots are drawn the moment you click (the server's copy is never sent back to you),
//! so firing feels zero-latency even though hits are decided by the server.

use bevy::prelude::*;
use evoclash_game::items::WeaponKind;
use evoclash_game::protocol::*;
use evoclash_game::sim::{Arena, PLAYER_RADIUS};
use lightyear::prelude::input::native::ActionState;
use lightyear::prelude::*;
use rand::Rng;

use crate::input::{GUN_HEIGHT, LocalPlayer};
use crate::render::{CameraShake, UiFont, tracer_length};

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
                draw_effects,
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
    age: f32,
    life: f32,
}

enum FxKind {
    Swing {
        pos: Vec2,
        dir: Vec2,
    },
    Ring {
        pos: Vec2,
        radius: f32,
        color: Color,
    },
}

#[derive(Component)]
struct DamageNumber {
    age: f32,
}

fn local_fire(
    mut commands: Commands,
    time: Res<Time>,
    mut gun: ResMut<LocalGun>,
    state: Query<&MatchState>,
    me: Query<(&Pos, &Vitals, &Loadout, &ActionState<PlayerInput>), With<LocalPlayer>>,
) {
    gun.cooldown -= time.delta_secs();
    let Ok((pos, vitals, loadout, action)) = me.single() else {
        return;
    };
    let input = &action.0;
    let intermission = state.iter().any(|s| s.phase == Phase::Intermission);
    if !input.fire || !vitals.alive || intermission || gun.cooldown > 0.0 {
        return;
    }
    let stats = loadout.weapon.stats();
    gun.cooldown = stats.cooldown as f32 / 64.0;
    let aim = input.aim.normalize_or(Vec2::X);
    if stats.melee {
        commands.spawn(Fx {
            kind: FxKind::Swing {
                pos: pos.0,
                dir: aim,
            },
            age: 0.0,
            life: 0.15,
        });
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
        commands.spawn(Tracer {
            pos: pos.0 + dir * (PLAYER_RADIUS + 4.0),
            vel: dir * stats.range_or_speed,
            ttl: stats.lifetime as f32 / 64.0,
            weapon: loadout.weapon,
        });
    }
}

fn step_tracers(
    mut commands: Commands,
    time: Res<Time>,
    arena: Res<Arena>,
    mut tracers: Query<(Entity, &mut Tracer)>,
    others: Query<
        (&Pos, &Vitals),
        (
            Or<(With<Player>, With<Monster>, With<Prop>)>,
            Without<LocalPlayer>,
        ),
    >,
) {
    let dt = time.delta_secs();
    for (entity, mut tracer) in &mut tracers {
        let from = tracer.pos;
        let to = from + tracer.vel * dt;
        tracer.ttl -= dt;
        let piercing = tracer.weapon.stats().piercing;
        let blocked = !piercing
            && (arena.segment_hits_wall(from, to).is_some()
                || others.iter().any(|(p, v)| {
                    v.alive
                        && evoclash_game::map::segment_circle(from, to, p.0, PLAYER_RADIUS)
                            .is_some()
                }));
        if blocked || tracer.ttl <= 0.0 {
            commands.entity(entity).despawn();
        } else {
            tracer.pos = to;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn receive_events(
    mut commands: Commands,
    time: Res<Time>,
    font: Res<UiFont>,
    mut receivers: Query<&mut MessageReceiver<GameEvent>, With<Client>>,
    me: Query<(&Player, &Pos), With<LocalPlayer>>,
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
                    commands.spawn((
                        DamageNumber { age: 0.0 },
                        Text2d::new(format!("{amount:.0}")),
                        font.sized(16.0),
                        TextColor(color),
                        Transform::from_translation((pos + jitter + Vec2::Y * 64.0).extend(8.0)),
                    ));
                }
                GameEvent::Melee { pos, dir } => {
                    // Our own swing was already drawn locally.
                    if me.is_some_and(|(_, p)| p.0.distance(pos) < 40.0) {
                        continue;
                    }
                    commands.spawn(Fx {
                        kind: FxKind::Swing { pos, dir },
                        age: 0.0,
                        life: 0.15,
                    });
                }
                GameEvent::Boom { pos, radius, fire } => {
                    if me.is_some_and(|(_, p)| p.0.distance(pos) < radius * 1.5) {
                        shake.0 = shake.0.max(if fire { 2.0 } else { 1.2 });
                    }
                    let color = if fire {
                        Color::srgb(1.0, 0.55, 0.15)
                    } else {
                        Color::srgb(0.4, 0.8, 1.0)
                    };
                    commands.spawn(Fx {
                        kind: FxKind::Ring { pos, radius, color },
                        age: 0.0,
                        life: 0.45,
                    });
                    if fire {
                        // A hot core inside the shock ring.
                        commands.spawn(Fx {
                            kind: FxKind::Ring {
                                pos,
                                radius: radius * 0.5,
                                color: Color::srgb(1.0, 0.9, 0.4),
                            },
                            age: 0.0,
                            life: 0.3,
                        });
                    }
                }
            }
        }
    }
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
    mut fx: Query<(Entity, &mut Fx)>,
    mut numbers: Query<(Entity, &mut DamageNumber, &mut Transform, &mut TextColor)>,
    mut marker: ResMut<HitMarker>,
) {
    let dt = time.delta_secs();
    marker.0 += dt;
    for (entity, mut fx) in &mut fx {
        fx.age += dt;
        if fx.age > fx.life {
            commands.entity(entity).despawn();
        }
    }
    for (entity, mut number, mut transform, mut color) in &mut numbers {
        number.age += dt;
        transform.translation.y += 50.0 * dt;
        color.0 = color.0.with_alpha((1.0 - number.age / 0.8).max(0.0));
        if number.age > 0.8 {
            commands.entity(entity).despawn();
        }
    }
}

fn draw_effects(
    mut gizmos: Gizmos,
    tracers: Query<&Tracer>,
    fx: Query<&Fx>,
    marker: Res<HitMarker>,
    cursor: Res<crate::input::CursorWorld>,
) {
    let lift = Vec2::Y * GUN_HEIGHT;
    for tracer in &tracers {
        let tail = tracer.pos - tracer.vel.normalize_or_zero() * tracer_length(tracer.weapon);
        gizmos.line_2d(tail + lift, tracer.pos + lift, tracer.weapon.stats().color);
    }
    for fx in &fx {
        let t = fx.age / fx.life;
        match fx.kind {
            FxKind::Swing { pos, dir } => {
                let reach = WeaponKind::Stick.stats().range_or_speed + PLAYER_RADIUS;
                let base = dir.to_angle();
                let sweep = -1.0 + 2.0 * t;
                let a = Vec2::from_angle(base + sweep * 1.0);
                gizmos.line_2d(
                    pos + lift + a * PLAYER_RADIUS,
                    pos + lift + a * reach,
                    Color::srgba(1.0, 0.9, 0.7, 1.0 - t),
                );
            }
            FxKind::Ring { pos, radius, color } => {
                gizmos.circle_2d(pos, radius * (0.3 + 0.7 * t), color.with_alpha(1.0 - t));
            }
        }
    }
    if let Some(c) = cursor.0 {
        let flash = marker.0 < 0.12;
        let (size, color) = if flash {
            (11.0, Color::srgb(1.0, 0.3, 0.2))
        } else {
            (7.0, Color::WHITE.with_alpha(0.8))
        };
        for d in [Vec2::X, Vec2::Y, -Vec2::X, -Vec2::Y] {
            gizmos.line_2d(c + d * 4.0, c + d * size, color);
        }
    }
}

//! Monsters, their splats and puddles, and the arrow toward the boss.

use super::*;

pub(super) fn monster_image(kind: MonsterKind, textures: &Textures) -> Handle<Image> {
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

pub(super) fn monster_size(kind: MonsterKind, textures: &Textures, images: &Assets<Image>) -> Vec2 {
    images
        .get(&monster_image(kind, textures))
        .map_or(Vec2::splat(32.0), |i| i.size_f32())
        * PIXEL
}

/// Height of the top of a monster's sprite above its ground point.
pub(super) fn monster_top(monster: &Monster, textures: &Textures, images: &Assets<Image>) -> f32 {
    let size = monster_size(monster.kind, textures, images);
    let top = match monster.kind {
        MonsterKind::Drone => size.y + DRONE_HOVER - FOOT_INSET,
        MonsterKind::Crawler => CRAWLER_LIFT + size.y / 2.0,
        _ => size.y,
    };
    top * if monster.elite { ELITE_SCALE } else { 1.0 }
}

pub(super) fn dress_monster(
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

pub(super) fn sync_monsters(
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

pub(super) fn splat_monster(
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

pub(super) fn fade_splats(
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

pub(super) fn age_puddles(
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
pub(super) fn boss_arrow(
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

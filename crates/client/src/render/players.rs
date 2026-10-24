//! Combatants: dressing, walk cycle, aim and status tints.

use super::*;

pub(super) fn player_color(player: &Player) -> Color {
    let [r, g, b] = player.color;
    Color::srgb(r, g, b)
}

pub(super) fn dress_player(
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

pub(super) fn dress_pickup(
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

pub(super) fn sync_players(
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
pub(super) fn pose_players(
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

/// 狂暴 glows red, its aftermath looks drained, and 隐身 fades the whole figure.
#[allow(clippy::type_complexity)]
pub(super) fn tint_players(
    time: Res<Time>,
    players: Query<(
        &Player,
        &Vitals,
        Option<&SkillState>,
        &PlayerParts,
        &Children,
        Has<LocalPlayer>,
    )>,
    mut sprites: Query<(&mut Sprite, Has<GroundShadow>), Without<Player>>,
    mut labels: Query<&mut TextColor, With<NameLabel>>,
) {
    let t = time.elapsed_secs();
    for (player, vitals, skills, parts, children, local) in &players {
        let skills = skills.copied().unwrap_or_default();
        let alpha = if vitals.stealth > 0 && vitals.revealed == 0 {
            if local {
                STEALTH_SELF_ALPHA
            } else {
                STEALTH_ALPHA
            }
        } else {
            1.0
        };
        let (tint, amount) = if skills.berserk() {
            (Color::srgb(1.0, 0.2, 0.15), 0.4 + 0.15 * (t * 10.0).sin())
        } else if skills.exhausted() {
            (Color::srgb(0.45, 0.42, 0.4), 0.45)
        } else {
            (Color::WHITE, 0.0)
        };
        for child in children.iter() {
            let Ok((mut sprite, is_shadow)) = sprites.get_mut(child) else {
                continue;
            };
            let color = if is_shadow {
                Color::WHITE.with_alpha(0.6 * alpha)
            } else {
                let base = if child == parts.jacket || child == parts.arm {
                    player_color(player)
                } else {
                    Color::WHITE
                };
                base.mix(&tint, amount).with_alpha(alpha)
            };
            if sprite.color != color {
                sprite.color = color;
            }
        }
        for child in children.iter() {
            if let Ok(mut label) = labels.get_mut(child) {
                label.0 = label.0.with_alpha(0.85 * alpha);
            }
        }
    }
}

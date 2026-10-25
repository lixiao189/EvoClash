//! Combatants: dressing, walk cycle, aim, attack moves and status tints.

use super::*;

/// A melee swing: cock back over `SWING_WINDUP`, sweep through, then settle back on the aim.
const SWING_WINDUP: f32 = 0.04;
const SWING_SWEEP: f32 = 0.12;
const SWING_RECOVER: f32 = 0.14;
/// Radians behind and past the aim the swing reaches.
const SWING_BACK: f32 = 1.2;
const SWING_THROUGH: f32 = 1.1;
/// How far the hands thrust toward the target mid-swing, in world pixels.
const SWING_LUNGE: f32 = 6.0;
/// Muzzle flash lifetime and its size for a rifle, in world pixels.
const FLASH_SECS: f32 = 0.06;
const FLASH_SIZE: Vec2 = Vec2::new(22.0, 14.0);
/// A new shot is credited to someone at most this far behind it along its line of fire,
/// and this far to either side of it.
const SHOT_MATCH_RANGE: f32 = 160.0;
const SHOT_MATCH_OFF: f32 = 24.0;

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
    let flash = commands
        .spawn((
            Sprite::from_image(textures.muzzle.clone()),
            Anchor::CENTER_LEFT,
            Transform::default(),
            Visibility::Hidden,
        ))
        .id();
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
                flash,
                shown: Some(WeaponKind::Stick),
                last: Vec2::ZERO,
                stride: 0.0,
            },
        ))
        .add_children(&[shadow, jacket, gear, weapon, arm, flash])
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

/// Others' shots only reach us as replicated projectiles; credit whoever's barrel a new one left.
pub(super) fn strike_on_shot(
    time: Res<Time>,
    mut commands: Commands,
    shots: Query<(&Projectile, &Pos), Added<Projectile>>,
    players: Query<(Entity, &Pos, &Vitals, &Loadout), (With<Player>, Without<LocalPlayer>)>,
) {
    for (projectile, shot) in &shots {
        let ProjectileKind::Shot(weapon) = projectile.kind else {
            continue;
        };
        let dir = projectile.vel.normalize_or_zero();
        // By the time it replicates the shot has flown a little way down the line of fire.
        let shooter = players
            .iter()
            .filter(|(_, _, vitals, loadout)| vitals.alive && loadout.weapon == weapon)
            .filter_map(|(entity, pos, _, _)| {
                let back = shot.0 - pos.0;
                let along = back.dot(dir);
                let off = (back - dir * along).length();
                ((-PLAYER_RADIUS..SHOT_MATCH_RANGE).contains(&along) && off < SHOT_MATCH_OFF)
                    .then_some((entity, along))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((entity, _)) = shooter {
            commands.entity(entity).insert(Strike(time.elapsed_secs()));
        }
    }
}

/// How a gun kicks: (seconds to settle, world pixels pushed back, radians the muzzle climbs).
fn recoil(weapon: WeaponKind) -> (f32, f32, f32) {
    match weapon {
        WeaponKind::Stick => (0.0, 0.0, 0.0),
        WeaponKind::Bow => (0.14, 2.0, 0.0),
        WeaponKind::PipeShotgun => (0.22, 7.0, 0.35),
        WeaponKind::Rifle => (0.09, 3.5, 0.08),
        WeaponKind::Laser => (0.1, 2.5, 0.05),
        WeaponKind::Railgun => (0.3, 9.0, 0.28),
    }
}

/// Muzzle flash scale relative to a rifle's; the bow has none.
fn flash_scale(weapon: WeaponKind) -> f32 {
    match weapon {
        WeaponKind::Stick | WeaponKind::Bow => 0.0,
        WeaponKind::PipeShotgun => 1.4,
        WeaponKind::Rifle => 1.0,
        WeaponKind::Laser => 0.85,
        WeaponKind::Railgun => 1.7,
    }
}

/// How an attack bends the held pose.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AttackPose {
    /// Radians the weapon is turned off the aim.
    pub swing: f32,
    /// World pixels the hands move from where they rest.
    pub push: Vec2,
    /// Muzzle flash size in world pixels (its left edge sits on the muzzle) and opacity;
    /// zero opacity when there is none.
    pub flash: Vec2,
    pub flash_alpha: f32,
}

/// The pose `since` seconds after a strike with `weapon` aimed along `aim`: a melee weapon arcs
/// through the aim with a little lunge, a gun kicks back and its muzzle climbs, then both settle.
pub fn attack_pose(weapon: WeaponKind, aim: Vec2, since: f32) -> AttackPose {
    if weapon.melee().is_some() {
        let total = SWING_WINDUP + SWING_SWEEP + SWING_RECOVER;
        let t = since.min(total);
        let lunge = SWING_LUNGE * (std::f32::consts::PI * t / total).sin();
        return AttackPose {
            swing: swing_side(aim) * swing_angle(t),
            push: aim * lunge,
            flash: Vec2::ZERO,
            flash_alpha: 0.0,
        };
    }
    let (secs, back, kick) = recoil(weapon);
    let k = (1.0 - since / secs).clamp(0.0, 1.0).powi(2);
    let climb = if aim.x < 0.0 { -1.0 } else { 1.0 };
    let lit = (1.0 - since / FLASH_SECS).max(0.0);
    AttackPose {
        swing: climb * kick * k,
        push: -aim * back * k,
        flash: FLASH_SIZE * flash_scale(weapon) * (0.7 + 0.3 * lit),
        flash_alpha: if flash_scale(weapon) > 0.0 {
            lit.sqrt()
        } else {
            0.0
        },
    }
}

/// Turns `swing_angle` so a swing chops down from overhead whichever way you face.
pub fn swing_side(aim: Vec2) -> f32 {
    if aim.x < 0.0 { 1.0 } else { -1.0 }
}

/// Angle off the aim (counter-clockwise when aiming left): cock back, sweep through, settle.
fn swing_angle(t: f32) -> f32 {
    let ease = |u: f32| {
        let u = u.clamp(0.0, 1.0);
        u * u * (3.0 - 2.0 * u)
    };
    if t < SWING_WINDUP {
        -SWING_BACK * ease(t / SWING_WINDUP)
    } else if t < SWING_WINDUP + SWING_SWEEP {
        let u = (t - SWING_WINDUP) / SWING_SWEEP;
        -SWING_BACK + (SWING_BACK + SWING_THROUGH) * (1.0 - (1.0 - u) * (1.0 - u))
    } else {
        SWING_THROUGH * (1.0 - ease((t - SWING_WINDUP - SWING_SWEEP) / SWING_RECOVER))
    }
}

/// Walk cycle, facing, the weapon swinging around to point at the target, and attack moves.
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
        Option<&Strike>,
        &mut PlayerParts,
        Has<LocalPlayer>,
    )>,
    mut parts_q: Query<(&mut Sprite, &mut Transform), Without<Player>>,
    mut shown: Query<&mut Visibility, Without<Player>>,
) {
    let now = time.elapsed_secs();
    for (pos, samples, aim, loadout, strike, mut parts, local) in &mut players {
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

        let since = strike.map_or(f32::INFINITY, |s| now - s.0);
        let left = aim.x < 0.0;
        let attack = attack_pose(loadout.weapon, aim, since);
        let held = Vec2::from_angle(aim.to_angle() + attack.swing);

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
        ) + attack.push;
        let (center, arm_at) =
            textures::held_layout(pivot, held, sprite_info.size, sprite_info.grip, left);
        let z = if behind { -2.0 * Z_STEP } else { 3.0 * Z_STEP };
        if let Ok((mut sprite, mut transform)) = parts_q.get_mut(parts.weapon) {
            if parts.shown != Some(loadout.weapon) {
                sprite.image = sprite_info.held.clone();
                parts.shown = Some(loadout.weapon);
            }
            // Keep the gun upright when it points left.
            sprite.flip_y = left;
            transform.translation = center.extend(z);
            transform.rotation = Quat::from_rotation_z(held.to_angle());
        }
        // Under the weapon, so the fist closes over the cuff.
        if let Ok((mut sprite, mut transform)) = parts_q.get_mut(parts.arm) {
            sprite.flip_y = left;
            transform.translation = arm_at.extend(z - Z_STEP);
            transform.rotation = Quat::from_rotation_z(held.to_angle());
        }
        // Drawn over everything, even a gun raised behind the head.
        let lit = attack.flash_alpha > 0.0;
        if let Ok(mut visibility) = shown.get_mut(parts.flash) {
            visibility.set_if_neq(if lit {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            });
        }
        if lit && let Ok((mut sprite, mut transform)) = parts_q.get_mut(parts.flash) {
            let muzzle = center + held * (sprite_info.size.x / 2.0 - 1.0) * PIXEL;
            sprite.custom_size = Some(attack.flash);
            sprite.color = loadout
                .weapon
                .spec()
                .color()
                .mix(&Color::WHITE, 0.45)
                .with_alpha(attack.flash_alpha);
            transform.translation = muzzle.extend(4.0 * Z_STEP);
            transform.rotation = Quat::from_rotation_z(held.to_angle());
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
            // The flash keeps its own weapon color, set as it is posed.
            if child == parts.flash {
                continue;
            }
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

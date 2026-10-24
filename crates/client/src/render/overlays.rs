//! Per-frame overlay sprites: health bars, ground rings and hazard zones.

use super::*;

/// Marks one frame's worth of overlay sprites (bars, rings); redrawn from scratch every frame.
#[derive(Component)]
pub(super) struct Overlay;

pub(super) const Z_OVERLAY: f32 = 6.0;

pub(super) struct Painter<'a, 'w, 's> {
    pub(super) commands: &'a mut Commands<'w, 's>,
    pub(super) textures: &'a Textures,
}

impl Painter<'_, '_, '_> {
    /// An ellipse of the ring texture, centered on `center`.
    pub(super) fn oval(&mut self, center: Vec2, radii: Vec2, color: Color) {
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
    pub(super) fn ring(&mut self, at: Vec2, r: f32, color: Color) {
        self.oval(at, Vec2::new(r, r * 0.5), color);
    }

    /// A true-radius circle on the ground (world distances are not foreshortened).
    pub(super) fn zone(&mut self, at: Vec2, r: f32, color: Color) {
        self.oval(at, Vec2::splat(r), color);
    }

    /// A straight stripe from `from` toward `angle`.
    pub(super) fn line(&mut self, from: Vec2, angle: f32, length: f32, width: f32, color: Color) {
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

    pub(super) fn disc(&mut self, at: Vec2, r: f32, color: Color) {
        self.oval(at, Vec2::splat(r), color);
    }

    /// A bar growing rightward from `left`.
    pub(super) fn bar(&mut self, left: Vec2, width: f32, height: f32, color: Color) {
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

    /// A soft blob of light (acid spray, sparks).
    pub(super) fn glow(&mut self, at: Vec2, size: f32, color: Color) {
        self.commands.spawn((
            Overlay,
            Sprite {
                image: self.textures.glow.clone(),
                color,
                custom_size: Some(Vec2::splat(size)),
                ..default()
            },
            Transform::from_translation(at.extend(Z_OVERLAY - 0.1)),
        ));
    }

    pub(super) fn gauge(&mut self, left: Vec2, width: f32, height: f32, frac: f32, color: Color) {
        self.bar(left, width, height, Color::srgba(0.0, 0.0, 0.0, 0.6));
        self.bar(left, width * frac.clamp(0.0, 1.0), height, color);
    }
}

#[allow(clippy::type_complexity)]
pub(super) fn draw_overlays(
    mut commands: Commands,
    old: Query<Entity, With<Overlay>>,
    time: Res<Time>,
    fixed: Res<Time<Fixed>>,
    players: Query<(
        &Pos,
        Option<&TickSamples>,
        &Vitals,
        &Stats,
        (
            &Aim,
            Option<&Evolution>,
            Option<&SkillState>,
            Option<&ActionState<PlayerInput>>,
        ),
        Has<LocalPlayer>,
    )>,
    monsters: Query<(&Pos, &Vitals, &Monster, Option<&MonsterState>)>,
    puddles: Query<(&Pos, &Puddle, &PuddleAge)>,
    owners: Query<&Player>,
    turrets: Query<(&Pos, &Vitals, &Turret)>,
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

    // Turrets: the owner's color at the foot and a health bar.
    for (pos, vitals, turret) in &turrets {
        if indoors.conceals(&arena, pos.0) {
            continue;
        }
        if let Some(owner) = owners.iter().find(|p| p.client_id == turret.owner) {
            paint.ring(pos.0, 20.0, player_color(owner).with_alpha(0.7));
        }
        let w = 36.0;
        let frac = vitals.hp / vitals.max_hp.max(1.0);
        paint.gauge(
            pos.0 + Vec2::new(-w / 2.0, 46.0),
            w,
            4.0,
            frac,
            Color::srgb(0.5, 0.85, 1.0),
        );
    }

    for (pos, samples, vitals, stats, (aim, evo, skills, action), local) in &players {
        if !vitals.alive {
            continue;
        }
        let p = visual_pos(pos, samples, &fixed);
        if !local && indoors.conceals(&arena, p) {
            continue;
        }
        let skills = skills.copied().unwrap_or_default();
        // Someone else under 隐身 shows nothing but their faint figure.
        if !local && vitals.stealth > 0 && vitals.revealed == 0 {
            continue;
        }
        if vitals.revealed > 0 {
            // Caught in a trap: marked for everyone.
            let pulse = 0.5 + 0.4 * (t * 9.0).sin();
            paint.ring(
                p,
                PLAYER_RADIUS + 18.0,
                Color::srgb(1.0, 0.25, 0.2).with_alpha(pulse),
            );
        }
        if skills.breathing_acid() {
            let dir = match action {
                Some(a) if local => a.0.aim,
                _ => aim.0,
            }
            .normalize_or(Vec2::X);
            let mouth = p + Vec2::Y * GUN_HEIGHT;
            for i in 0..22 {
                let seed = (i as f32 * 12.9898).sin() * 43758.547;
                let spread = 2.0 * (seed - seed.floor()) - 1.0;
                let f = (t * 1.7 + i as f32 * 0.618).fract();
                let d = 24.0 + f * (ACID_RANGE - 24.0);
                let side = dir.perp() * spread * ACID_HALF_ANGLE.tan() * d * 0.85;
                let color = Color::srgb(0.6, 1.0, 0.2).with_alpha(0.55 * (1.0 - f * f));
                paint.glow(mouth + dir * d + side, 14.0 + 46.0 * f, color);
            }
        }
        if skills.shielded() {
            let left = (skills.shield_hp / SHIELD_HP).clamp(0.0, 1.0);
            let shimmer = 0.35 + 0.35 * left + 0.1 * (t * 7.0).sin();
            let bubble = Color::srgb(0.7, 0.55, 1.0).with_alpha(shimmer);
            let center = p + Vec2::Y * (SURVIVOR_HEIGHT / 2.0 - FOOT_INSET);
            paint.oval(center, Vec2::new(34.0, SURVIVOR_HEIGHT / 2.0 + 8.0), bubble);
            paint.ring(p, PLAYER_RADIUS + 9.0, bubble);
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
        let hp = (vitals.hp / vitals.max_hp.max(1.0)).clamp(0.0, 1.0);
        let hp_color = Color::srgb(1.0 - hp, 0.25 + 0.7 * hp, 0.25);
        paint.gauge(left, bar_w, 5.0, hp, hp_color);
        if vitals.armor_points > 0.0 {
            // 复合装甲 raises the cap.
            let cap = evo.map_or(vitals.armor.max_points(), |e| e.armor_cap(vitals.armor));
            let ap = (vitals.armor_points / cap.max(1.0)).clamp(0.0, 1.0);
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

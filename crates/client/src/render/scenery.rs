//! Ground, roads, buildings, walls, roofs and clutter for the current layout.

use super::*;

/// Ground cover, roads, buildings, walls and clutter for the current layout.
pub(super) fn build_scenery(
    mut commands: Commands,
    arena: Res<Arena>,
    state: Query<(), With<MatchState>>,
    textures: Res<Textures>,
    map: Res<LpcMap>,
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
    // LPC tiles, one texture pixel per world pixel.
    let floor = |commands: &mut Commands, image: Handle<Image>, rect: Rect, z: f32| {
        commands.spawn((
            Scenery,
            tiled_at(image, rect.size(), 1.0),
            Transform::from_translation(rect.center().extend(z)),
        ));
    };

    // Dirt everywhere, running a little past the boundary walls.
    floor(
        &mut commands,
        map.dirt.clone(),
        Rect::from_center_size(Vec2::ZERO, ARENA_HALF * 2.0 + 240.0),
        Z_FLOOR,
    );
    let mut rng = rand::rng();
    let mut sidewalks = Vec::new();
    for block in &arena.blocks {
        let cover = block.rect.inflate(-MARGIN + 12.0);
        match block.biome {
            Biome::Urban | Biome::Vault => {
                floor(&mut commands, map.sidewalk.clone(), cover, Z_FLOOR + 0.2);
                sidewalks.push(cover);
            }
            // Gravel frays into the surrounding dirt instead of ending in a straight line.
            Biome::Industrial => ground_patch(
                &mut commands,
                &map.gravel_patch,
                cover.inflate(16.0),
                Z_FLOOR + 0.2,
            ),
            // A few gravel patches break up the open dirt. Cosmetic, so a fresh scatter is fine.
            Biome::Wasteland | Biome::Radiation => {
                for _ in 0..3 {
                    let size = Vec2::new(
                        rng.random_range(120.0..280.0),
                        rng.random_range(100.0..220.0),
                    );
                    let room = (cover.size() - size).max(Vec2::ZERO) / 2.0;
                    let at = cover.center()
                        + Vec2::new(
                            rng.random_range(-room.x..=room.x),
                            rng.random_range(-room.y..=room.y),
                        );
                    let rect = Rect::from_center_size(at, size);
                    ground_patch(&mut commands, &map.gravel_patch, rect, Z_FLOOR + 0.15);
                }
            }
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
    let mut streets = Vec::new();
    for x in &streets_x {
        streets.push(Rect::from_center_size(
            Vec2::new(*x, 0.0),
            Vec2::new(STREET, ARENA_HALF.y * 2.0),
        ));
    }
    for y in &streets_y {
        streets.push(Rect::from_center_size(
            Vec2::new(0.0, *y),
            Vec2::new(ARENA_HALF.x * 2.0, STREET),
        ));
    }
    for rect in &streets {
        floor(&mut commands, map.road.clone(), *rect, Z_FLOOR + 0.3);
    }
    // Curbs along both edges of every street, their shadow on the road.
    for rect in &streets {
        let vertical = rect.height() > rect.width();
        for side in [-1.0, 1.0] {
            let across = if vertical { Vec2::X } else { Vec2::Y };
            let edge = rect.center() * (Vec2::ONE - across)
                + across * (rect.center() + across * side * STREET / 2.0);
            let length = if vertical {
                rect.height()
            } else {
                rect.width()
            };
            curb(
                &mut commands,
                edge - across * side * 2.0,
                length,
                vertical,
                -across * side,
            );
        }
    }
    // Sidewalks ending somewhere other than a street (the arena's edge) get a curb too, with
    // the drop on the dirt side.
    let under_street = |p: Vec2| streets.iter().any(|s| s.contains(p));
    for rect in &sidewalks {
        for (edge, vertical, out) in [
            (Vec2::new(rect.min.x, rect.center().y), true, -Vec2::X),
            (Vec2::new(rect.max.x, rect.center().y), true, Vec2::X),
            (Vec2::new(rect.center().x, rect.min.y), false, -Vec2::Y),
            (Vec2::new(rect.center().x, rect.max.y), false, Vec2::Y),
        ] {
            if !under_street(edge) {
                let length = if vertical {
                    rect.height()
                } else {
                    rect.width()
                };
                curb(&mut commands, edge - out * 2.0, length, vertical, out);
            }
        }
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
        let lpc_face = |i: usize| WallLook {
            front: map.wall_faces[i].clone(),
            top: map.wall_top.clone(),
            scale: 1.0,
            height: lpc::WALL_FACE_HEIGHTS[i] as f32,
        };
        let look = match style {
            WallStyle::Brick => lpc_face(0),
            WallStyle::Metal => lpc_face(1),
            WallStyle::Rock => lpc_face(2),
            // Low sandbag barricades keep their procedural look.
            WallStyle::Sandbag => WallLook {
                front: textures.sandbags.front.clone(),
                top: textures.sandbags.top.clone(),
                scale: PIXEL,
                height: 22.0,
            },
        };
        spawn_walls(&mut commands, &look, &rects, style);
    }

    // Roofs sit on the walls, so they're lifted by the wall height and sorted with the front wall:
    // whoever stands in front of the building is drawn over its roof, whoever is inside under it.
    for (i, building) in arena.buildings.iter().enumerate() {
        let r = building.rect;
        let z = depth(r.min.y) + 6.0 * Z_STEP;
        let size = Vec2::new(r.width() + 12.0, r.height() + 6.0);
        // A flat LPC roof stretched over the building, its frame kept whole.
        let (image, tint) = match building.roof {
            Roof::Tar => (&map.roof_tar, Color::WHITE),
            Roof::Tin => (&map.roof_tar, Color::srgb(0.78, 0.86, 0.95)),
            Roof::Concrete => (&map.roof_concrete, Color::WHITE),
        };
        let slicer = TextureSlicer {
            border: BorderRect::all(lpc::ROOF_BORDER),
            center_scale_mode: SliceScaleMode::Tile { stretch_value: 1.0 },
            sides_scale_mode: SliceScaleMode::Tile { stretch_value: 1.0 },
            max_corner_scale: 1.0,
        };
        commands.spawn((
            Scenery,
            RoofOf {
                building: i,
                alpha: 1.0,
            },
            Sprite {
                image: image.clone(),
                custom_size: Some(size),
                color: tint,
                image_mode: SpriteImageMode::Sliced(slicer),
                ..default()
            },
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

/// A stretch of ground cover with frayed edges: `image` 9-sliced over `rect` (see
/// `lpc::GROUND_EDGE`), one texture pixel per world pixel.
fn ground_patch(commands: &mut Commands, image: &Handle<Image>, rect: Rect, z: f32) {
    let tile = SliceScaleMode::Tile { stretch_value: 1.0 };
    commands.spawn((
        Scenery,
        Sprite {
            image: image.clone(),
            custom_size: Some(rect.size()),
            image_mode: SpriteImageMode::Sliced(TextureSlicer {
                border: BorderRect::all(lpc::GROUND_EDGE),
                center_scale_mode: tile,
                sides_scale_mode: tile,
                max_corner_scale: 1.0,
            }),
            ..default()
        },
        Transform::from_translation(rect.center().extend(z)),
    ));
}

/// A curb `length` long centered on `at`, running vertically or horizontally: a pale stone lip
/// and a shadow on the lower side (`down`).
fn curb(commands: &mut Commands, at: Vec2, length: f32, vertical: bool, down: Vec2) {
    let lip = Color::srgb(0.58, 0.6, 0.6);
    let gutter = Color::srgba(0.0, 0.0, 0.0, 0.35);
    let size = |w: f32| {
        if vertical {
            Vec2::new(w, length)
        } else {
            Vec2::new(length, w)
        }
    };
    for (color, at, size) in [(lip, at, size(4.0)), (gutter, at + down * 3.0, size(2.0))] {
        commands.spawn((
            Scenery,
            Sprite::from_color(color, size),
            Transform::from_translation(at.extend(Z_FLOOR + 0.32)),
        ));
    }
}

/// Fade the roof over your head; note which building you're in so others inside stay hidden.
pub(super) fn update_roofs(
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
pub(super) struct WallRun {
    pub(super) x0: f32,
    pub(super) x1: f32,
    pub(super) y0: f32,
    pub(super) y1: f32,
}

pub(super) const EPS: f32 = 0.01;

/// Walls overlap at corners. Drawing each rect on its own makes one wall's cap fold over the
/// other's brick face, so render their union instead: cut it into vertical strips at every wall
/// edge, merge the solid spans within each strip, then glue identical neighbouring strips back.
pub(super) fn wall_runs(walls: &[Rect]) -> Vec<WallRun> {
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
pub(super) const METAL_TINTS: [Color; 5] = [
    Color::srgb(0.78, 0.42, 0.32),
    Color::srgb(0.45, 0.6, 0.8),
    Color::srgb(0.5, 0.68, 0.45),
    Color::srgb(0.85, 0.85, 0.82),
    Color::srgb(0.9, 0.74, 0.35),
];

/// How a wall style is drawn: its face and top textures, at `scale` world pixels per texture
/// pixel, with a face `height` world pixels tall.
pub(super) struct WallLook {
    pub(super) front: Handle<Image>,
    pub(super) top: Handle<Image>,
    pub(super) scale: f32,
    pub(super) height: f32,
}

pub(super) fn spawn_walls(
    commands: &mut Commands,
    look: &WallLook,
    walls: &[Rect],
    style: WallStyle,
) {
    let runs = wall_runs(walls);
    let dark = Color::srgb(0.08, 0.06, 0.07);
    let wall_height = look.height;
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
            let mut sprite = tiled_at(image.clone(), size, look.scale);
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
            face(&look.front, Vec2::new(w, wall_height)),
            Transform::from_translation(at(run.y0 + wall_height / 2.0, 0.0)),
        ));
        commands.spawn((
            Scenery,
            face(&look.top, Vec2::new(w, depth_len)),
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

//! Minimap in the top-left corner: the layout painted once per match, with live markers for
//! you, bounty targets (and anyone a trap gave away) and the vault.

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::math::Rect;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use evoclash_game::map::{ARENA_HALF, ArenaMap, Biome, VAULT_CENTER, WallStyle};
use evoclash_game::protocol::*;
use evoclash_game::sim::Arena;

use crate::input::LocalPlayer;

/// Screen pixels per world pixel.
const SCALE: f32 = 0.05;
const SIZE: Vec2 = Vec2::new(ARENA_HALF.x * 2.0 * SCALE, ARENA_HALF.y * 2.0 * SCALE);
const MARGIN: f32 = 16.0;

pub struct MinimapPlugin;

impl Plugin for MinimapPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_minimap);
        app.add_systems(Update, (repaint, place_markers).chain());
    }
}

#[derive(Component)]
struct MapImage;

#[derive(Component, Clone, Copy, PartialEq)]
enum Marker {
    Me,
    /// A combatant with a price on their head; `usize` picks which of the pool.
    Bounty(usize),
}

const BOUNTY_MARKERS: usize = 4;

fn spawn_minimap(mut commands: Commands) {
    commands
        .spawn((
            MapImage,
            ImageNode::default(),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(MARGIN),
                top: Val::Px(MARGIN),
                width: Val::Px(SIZE.x),
                height: Val::Px(SIZE.y),
                border: UiRect::all(Val::Px(2.0)),
                ..default()
            },
            BorderColor::all(Color::srgba(0.0, 0.0, 0.0, 0.7)),
        ))
        .with_children(|map| {
            map.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px((VAULT_CENTER.x + ARENA_HALF.x) * SCALE - 4.0),
                    top: Val::Px((ARENA_HALF.y - VAULT_CENTER.y) * SCALE - 4.0),
                    width: Val::Px(8.0),
                    height: Val::Px(8.0),
                    ..default()
                },
                BackgroundColor(Color::srgb(1.0, 0.75, 0.2)),
            ));
            let dot = |size: f32, color: Color| {
                (
                    Node {
                        position_type: PositionType::Absolute,
                        width: Val::Px(size),
                        height: Val::Px(size),
                        border_radius: BorderRadius::MAX,
                        ..default()
                    },
                    BackgroundColor(color),
                    Visibility::Hidden,
                )
            };
            for i in 0..BOUNTY_MARKERS {
                map.spawn((Marker::Bounty(i), dot(7.0, Color::srgb(1.0, 0.82, 0.1))));
            }
            map.spawn((Marker::Me, dot(7.0, Color::WHITE)));
        });
}

/// Paint the layout: ground by biome, roads, radiation, roofs and walls, rect by rect.
fn paint(arena: &ArenaMap) -> Image {
    let (w, h) = (SIZE.x as i32, SIZE.y as i32);
    let rgb =
        |r: f32, g: f32, b: f32| [(r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, 215];
    let mut px = vec![rgb(0.16, 0.15, 0.16); (w * h) as usize];
    let mut fill = |rect: Rect, col: [u8; 4]| {
        let x0 = (((rect.min.x + ARENA_HALF.x) * SCALE).floor() as i32).max(0);
        let x1 = (((rect.max.x + ARENA_HALF.x) * SCALE).ceil() as i32).min(w);
        let y0 = (((ARENA_HALF.y - rect.max.y) * SCALE).floor() as i32).max(0);
        let y1 = (((ARENA_HALF.y - rect.min.y) * SCALE).ceil() as i32).min(h);
        for y in y0..y1 {
            for x in x0..x1 {
                px[(y * w + x) as usize] = col;
            }
        }
    };
    for block in &arena.blocks {
        let col = match block.biome {
            Biome::Urban | Biome::Vault => rgb(0.34, 0.32, 0.3),
            Biome::Industrial => rgb(0.3, 0.29, 0.26),
            Biome::Radiation => rgb(0.3, 0.38, 0.14),
            Biome::Wasteland => rgb(0.36, 0.24, 0.18),
        };
        fill(block.rect.inflate(-evoclash_game::map::MARGIN), col);
    }
    for hazard in &arena.hazards {
        fill(*hazard, rgb(0.3, 0.38, 0.14));
    }
    for building in &arena.buildings {
        fill(building.rect, rgb(0.52, 0.5, 0.47));
    }
    for (wall, style) in arena.walls.iter().zip(&arena.wall_styles) {
        let col = match style {
            WallStyle::Metal => rgb(0.5, 0.55, 0.62),
            WallStyle::Rock => rgb(0.5, 0.42, 0.36),
            WallStyle::Sandbag => rgb(0.6, 0.55, 0.38),
            WallStyle::Brick => rgb(0.1, 0.09, 0.09),
        };
        fill(*wall, col);
    }
    let mut image = Image::new(
        Extent3d {
            width: w as u32,
            height: h as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        px.into_iter().flatten().collect(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::nearest();
    image
}

fn repaint(
    arena: Res<Arena>,
    state: Query<(), With<MatchState>>,
    mut images: ResMut<Assets<Image>>,
    mut node: Query<&mut ImageNode, With<MapImage>>,
    mut painted: Local<Option<u64>>,
) {
    if state.is_empty() || *painted == Some(arena.seed) {
        return;
    }
    *painted = Some(arena.seed);
    let handle = images.add(paint(&arena));
    for mut node in &mut node {
        node.image = handle.clone();
    }
}

fn place_markers(
    me: Query<(&Pos, &Vitals), With<LocalPlayer>>,
    players: Query<(&Pos, &Vitals, &Stats), (With<Player>, Without<LocalPlayer>)>,
    mut markers: Query<(&Marker, &mut Node, &mut Visibility)>,
) {
    let at = |p: Vec2| {
        (
            (p.x + ARENA_HALF.x) * SCALE - 3.5,
            (ARENA_HALF.y - p.y) * SCALE - 3.5,
        )
    };
    let bounties: Vec<Vec2> = players
        .iter()
        .filter(|(_, v, s)| v.alive && (s.has_bounty() || v.revealed > 0))
        .map(|(p, ..)| p.0)
        .take(BOUNTY_MARKERS)
        .collect();
    for (marker, mut node, mut visibility) in &mut markers {
        let pos = match marker {
            Marker::Me => me.single().ok().filter(|(_, v)| v.alive).map(|(p, _)| p.0),
            Marker::Bounty(i) => bounties.get(*i).copied(),
        };
        let Some(pos) = pos else {
            visibility.set_if_neq(Visibility::Hidden);
            continue;
        };
        let (left, top) = at(pos);
        node.left = Val::Px(left);
        node.top = Val::Px(top);
        visibility.set_if_neq(Visibility::Inherited);
    }
}

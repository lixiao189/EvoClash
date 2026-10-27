//! Minimap in the top-left corner: the layout painted once per match, with live markers for
//! you, bounty targets (and anyone a trap gave away), the vault, and a finale leader blip.

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
        app.add_systems(Update, (repaint, place_markers, leader_marker).chain());
    }
}

#[derive(Component)]
struct MapImage;

#[derive(Component, Clone, Copy, PartialEq)]
enum Marker {
    Me,
    /// A combatant with a price on their head; `usize` picks which of the pool.
    Bounty(usize),
    /// The behemoth, while it is out.
    Boss,
}

/// Finale blip of whoever is first. A square, so it is not the round gold bounty dot
/// or the round boss dot. Shown even when that combatant is the local player.
#[derive(Component)]
struct LeaderMarker;

const LEADER_SIZE: f32 = 10.0;
/// How long one leader blip stays up after its `tick` changes.
const LEADER_FLASH_SECS: f32 = 3.0;

/// Opacity of a leader blip `age` seconds after the flash tick changed.
/// `None` once the blip should hide. While visible it fades out and pulses.
pub(crate) fn leader_flash_alpha(age: f32) -> Option<f32> {
    if !age.is_finite() || age < 0.0 || age >= LEADER_FLASH_SECS {
        return None;
    }
    let fade = 1.0 - age / LEADER_FLASH_SECS;
    let pulse = 0.55 + 0.45 * (age * std::f32::consts::TAU).cos().abs();
    Some(fade * pulse)
}

fn map_point(p: Vec2, size: f32) -> (f32, f32) {
    let half = size * 0.5;
    (
        (p.x + ARENA_HALF.x) * SCALE - half,
        (ARENA_HALF.y - p.y) * SCALE - half,
    )
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
            map.spawn((Marker::Boss, dot(11.0, Color::srgb(1.0, 0.25, 0.12))));
            map.spawn((Marker::Me, dot(7.0, Color::WHITE)));
            map.spawn((
                LeaderMarker,
                Node {
                    position_type: PositionType::Absolute,
                    width: Val::Px(LEADER_SIZE),
                    height: Val::Px(LEADER_SIZE),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.95, 0.08, 0.1)),
                BorderColor::all(Color::srgb(1.0, 0.82, 0.75)),
                Visibility::Hidden,
            ));
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
    state: Query<&MatchState>,
    me: Query<(&Pos, &Vitals), With<LocalPlayer>>,
    players: Query<(&Pos, &Vitals, &Stats), (With<Player>, Without<LocalPlayer>)>,
    bosses: Query<(&Pos, &Monster, &Vitals)>,
    mut markers: Query<(&Marker, &mut Node, &mut Visibility)>,
) {
    let practice = state.iter().any(|s| s.phase == Phase::Lobby);
    let at = |p: Vec2| {
        (
            (p.x + ARENA_HALF.x) * SCALE - 3.5,
            (ARENA_HALF.y - p.y) * SCALE - 3.5,
        )
    };
    let bounties: Vec<Vec2> = players
        .iter()
        .filter(|(_, v, s)| v.alive && ((!practice && s.has_bounty()) || v.revealed > 0))
        .map(|(p, ..)| p.0)
        .take(BOUNTY_MARKERS)
        .collect();
    let boss = bosses
        .iter()
        .find(|(_, m, v)| m.kind == MonsterKind::Behemoth && v.alive)
        .map(|(p, ..)| p.0);
    for (marker, mut node, mut visibility) in &mut markers {
        let pos = match marker {
            Marker::Boss => boss,
            Marker::Me => me.single().ok().filter(|(_, v)| v.alive).map(|(p, _)| p.0),
            Marker::Bounty(i) => bounties.get(*i).copied(),
        };
        let Some(pos) = pos else {
            visibility.set_if_neq(Visibility::Hidden);
            continue;
        };
        let (mut left, mut top) = at(pos);
        if *marker == Marker::Boss {
            // Bigger dot: keep it centered on the same spot.
            left -= 2.0;
            top -= 2.0;
        }
        node.left = Val::Px(left);
        node.top = Val::Px(top);
        visibility.set_if_neq(Visibility::Inherited);
    }
}

#[derive(Default)]
struct FlashSeen {
    /// False until the match entity has been seen once. That first sample is not a new flash.
    primed: bool,
    blipped: Option<u32>,
}

struct LeaderBlip {
    pos: Vec2,
    started: f32,
}

fn leader_marker(
    time: Res<Time>,
    state: Query<(&MatchState, &LeaderPing)>,
    mut marker: Query<
        (
            &mut Node,
            &mut Visibility,
            &mut BackgroundColor,
            &mut BorderColor,
        ),
        With<LeaderMarker>,
    >,
    mut seen: Local<FlashSeen>,
    mut blip: Local<Option<LeaderBlip>>,
) {
    let now = time.elapsed_secs();
    match state.iter().next() {
        None => {
            *seen = FlashSeen::default();
            *blip = None;
        }
        Some((_, ping)) if !seen.primed => {
            seen.primed = true;
            seen.blipped = ping.latest.map(|flash| flash.tick);
            *blip = None;
        }
        Some((match_state, ping)) => {
            if match_state.finale() {
                if let Some(flash) = ping.latest
                    && seen.blipped != Some(flash.tick)
                {
                    seen.blipped = Some(flash.tick);
                    *blip = Some(LeaderBlip {
                        pos: flash.pos,
                        started: now,
                    });
                }
            } else {
                *blip = None;
            }
        }
    }
    let show = blip
        .as_ref()
        .and_then(|blip| leader_flash_alpha(now - blip.started));
    let pos = blip.as_ref().map(|blip| blip.pos);
    if show.is_none() {
        *blip = None;
    }
    for (mut node, mut visibility, mut background, mut border) in &mut marker {
        let Some(alpha) = show else {
            visibility.set_if_neq(Visibility::Hidden);
            continue;
        };
        if let Some(pos) = pos {
            let (left, top) = map_point(pos, LEADER_SIZE);
            node.left = Val::Px(left);
            node.top = Val::Px(top);
        }
        background.0 = Color::srgb(0.95, 0.08, 0.1).with_alpha(alpha);
        *border = BorderColor::all(Color::srgb(1.0, 0.82, 0.75).with_alpha(alpha));
        visibility.set_if_neq(Visibility::Inherited);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leader_flash_lasts_three_seconds_then_fades_out() {
        assert_eq!(LEADER_FLASH_SECS, 3.0);
        assert_eq!(leader_flash_alpha(0.0), Some(1.0));
        assert!(leader_flash_alpha(-0.001).is_none());
        assert!(leader_flash_alpha(LEADER_FLASH_SECS).is_none());
        assert!(leader_flash_alpha(LEADER_FLASH_SECS + 0.5).is_none());
        assert!(leader_flash_alpha(f32::NAN).is_none());
        assert!(leader_flash_alpha(f32::INFINITY).is_none());
        for age in [0.0, 0.25, 0.8, 1.5, 2.2, 2.99] {
            let alpha = leader_flash_alpha(age).unwrap();
            let fade = 1.0 - age / LEADER_FLASH_SECS;
            assert!(alpha > 0.0 && alpha <= fade + 1e-5, "{age} -> {alpha}");
            assert!(alpha + 1e-5 >= fade * 0.55, "{age} -> {alpha}");
        }
        assert!(leader_flash_alpha(2.8).unwrap() < leader_flash_alpha(0.2).unwrap());
    }
}

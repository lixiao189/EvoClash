//! Lobby chrome and the full-screen Starting / Recovering overlay.
//!
//! The host crown is not drawn: the wire has no host id yet. Bots are `Player.bot`.

use bevy::prelude::*;
use evoclash_game::protocol::{MatchState, Phase, Player};

use crate::hud::GOLD;
use crate::render::UiFont;
use crate::session::JoinNotice;
use crate::textures::{PIXEL, Textures};

const TICKS_PER_SEC: u32 = 64;
const STARTING_SECS: u32 = 3;
/// Texels of `Textures.bot_tag`; drawn at [`PIXEL`].
const BOT_TAG_TEXELS: Vec2 = Vec2::new(14.0, 7.0);

const TITLE: &str = "等待大厅";
const PUBLIC_ROOM: &str = "公开房间";
const PRACTICE: &str = "练习场：不计分、不掉落，开局会重置";
const RECOVERING: &str = "服务器恢复中…";

pub struct LobbyUiPlugin;

impl Plugin for LobbyUiPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_lobby_ui);
        app.add_systems(Update, (update_lobby, sync_roster, update_phase_banner));
    }
}

/// Lobby seconds until Playing: this phase's `ticks_left` plus the 3s Starting countdown.
fn seconds_until_start(ticks_left: u32) -> u32 {
    ticks_left.saturating_add(STARTING_SECS * TICKS_PER_SEC) / TICKS_PER_SEC
}

/// Full-screen 3, 2, 1 from Starting `ticks_left`.
fn starting_count(ticks_left: u32) -> u32 {
    ticks_left.div_ceil(TICKS_PER_SEC).clamp(1, 3)
}

#[derive(Component)]
struct LobbyChrome;

#[derive(Component, Clone, Copy)]
enum LobbyLine {
    Humans,
    Bots,
    Countdown,
}

#[derive(Component)]
struct RosterRoot;

#[derive(Component)]
struct RosterRow;

#[derive(Component)]
struct PhaseShade;

#[derive(Component, Clone, Copy)]
enum BannerSlot {
    Count,
    Recover,
    Notice,
}

fn label(font: &UiFont, size: f32, value: &str, color: Color) -> impl Bundle {
    (
        Text::new(value),
        font.sized(size),
        TextColor(color),
        TextLayout::justify(Justify::Center),
        TextShadow::default(),
    )
}

fn line(font: &UiFont, size: f32, kind: LobbyLine, color: Color) -> impl Bundle {
    (
        kind,
        Text::new(""),
        font.sized(size),
        TextColor(color),
        TextLayout::justify(Justify::Center),
        TextShadow::default(),
    )
}

fn spawn_lobby_ui(mut commands: Commands, font: Res<UiFont>, textures: Res<Textures>) {
    // No host id on the wire yet, so the crown is not drawn. The read keeps the
    // artist-owned `Textures` field live for the wasm build.
    let _ = &textures.host_crown;
    commands
        .spawn((
            LobbyChrome,
            Visibility::Hidden,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(12.0),
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(2.0),
                ..default()
            },
        ))
        .with_children(|col| {
            col.spawn(Node {
                align_items: AlignItems::Baseline,
                column_gap: Val::Px(14.0),
                ..default()
            })
            .with_children(|row| {
                row.spawn(label(&font, 22.0, TITLE, Color::WHITE));
                row.spawn(label(
                    &font,
                    16.0,
                    PUBLIC_ROOM,
                    Color::srgba(1.0, 0.9, 0.7, 0.9),
                ));
            });
            col.spawn(line(&font, 18.0, LobbyLine::Humans, Color::WHITE));
            col.spawn(line(
                &font,
                13.0,
                LobbyLine::Bots,
                Color::srgba(1.0, 1.0, 1.0, 0.75),
            ));
            col.spawn(line(&font, 20.0, LobbyLine::Countdown, GOLD));
            col.spawn(label(
                &font,
                14.0,
                PRACTICE,
                Color::srgba(1.0, 0.86, 0.62, 0.9),
            ));
            col.spawn((
                RosterRoot,
                Node {
                    margin: UiRect::top(Val::Px(10.0)),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(3.0),
                    max_height: Val::Px(280.0),
                    overflow: Overflow::clip(),
                    ..default()
                },
            ));
        });

    commands.spawn((
        PhaseShade,
        GlobalZIndex(40),
        Visibility::Hidden,
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
        BackgroundColor(Color::srgba(0.02, 0.01, 0.02, 0.72)),
    ));
    commands
        .spawn((
            GlobalZIndex(41),
            Node {
                position_type: PositionType::Absolute,
                top: Val::Percent(34.0),
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                ..default()
            },
        ))
        .with_children(|col| {
            col.spawn(banner(&font, 112.0, BannerSlot::Count, None));
            col.spawn(banner(&font, 32.0, BannerSlot::Recover, Some(520.0)));
            col.spawn(banner(&font, 20.0, BannerSlot::Notice, Some(560.0)));
        });
}

fn banner(font: &UiFont, size: f32, slot: BannerSlot, width: Option<f32>) -> impl Bundle {
    (
        slot,
        Text::new(""),
        font.sized(size),
        TextColor(Color::WHITE),
        TextLayout::justify(Justify::Center),
        TextShadow::default(),
        Node {
            display: Display::None,
            width: width.map_or(Val::Auto, Val::Px),
            ..default()
        },
    )
}

fn update_lobby(
    state: Query<&MatchState>,
    players: Query<&Player>,
    mut chrome: Query<&mut Visibility, With<LobbyChrome>>,
    mut lines: Query<(&LobbyLine, &mut Text, &mut Node)>,
) {
    let current = state.iter().next();
    let lobby = current.is_some_and(|s| s.phase == Phase::Lobby);
    let shown = if lobby {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut vis in &mut chrome {
        vis.set_if_neq(shown);
    }
    let Some(s) = current.filter(|s| s.phase == Phase::Lobby) else {
        return;
    };
    let humans = players.iter().filter(|p| !p.bot).count();
    let bots = players.iter().filter(|p| p.bot).count();
    let countdown = if s.countdown_active {
        format!("{} 秒后开始", seconds_until_start(s.ticks_left))
    } else {
        String::new()
    };
    for (kind, mut text, mut node) in &mut lines {
        let (value, on) = match kind {
            LobbyLine::Humans => (format!("真人 {humans}/8"), true),
            LobbyLine::Bots => (format!("机器人 {bots}"), true),
            LobbyLine::Countdown => (countdown.clone(), s.countdown_active),
        };
        node.display = if on { Display::Flex } else { Display::None };
        if text.0 != value {
            text.0 = value;
        }
    }
}

fn sync_roster(
    mut commands: Commands,
    font: Res<UiFont>,
    textures: Res<Textures>,
    state: Query<&MatchState>,
    players: Query<&Player>,
    roots: Query<Entity, With<RosterRoot>>,
    existing: Query<Entity, With<RosterRow>>,
    mut signature: Local<String>,
) {
    let lobby = state.iter().any(|s| s.phase == Phase::Lobby);
    if !lobby {
        if !signature.is_empty() {
            signature.clear();
            for entity in existing.iter().collect::<Vec<_>>() {
                commands.entity(entity).despawn();
            }
        }
        return;
    }
    let Ok(root) = roots.single() else {
        return;
    };
    let mut rows: Vec<(bool, String, u64, [f32; 3])> = players
        .iter()
        .map(|p| (p.bot, p.name.clone(), p.client_id, p.color))
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let mut sig = String::new();
    for (bot, name, id, color) in &rows {
        sig.push_str(&format!("{id}|{bot}|{name}|{color:?}\n"));
    }
    if sig == *signature {
        return;
    }
    *signature = sig;
    for entity in existing.iter().collect::<Vec<_>>() {
        commands.entity(entity).despawn();
    }
    let tag = textures.bot_tag.clone();
    commands.entity(root).with_children(|parent| {
        for (bot, name, _, color) in &rows {
            parent
                .spawn((
                    RosterRow,
                    Node {
                        align_items: AlignItems::Center,
                        column_gap: Val::Px(6.0),
                        ..default()
                    },
                ))
                .with_children(|row| {
                    row.spawn((
                        Text::new(name.clone()),
                        font.sized(16.0),
                        TextColor(Color::srgb(color[0], color[1], color[2])),
                        TextShadow::default(),
                    ));
                    if *bot {
                        row.spawn((
                            ImageNode {
                                image: tag.clone(),
                                ..default()
                            },
                            Node {
                                width: Val::Px(BOT_TAG_TEXELS.x * PIXEL),
                                height: Val::Px(BOT_TAG_TEXELS.y * PIXEL),
                                ..default()
                            },
                        ));
                    }
                });
        }
    });
}

fn update_phase_banner(
    state: Query<&MatchState>,
    notice: Res<JoinNotice>,
    mut shade: Query<(&mut Visibility, &mut BackgroundColor), With<PhaseShade>>,
    mut slots: Query<(&BannerSlot, &mut Text, &mut Node)>,
) {
    let current = state.iter().next();
    let phase = current.map(|s| s.phase);
    let ticks = current.map(|s| s.ticks_left).unwrap_or(0);
    let (dim, alpha) = match phase {
        Some(Phase::Recovering) => (true, 0.78),
        Some(Phase::Starting) => (true, 0.45),
        _ => (false, 0.0),
    };
    for (mut vis, mut bg) in &mut shade {
        vis.set_if_neq(if dim {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        let color = Color::srgba(0.02, 0.01, 0.02, alpha);
        if bg.0 != color {
            bg.0 = color;
        }
    }
    let covered = matches!(phase, Some(Phase::Starting | Phase::Recovering));
    for (slot, mut text, mut node) in &mut slots {
        let (value, on) = match slot {
            BannerSlot::Count => match phase {
                Some(Phase::Starting) => (starting_count(ticks).to_string(), true),
                _ => (String::new(), false),
            },
            BannerSlot::Recover => match phase {
                Some(Phase::Recovering) => (RECOVERING.to_string(), true),
                _ => (String::new(), false),
            },
            BannerSlot::Notice => {
                if covered || notice.0.is_empty() {
                    (String::new(), false)
                } else {
                    (notice.0.clone(), true)
                }
            }
        };
        node.display = if on { Display::Flex } else { Display::None };
        if text.0 != value {
            text.0 = value;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lobby_countdown_includes_the_three_second_start() {
        assert_eq!(seconds_until_start(17 * TICKS_PER_SEC), 20);
        assert_eq!(seconds_until_start(1), STARTING_SECS);
        assert_eq!(seconds_until_start(0), STARTING_SECS);
    }

    #[test]
    fn starting_count_holds_each_digit_for_one_second() {
        assert_eq!(starting_count(3 * TICKS_PER_SEC), 3);
        assert_eq!(starting_count(2 * TICKS_PER_SEC + 1), 3);
        assert_eq!(starting_count(2 * TICKS_PER_SEC), 2);
        assert_eq!(starting_count(TICKS_PER_SEC + 1), 2);
        assert_eq!(starting_count(TICKS_PER_SEC), 1);
        assert_eq!(starting_count(1), 1);
    }
}

//! Room cumulative board. Lobby only; Playing, Starting, and Recovering stay blank.
//! Results reuses [`round_gain`] for the "+N" on a placed human. Spectators see the lobby
//! board too: the local id is the combatant when we have one, otherwise the netcode id.

use bevy::prelude::*;
use evoclash_game::protocol::{
    BoardRow, MatchState, Phase, Player, RoomBoard, board_order, board_points,
};
use lightyear::netcode::NetcodeClient;
use lightyear::prelude::PeerId;

use crate::hud::{GOLD, PANEL_BG};
use crate::input::LocalPlayer;
use crate::render::UiFont;

/// 8 combat seats plus 4 spectators.
const MAX_BOARD_ROWS: usize = 12;

const YOU_BG: Color = Color::srgba(0.55, 0.4, 0.12, 0.92);
const HEADER: Color = Color::srgba(1.0, 1.0, 1.0, 0.55);

const W_RANK: f32 = 36.0;
const W_NAME: f32 = 168.0;
const W_POINTS: f32 = 44.0;
const W_PLAYED: f32 = 44.0;
const W_LAST: f32 = 36.0;

pub struct BoardUiPlugin;

impl Plugin for BoardUiPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_board);
        app.add_systems(Update, sync_board);
    }
}

/// `board_order`, then at most [`MAX_BOARD_ROWS`]. Rank on screen is the index plus one.
fn ordered_board(rows: &[BoardRow]) -> Vec<&BoardRow> {
    let mut rows: Vec<&BoardRow> = rows.iter().collect();
    rows.sort_by(|a, b| board_order(a, b));
    rows.truncate(MAX_BOARD_ROWS);
    rows
}

/// `Some(n)` when a results row shows "+n", including "+0".
/// Bots never do. A human does only when that row's `last_round` is this match.
/// `n` is [`board_points`] of `last_place`, not the row's cumulative total.
pub(crate) fn round_gain(bot: bool, row: Option<&BoardRow>, match_index: u32) -> Option<u16> {
    if bot {
        return None;
    }
    let row = row?;
    (row.last_round == match_index).then(|| board_points(row.last_place))
}

pub(crate) fn gain_label(points: u16) -> String {
    format!("+{points}")
}

/// Seated clients use the replicated player. A spectator has no [`LocalPlayer`]; the
/// netcode id is the same `client_id` once the link is up (0 means not connected yet).
fn local_client_id(
    players: &Query<(&Player, Has<LocalPlayer>)>,
    net: &Query<&NetcodeClient>,
) -> Option<u64> {
    players
        .iter()
        .find_map(|(player, local)| local.then_some(player.client_id))
        .or_else(|| {
            net.iter().find_map(|net| match net.id() {
                PeerId::Netcode(id) if id != 0 => Some(id),
                _ => None,
            })
        })
}

#[derive(Component)]
struct BoardPanel;

#[derive(Component)]
struct BoardRows;

#[derive(Component)]
struct BoardLine;

fn cell(font: &UiFont, value: &str, color: Color, width: f32) -> impl Bundle {
    (
        Text::new(value),
        font.sized(14.0),
        TextColor(color),
        TextLayout::justify(Justify::Left),
        TextShadow::default(),
        Node {
            width: Val::Px(width),
            overflow: Overflow::clip(),
            ..default()
        },
    )
}

fn spawn_line(
    parent: &mut ChildSpawnerCommands,
    font: &UiFont,
    bg: Color,
    mark: bool,
    cells: [(&str, Color, f32); 5],
) {
    let mut entity = parent.spawn((
        Node {
            column_gap: Val::Px(8.0),
            align_items: AlignItems::Center,
            padding: UiRect::axes(Val::Px(4.0), Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(3.0)),
            ..default()
        },
        BackgroundColor(bg),
    ));
    if mark {
        entity.insert(BoardLine);
    }
    entity.with_children(|line| {
        for (text, color, width) in cells {
            line.spawn(cell(font, text, color, width));
        }
    });
}

fn spawn_board(mut commands: Commands, font: Res<UiFont>) {
    // Top-right is the in-match scoreboard slot. That line is cleared in Lobby, and the
    // lobby column (title, roster, buttons) stays centered, so this panel does not cover it.
    commands
        .spawn((
            BoardPanel,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(16.0),
                right: Val::Px(16.0),
                display: Display::None,
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::FlexStart,
                row_gap: Val::Px(2.0),
                padding: UiRect::axes(Val::Px(10.0), Val::Px(8.0)),
                border_radius: BorderRadius::all(Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(PANEL_BG),
        ))
        .with_children(|panel| {
            panel.spawn((
                Text::new("累计榜"),
                font.sized(16.0),
                TextColor(GOLD),
                TextShadow::default(),
            ));
            spawn_line(
                panel,
                &font,
                Color::srgba(0.0, 0.0, 0.0, 0.0),
                false,
                [
                    ("名次", HEADER, W_RANK),
                    ("名字", HEADER, W_NAME),
                    ("总分", HEADER, W_POINTS),
                    ("局数", HEADER, W_PLAYED),
                    ("上局", HEADER, W_LAST),
                ],
            );
            panel.spawn((
                BoardRows,
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::FlexStart,
                    row_gap: Val::Px(1.0),
                    ..default()
                },
            ));
        });
}

fn sync_board(
    mut commands: Commands,
    font: Res<UiFont>,
    state: Query<&MatchState>,
    boards: Query<&RoomBoard>,
    players: Query<(&Player, Has<LocalPlayer>)>,
    net: Query<&NetcodeClient>,
    mut panel: Query<&mut Node, With<BoardPanel>>,
    roots: Query<Entity, With<BoardRows>>,
    existing: Query<Entity, With<BoardLine>>,
    mut signature: Local<String>,
) {
    let show = state.iter().any(|state| state.phase == Phase::Lobby);
    let ordered = if show {
        boards
            .iter()
            .next()
            .map(|board| ordered_board(&board.rows))
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    if ordered.is_empty() {
        if !signature.is_empty() {
            signature.clear();
            for entity in existing.iter().collect::<Vec<_>>() {
                commands.entity(entity).despawn();
            }
        }
        for mut node in &mut panel {
            if node.display != Display::None {
                node.display = Display::None;
            }
        }
        return;
    }
    for mut node in &mut panel {
        if node.display != Display::Flex {
            node.display = Display::Flex;
        }
    }
    let Ok(root) = roots.single() else {
        return;
    };
    let me = local_client_id(&players, &net);
    let mut sig = format!("me:{me:?}\n");
    for row in &ordered {
        sig.push_str(&format!(
            "{} {} {} {} {}\n",
            row.client_id, row.points, row.played, row.last_place, row.last_round
        ));
        sig.push_str(&row.name);
        sig.push('\n');
    }
    if sig == *signature {
        return;
    }
    *signature = sig;
    for entity in existing.iter().collect::<Vec<_>>() {
        commands.entity(entity).despawn();
    }
    commands.entity(root).with_children(|parent| {
        for (index, row) in ordered.iter().enumerate() {
            let mine = me == Some(row.client_id);
            let fg = if mine { GOLD } else { Color::WHITE };
            let bg = if mine {
                YOU_BG
            } else {
                Color::srgba(0.0, 0.0, 0.0, 0.0)
            };
            let rank = (index + 1).to_string();
            let points = row.points.to_string();
            let played = row.played.to_string();
            let last = row.last_place.to_string();
            spawn_line(
                parent,
                &font,
                bg,
                true,
                [
                    (rank.as_str(), fg, W_RANK),
                    (row.name.as_str(), fg, W_NAME),
                    (points.as_str(), fg, W_POINTS),
                    (played.as_str(), fg, W_PLAYED),
                    (last.as_str(), fg, W_LAST),
                ],
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(
        client_id: u64,
        points: u16,
        played: u16,
        last_place: u8,
        last_round: u32,
    ) -> BoardRow {
        BoardRow {
            client_id,
            name: format!("n{client_id}"),
            points,
            played,
            last_place,
            last_round,
        }
    }

    #[test]
    fn ties_break_by_fewer_rounds_then_place_then_id() {
        // Points win first, even with more rounds and a worse place.
        // Then fewer rounds, then a better (lower) place, then a lower client id.
        let rows = vec![
            sample(8, 5, 1, 1, 1),
            sample(2, 5, 1, 1, 1),
            sample(3, 5, 1, 4, 1),
            sample(4, 5, 3, 1, 1),
            sample(1, 9, 9, 8, 1),
        ];
        let ids: Vec<u64> = ordered_board(&rows)
            .iter()
            .map(|row| row.client_id)
            .collect();
        assert_eq!(ids, vec![1, 2, 8, 3, 4]);
    }

    #[test]
    fn ordered_board_keeps_twelve() {
        let rows: Vec<BoardRow> = (0..13)
            .map(|id| sample(id, 100 - id as u16, 1, 1, 1))
            .collect();
        let ids: Vec<u64> = ordered_board(&rows)
            .iter()
            .map(|row| row.client_id)
            .collect();
        assert_eq!(ids.len(), 12);
        assert_eq!(ids[0], 0);
        assert!(!ids.contains(&12));
    }

    #[test]
    fn no_gain_when_last_round_is_not_this_match() {
        // Cumulative points are 99; the bonus comes from place 2, which is 6.
        let placed = sample(1, 99, 4, 2, 3);
        assert_eq!(round_gain(false, Some(&placed), 3), Some(6));
        assert_eq!(
            round_gain(false, Some(&placed), 3).map(gain_label),
            Some("+6".to_string())
        );
        assert_eq!(round_gain(false, Some(&placed), 4), None);
        assert_eq!(round_gain(true, Some(&placed), 3), None);
        assert_eq!(round_gain(false, None, 3), None);
        let eighth = sample(1, 0, 1, 8, 3);
        assert_eq!(
            round_gain(false, Some(&eighth), 3).map(gain_label),
            Some("+0".to_string())
        );
    }
}

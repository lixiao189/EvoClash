//! Results screen: standings, leavers, the next-round countdown, and leave.
//! A private-room host can pause or resume automatic continue from here.

use std::cmp::Ordering;

use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;
use evoclash_game::evolution::{EvoPath, Evolution};
use evoclash_game::protocol::{
    CommandChannel, Departed, MatchState, Phase, Player, RoomCommand, RoomSettings, Stats,
};
use lightyear::prelude::{Client, MessageSender};

use crate::buttons::BlocksPointer;
use crate::hud::{GOLD, PANEL_BG, path_color};
use crate::input::LocalPlayer;
use crate::menu::MenuOpen;
use crate::render::UiFont;
use crate::session::Depart;

const HOVER_BG: Color = Color::srgba(0.3, 0.28, 0.2, 0.95);
const YOU_BG: Color = Color::srgba(0.55, 0.4, 0.12, 0.92);
const GONE: Color = Color::srgba(0.62, 0.62, 0.62, 0.85);
const HEADER: Color = Color::srgba(1.0, 1.0, 1.0, 0.55);
const NOTE: Color = Color::srgba(1.0, 0.86, 0.62, 0.95);

const W_RANK: f32 = 40.0;
const W_NAME: f32 = 180.0;
const W_SCORE: f32 = 48.0;
const W_KD: f32 = 88.0;
const W_LEVEL: f32 = 44.0;
const W_PATH: f32 = 72.0;
const W_TAG: f32 = 64.0;

pub struct ResultsUiPlugin;

impl Plugin for ResultsUiPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_results_bar);
        app.add_systems(
            Update,
            (sync_results_table, update_results_bar, click_results_bar).chain(),
        );
    }
}

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum ResultsButton {
    Leave,
    PauseAuto,
    ResumeAuto,
}

#[derive(Component)]
struct ResultsBar;

#[derive(Component)]
struct ResultsTable;

#[derive(Component)]
struct ResultsRow;

#[derive(Component)]
struct ResultsCountdown;

#[derive(Component)]
struct ResultsPaused;

/// Sort key shared with the server: score, kills, deaths, earlier final score, client id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RankKey {
    score: u32,
    kills: u32,
    deaths: u32,
    last_score_tick: u32,
    client_id: u64,
}

fn cmp_rank(a: RankKey, b: RankKey) -> Ordering {
    b.score
        .cmp(&a.score)
        .then(b.kills.cmp(&a.kills))
        .then(a.deaths.cmp(&b.deaths))
        .then(a.last_score_tick.cmp(&b.last_score_tick))
        .then(a.client_id.cmp(&b.client_id))
}

fn sort_by_rank<T>(rows: &mut [T], key: impl Fn(&T) -> RankKey) {
    rows.sort_by(|a, b| cmp_rank(key(a), key(b)));
}

struct Standing {
    key: RankKey,
    name: String,
    bot: bool,
    level: u8,
    path: Option<EvoPath>,
    local: bool,
}

struct DepartedLine {
    name: String,
    score: u32,
    kills: u32,
    deaths: u32,
    level: u8,
    path: Option<EvoPath>,
}

fn fmt_time(ticks: u32) -> String {
    let secs = ticks / 64;
    format!("{}:{:02}", secs / 60, secs % 60)
}

fn results_button(font: &UiFont, kind: ResultsButton, value: &str, shown: bool) -> impl Bundle {
    (
        kind,
        BlocksPointer,
        Node {
            display: if shown { Display::Flex } else { Display::None },
            padding: UiRect::axes(Val::Px(14.0), Val::Px(8.0)),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(4.0)),
            ..default()
        },
        BackgroundColor(PANEL_BG),
        BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.25)),
        children![(Text::new(value), font.sized(18.0), TextColor(Color::WHITE))],
    )
}

fn cell(font: &UiFont, value: &str, color: Color, width: f32) -> impl Bundle {
    (
        Text::new(value),
        font.sized(15.0),
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

fn spawn_cells(
    parent: &mut ChildSpawnerCommands,
    font: &UiFont,
    bg: Color,
    gap_top: f32,
    mark_row: bool,
    cells: [(&str, Color, f32); 7],
) {
    let mut entity = parent.spawn((
        Node {
            column_gap: Val::Px(8.0),
            align_items: AlignItems::Center,
            padding: UiRect::axes(Val::Px(6.0), Val::Px(1.0)),
            margin: UiRect::top(Val::Px(gap_top)),
            border_radius: BorderRadius::all(Val::Px(3.0)),
            ..default()
        },
        BackgroundColor(bg),
    ));
    if mark_row {
        entity.insert(ResultsRow);
    }
    entity.with_children(|line| {
        for (text, color, width) in cells {
            line.spawn(cell(font, text, color, width));
        }
    });
}

fn header_cells() -> [(&'static str, Color, f32); 7] {
    [
        ("名次", HEADER, W_RANK),
        ("名字", HEADER, W_NAME),
        ("分数", HEADER, W_SCORE),
        ("击杀/死亡", HEADER, W_KD),
        ("等级", HEADER, W_LEVEL),
        ("路线", HEADER, W_PATH),
        ("", HEADER, W_TAG),
    ]
}

fn name_text(bot: bool, name: &str) -> String {
    if bot {
        format!("[AI] {name}")
    } else {
        name.to_string()
    }
}

fn path_text(path: Option<EvoPath>) -> String {
    path.map(|path| path.name().to_string())
        .unwrap_or_else(|| "-".to_string())
}

/// Under the winner line. Staying is automatic; 离开 drops back to the name page.
fn spawn_results_bar(mut commands: Commands, font: Res<UiFont>) {
    commands
        .spawn((
            ResultsBar,
            Visibility::Hidden,
            GlobalZIndex(8),
            Node {
                position_type: PositionType::Absolute,
                top: Val::Percent(46.0),
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(8.0),
                ..default()
            },
        ))
        .with_children(|column| {
            column
                .spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        row_gap: Val::Px(2.0),
                        padding: UiRect::axes(Val::Px(12.0), Val::Px(8.0)),
                        border_radius: BorderRadius::all(Val::Px(6.0)),
                        ..default()
                    },
                    BackgroundColor(PANEL_BG),
                ))
                .with_children(|panel| {
                    spawn_cells(
                        panel,
                        &font,
                        Color::srgba(0.0, 0.0, 0.0, 0.0),
                        0.0,
                        false,
                        header_cells(),
                    );
                    panel.spawn((
                        ResultsTable,
                        Node {
                            flex_direction: FlexDirection::Column,
                            align_items: AlignItems::Center,
                            row_gap: Val::Px(2.0),
                            ..default()
                        },
                    ));
                });
            column.spawn((
                ResultsCountdown,
                Text::new(""),
                font.sized(18.0),
                TextColor(GOLD),
                TextLayout::justify(Justify::Center),
                TextShadow::default(),
            ));
            column.spawn((
                ResultsPaused,
                Text::new("房主已暂停续局"),
                font.sized(15.0),
                TextColor(NOTE),
                TextShadow::default(),
                Node {
                    display: Display::None,
                    ..default()
                },
            ));
            column
                .spawn(Node {
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(12.0),
                    ..default()
                })
                .with_children(|row| {
                    row.spawn((
                        Text::new("留在房间即可进入下一局"),
                        font.sized(15.0),
                        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.78)),
                        TextShadow::default(),
                    ));
                    row.spawn(results_button(
                        &font,
                        ResultsButton::PauseAuto,
                        "暂停续局",
                        false,
                    ));
                    row.spawn(results_button(
                        &font,
                        ResultsButton::ResumeAuto,
                        "恢复续局",
                        false,
                    ));
                    row.spawn(results_button(&font, ResultsButton::Leave, "离开", true));
                });
        });
}

fn spawn_ranked(parent: &mut ChildSpawnerCommands, font: &UiFont, rank: usize, row: &Standing) {
    let fg = if row.local { GOLD } else { Color::WHITE };
    let (path, path_tint) = match row.path {
        Some(path) => (path.name().to_string(), path_color(path)),
        None => ("-".to_string(), Color::srgba(1.0, 1.0, 1.0, 0.45)),
    };
    let rank = rank.to_string();
    let name = name_text(row.bot, &row.name);
    let score = row.key.score.to_string();
    let kd = format!("{}/{}", row.key.kills, row.key.deaths);
    let level = row.level.to_string();
    let bg = if row.local {
        YOU_BG
    } else {
        Color::srgba(0.0, 0.0, 0.0, 0.0)
    };
    spawn_cells(
        parent,
        font,
        bg,
        0.0,
        true,
        [
            (rank.as_str(), fg, W_RANK),
            (name.as_str(), fg, W_NAME),
            (score.as_str(), fg, W_SCORE),
            (kd.as_str(), fg, W_KD),
            (level.as_str(), fg, W_LEVEL),
            (path.as_str(), path_tint, W_PATH),
            ("", fg, W_TAG),
        ],
    );
}

fn spawn_departed(
    parent: &mut ChildSpawnerCommands,
    font: &UiFont,
    row: &DepartedLine,
    first: bool,
) {
    let score = row.score.to_string();
    let kd = format!("{}/{}", row.kills, row.deaths);
    let level = row.level.to_string();
    let path = path_text(row.path);
    spawn_cells(
        parent,
        font,
        Color::srgba(0.0, 0.0, 0.0, 0.0),
        if first { 8.0 } else { 0.0 },
        true,
        [
            ("", GONE, W_RANK),
            (row.name.as_str(), GONE, W_NAME),
            (score.as_str(), GONE, W_SCORE),
            (kd.as_str(), GONE, W_KD),
            (level.as_str(), GONE, W_LEVEL),
            (path.as_str(), GONE, W_PATH),
            ("已离开", GONE, W_TAG),
        ],
    );
}

fn sync_results_table(
    mut commands: Commands,
    font: Res<UiFont>,
    state: Query<&MatchState>,
    players: Query<(&Player, &Stats, Option<&Evolution>, Has<LocalPlayer>)>,
    departed: Query<&Departed>,
    roots: Query<Entity, With<ResultsTable>>,
    existing: Query<Entity, With<ResultsRow>>,
    mut signature: Local<String>,
) {
    let show = state.iter().any(|s| s.phase == Phase::Results);
    if !show {
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
    let mut rows: Vec<Standing> = players
        .iter()
        .map(|(player, stats, evo, local)| Standing {
            key: RankKey {
                score: stats.score,
                kills: stats.kills,
                deaths: stats.deaths,
                last_score_tick: stats.last_score_tick,
                client_id: player.client_id,
            },
            name: player.name.clone(),
            bot: player.bot,
            level: evo.map(|evo| evo.level).unwrap_or(stats.level),
            path: evo.and_then(|evo| evo.lead_path()),
            local,
        })
        .collect();
    sort_by_rank(&mut rows, |row| row.key);
    let gone: Vec<DepartedLine> = departed
        .iter()
        .flat_map(|list| list.0.iter())
        .map(|row| DepartedLine {
            name: row.name.clone(),
            score: row.score,
            kills: row.kills,
            deaths: row.deaths,
            level: row.level,
            path: row.path,
        })
        .collect();
    let mut sig = String::new();
    for row in &rows {
        sig.push_str(&format!(
            "r {} {} {} {} {} {} {} {} {:?}\n",
            row.key.client_id,
            row.key.score,
            row.key.kills,
            row.key.deaths,
            row.key.last_score_tick,
            row.level,
            row.bot,
            row.local,
            row.path,
        ));
        sig.push_str(&row.name);
        sig.push('\n');
    }
    for row in &gone {
        sig.push_str(&format!(
            "d {} {} {} {} {:?}\n",
            row.score, row.kills, row.deaths, row.level, row.path
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
        for (i, row) in rows.iter().enumerate() {
            spawn_ranked(parent, &font, i + 1, row);
        }
        for (i, row) in gone.iter().enumerate() {
            spawn_departed(parent, &font, row, i == 0);
        }
    });
}

fn host_here(settings: Option<&RoomSettings>, me: Option<&Player>) -> bool {
    let Some(settings) = settings else {
        return false;
    };
    settings.private_room
        && settings.host != 0
        && me.is_some_and(|player| player.client_id == settings.host)
}

fn update_results_bar(
    state: Query<&MatchState>,
    settings: Query<&RoomSettings>,
    me: Query<&Player, With<LocalPlayer>>,
    mut bars: Query<&mut Visibility, With<ResultsBar>>,
    mut buttons: Query<(
        &ResultsButton,
        &RelativeCursorPosition,
        &mut Node,
        &mut BackgroundColor,
    )>,
    mut notes: Query<&mut Node, (With<ResultsPaused>, Without<ResultsButton>)>,
    mut countdown: Query<&mut Text, With<ResultsCountdown>>,
) {
    let current = state.iter().next();
    let show = current.is_some_and(|s| s.phase == Phase::Results);
    let vis = if show {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut bar in &mut bars {
        bar.set_if_neq(vis);
    }
    let settings = settings.iter().next();
    let host = host_here(settings, me.iter().next());
    let auto_paused = settings.is_some_and(|room| room.auto_paused);
    for (kind, cursor, mut node, mut bg) in &mut buttons {
        let on = show
            && match *kind {
                ResultsButton::Leave => true,
                ResultsButton::PauseAuto => host && !auto_paused,
                ResultsButton::ResumeAuto => host && auto_paused,
            };
        let display = if on { Display::Flex } else { Display::None };
        if node.display != display {
            node.display = display;
        }
        if !on {
            continue;
        }
        let color = if cursor.cursor_over() {
            HOVER_BG
        } else {
            PANEL_BG
        };
        if bg.0 != color {
            bg.0 = color;
        }
    }
    let note = if show && auto_paused && !host {
        Display::Flex
    } else {
        Display::None
    };
    for mut node in &mut notes {
        if node.display != note {
            node.display = note;
        }
    }
    if let Some(s) = current.filter(|s| s.phase == Phase::Results) {
        let value = format!("下一局 {}", fmt_time(s.ticks_left));
        for mut text in &mut countdown {
            if text.0 != value {
                text.0 = value.clone();
            }
        }
    }
}

fn send_room(senders: &mut Query<&mut MessageSender<RoomCommand>, With<Client>>, cmd: RoomCommand) {
    for mut sender in senders.iter_mut() {
        sender.send::<CommandChannel>(cmd.clone());
    }
}

fn click_results_bar(
    menu: Res<MenuOpen>,
    mouse: Option<Res<ButtonInput<MouseButton>>>,
    state: Query<&MatchState>,
    buttons: Query<(
        &ResultsButton,
        &RelativeCursorPosition,
        &InheritedVisibility,
        &Node,
    )>,
    mut senders: Query<&mut MessageSender<RoomCommand>, With<Client>>,
    mut depart: ResMut<Depart>,
) {
    if menu.0 {
        return;
    }
    if !state.iter().any(|s| s.phase == Phase::Results) {
        return;
    }
    if !mouse.is_some_and(|m| m.just_pressed(MouseButton::Left)) {
        return;
    }
    let hit = buttons.iter().find_map(|(button, cursor, vis, node)| {
        (node.display != Display::None && vis.get() && cursor.cursor_over()).then_some(*button)
    });
    match hit {
        Some(ResultsButton::Leave) => {
            send_room(&mut senders, RoomCommand::Leave);
            depart.request();
        }
        Some(ResultsButton::PauseAuto) => {
            send_room(&mut senders, RoomCommand::PauseAutoStart);
        }
        Some(ResultsButton::ResumeAuto) => {
            send_room(&mut senders, RoomCommand::ResumeAutoStart);
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(client_id: u64, score: u32, kills: u32, deaths: u32, last_score_tick: u32) -> RankKey {
        RankKey {
            score,
            kills,
            deaths,
            last_score_tick,
            client_id,
        }
    }

    #[test]
    fn earlier_last_score_tick_ranks_first_when_the_rest_ties() {
        let mut rows = vec![
            key(7, 10, 4, 2, 300),
            key(9, 10, 4, 2, 80),
            key(3, 10, 4, 2, 80),
        ];
        sort_by_rank(&mut rows, |row| *row);
        assert_eq!(
            rows.iter().map(|row| row.client_id).collect::<Vec<_>>(),
            vec![3, 9, 7]
        );
    }

    #[test]
    fn score_then_kills_then_fewer_deaths() {
        let mut rows = vec![
            key(1, 5, 9, 0, 1),
            key(2, 8, 1, 4, 50),
            key(3, 5, 9, 2, 1),
            key(4, 5, 3, 0, 1),
        ];
        sort_by_rank(&mut rows, |row| *row);
        assert_eq!(
            rows.iter().map(|row| row.client_id).collect::<Vec<_>>(),
            vec![2, 1, 3, 4]
        );
    }
}

//! Lobby chrome, private-room controls, and the full-screen Starting overlay.
//! Crash-recovery copy lives in `recover_ui`.
//!
//! The host crown is `Textures.host_crown` (11×8 texels at [`PIXEL`]) next to `RoomSettings.host`.
//! Bots keep `Textures.bot_tag`.

use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;
use evoclash_game::protocol::{
    CommandChannel, MatchState, Phase, Player, ROOM_COMBATANTS, RoomCommand, RoomSettings,
};
use lightyear::prelude::{Client, MessageSender};

use crate::buttons::BlocksPointer;
use crate::hud::GOLD;
use crate::input::LocalPlayer;
use crate::menu::MenuOpen;
use crate::render::UiFont;
use crate::session::{Depart, JoinNotice, JoinOffer, JoinOfferButton, JoinOffers, RecoverHold};
use crate::textures::{PIXEL, Textures};

const TICKS_PER_SEC: u32 = 64;
const STARTING_SECS: u32 = 3;
/// Texels of `Textures.bot_tag`; drawn at [`PIXEL`].
const BOT_TAG_TEXELS: Vec2 = Vec2::new(14.0, 7.0);
/// Texels of `Textures.host_crown`; drawn at [`PIXEL`].
const CROWN_TEXELS: Vec2 = Vec2::new(11.0, 8.0);

const TITLE: &str = "等待大厅";
const PUBLIC_ROOM: &str = "公开房间";
const PRACTICE: &str = "练习场：不计分、不掉落，开局会重置";
const WAITING_HOST: &str = "等待房主开始";

const NORMAL_BG: Color = Color::srgba(0.12, 0.12, 0.12, 0.85);
const HOVER_BG: Color = Color::srgba(0.3, 0.28, 0.2, 0.95);
const DISABLED_BG: Color = Color::srgba(0.08, 0.08, 0.08, 0.55);
const SELECTED_BG: Color = Color::srgb(0.55, 0.4, 0.12);
const DIM_SELECTED_BG: Color = Color::srgba(0.45, 0.36, 0.14, 0.75);

pub struct LobbyUiPlugin;

impl Plugin for LobbyUiPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_lobby_ui);
        app.add_systems(
            Update,
            (
                layout_lobby,
                update_lobby,
                sync_roster,
                drive_room_controls,
                update_phase_banner,
                sync_join_offers,
            )
                .chain(),
        );
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
struct LobbyColumn;

#[derive(Component)]
struct LobbyBody;

#[derive(Component)]
struct RoomTitle;

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum Chrome {
    Leave,
    Start,
    Cancel,
    Monsters(bool),
    PauseAuto,
    ResumeAuto,
}

#[derive(Component)]
struct SettingsPanel;

#[derive(Component, Clone, Copy)]
enum LobbyLine {
    Humans,
    Bots,
    Countdown,
    Paused,
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
    Notice,
}

struct RoomView {
    private: bool,
    host: bool,
    phase: Option<Phase>,
    countdown: bool,
    monsters: bool,
    code: String,
    auto_paused: bool,
}

impl RoomView {
    fn read(
        settings: &Query<&RoomSettings>,
        state: &Query<&MatchState>,
        me: &Query<&Player, With<LocalPlayer>>,
    ) -> Self {
        let settings = settings.iter().next();
        let state = state.iter().next();
        let private = settings.is_some_and(|s| s.private_room);
        let host_id = settings.map(|s| s.host).unwrap_or(0);
        let host =
            private && host_id != 0 && me.iter().next().is_some_and(|p| p.client_id == host_id);
        Self {
            private,
            host,
            phase: state.map(|s| s.phase),
            countdown: state.is_some_and(|s| s.countdown_active),
            monsters: settings.is_some_and(|s| s.monsters),
            code: settings.map(|s| s.code.clone()).unwrap_or_default(),
            auto_paused: settings.is_some_and(|s| s.auto_paused),
        }
    }

    fn lobby(&self) -> bool {
        self.phase == Some(Phase::Lobby)
    }

    /// Host may edit only in a private lobby before the countdown.
    fn editable(&self) -> bool {
        self.host && self.lobby() && !self.countdown
    }

    fn show_settings(&self) -> bool {
        // The host can only change these before the countdown; keeping the row on screen
        // during play just blocks clicks and looks like it should still work.
        self.private && self.lobby()
    }

    fn can_start(&self) -> bool {
        self.host && self.lobby() && !self.countdown
    }

    fn can_cancel(&self) -> bool {
        self.host && self.lobby() && self.countdown
    }

    /// Host pause/resume, only before the lobby countdown is running.
    fn show_pause(&self) -> bool {
        self.host && self.lobby() && !self.countdown
    }
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

fn chrome_button(font: &UiFont, chrome: Chrome, value: &str) -> impl Bundle {
    (
        chrome,
        BlocksPointer,
        Node {
            padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(4.0)),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        BackgroundColor(DISABLED_BG),
        BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.2)),
        children![(
            Text::new(value),
            font.sized(15.0),
            TextColor(Color::WHITE),
            TextShadow::default(),
        )],
    )
}

fn chip_color(enabled: bool, selected: bool, over: bool) -> Color {
    if !enabled {
        if selected {
            DIM_SELECTED_BG
        } else {
            DISABLED_BG
        }
    } else if selected {
        SELECTED_BG
    } else if over {
        HOVER_BG
    } else {
        NORMAL_BG
    }
}

fn send_room(senders: &mut Query<&mut MessageSender<RoomCommand>, With<Client>>, cmd: RoomCommand) {
    for mut sender in senders.iter_mut() {
        sender.send::<CommandChannel>(cmd.clone());
    }
}

fn copy_room_link(code: &str) {
    #[cfg(target_family = "wasm")]
    {
        let Some(window) = web_sys::window() else {
            return;
        };
        let Ok(origin) = window.location().origin() else {
            return;
        };
        let url = format!("{origin}/play/?room={code}");
        let _ = window.navigator().clipboard().write_text(&url);
    }
    #[cfg(not(target_family = "wasm"))]
    {
        let _ = code;
    }
}

fn spawn_lobby_ui(mut commands: Commands, font: Res<UiFont>) {
    commands
        .spawn((
            LobbyColumn,
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
            col.spawn((
                LobbyBody,
                Node {
                    display: Display::None,
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(2.0),
                    ..default()
                },
            ))
            .with_children(|body| {
                body.spawn(Node {
                    align_items: AlignItems::Baseline,
                    column_gap: Val::Px(14.0),
                    ..default()
                })
                .with_children(|row| {
                    row.spawn(label(&font, 22.0, TITLE, Color::WHITE));
                    row.spawn((
                        RoomTitle,
                        BlocksPointer,
                        Text::new(PUBLIC_ROOM),
                        font.sized(16.0),
                        TextColor(Color::srgba(1.0, 0.9, 0.7, 0.9)),
                        TextLayout::justify(Justify::Center),
                        TextShadow::default(),
                        Node {
                            padding: UiRect::axes(Val::Px(4.0), Val::Px(2.0)),
                            ..default()
                        },
                    ));
                });
                body.spawn(line(&font, 18.0, LobbyLine::Humans, Color::WHITE));
                body.spawn(line(
                    &font,
                    13.0,
                    LobbyLine::Bots,
                    Color::srgba(1.0, 1.0, 1.0, 0.75),
                ));
                body.spawn(line(&font, 20.0, LobbyLine::Countdown, GOLD));
                body.spawn(label(
                    &font,
                    14.0,
                    PRACTICE,
                    Color::srgba(1.0, 0.86, 0.62, 0.9),
                ));
                body.spawn((
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
                body.spawn(Node {
                    margin: UiRect::top(Val::Px(8.0)),
                    column_gap: Val::Px(8.0),
                    align_items: AlignItems::Center,
                    ..default()
                })
                .with_children(|row| {
                    row.spawn(chrome_button(&font, Chrome::Leave, "离开"));
                    row.spawn(chrome_button(&font, Chrome::Start, "开始"));
                    row.spawn(chrome_button(&font, Chrome::Cancel, "取消"));
                    row.spawn(chrome_button(&font, Chrome::PauseAuto, "暂停续局"));
                    row.spawn(chrome_button(&font, Chrome::ResumeAuto, "恢复续局"));
                    row.spawn(line(
                        &font,
                        15.0,
                        LobbyLine::Paused,
                        Color::srgba(1.0, 0.86, 0.62, 0.95),
                    ));
                });
            });
            col.spawn((
                SettingsPanel,
                BlocksPointer,
                Node {
                    display: Display::None,
                    flex_direction: FlexDirection::Row,
                    flex_wrap: FlexWrap::Wrap,
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(4.0),
                    row_gap: Val::Px(4.0),
                    padding: UiRect::all(Val::Px(6.0)),
                    margin: UiRect::top(Val::Px(8.0)),
                    border_radius: BorderRadius::all(Val::Px(6.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.35)),
            ))
            .with_children(|panel| {
                panel.spawn(label(
                    &font,
                    14.0,
                    "房间设置",
                    Color::srgba(1.0, 0.9, 0.7, 0.9),
                ));
                panel.spawn(label(&font, 14.0, "怪物", Color::WHITE));
                panel.spawn(chrome_button(&font, Chrome::Monsters(false), "关"));
                panel.spawn(chrome_button(&font, Chrome::Monsters(true), "开"));
            });
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
                row_gap: Val::Px(10.0),
                ..default()
            },
        ))
        .with_children(|col| {
            col.spawn(banner(&font, 112.0, BannerSlot::Count, None));
            col.spawn(banner(&font, 20.0, BannerSlot::Notice, Some(560.0)));
            col.spawn(Node {
                column_gap: Val::Px(10.0),
                ..default()
            })
            .with_children(|row| {
                row.spawn(join_button(&font, JoinOffer::QuickStart, "快速开始"));
                row.spawn(join_button(&font, JoinOffer::CreateRoom, "创建房间"));
                row.spawn(join_button(&font, JoinOffer::ChangeRoom, "返回首页"));
            });
        });
}

fn join_button(font: &UiFont, offer: JoinOffer, value: &str) -> impl Bundle {
    (
        JoinOfferButton(offer),
        BlocksPointer,
        Node {
            display: Display::None,
            padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
            border_radius: BorderRadius::all(Val::Px(4.0)),
            ..default()
        },
        BackgroundColor(Color::srgb(0.88, 0.64, 0.35)),
        children![(
            Text::new(value),
            font.sized(16.0),
            TextColor(Color::srgb(0.11, 0.09, 0.07)),
        )],
    )
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

fn layout_lobby(
    settings: Query<&RoomSettings>,
    state: Query<&MatchState>,
    me: Query<&Player, With<LocalPlayer>>,
    mut column: Query<&mut Node, With<LobbyColumn>>,
    mut body: Query<&mut Node, (With<LobbyBody>, Without<LobbyColumn>)>,
    mut panel: Query<
        &mut Node,
        (
            With<SettingsPanel>,
            Without<LobbyColumn>,
            Without<LobbyBody>,
        ),
    >,
) {
    let view = RoomView::read(&settings, &state, &me);
    // Below the match clock and boss bar; the lobby itself stays at the top.
    let top = Val::Px(if view.lobby() { 12.0 } else { 148.0 });
    for mut node in &mut column {
        if node.top != top {
            node.top = top;
        }
    }
    let body_display = if view.lobby() {
        Display::Flex
    } else {
        Display::None
    };
    for mut node in &mut body {
        if node.display != body_display {
            node.display = body_display;
        }
    }
    let settings_display = if view.show_settings() {
        Display::Flex
    } else {
        Display::None
    };
    for mut node in &mut panel {
        if node.display != settings_display {
            node.display = settings_display;
        }
    }
}

fn update_lobby(
    settings: Query<&RoomSettings>,
    state: Query<&MatchState>,
    players: Query<(&Player, Has<LocalPlayer>)>,
    mut lines: Query<(&LobbyLine, &mut Text, &mut Node)>,
    mut title: Query<
        (&mut Text, &mut TextFont, &mut TextColor),
        (With<RoomTitle>, Without<LobbyLine>),
    >,
) {
    let Some(s) = state.iter().find(|s| s.phase == Phase::Lobby) else {
        return;
    };
    let settings = settings.iter().next();
    let private = settings.is_some_and(|room| room.private_room);
    let mut humans = 0usize;
    let mut bots = 0usize;
    let mut local_id = None;
    for (player, local) in &players {
        if player.bot {
            bots += 1;
        } else {
            humans += 1;
        }
        if local {
            local_id = Some(player.client_id);
        }
    }
    let host_id = settings.map(|room| room.host).unwrap_or(0);
    let host = private && host_id != 0 && local_id == Some(host_id);
    let paused_note = settings.is_some_and(|room| room.auto_paused) && !host && !s.countdown_active;
    let countdown = if s.countdown_active {
        format!("{} 秒后开始", seconds_until_start(s.ticks_left))
    } else if private {
        WAITING_HOST.to_string()
    } else {
        String::new()
    };
    let countdown_on = s.countdown_active || private;
    for (kind, mut text, mut node) in &mut lines {
        let (value, on) = match kind {
            LobbyLine::Humans => (format!("真人 {humans}/{ROOM_COMBATANTS}"), true),
            LobbyLine::Bots => (format!("机器人 {bots}"), true),
            LobbyLine::Countdown => (countdown.clone(), countdown_on),
            LobbyLine::Paused => ("房主已暂停续局".to_string(), paused_note),
        };
        node.display = if on { Display::Flex } else { Display::None };
        if text.0 != value {
            text.0 = value;
        }
    }
    let (value, size, color) = if private {
        (
            settings.map(|room| room.code.clone()).unwrap_or_default(),
            32.0,
            GOLD,
        )
    } else {
        (
            PUBLIC_ROOM.to_string(),
            16.0,
            Color::srgba(1.0, 0.9, 0.7, 0.9),
        )
    };
    for (mut text, mut font, mut tint) in &mut title {
        if text.0 != value {
            text.0 = value.clone();
        }
        let size = FontSize::Px(size);
        if font.font_size != size {
            font.font_size = size;
        }
        if tint.0 != color {
            tint.0 = color;
        }
    }
}

fn sync_roster(
    mut commands: Commands,
    font: Res<UiFont>,
    textures: Res<Textures>,
    state: Query<&MatchState>,
    settings: Query<&RoomSettings>,
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
    let host = settings.iter().next().map(|s| s.host).unwrap_or(0);
    let mut rows: Vec<(bool, String, u64, [f32; 3])> = players
        .iter()
        .map(|p| (p.bot, p.name.clone(), p.client_id, p.color))
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let mut sig = format!("host:{host}\n");
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
    let crown = textures.host_crown.clone();
    commands.entity(root).with_children(|parent| {
        for (bot, name, id, color) in &rows {
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
                    if host != 0 && *id == host {
                        row.spawn((
                            ImageNode {
                                image: crown.clone(),
                                ..default()
                            },
                            Node {
                                width: Val::Px(CROWN_TEXELS.x * PIXEL),
                                height: Val::Px(CROWN_TEXELS.y * PIXEL),
                                ..default()
                            },
                        ));
                    }
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

fn drive_room_controls(
    menu: Res<MenuOpen>,
    mouse: Option<Res<ButtonInput<MouseButton>>>,
    settings: Query<&RoomSettings>,
    state: Query<&MatchState>,
    me: Query<&Player, With<LocalPlayer>>,
    titles: Query<(&RelativeCursorPosition, &InheritedVisibility), With<RoomTitle>>,
    mut chrome: Query<(
        &Chrome,
        &RelativeCursorPosition,
        &InheritedVisibility,
        &mut Node,
        &mut BackgroundColor,
        &mut BorderColor,
    )>,
    mut senders: Query<&mut MessageSender<RoomCommand>, With<Client>>,
    mut depart: ResMut<Depart>,
) {
    if menu.0 {
        return;
    }
    let view = RoomView::read(&settings, &state, &me);
    let pressed = mouse.is_some_and(|m| m.just_pressed(MouseButton::Left));
    let mut hit: Option<Chrome> = None;
    for (chrome, cursor, vis, mut node, mut bg, mut border) in &mut chrome {
        let (show, enabled, selected) = match *chrome {
            Chrome::Leave => (true, view.lobby(), false),
            Chrome::Start => (view.host, view.can_start(), false),
            Chrome::Cancel => (view.host, view.can_cancel(), false),
            Chrome::Monsters(on) => (true, view.editable(), view.monsters == on),
            Chrome::PauseAuto => {
                let on = view.show_pause() && !view.auto_paused;
                (on, on, false)
            }
            Chrome::ResumeAuto => {
                let on = view.show_pause() && view.auto_paused;
                (on, on, false)
            }
        };
        let display = if show { Display::Flex } else { Display::None };
        if node.display != display {
            node.display = display;
        }
        let over = cursor.cursor_over();
        let color = chip_color(enabled, selected, over);
        if bg.0 != color {
            bg.0 = color;
        }
        let edge = if selected {
            Color::srgb(0.95, 0.78, 0.35)
        } else {
            Color::srgba(1.0, 1.0, 1.0, 0.2)
        };
        border.set_if_neq(BorderColor::all(edge));
        if pressed && enabled && show && vis.get() && over {
            hit = Some(*chrome);
        }
    }
    if let Some(chrome) = hit {
        let cmd = match chrome {
            Chrome::Leave => Some(RoomCommand::Leave),
            Chrome::Start => Some(RoomCommand::Start),
            Chrome::Cancel => Some(RoomCommand::Cancel),
            Chrome::PauseAuto => Some(RoomCommand::PauseAutoStart),
            Chrome::ResumeAuto => Some(RoomCommand::ResumeAutoStart),
            Chrome::Monsters(on) => {
                (view.editable() && view.monsters != on).then_some(RoomCommand::SetSettings {
                    bots: Some(ROOM_COMBATANTS),
                    monsters: on,
                })
            }
        };
        if let Some(cmd) = cmd {
            let leaving = matches!(cmd, RoomCommand::Leave);
            send_room(&mut senders, cmd);
            if leaving {
                depart.request();
            }
        }
        return;
    }
    if pressed
        && view.private
        && !view.code.is_empty()
        && titles
            .iter()
            .any(|(cursor, vis)| vis.get() && cursor.cursor_over())
    {
        copy_room_link(&view.code);
    }
}

fn update_phase_banner(
    state: Query<&MatchState>,
    notice: Res<JoinNotice>,
    hold: Res<RecoverHold>,
    mut shade: Query<(&mut Visibility, &mut BackgroundColor), With<PhaseShade>>,
    mut slots: Query<(&BannerSlot, &mut Text, &mut Node)>,
) {
    let current = state.iter().next();
    let phase = current.map(|s| s.phase);
    let ticks = current.map(|s| s.ticks_left).unwrap_or(0);
    let (dim, alpha) = match phase {
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
            BannerSlot::Notice => {
                // `hold` is the fast-forward shade; its line is drawn in `recover_ui`.
                if covered || hold.0 || notice.0.is_empty() {
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

fn sync_join_offers(
    offers: Res<JoinOffers>,
    hold: Res<RecoverHold>,
    state: Query<&MatchState>,
    mut buttons: Query<(&JoinOfferButton, &mut Node)>,
) {
    let covered = hold.0
        || state
            .iter()
            .any(|s| matches!(s.phase, Phase::Starting | Phase::Recovering));
    for (button, mut node) in &mut buttons {
        let on = !covered && offers.buttons.contains(&button.0);
        let display = if on { Display::Flex } else { Display::None };
        if node.display != display {
            node.display = display;
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

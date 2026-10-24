//! Screen-space UI: status, match clock, scoreboard, kill feed and banners.

use bevy::prelude::*;
use evoclash_game::items::{ArmorKind, tier_color};
use evoclash_game::map::PropKind;
use evoclash_game::protocol::*;
use evoclash_game::sim::{Arena, DASH_COOLDOWN, MAX_HP, PLAYER_RADIUS};

use crate::effects::{Banner, KillFeed};
use crate::input::LocalPlayer;
use crate::render::UiFont;

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_hud);
        app.add_systems(Update, update_hud);
    }
}

#[derive(Component)]
enum HudText {
    Status,
    Clock,
    Scoreboard,
    Feed,
    Banner,
    Center,
    /// Where you are, under the minimap.
    Location,
    /// What you can do right here (crack a safe).
    Hint,
}

fn text_node(font: &UiFont, size: f32, kind: HudText, node: Node, justify: Justify) -> impl Bundle {
    (
        kind,
        Text::new(""),
        font.sized(size),
        TextColor(Color::WHITE),
        TextLayout::justify(justify),
        TextShadow::default(),
        node,
    )
}

fn spawn_hud(mut commands: Commands, font: Res<UiFont>) {
    let at = |top: Option<f32>, bottom: Option<f32>, left: Option<f32>, right: Option<f32>| Node {
        position_type: PositionType::Absolute,
        top: top.map_or(Val::Auto, Val::Px),
        bottom: bottom.map_or(Val::Auto, Val::Px),
        left: left.map_or(Val::Auto, Val::Px),
        right: right.map_or(Val::Auto, Val::Px),
        ..default()
    };
    commands.spawn(text_node(
        &font,
        18.0,
        HudText::Status,
        at(None, Some(16.0), Some(16.0), None),
        Justify::Left,
    ));
    commands.spawn(text_node(
        &font,
        15.0,
        HudText::Location,
        at(Some(178.0), None, Some(16.0), None),
        Justify::Left,
    ));
    commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(150.0),
            width: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            ..default()
        })
        .with_child(text_node(
            &font,
            18.0,
            HudText::Hint,
            Node::default(),
            Justify::Center,
        ));
    commands.spawn(text_node(
        &font,
        16.0,
        HudText::Scoreboard,
        at(Some(16.0), None, None, Some(16.0)),
        Justify::Right,
    ));
    commands.spawn(text_node(
        &font,
        15.0,
        HudText::Feed,
        at(None, Some(16.0), None, Some(16.0)),
        Justify::Right,
    ));
    // Centered rows span the full width.
    let centered = |top: f32| Node {
        position_type: PositionType::Absolute,
        top: Val::Px(top),
        width: Val::Percent(100.0),
        justify_content: JustifyContent::Center,
        ..default()
    };
    commands.spawn(centered(12.0)).with_child(text_node(
        &font,
        20.0,
        HudText::Clock,
        Node::default(),
        Justify::Center,
    ));
    commands.spawn(centered(52.0)).with_child(text_node(
        &font,
        26.0,
        HudText::Banner,
        Node::default(),
        Justify::Center,
    ));
    commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            top: Val::Percent(40.0),
            width: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            ..default()
        })
        .with_child(text_node(
            &font,
            34.0,
            HudText::Center,
            Node::default(),
            Justify::Center,
        ));
}

fn fmt_time(ticks: u32) -> String {
    let secs = ticks / 64;
    format!("{}:{:02}", secs / 60, secs % 60)
}

#[allow(clippy::type_complexity)]
fn update_hud(
    time: Res<Time>,
    arena: Res<Arena>,
    me: Query<(&Player, &Vitals, &Loadout, &Stats, &Dash, &Pos), With<LocalPlayer>>,
    props: Query<(&Pos, &Prop)>,
    players: Query<(&Player, &Stats)>,
    state: Query<&MatchState>,
    feed: Res<KillFeed>,
    banner: Res<Banner>,
    mut texts: Query<(&HudText, &mut Text, &mut TextColor)>,
) {
    let now = time.elapsed_secs();
    let me = me.single().ok();
    let state = state.iter().next();

    for (kind, mut text, mut color) in &mut texts {
        let value = match kind {
            HudText::Status => match me {
                Some((_, vitals, loadout, stats, dash, _)) => {
                    let weapon = loadout.weapon.stats();
                    color.0 = tier_color(weapon.tier).mix(&Color::WHITE, 0.4);
                    let ammo = if weapon.melee {
                        "∞".to_string()
                    } else {
                        loadout.ammo.to_string()
                    };
                    let dash = if dash.cooldown == 0 {
                        "就绪".to_string()
                    } else {
                        format!(
                            "{:.0}%",
                            100.0 * (1.0 - dash.cooldown as f32 / DASH_COOLDOWN as f32)
                        )
                    };
                    format!(
                        "生命 {:.0}/{MAX_HP:.0}   {} {:.0}\n武器 [T{}] {}  弹药 {ammo}\n电磁脉冲 ×{}   冲刺 {dash}   连杀 {}\n\nWASD 移动  鼠标 瞄准/射击  空格 冲刺  E 拾取（按住撬保险柜）  Q/右键 电磁脉冲",
                        vitals.hp,
                        vitals.armor.name(),
                        vitals.armor_points,
                        weapon.tier,
                        weapon.name,
                        loadout.emps,
                        stats.streak,
                    )
                }
                None => "连接中…".to_string(),
            },
            HudText::Clock => match state {
                Some(s) => match s.phase {
                    Phase::Warmup => {
                        format!("热身 {}  (至少 2 名玩家开局)", fmt_time(s.ticks_left))
                    }
                    Phase::Playing => format!(
                        "剩余 {}    地堡开启 {}",
                        fmt_time(s.ticks_left),
                        fmt_time(s.vault_in)
                    ),
                    Phase::Intermission => format!("下一局 {}", fmt_time(s.ticks_left)),
                },
                None => String::new(),
            },
            HudText::Scoreboard => {
                let mut rows: Vec<(&Player, &Stats)> = players.iter().collect();
                rows.sort_by(|a, b| b.1.score.cmp(&a.1.score).then(a.1.deaths.cmp(&b.1.deaths)));
                let mut out = String::from("分数  击杀/阵亡\n");
                for (player, stats) in rows {
                    let you = me.is_some_and(|(p, ..)| p.client_id == player.client_id);
                    let bounty = if stats.has_bounty() { "★" } else { "" };
                    let bot = if player.bot { "·" } else { "" };
                    let arrow = if you { "▶ " } else { "" };
                    out.push_str(&format!(
                        "{arrow}{bounty}{}{bot}  {}  {}/{}\n",
                        player.name, stats.score, stats.kills, stats.deaths
                    ));
                }
                out
            }
            HudText::Feed => feed
                .0
                .iter()
                .filter(|(_, at)| now - at < 8.0)
                .map(|(line, _)| line.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            HudText::Banner => match &banner.0 {
                Some((line, at)) if now - at < 4.0 => {
                    color.0 = Color::srgb(1.0, 0.85, 0.35).with_alpha((4.0 - (now - at)).min(1.0));
                    line.clone()
                }
                _ => String::new(),
            },
            HudText::Location => match me.filter(|(_, v, ..)| v.alive) {
                Some((_, vitals, .., pos)) => {
                    let place = arena.block_at(pos.0).map_or("街道", |b| b.name);
                    if arena.in_hazard(pos.0) {
                        let sealed =
                            vitals.armor == ArmorKind::PowerArmor && vitals.armor_points > 0.0;
                        if sealed {
                            color.0 = Color::srgb(0.6, 1.0, 0.4);
                            format!("{place}\n辐射区：动力装甲隔绝了辐射")
                        } else {
                            let blink = 0.6 + 0.4 * (now * 6.0).sin();
                            color.0 = Color::srgb(1.0, 0.45, 0.3).with_alpha(blink);
                            format!("{place}\n辐射区！持续掉血，动力装甲可免疫")
                        }
                    } else {
                        color.0 = Color::srgba(1.0, 1.0, 1.0, 0.85);
                        place.to_string()
                    }
                }
                None => String::new(),
            },
            HudText::Hint => {
                let near_safe = me.filter(|(_, v, ..)| v.alive).and_then(|(.., pos)| {
                    props.iter().find(|(p, prop)| {
                        prop.kind == PropKind::Safe
                            && p.0.distance(pos.0) < prop.kind.radius() + PLAYER_RADIUS + 40.0
                    })
                });
                match near_safe {
                    Some((_, prop)) if prop.open => {
                        color.0 = Color::srgba(0.8, 0.8, 0.8, 0.8);
                        "保险柜已被撬空".to_string()
                    }
                    Some((_, prop)) if prop.progress > 0 => {
                        color.0 = Color::srgb(1.0, 0.85, 0.3);
                        format!("撬保险柜中… {}%", prop.progress)
                    }
                    Some(_) => {
                        color.0 = Color::srgb(1.0, 0.85, 0.3);
                        "按住 E 撬开保险柜（站着别动）".to_string()
                    }
                    None => String::new(),
                }
            }
            HudText::Center => {
                if let Some(s) = state.filter(|s| s.phase == Phase::Intermission) {
                    format!("{} 获胜", s.winner.as_deref().unwrap_or("无人"))
                } else if let Some((_, vitals, ..)) = me.filter(|(_, v, ..)| !v.alive) {
                    color.0 = Color::srgb(1.0, 0.4, 0.35);
                    format!(
                        "你倒下了 — {:.1} 秒后带着木棍重生",
                        vitals.respawn_in as f32 / 64.0
                    )
                } else {
                    String::new()
                }
            }
        };
        if text.0 != value {
            text.0 = value;
        }
    }
}

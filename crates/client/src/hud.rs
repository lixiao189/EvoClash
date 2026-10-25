//! Screen-space UI: status, match clock, scoreboard, kill feed, banners, and the evolution bar
//! (level, XP, skill slots and the perk pick).

use bevy::prelude::*;
use evoclash_game::evolution::{
    EvoPath, Evolution, MAX_LEVEL, MAX_RANK, SKILL_A_RANK, SKILL_B_RANK, SkillState, xp_to_reach,
};
use evoclash_game::items::{ArmorKind, tier_color};
use evoclash_game::map::PropKind;
use evoclash_game::protocol::*;
use evoclash_game::sim::{Arena, DASH_COOLDOWN, PLAYER_RADIUS};

use crate::assets::{Loading, PendingAssets};
use crate::effects::{Banner, BossBar, KillFeed};
use crate::input::LocalPlayer;
use crate::render::UiFont;

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_hud);
        app.add_systems(Update, (update_hud, update_evolution_hud));
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
    /// Countdown to the behemoth's arrival.
    BossEta,
    /// Name and health numbers above the boss bar.
    BossName,
}

/// The boss bar's root, and the part of it that drains.
#[derive(Component)]
struct BossPanel;

#[derive(Component)]
struct BossFill;

/// Each evolution path has its own color in the HUD and effects.
pub fn path_color(path: EvoPath) -> Color {
    match path {
        EvoPath::Mutant => Color::srgb(0.55, 0.95, 0.3),
        EvoPath::Cyborg => Color::srgb(0.35, 0.75, 1.0),
        EvoPath::Scavenger => Color::srgb(1.0, 0.7, 0.3),
    }
}

const PANEL_BG: Color = Color::srgba(0.0, 0.0, 0.0, 0.6);
const LOCKED: Color = Color::srgba(0.5, 0.5, 0.5, 0.6);
const GOLD: Color = Color::srgb(1.0, 0.85, 0.35);

#[derive(Component)]
enum EvoUi {
    Level,
    XpFill,
    XpText,
    Pending,
    /// Shown only while an offer is up.
    PerkPanel,
    PerkTitle,
    /// Card `i` of the offer, with its border.
    Card(usize),
    CardHead(usize),
    CardName(usize),
    CardDetail(usize),
    /// Skill slot 0 (R) / 1 (T), with its border.
    Slot(usize),
    SlotText(usize),
    /// Darkens the slot from the bottom up while cooling down.
    SlotShade(usize),
    /// Drains while the skill is active.
    SlotActive(usize),
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
    spawn_bottom_center(&mut commands, &font);
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
    commands.spawn(centered(36.0)).with_child(text_node(
        &font,
        17.0,
        HudText::BossEta,
        Node::default(),
        Justify::Center,
    ));
    // Boss bar: a drained red fill with ticks where the phases change (60% and 30%).
    commands
        .spawn((
            BossPanel,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(100.0),
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(3.0),
                ..default()
            },
            Visibility::Hidden,
        ))
        .with_children(|panel| {
            panel.spawn(text_node(
                &font,
                18.0,
                HudText::BossName,
                Node::default(),
                Justify::Center,
            ));
            panel
                .spawn((
                    Node {
                        width: Val::Px(420.0),
                        height: Val::Px(14.0),
                        border: UiRect::all(Val::Px(2.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.65)),
                    BorderColor::all(Color::srgb(0.12, 0.1, 0.1)),
                ))
                .with_children(|bar| {
                    bar.spawn((
                        BossFill,
                        Node {
                            width: Val::Percent(100.0),
                            height: Val::Percent(100.0),
                            ..default()
                        },
                        BackgroundColor(Color::srgb(0.85, 0.2, 0.12)),
                    ));
                    for at in [60.0, 30.0] {
                        bar.spawn((
                            Node {
                                position_type: PositionType::Absolute,
                                left: Val::Percent(at),
                                top: Val::Px(0.0),
                                width: Val::Px(2.0),
                                height: Val::Percent(100.0),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.8)),
                        ));
                    }
                });
        });
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

fn ui_text(font: &UiFont, size: f32, kind: EvoUi, color: Color) -> impl Bundle {
    (
        kind,
        Text::new(""),
        font.sized(size),
        TextColor(color),
        TextShadow::default(),
    )
}

/// Bottom-center column, top to bottom: the safe hint, the perk pick, skill slots, level and XP.
fn spawn_bottom_center(commands: &mut Commands, font: &UiFont) {
    commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(14.0),
            width: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            row_gap: Val::Px(6.0),
            ..default()
        })
        .with_children(|col| {
            col.spawn(text_node(
                font,
                18.0,
                HudText::Hint,
                Node {
                    margin: UiRect::bottom(Val::Px(8.0)),
                    ..default()
                },
                Justify::Center,
            ));
            col.spawn((
                EvoUi::PerkPanel,
                Node {
                    display: Display::None,
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(4.0),
                    margin: UiRect::bottom(Val::Px(6.0)),
                    ..default()
                },
            ))
            .with_children(|panel| {
                panel.spawn(ui_text(font, 15.0, EvoUi::PerkTitle, GOLD));
                panel
                    .spawn(Node {
                        column_gap: Val::Px(8.0),
                        ..default()
                    })
                    .with_children(|row| {
                        for i in 0..3 {
                            row.spawn((
                                EvoUi::Card(i),
                                Node {
                                    width: Val::Px(200.0),
                                    flex_direction: FlexDirection::Column,
                                    row_gap: Val::Px(2.0),
                                    padding: UiRect::all(Val::Px(6.0)),
                                    border: UiRect::all(Val::Px(2.0)),
                                    border_radius: BorderRadius::all(Val::Px(4.0)),
                                    ..default()
                                },
                                BackgroundColor(PANEL_BG),
                                BorderColor::all(LOCKED),
                            ))
                            .with_children(|card| {
                                card.spawn(ui_text(font, 13.0, EvoUi::CardHead(i), Color::WHITE));
                                card.spawn(ui_text(font, 17.0, EvoUi::CardName(i), Color::WHITE));
                                card.spawn(ui_text(
                                    font,
                                    12.0,
                                    EvoUi::CardDetail(i),
                                    Color::srgba(0.9, 0.9, 0.9, 0.9),
                                ));
                            });
                        }
                    });
            });
            col.spawn(Node {
                column_gap: Val::Px(8.0),
                ..default()
            })
            .with_children(|row| {
                for slot in 0..2 {
                    row.spawn((
                        EvoUi::Slot(slot),
                        Node {
                            width: Val::Px(136.0),
                            height: Val::Px(42.0),
                            padding: UiRect::axes(Val::Px(6.0), Val::Px(3.0)),
                            border: UiRect::all(Val::Px(2.0)),
                            border_radius: BorderRadius::all(Val::Px(4.0)),
                            overflow: Overflow::clip(),
                            ..default()
                        },
                        BackgroundColor(PANEL_BG),
                        BorderColor::all(LOCKED),
                    ))
                    .with_children(|slot_node| {
                        slot_node.spawn((
                            EvoUi::SlotShade(slot),
                            Node {
                                position_type: PositionType::Absolute,
                                left: Val::Px(0.0),
                                bottom: Val::Px(0.0),
                                width: Val::Percent(100.0),
                                height: Val::Percent(0.0),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
                        ));
                        slot_node.spawn((
                            EvoUi::SlotActive(slot),
                            Node {
                                position_type: PositionType::Absolute,
                                left: Val::Px(0.0),
                                bottom: Val::Px(0.0),
                                width: Val::Percent(0.0),
                                height: Val::Px(3.0),
                                ..default()
                            },
                            BackgroundColor(GOLD),
                        ));
                        slot_node.spawn(ui_text(font, 13.0, EvoUi::SlotText(slot), Color::WHITE));
                    });
                }
            });
            col.spawn(Node {
                align_items: AlignItems::Center,
                column_gap: Val::Px(8.0),
                ..default()
            })
            .with_children(|row| {
                row.spawn((
                    Node {
                        padding: UiRect::axes(Val::Px(6.0), Val::Px(1.0)),
                        border: UiRect::all(Val::Px(2.0)),
                        border_radius: BorderRadius::all(Val::Px(4.0)),
                        ..default()
                    },
                    BackgroundColor(PANEL_BG),
                    BorderColor::all(GOLD),
                ))
                .with_child(ui_text(font, 16.0, EvoUi::Level, GOLD));
                row.spawn((
                    Node {
                        width: Val::Px(240.0),
                        height: Val::Px(10.0),
                        border: UiRect::all(Val::Px(1.0)),
                        border_radius: BorderRadius::all(Val::Px(3.0)),
                        overflow: Overflow::clip(),
                        ..default()
                    },
                    BackgroundColor(PANEL_BG),
                    BorderColor::all(Color::srgba(0.0, 0.0, 0.0, 0.8)),
                ))
                .with_child((
                    EvoUi::XpFill,
                    Node {
                        width: Val::Percent(0.0),
                        height: Val::Percent(100.0),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.55, 1.0, 0.65)),
                ));
                row.spawn(ui_text(font, 13.0, EvoUi::XpText, Color::WHITE));
                row.spawn(ui_text(font, 14.0, EvoUi::Pending, GOLD));
            });
        });
}

/// Seconds left before the server picks the open offer at random (it replicates the ticks left).
fn auto_pick_secs(evo: &Evolution) -> Option<f32> {
    evo.offer.map(|_| evo.offer_ticks_left as f32 / 64.0)
}

#[allow(clippy::type_complexity)]
fn update_evolution_hud(
    time: Res<Time>,
    me: Query<(&Evolution, &SkillState), With<LocalPlayer>>,
    mut ui: Query<(
        &EvoUi,
        Option<&mut Text>,
        Option<&mut TextColor>,
        &mut Node,
        Option<&mut BorderColor>,
        Option<&mut BackgroundColor>,
    )>,
) {
    let now = time.elapsed_secs();
    let default_evo = Evolution::default();
    let (evo, skills) = me
        .single()
        .map_or((&default_evo, SkillState::default()), |(e, s)| (e, *s));
    let countdown = auto_pick_secs(evo);
    let (lead_rank, lead) = evo
        .lead_path()
        .map_or((0, None), |p| (evo.path_rank(p), Some(p)));
    let blink = 0.65 + 0.35 * (now * 5.0).sin();

    for (kind, text, color, mut node, border, background) in &mut ui {
        let set_text = |value: String, c: Option<Color>| {
            if let Some(mut text) = text
                && text.0 != value
            {
                text.0 = value;
            }
            if let (Some(mut color), Some(c)) = (color, c) {
                color.0 = c;
            }
        };
        match *kind {
            EvoUi::Level => set_text(format!("Lv {}", evo.level), None),
            EvoUi::XpFill => {
                let floor = xp_to_reach(evo.level);
                let frac = if evo.level >= MAX_LEVEL {
                    1.0
                } else {
                    (evo.xp - floor) as f32 / (evo.xp_next - floor).max(1) as f32
                };
                node.width = Val::Percent(100.0 * frac.clamp(0.0, 1.0));
            }
            EvoUi::XpText => {
                let value = if evo.level >= MAX_LEVEL {
                    format!("{} XP  满级", evo.xp)
                } else {
                    format!("{}/{} XP", evo.xp, evo.xp_next)
                };
                set_text(value, None);
            }
            EvoUi::Pending => {
                let value = match evo.pending {
                    0 => String::new(),
                    n => format!("待选进化 ×{n}"),
                };
                set_text(value, Some(GOLD.with_alpha(blink)));
            }
            EvoUi::PerkPanel => {
                node.display = if evo.offer.is_some() {
                    Display::Flex
                } else {
                    Display::None
                };
            }
            EvoUi::PerkTitle => {
                let value = match countdown {
                    Some(secs) => format!("选择进化（按 1/2/3）  {secs:.0} 秒后随机选择"),
                    None => String::new(),
                };
                set_text(value, None);
            }
            EvoUi::Card(i) => {
                if let (Some(mut border), Some(perk)) = (border, evo.offer.map(|o| o[i])) {
                    *border = BorderColor::all(path_color(perk.evo_path()));
                }
            }
            EvoUi::CardHead(i) => {
                let Some(perk) = evo.offer.map(|o| o[i]) else {
                    continue;
                };
                let path = perk.evo_path();
                set_text(
                    format!("[{}]  {}", i + 1, path.name()),
                    Some(path_color(path)),
                );
            }
            EvoUi::CardName(i) => {
                if let Some(perk) = evo.offer.map(|o| o[i]) {
                    set_text(perk.name().to_string(), None);
                }
            }
            EvoUi::CardDetail(i) => {
                if let Some(perk) = evo.offer.map(|o| o[i]) {
                    let rank = evo.rank(perk);
                    set_text(
                        format!(
                            "等级 {rank} → {}/{MAX_RANK}
{}",
                            rank + 1,
                            perk.desc()
                        ),
                        None,
                    );
                }
            }
            EvoUi::Slot(slot) => {
                let unlocked = [skills.skill_a, skills.skill_b][slot];
                let color = match (unlocked, lead) {
                    (Some(_), Some(path)) => path_color(path),
                    _ => LOCKED,
                };
                if let Some(mut border) = border {
                    *border = BorderColor::all(color);
                }
            }
            EvoUi::SlotText(slot) => {
                let key = ["R", "T"][slot];
                let (skill, cd, active, need) = if slot == 0 {
                    (skills.skill_a, skills.cd_a, skills.active_a, SKILL_A_RANK)
                } else {
                    (skills.skill_b, skills.cd_b, skills.active_b, SKILL_B_RANK)
                };
                let (value, c) = match skill {
                    Some(skill) => {
                        let line = if active > 0 && skill.duration() > 0 {
                            format!("生效 {:.1} 秒", active as f32 / 64.0)
                        } else if cd > 0 {
                            format!("冷却 {:.0} 秒", (cd as f32 / 64.0).ceil())
                        } else {
                            "就绪".to_string()
                        };
                        let c = if cd > 0 && active == 0 {
                            Color::srgba(1.0, 1.0, 1.0, 0.7)
                        } else {
                            Color::WHITE
                        };
                        (
                            format!(
                                "[{key}] {}
{line}",
                                skill.name()
                            ),
                            c,
                        )
                    }
                    None => {
                        // Preview what the leading path will give.
                        let name = lead.map_or("技能", |p| {
                            let (a, b) = p.skills();
                            if slot == 0 { a.name() } else { b.name() }
                        });
                        let left = need.saturating_sub(lead_rank);
                        (
                            format!(
                                "[{key}] {name}
未解锁 · 还需 {left} 次进化"
                            ),
                            LOCKED,
                        )
                    }
                };
                set_text(value, Some(c));
            }
            EvoUi::SlotShade(slot) => {
                let (skill, cd, active) = if slot == 0 {
                    (skills.skill_a, skills.cd_a, skills.active_a)
                } else {
                    (skills.skill_b, skills.cd_b, skills.active_b)
                };
                // Cooldown counts from activation; only shade once the active phase ends.
                let frac = match skill {
                    Some(skill) if active == 0 => cd as f32 / skill.cooldown().max(1) as f32,
                    _ => 0.0,
                };
                node.height = Val::Percent(100.0 * frac.clamp(0.0, 1.0));
            }
            EvoUi::SlotActive(slot) => {
                let (skill, active) = if slot == 0 {
                    (skills.skill_a, skills.active_a)
                } else {
                    (skills.skill_b, skills.active_b)
                };
                let frac = match skill {
                    Some(skill) if skill.duration() > 0 => active as f32 / skill.duration() as f32,
                    _ => 0.0,
                };
                node.width = Val::Percent(100.0 * frac.clamp(0.0, 1.0));
                if let (Some(mut bg), Some(path)) = (background, lead) {
                    bg.0 = path_color(path);
                }
            }
        }
    }
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
    boss_bar: Res<BossBar>,
    bosses: Query<(&Monster, &Vitals, Option<&MonsterState>)>,
    mut panel: Query<&mut Visibility, With<BossPanel>>,
    mut fill: Query<(&mut Node, &mut BackgroundColor), With<BossFill>>,
    mut texts: Query<(&HudText, &mut Text, &mut TextColor)>,
    assets: Res<PendingAssets>,
) {
    let now = time.elapsed_secs();
    let me = me.single().ok();
    let state = state.iter().next();

    // The boss bar tracks the live monster, with the max and phase from the server's report.
    let boss = bosses
        .iter()
        .find(|(m, v, _)| m.kind == MonsterKind::Behemoth && v.alive)
        .and_then(|(_, vitals, mstate)| {
            let report = boss_bar.0?;
            let phase = mstate.map_or(report.phase, |s| s.phase.max(1));
            Some((vitals.hp.min(report.max), report.max, phase))
        });
    for mut visibility in &mut panel {
        visibility.set_if_neq(if boss.is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
    if let Some((hp, max, phase)) = boss {
        for (mut node, mut bg) in &mut fill {
            node.width = Val::Percent(100.0 * (hp / max.max(1.0)).clamp(0.0, 1.0));
            bg.0 = match phase {
                1 => Color::srgb(0.85, 0.2, 0.12),
                2 => Color::srgb(0.95, 0.4, 0.1),
                _ => Color::srgb(1.0, 0.7, 0.1),
            };
        }
    }

    for (kind, mut text, mut color) in &mut texts {
        let value = match kind {
            HudText::BossName => match boss {
                Some((hp, max, phase)) => {
                    color.0 = Color::srgb(1.0, 0.55, 0.35);
                    format!("辐射巨兽  阶段 {phase}   {hp:.0}/{max:.0}")
                }
                None => String::new(),
            },
            HudText::BossEta => match state.filter(|s| s.boss_eta > 0) {
                Some(s) => {
                    let blink = if s.boss_eta < 64 * 10 {
                        0.7 + 0.3 * (now * 8.0).sin()
                    } else {
                        1.0
                    };
                    color.0 = Color::srgb(1.0, 0.55, 0.3).with_alpha(blink);
                    format!(
                        "辐射巨兽 {:02}:{:02}",
                        s.boss_eta / 64 / 60,
                        s.boss_eta / 64 % 60
                    )
                }
                None => String::new(),
            },
            HudText::Status => match me {
                Some((_, vitals, loadout, stats, dash, _)) => {
                    let weapon = loadout.weapon.spec();
                    color.0 = tier_color(weapon.tier()).mix(&Color::WHITE, 0.4);
                    let ammo = if loadout.weapon.melee().is_some() {
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
                        "生命 {:.0}/{:.0}   {} {:.0}\n武器 [T{}] {}  弹药 {ammo}\n电磁脉冲 ×{}   冲刺 {dash}   连杀 {}\n\nWASD 移动  鼠标 瞄准/射击  空格 冲刺\nE 拾取（按住撬保险柜）  Q/右键 电磁脉冲\nR/T 技能  1/2/3 选择进化",
                        vitals.hp,
                        vitals.max_hp,
                        vitals.armor.name(),
                        vitals.armor_points,
                        weapon.tier(),
                        weapon.name(),
                        loadout.emps,
                        stats.streak,
                    )
                }
                None => match &assets.state {
                    Loading::Waiting => "加载资源中…".to_string(),
                    Loading::Failed(path) => format!("资源加载失败：{path}\n请刷新页面重试"),
                    Loading::Ready => "连接中…".to_string(),
                },
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
                let mut out = String::from("等级  分数  击杀/阵亡\n");
                for (player, stats) in rows {
                    let you = me.is_some_and(|(p, ..)| p.client_id == player.client_id);
                    let bounty = if stats.has_bounty() { "★" } else { "" };
                    let bot = if player.bot { "·" } else { "" };
                    let arrow = if you { "▶ " } else { "" };
                    out.push_str(&format!(
                        "{arrow}{bounty}{}{bot}  Lv{}  {}  {}/{}\n",
                        player.name, stats.level, stats.score, stats.kills, stats.deaths
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

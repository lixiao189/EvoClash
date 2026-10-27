//! Full-screen shade while a crashed match is coming back.
//!
//! `RecoverHold` (rejoin 503) shows [`RECOVERING`]. Once the link is up and the phase is
//! [`Phase::Recovering`], the same shade lists who is back, then the resume countdown.

use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;
use evoclash_game::protocol::{MatchState, Phase, RecoverWait};

use crate::buttons::BlocksPointer;
use crate::render::UiFont;
use crate::session::{RECOVERING, RecoverHold};

const TICKS_PER_SEC: u32 = 64;

pub struct RecoverUiPlugin;

impl Plugin for RecoverUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RecoverHold>();
        app.add_systems(Startup, spawn_recover_ui);
        app.add_systems(Update, sync_recover_ui);
    }
}

#[derive(Component)]
#[require(RelativeCursorPosition)]
struct RecoverShade;

#[derive(Component)]
struct RecoverText;

/// Copy and font size for a replicated [`RecoverWait`].
fn recover_caption(wait: RecoverWait) -> (String, f32) {
    if wait.resuming {
        let n = wait.ticks_left.div_ceil(TICKS_PER_SEC).clamp(1, 3);
        (n.to_string(), 112.0)
    } else {
        let line = format!("等待其他玩家（{}/{}）", wait.back, wait.total);
        let secs = wait.ticks_left.div_ceil(TICKS_PER_SEC);
        if secs > 0 {
            (format!("{line}\n{secs} 秒"), 32.0)
        } else {
            (line, 32.0)
        }
    }
}

fn spawn_recover_ui(mut commands: Commands, font: Res<UiFont>) {
    commands
        .spawn((
            RecoverShade,
            BlocksPointer,
            Visibility::Hidden,
            GlobalZIndex(50),
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.01, 0.02, 0.78)),
        ))
        .with_children(|root| {
            root.spawn((
                RecoverText,
                Text::new(""),
                font.sized(32.0),
                TextColor(Color::WHITE),
                TextLayout::justify(Justify::Center),
                TextShadow::default(),
                Node {
                    width: Val::Px(560.0),
                    ..default()
                },
            ));
        });
}

fn sync_recover_ui(
    hold: Res<RecoverHold>,
    state: Query<&MatchState>,
    mut shade: Query<&mut Visibility, With<RecoverShade>>,
    mut text: Query<(&mut Text, &mut TextFont), With<RecoverText>>,
) {
    let phase = state.iter().find(|s| s.phase == Phase::Recovering);
    // Fast-forward wins over a stale phase that has not been despawned yet.
    let (value, size) = if hold.0 {
        (RECOVERING.to_string(), 32.0)
    } else if let Some(s) = phase {
        recover_caption(s.recover)
    } else {
        (String::new(), 32.0)
    };
    let show = hold.0 || phase.is_some();
    for mut vis in &mut shade {
        vis.set_if_neq(if show {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
    let size = FontSize::Px(size);
    for (mut text, mut font) in &mut text {
        if text.0 != value {
            text.0 = value.clone();
        }
        if font.font_size != size {
            font.font_size = size;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waiting_lists_who_is_back_and_the_grace() {
        let wait = RecoverWait {
            back: 2,
            total: 5,
            resuming: false,
            ticks_left: 20 * TICKS_PER_SEC,
        };
        assert_eq!(recover_caption(wait).0, "等待其他玩家（2/5）\n20 秒");
        let almost = RecoverWait {
            ticks_left: 1,
            ..wait
        };
        assert_eq!(recover_caption(almost).0, "等待其他玩家（2/5）\n1 秒");
        let done = RecoverWait {
            ticks_left: 0,
            ..wait
        };
        assert_eq!(recover_caption(done).0, "等待其他玩家（2/5）");
    }

    #[test]
    fn resuming_is_a_three_second_count() {
        let wait = RecoverWait {
            back: 5,
            total: 5,
            resuming: true,
            ticks_left: 3 * TICKS_PER_SEC,
        };
        assert_eq!(recover_caption(wait), ("3".to_string(), 112.0));
        let mid = RecoverWait {
            ticks_left: 2 * TICKS_PER_SEC + 1,
            ..wait
        };
        assert_eq!(recover_caption(mid).0, "3");
        let two = RecoverWait {
            ticks_left: 2 * TICKS_PER_SEC,
            ..wait
        };
        assert_eq!(recover_caption(two).0, "2");
        let one = RecoverWait {
            ticks_left: 1,
            ..wait
        };
        assert_eq!(recover_caption(one).0, "1");
    }
}

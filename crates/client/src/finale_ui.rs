//! Finale hint under the match clock. The clock itself turns red in `hud`.
//! Shown only while `MatchState::finale` is true.

use bevy::prelude::*;
use evoclash_game::protocol::{MatchState, Player, Stats};

use crate::input::LocalPlayer;
use crate::render::UiFont;
use crate::results_ui::{RankKey, cmp_rank};

const HINT: &str = "终局：第一名位置每 10 秒闪现，击杀第一名 +2";
const YOU: &str = "你是第一名，位置会被闪现";

pub struct FinaleUiPlugin;

impl Plugin for FinaleUiPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_finale_hint);
        app.add_systems(Update, update_finale_hint);
    }
}

#[derive(Component)]
struct FinaleHint;

fn spawn_finale_hint(mut commands: Commands, font: Res<UiFont>) {
    commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            top: Val::Px(40.0),
            width: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            ..default()
        })
        .with_child((
            FinaleHint,
            Text::new(""),
            font.sized(15.0),
            TextColor(Color::srgb(1.0, 0.42, 0.28)),
            TextLayout::justify(Justify::Center),
            TextShadow::default(),
        ));
}

fn update_finale_hint(
    state: Query<&MatchState>,
    me: Query<&Player, With<LocalPlayer>>,
    players: Query<(&Player, &Stats)>,
    mut hint: Query<&mut Text, With<FinaleHint>>,
) {
    let text = if state.iter().any(|s| s.finale()) {
        let me = me.single().ok().map(|player| player.client_id);
        let mut rows: Vec<RankKey> = players
            .iter()
            .map(|(player, stats)| RankKey {
                score: stats.score,
                kills: stats.kills,
                deaths: stats.deaths,
                last_score_tick: stats.last_score_tick,
                client_id: player.client_id,
            })
            .collect();
        if you_are_leader(me, &mut rows) {
            format!("{HINT}\n{YOU}")
        } else {
            HINT.to_string()
        }
    } else {
        String::new()
    };
    for mut hint in &mut hint {
        if hint.0 != text {
            hint.0 = text.clone();
        }
    }
}

/// `rows` is ordered with [`cmp_rank`]. A score of 0 is not first: the server skips the flash
/// until someone has scored.
fn you_are_leader(me: Option<u64>, rows: &mut [RankKey]) -> bool {
    let Some(me) = me else {
        return false;
    };
    rows.sort_by(|a, b| cmp_rank(*a, *b));
    rows.first()
        .is_some_and(|top| top.client_id == me && top.score > 0)
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
    fn you_are_leader_follows_cmp_rank_and_skips_an_unscored_field() {
        let mut rows = vec![key(3, 4, 1, 2, 10), key(9, 4, 1, 0, 10), key(1, 2, 3, 0, 4)];
        assert!(!you_are_leader(Some(3), &mut rows));
        assert!(you_are_leader(Some(9), &mut rows));
        assert!(!you_are_leader(None, &mut rows));

        let mut zeros = vec![key(1, 0, 0, 0, 0), key(2, 0, 0, 0, 0)];
        assert!(!you_are_leader(Some(1), &mut zeros));
    }
}

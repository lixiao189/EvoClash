//! Room standings. Humans who place when a round ends gain board points.
//! A row is removed only after that client has been in this process and then left.
//! Restored rows wait for their owner through Lobby and Results: a lobby rebuild has no
//! bodies yet. Once the next round starts, a restored row whose owner never came back goes.

use super::*;
use crate::journal::{CatchingUp, Journal, Replaying};
use crate::spectate::SpectatorSession;

/// Last Playing tick starts at 1. `match_flow` then stores `phase=results`, so the
/// scored room line is flushed first. A clock already at 0 (replay consumed the end)
/// still scores once; `last_round` stops a second grant.
pub(super) fn score_if_ending(
    mut matched: Query<(&MatchState, &mut RoomBoard)>,
    settings: Query<&RoomSettings>,
    players: Query<(&Player, &Stats)>,
    mut journal: Option<ResMut<Journal>>,
    replaying: Option<Res<Replaying>>,
    catching: Option<Res<CatchingUp>>,
) {
    if replaying.is_some() || catching.is_some() {
        return;
    }
    let Some((state, mut board)) = matched.iter_mut().next() else {
        return;
    };
    if state.phase != Phase::Playing || state.ticks_left > 1 {
        return;
    }
    let index = state.match_index;
    let snapshot = state.clone();
    let ranked: Vec<(Player, Stats)> = players
        .iter()
        .map(|(player, stats)| (player.clone(), *stats))
        .collect();
    if !award(&mut board, index, &ranked) {
        return;
    }
    let Some(settings) = settings.iter().next().cloned() else {
        return;
    };
    if let Some(journal) = journal.as_mut()
        && journal.note_standings(&snapshot, &settings, &board)
    {
        journal.flush();
    }
}

/// `place` is 1-based and counts bots. Bots take a place and are not listed.
/// Returns false when this `match_index` was already applied.
pub(super) fn award(board: &mut RoomBoard, match_index: u32, players: &[(Player, Stats)]) -> bool {
    if board.rows.iter().any(|row| row.last_round == match_index) {
        return false;
    }
    let mut ranked = players.iter().collect::<Vec<_>>();
    ranked.sort_by(|a, b| {
        final_rank(
            b.1.score,
            b.1.kills,
            b.1.deaths,
            b.1.last_score_tick,
            b.0.client_id,
        )
        .cmp(&final_rank(
            a.1.score,
            a.1.kills,
            a.1.deaths,
            a.1.last_score_tick,
            a.0.client_id,
        ))
    });
    let mut changed = false;
    for (offset, (player, _)) in ranked.into_iter().enumerate() {
        if player.bot {
            continue;
        }
        let place = (offset + 1) as u8;
        let gain = board_points(place);
        if let Some(row) = board
            .rows
            .iter_mut()
            .find(|row| row.client_id == player.client_id)
        {
            row.name.clone_from(&player.name);
            row.points = row.points.saturating_add(gain);
            row.played = row.played.saturating_add(1);
            row.last_place = place;
            row.last_round = match_index;
        } else {
            board.rows.push(BoardRow {
                client_id: player.client_id,
                name: player.name.clone(),
                points: gain,
                played: 1,
                last_place: place,
                last_round: match_index,
            });
        }
        changed = true;
    }
    if changed {
        board.rows.sort_by_key(|row| row.client_id);
    }
    changed
}

pub(super) fn prune_board(
    mut boards: Query<&mut RoomBoard>,
    state: Query<&MatchState>,
    settings: Query<&RoomSettings>,
    humans: Query<&Player, With<Human>>,
    watchers: Query<&SpectatorSession>,
    mut journal: Option<ResMut<Journal>>,
    replaying: Option<Res<Replaying>>,
    catching: Option<Res<CatchingUp>>,
    mut seen: Local<Vec<u64>>,
) {
    if replaying.is_some() || catching.is_some() {
        return;
    }
    let mut present = Vec::new();
    for player in &humans {
        present.push(player.client_id);
    }
    for session in &watchers {
        present.push(session.client_id);
    }
    for id in &present {
        if !seen.contains(id) {
            seen.push(*id);
        }
    }
    let Some(mut board) = boards.iter_mut().next() else {
        return;
    };
    let waiting = state
        .iter()
        .next()
        .is_some_and(|state| matches!(state.phase, Phase::Lobby | Phase::Results));
    let before = board.rows.len();
    board.rows.retain(|row| {
        present.contains(&row.client_id) || (waiting && !seen.contains(&row.client_id))
    });
    if board.rows.len() == before {
        return;
    }
    write_board(&state, &settings, &board, journal.as_deref_mut());
}

/// After a Playing catch-up the world is the roster. Drop rows the replay already removed.
pub(crate) fn sync_present(world: &mut World) {
    let mut present = Vec::new();
    {
        let mut humans = world.query_filtered::<&Player, With<Human>>();
        for player in humans.iter(world) {
            present.push(player.client_id);
        }
        let mut watchers = world.query::<&SpectatorSession>();
        for session in watchers.iter(world) {
            present.push(session.client_id);
        }
    }
    let changed = {
        let mut boards = world.query::<&mut RoomBoard>();
        let Some(mut board) = boards.iter_mut(world).next() else {
            return;
        };
        let before = board.rows.len();
        board.rows.retain(|row| present.contains(&row.client_id));
        before != board.rows.len()
    };
    if !changed {
        return;
    }
    let state = {
        let mut states = world.query::<&MatchState>();
        states.iter(world).next().cloned()
    };
    let settings = {
        let mut settings = world.query::<&RoomSettings>();
        settings.iter(world).next().cloned()
    };
    let board = {
        let mut boards = world.query::<&RoomBoard>();
        boards.iter(world).next().cloned().unwrap_or_default()
    };
    let (Some(state), Some(settings)) = (state, settings) else {
        return;
    };
    if let Some(mut journal) = world.get_resource_mut::<Journal>()
        && journal.note_standings(&state, &settings, &board)
    {
        journal.flush();
    }
}

fn write_board(
    state: &Query<&MatchState>,
    settings: &Query<&RoomSettings>,
    board: &RoomBoard,
    journal: Option<&mut Journal>,
) {
    let Some(journal) = journal else {
        return;
    };
    let Some(state) = state.iter().next().cloned() else {
        return;
    };
    let Some(settings) = settings.iter().next().cloned() else {
        return;
    };
    if journal.note_standings(&state, &settings, board) {
        journal.flush();
    }
}

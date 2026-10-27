//! Finale: the last two minutes. The clock on `MatchState` is the only switch.
//! `resolve_damage` (Resolve) reads it before this tick's decrement; `step_finale`
//! (Flow, chained right after `match_flow`) reads it after.

use super::*;

/// Broadcast once, on the tick the clock first shows 2:00.
pub(super) const FINALE_ANNOUNCE: &str =
    "终局开始：第一名的位置每 10 秒在小地图上闪现一次，击杀第一名额外 +2 分";

pub(super) const FINALE_PING_TICKS: u32 = 10 * TICKS_PER_SEC;

/// Extra points for killing whoever is first. Stacks on the bounty score of 3.
pub(super) const FINALE_LEADER_BONUS: u32 = 2;

/// True when this tick's kill should pay the finale bonus.
/// `resolve_damage` runs before `match_flow` decrements `ticks_left`, so the tick
/// that is about to land on `FINALE_TICKS` already counts, and so does the last tick.
pub(super) fn finale_for_kills(state: &MatchState) -> bool {
    state.phase == Phase::Playing
        && state.ticks_left > 0
        && state.ticks_left.saturating_sub(1) <= FINALE_TICKS
}

/// First place by [`final_rank`], or `None` when every score is still 0.
/// Humans and bots both count. The same order the results screen uses.
pub(super) fn leader_client_id<'a>(players: impl Iterator<Item = (u64, &'a Stats)>) -> Option<u64> {
    players
        .max_by_key(|(id, stats)| {
            final_rank(
                stats.score,
                stats.kills,
                stats.deaths,
                stats.last_score_tick,
                *id,
            )
        })
        .filter(|(_, stats)| stats.score > 0)
        .map(|(id, _)| id)
}

pub(super) fn clear_pings(pings: &mut Query<&mut LeaderPing>) {
    for mut ping in pings.iter_mut() {
        if ping.latest.is_some() {
            *ping = LeaderPing::default();
        }
    }
}

/// Flow, immediately after `match_flow` has stepped the clock.
/// Clears the flash when a round starts or ends. Announces the finale and records
/// a flash on each 10s boundary. A dead leader, or no leader, skips that flash only.
pub fn step_finale(
    state: Single<&MatchState>,
    players: Query<(&Player, &Stats)>,
    bodies: Query<(Entity, &Player, &Pos)>,
    vitals: Query<&Vitals, With<Player>>,
    mut pings: Query<&mut LeaderPing>,
    mut outbox: ResMut<Outbox>,
    tick: Res<SimTick>,
) {
    // `begin_playing` leaves `ticks_left == MATCH_TICKS` for the rest of that tick.
    if (state.phase == Phase::Playing && state.ticks_left == MATCH_TICKS)
        || state.phase == Phase::Results
    {
        clear_pings(&mut pings);
    }
    if !state.finale() {
        return;
    }
    let elapsed = FINALE_TICKS - state.ticks_left;
    if elapsed == 0 {
        outbox.announce(FINALE_ANNOUNCE);
    }
    if !elapsed.is_multiple_of(FINALE_PING_TICKS) {
        return;
    }
    let Some(leader) = leader_client_id(
        players
            .iter()
            .map(|(player, stats)| (player.client_id, stats)),
    ) else {
        return;
    };
    let Some(pos) = alive_pos(leader, &bodies, &vitals) else {
        return;
    };
    for mut ping in pings.iter_mut() {
        ping.latest = Some(LeaderFlash {
            client_id: leader,
            pos,
            tick: tick.0,
        });
    }
}

fn alive_pos(
    leader: u64,
    bodies: &Query<(Entity, &Player, &Pos)>,
    vitals: &Query<&Vitals, With<Player>>,
) -> Option<Vec2> {
    let (entity, pos) = bodies.iter().find_map(|(entity, player, pos)| {
        (player.client_id == leader).then_some((entity, pos.0))
    })?;
    vitals
        .get(entity)
        .ok()
        .filter(|vitals| vitals.alive)
        .map(|_| pos)
}

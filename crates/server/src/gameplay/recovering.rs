//! Recovering: a frozen match waits for humans, then counts down back into Playing.
//!
//! Match clocks (`ticks_left`, `vault_in`, `boss_eta`) are not touched here. Movement and the
//! other sim systems already return while the phase is `Recovering`.

use super::*;
use crate::connections::Human;
use crate::journal::{Journal, Replaying};
use crate::spectate::SpectatorSession;

/// Reconnect grace, counted from the moment catch-up finishes. Not the pre-crash remainder.
pub const RECOVER_GRACE_TICKS: u32 = 20 * TICKS_PER_SEC;
/// Everyone still here (or whoever made it back) gets this long before Playing resumes.
pub const RECOVER_RESUME_TICKS: u32 = 3 * TICKS_PER_SEC;

/// Set on the tick the resume countdown hits 0. Applied at the start of the next tick so this
/// tick's systems still see a frozen match (the thaw line is stamped on this tick).
#[derive(Resource, Default)]
pub struct ResumePending(pub bool);

/// After a Playing catch-up: every human and watcher counts as just disconnected, and the
/// 20s grace starts now. A human who does not make it back keeps the seat (`track_presence`
/// marks the drop); only watchers are dropped when the wait runs out.
pub fn begin_wait(world: &mut World) {
    let mut humans = world.query::<&mut Human>();
    let mut total = 0u8;
    for mut human in humans.iter_mut(world) {
        // Everyone counts as away until their link comes back; a seat that is never reclaimed
        // is held like any other drop instead of being cleared here.
        human.holds = 1;
        if !human.left {
            total = total.saturating_add(1);
        }
    }
    let mut sessions = world.query::<&mut SpectatorSession>();
    for mut session in sessions.iter_mut(world) {
        session.disconnected_ticks = 1;
    }
    let mut states = world.query::<&mut MatchState>();
    for mut state in states.iter_mut(world) {
        state.phase = Phase::Recovering;
        state.recover = RecoverWait {
            back: 0,
            total,
            resuming: false,
            ticks_left: RECOVER_GRACE_TICKS,
        };
    }
}

/// Shields up and the match clock runs again. Replay applies this when it reads a thaw line.
pub fn resume_playing(world: &mut World) {
    let mut players = world.query::<(&Player, &mut Vitals)>();
    for (_, mut vitals) in players.iter_mut(world) {
        vitals.shield = SPAWN_SHIELD_TICKS;
    }
    let mut states = world.query::<&mut MatchState>();
    for mut state in states.iter_mut(world) {
        state.phase = Phase::Playing;
        state.recover = RecoverWait::default();
    }
}

/// Input, before movement, so the first Playing tick after a thaw actually simulates.
pub(super) fn apply_resume(
    mut resume: ResMut<ResumePending>,
    mut state: Query<&mut MatchState>,
    mut players: Query<&mut Vitals, With<Player>>,
) {
    if !resume.0 {
        return;
    }
    resume.0 = false;
    for mut vitals in &mut players {
        vitals.shield = SPAWN_SHIELD_TICKS;
    }
    for mut state in &mut state {
        state.phase = Phase::Playing;
        state.recover = RecoverWait::default();
    }
}

/// Flow, immediately after `match_flow`. Replay drives freeze and thaw from the log instead.
pub(super) fn step_recovering(
    mut commands: Commands,
    mut state: Query<&mut MatchState>,
    humans: Query<&Human>,
    sessions: Query<(Entity, &SpectatorSession)>,
    mut journal: Option<ResMut<Journal>>,
    tick: Res<SimTick>,
    replaying: Option<Res<Replaying>>,
    mut resume: ResMut<ResumePending>,
) {
    if replaying.is_some() || resume.0 {
        return;
    }
    let Ok(mut state) = state.single_mut() else {
        return;
    };
    if state.phase != Phase::Recovering {
        return;
    }
    let back = humans
        .iter()
        .filter(|human| human.connected() && !human.left)
        .count();
    state.recover.back = u8::try_from(back).unwrap_or(u8::MAX);
    if !state.recover.resuming {
        let everyone = state.recover.total == 0 || back >= usize::from(state.recover.total);
        if !everyone && state.recover.ticks_left > 0 {
            state.recover.ticks_left -= 1;
            if state.recover.ticks_left > 0 {
                return;
            }
        }
        let mut watchers = Vec::new();
        for (entity, session) in &sessions {
            if session.disconnected_ticks == 0 {
                continue;
            }
            watchers.push((entity, session.client_id));
        }
        for (entity, id) in &watchers {
            if let Some(journal) = journal.as_mut() {
                journal.mark(tick.0, &format!("unspectate {id}"));
            }
            commands.entity(*entity).despawn();
        }
        state.recover.resuming = true;
        state.recover.ticks_left = RECOVER_RESUME_TICKS;
        state.recover.back = u8::try_from(back).unwrap_or(u8::MAX);
        return;
    }
    state.recover.ticks_left = state.recover.ticks_left.saturating_sub(1);
    if state.recover.ticks_left == 0 {
        resume.0 = true;
        if let Some(journal) = journal.as_mut() {
            journal.mark(tick.0, "thaw");
        }
    }
}

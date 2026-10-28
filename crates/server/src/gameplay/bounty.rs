//! Server-only clock for a bounty forced by picking up bunker T4.
//! The streak itself stays on replicated `Stats`; this expiry is not on the wire.

use super::*;

/// A bounty forced by picking up bunker T4 clears if no player claims it in this long.
pub const BUNKER_BOUNTY_TICKS: u32 = 50 * TICKS_PER_SEC;

/// Ground loot the vault just spat out. Not replicated; picking the T4 up may force a bounty.
#[derive(Component)]
pub struct BunkerDrop;

/// This combatant's bounty came from a bunker T4, and it expires when `left` hits 0.
#[derive(Component)]
pub struct BunkerBounty {
    pub left: u32,
}

/// If `streak` is still under [`sim::BOUNTY_STREAK`], picking up bunker T4 sets it there
/// and arms the 50s expiry. An existing kill-streak bounty is left alone.
pub fn grant_bunker_bounty(
    commands: &mut Commands,
    entity: Entity,
    name: &str,
    stats: &mut Stats,
    playing: bool,
    outbox: &mut Outbox,
) {
    if !playing || stats.streak >= sim::BOUNTY_STREAK {
        return;
    }
    stats.streak = sim::BOUNTY_STREAK;
    commands.entity(entity).insert(BunkerBounty {
        left: BUNKER_BOUNTY_TICKS,
    });
    outbox.announce(format!("{name} 拾取了地堡的尖端科技，已被悬赏！"));
}

/// Count down bunker bounties. A player kill still pays while `left` has not hit 0;
/// `resolve_damage` removes the mark when that player dies. Normal kill-streak bounties
/// have no component here, so they never time out.
pub fn expire_bunker_bounties(
    mut commands: Commands,
    state: Query<&MatchState>,
    mut marked: Query<(Entity, &mut BunkerBounty, &Player, &mut Stats)>,
    mut outbox: ResMut<Outbox>,
) {
    if !state.iter().any(|s| s.phase == Phase::Playing) {
        return;
    }
    for (entity, mut mark, player, mut stats) in &mut marked {
        mark.left = mark.left.saturating_sub(1);
        if mark.left > 0 {
            continue;
        }
        if stats.streak > 0 {
            stats.streak = 0;
            outbox.announce(format!("{} 的悬赏解除了", player.name));
        }
        commands.entity(entity).remove::<BunkerBounty>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(app: &mut App, id: u64, name: &str, score: u32) -> Entity {
        app.world_mut()
            .spawn((
                Player {
                    client_id: id,
                    name: name.into(),
                    color: [1.0; 3],
                    bot: false,
                },
                Pos(Vec2::ZERO),
                Vitals::default(),
                Loadout::default(),
                Stats { score, ..default() },
                Evolution::default(),
                SkillState::default(),
                Recovery::default(),
                Wallet::default(),
                LifeLog::default(),
            ))
            .id()
    }

    fn announces(app: &App) -> Vec<String> {
        app.world()
            .resource::<Outbox>()
            .reliable
            .iter()
            .filter_map(|event| match event {
                GameEvent::Announce(text) => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn bunker_mark_expires_after_fifty_seconds_and_spares_a_normal_streak() {
        let mut app = App::new();
        app.init_resource::<Outbox>().insert_resource(SimTick(0));
        app.world_mut().spawn(MatchState {
            phase: Phase::Playing,
            ticks_left: MATCH_TICKS,
            ..default()
        });
        app.add_systems(Update, expire_bunker_bounties);
        let marked = body(&mut app, 1, "ann", 0);
        let normal = body(&mut app, 2, "bee", 0);
        {
            let world = app.world_mut();
            world.get_mut::<Stats>(marked).unwrap().streak = 3;
            world.entity_mut(marked).insert(BunkerBounty {
                left: BUNKER_BOUNTY_TICKS,
            });
            world.get_mut::<Stats>(normal).unwrap().streak = 5;
        }
        for _ in 0..BUNKER_BOUNTY_TICKS - 1 {
            app.update();
        }
        assert_eq!(app.world().get::<Stats>(marked).unwrap().streak, 3);
        assert!(app.world().get::<BunkerBounty>(marked).is_some());
        app.update();
        assert_eq!(app.world().get::<Stats>(marked).unwrap().streak, 0);
        assert!(app.world().get::<BunkerBounty>(marked).is_none());
        assert_eq!(app.world().get::<Stats>(normal).unwrap().streak, 5);
        assert!(announces(&app).iter().any(|t| t.contains("悬赏解除")));
    }

    fn force_bounty(streak: u32, playing: bool) -> (u32, bool, bool) {
        let mut app = App::new();
        app.init_resource::<Outbox>();
        let who = body(&mut app, 1, "ann", 0);
        app.world_mut().get_mut::<Stats>(who).unwrap().streak = streak;
        let playing_flag = playing;
        app.add_systems(
            Update,
            move |mut commands: Commands,
                  mut players: Query<(Entity, &Player, &mut Stats)>,
                  mut outbox: ResMut<Outbox>| {
                let (entity, player, mut stats) = players.single_mut().unwrap();
                grant_bunker_bounty(
                    &mut commands,
                    entity,
                    &player.name,
                    &mut stats,
                    playing_flag,
                    &mut outbox,
                );
            },
        );
        app.update();
        let stats = app.world().get::<Stats>(who).unwrap();
        let marked = app.world().get::<BunkerBounty>(who).is_some();
        let announced = announces(&app).iter().any(|t| t.contains("尖端科技"));
        (stats.streak, marked, announced)
    }

    #[test]
    fn bunker_t4_sets_streak_3_and_times_out_only_that_source() {
        let (streak, marked, announced) = force_bounty(0, true);
        assert_eq!(streak, 3);
        assert!(marked);
        assert!(announced);

        // Already a normal bounty: streak stays put, no bunker timer.
        let (streak, marked, announced) = force_bounty(3, true);
        assert_eq!(streak, 3);
        assert!(!marked);
        assert!(!announced);

        // Streak 2 is not a bounty yet, so the pickup still forces one.
        let (streak, marked, announced) = force_bounty(2, true);
        assert_eq!(streak, 3);
        assert!(marked);
        assert!(announced);

        // Lobby practice does not mint a bounty.
        let (streak, marked, _) = force_bounty(0, false);
        assert_eq!(streak, 0);
        assert!(!marked);
    }
}

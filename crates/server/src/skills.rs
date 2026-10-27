//! Path skills (R / T): 酸液吐息, 狂暴, 能量护盾, 自动炮塔, 隐身 and 陷阱. Movement effects are in
//! the shared sim (`evolution_speed_mult`); everything here is server-only.

use bevy::prelude::*;
use evoclash_game::evolution::{
    ACID_CONE_COS, ACID_RANGE, SHIELD_HP, Skill, SkillState, seen_from,
};
use evoclash_game::items::WeaponKind;
use evoclash_game::protocol::*;
use evoclash_game::sim::{Arena, PLAYER_RADIUS};
use lightyear::prelude::*;

use crate::combat::Flight;
use crate::connections::Human;
use crate::gameplay::{
    CurrentInput, Damage, DamageQueue, Outbox, PosHistory, SimSet, TICKS_PER_SEC, resolve_damage,
};

/// cos(35°): the cone is 70° wide.
const ACID_EVERY: u16 = 8;
const ACID_DAMAGE: f32 = 45.0;
const ACID_ARMOR_MULT: f32 = 2.0;
const TURRET_HP: f32 = 60.0;
const TURRET_RADIUS: f32 = 18.0;
const TURRET_RANGE: f32 = 700.0;
const TURRET_EVERY: u16 = 12;
const TURRET_DAMAGE: f32 = 9.0;
const TURRET_SHOT_SPEED: f32 = 1600.0;
const MAX_TRAPS: usize = 3;
const TRAP_TICKS: u32 = 60 * TICKS_PER_SEC;
const TRAP_RADIUS: f32 = 40.0;
const TRAP_DAMAGE: f32 = 25.0;
const TRAP_SLOW_TICKS: u16 = 3 * TICKS_PER_SEC as u16;
const TRAP_REVEAL_TICKS: u16 = 3 * TICKS_PER_SEC as u16;
/// Stealth must have lasted this long before far viewers stop getting updates, so their
/// last copy already says `stealth > 0` (the client dims it) rather than freezing visibly.
const STEALTH_HIDE_DELAY: u16 = 16;

pub struct SkillsPlugin;

impl Plugin for SkillsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            FixedUpdate,
            (
                (
                    tick_skills,
                    activate_skills,
                    break_stealth,
                    acid_breath,
                    run_turrets,
                    spring_traps,
                )
                    .chain()
                    .in_set(SimSet::Combat),
                resolve_turret_damage
                    .in_set(SimSet::Resolve)
                    .before(resolve_damage),
                stealth_visibility.in_set(SimSet::Flow),
            ),
        );
    }
}

#[derive(Component)]
pub struct TurretBrain {
    owner: Entity,
    ttl: u16,
    fire_cd: u16,
}

impl TurretBrain {
    pub fn owner(&self) -> Entity {
        self.owner
    }
}

#[derive(Component)]
pub struct TrapState {
    owner: Entity,
    ttl: u32,
}

/// Clients (link entities) a stealthed combatant is currently hidden from.
#[derive(Component, Default)]
pub struct HiddenFrom(Vec<Entity>);

/// Can `viewer_pos` see someone at `pos` with these vitals?
pub fn visible(vitals: &Vitals, pos: Vec2, viewer_pos: Vec2) -> bool {
    seen_from(vitals, pos, viewer_pos)
}

fn combat_allowed(state: &MatchState) -> bool {
    !matches!(state.phase, Phase::Results | Phase::Recovering)
}

pub(crate) fn tick_skills(mut players: Query<&mut SkillState>) {
    for mut skills in &mut players {
        if skills.cd_a == 0 && skills.cd_b == 0 && skills.active_a == 0 && skills.active_b == 0 {
            continue;
        }
        skills.cd_a = skills.cd_a.saturating_sub(1);
        skills.cd_b = skills.cd_b.saturating_sub(1);
        skills.active_a = skills.active_a.saturating_sub(1);
        skills.active_b = skills.active_b.saturating_sub(1);
        if skills.active_a == 0 && skills.shield_hp > 0.0 {
            skills.shield_hp = 0.0;
        }
    }
}

#[allow(clippy::type_complexity)]
pub(crate) fn activate_skills(
    mut commands: Commands,
    state: Single<&MatchState>,
    arena: Res<Arena>,
    mut outbox: ResMut<Outbox>,
    mut players: Query<(
        Entity,
        &Player,
        &Pos,
        &CurrentInput,
        &mut Vitals,
        &mut SkillState,
        Option<&Human>,
    )>,
    turrets: Query<(Entity, &TurretBrain)>,
    traps: Query<(Entity, &TrapState)>,
) {
    if !combat_allowed(&state) {
        return;
    }
    for (entity, player, pos, input, mut vitals, mut skills, human) in &mut players {
        if !vitals.alive {
            continue;
        }
        let aim = input.0.aim.normalize_or(Vec2::X);
        let mut used = Vec::new();
        if input.0.skill_a
            && skills.cd_a == 0
            && let Some(skill) = skills.skill_a
        {
            skills.cd_a = skill.cooldown();
            skills.active_a = skill.duration();
            match skill {
                Skill::EnergyShield => skills.shield_hp = SHIELD_HP,
                Skill::Stealth => vitals.stealth = skill.duration(),
                _ => {}
            }
            used.push(skill);
        }
        if input.0.skill_b
            && skills.cd_b == 0
            && let Some(skill) = skills.skill_b
        {
            skills.cd_b = skill.cooldown();
            skills.active_b = skill.duration();
            match skill {
                Skill::AutoTurret => {
                    // One at a time: a new turret replaces the old one.
                    for (old, brain) in &turrets {
                        if brain.owner == entity {
                            commands.entity(old).despawn();
                        }
                    }
                    let want = pos.0 + aim * (PLAYER_RADIUS + TURRET_RADIUS + 6.0);
                    let spot = arena.resolve_circle(want, TURRET_RADIUS);
                    let spot = if arena.line_of_sight(pos.0, spot) {
                        spot
                    } else {
                        pos.0
                    };
                    commands.spawn((
                        Name::new("Turret"),
                        Turret {
                            owner: player.client_id,
                        },
                        Pos(spot),
                        Aim(aim),
                        Vitals {
                            hp: TURRET_HP,
                            max_hp: TURRET_HP,
                            ..default()
                        },
                        PosHistory::default(),
                        TurretBrain {
                            owner: entity,
                            ttl: skill.duration(),
                            fire_cd: 0,
                        },
                        Replicate::to_clients(NetworkTarget::All),
                    ));
                }
                Skill::Trap => {
                    let mut mine: Vec<(u32, Entity)> = traps
                        .iter()
                        .filter(|(_, t)| t.owner == entity)
                        .map(|(e, t)| (t.ttl, e))
                        .collect();
                    if mine.len() >= MAX_TRAPS {
                        mine.sort();
                        for (_, oldest) in &mine[..=mine.len() - MAX_TRAPS] {
                            commands.entity(*oldest).despawn();
                        }
                    }
                    let mut trap = commands.spawn((
                        Name::new("Trap"),
                        Trap {
                            owner: player.client_id,
                        },
                        Pos(pos.0),
                        TrapState {
                            owner: entity,
                            ttl: TRAP_TICKS,
                        },
                    ));
                    // Only its owner ever learns where it is.
                    if let Some(human) = human {
                        trap.insert(Replicate::to_clients(NetworkTarget::Single(human.peer)));
                    }
                }
                _ => {}
            }
            used.push(skill);
        }
        for skill in used {
            let event = GameEvent::SkillUsed {
                client: player.client_id,
                skill,
                pos: pos.0,
                dir: aim,
            };
            if skill == Skill::Trap {
                // Where a trap went down is the owner's secret.
                outbox.direct.push((entity, event));
            } else {
                outbox.fast.push(event);
            }
        }
    }
}

/// Firing (or throwing) gives a stealthed combatant away.
fn break_stealth(mut players: Query<(&CurrentInput, &mut Vitals, &mut SkillState)>) {
    for (input, mut vitals, mut skills) in &mut players {
        if vitals.stealth > 0 && (input.0.fire || input.0.throw) {
            vitals.stealth = 0;
            if skills.skill_a == Some(Skill::Stealth) {
                skills.active_a = 0;
            }
        }
    }
}

#[allow(clippy::type_complexity)]
fn acid_breath(
    state: Single<&MatchState>,
    arena: Res<Arena>,
    breathers: Query<(Entity, &Player, &Pos, &CurrentInput, &SkillState)>,
    targets: Query<
        (Entity, &Pos, &Vitals, Option<&Turret>),
        Or<(With<Player>, With<Monster>, With<Turret>)>,
    >,
    mut damage: ResMut<DamageQueue>,
) {
    if !combat_allowed(&state) {
        return;
    }
    let pulses = (Skill::AcidBreath.duration() / ACID_EVERY) as f32;
    for (entity, player, pos, input, skills) in &breathers {
        if !skills.breathing_acid() || !skills.active_a.is_multiple_of(ACID_EVERY) {
            continue;
        }
        let dir = input.0.aim.normalize_or(Vec2::X);
        for (target, target_pos, vitals, turret) in &targets {
            if target == entity
                || !vitals.alive
                || turret.is_some_and(|t| t.owner == player.client_id)
            {
                continue;
            }
            let offset = target_pos.0 - pos.0;
            if offset.length() > ACID_RANGE
                || offset.normalize_or_zero().dot(dir) < ACID_CONE_COS
                || !arena.line_of_sight(pos.0, target_pos.0)
            {
                continue;
            }
            damage.0.push(Damage {
                target,
                attacker: Some(entity),
                amount: ACID_DAMAGE / pulses,
                armor_mult: ACID_ARMOR_MULT,
                weapon: None,
                emp: false,
                cause: Some(Skill::AcidBreath.name()),
            });
        }
    }
}

#[allow(clippy::type_complexity)]
pub(crate) fn run_turrets(
    mut commands: Commands,
    state: Single<&MatchState>,
    arena: Res<Arena>,
    mut turrets: Query<(Entity, &Pos, &mut Aim, &mut TurretBrain)>,
    targets: Query<(Entity, &Pos, &Vitals), Or<(With<Player>, With<Monster>)>>,
) {
    for (entity, pos, mut aim, mut brain) in &mut turrets {
        brain.ttl = brain.ttl.saturating_sub(1);
        if brain.ttl == 0 {
            commands.entity(entity).despawn();
            continue;
        }
        brain.fire_cd = brain.fire_cd.saturating_sub(1);
        if !combat_allowed(&state) {
            continue;
        }
        let target = targets
            .iter()
            .filter(|(e, p, v)| {
                *e != brain.owner && v.alive && v.shield == 0 && visible(v, p.0, pos.0)
            })
            .map(|(_, p, _)| (p.0, p.0.distance(pos.0)))
            .filter(|(p, d)| *d <= TURRET_RANGE && arena.line_of_sight(pos.0, *p))
            .min_by(|a, b| a.1.total_cmp(&b.1));
        let Some((target_pos, _)) = target else {
            continue;
        };
        let dir = (target_pos - pos.0).normalize_or(aim.0);
        if dir.distance_squared(aim.0) > 1e-4 {
            aim.0 = dir;
        }
        if brain.fire_cd > 0 {
            continue;
        }
        brain.fire_cd = TURRET_EVERY;
        let ttl = (TURRET_RANGE / TURRET_SHOT_SPEED * TICKS_PER_SEC as f32).ceil() as u16;
        commands.spawn((
            Name::new("TurretShot"),
            Projectile {
                kind: ProjectileKind::Shot(WeaponKind::Rifle),
                vel: dir * TURRET_SHOT_SPEED,
            },
            Pos(pos.0 + dir * (TURRET_RADIUS + 8.0)),
            // Hits are credited to the owner.
            Flight::turret_shot(brain.owner, ttl, TURRET_DAMAGE, Skill::AutoTurret.name()),
            Replicate::to_clients(NetworkTarget::All),
            InterpolationTarget::to_clients(NetworkTarget::All),
        ));
    }
}

#[allow(clippy::type_complexity)]
pub(crate) fn spring_traps(
    mut commands: Commands,
    state: Single<&MatchState>,
    mut traps: Query<(Entity, &Pos, &mut TrapState)>,
    mut players: Query<(Entity, &Pos, &mut Vitals), With<Player>>,
    mut damage: ResMut<DamageQueue>,
    mut outbox: ResMut<Outbox>,
) {
    for (entity, pos, mut trap) in &mut traps {
        trap.ttl = trap.ttl.saturating_sub(1);
        if trap.ttl == 0 {
            commands.entity(entity).despawn();
            continue;
        }
        if !combat_allowed(&state) {
            continue;
        }
        let victim = players.iter_mut().find(|(e, p, v)| {
            *e != trap.owner && v.alive && v.shield == 0 && p.0.distance(pos.0) <= TRAP_RADIUS
        });
        let Some((victim, _, mut vitals)) = victim else {
            continue;
        };
        vitals.emp_slow = vitals.emp_slow.max(TRAP_SLOW_TICKS);
        vitals.revealed = vitals.revealed.max(TRAP_REVEAL_TICKS);
        damage.0.push(Damage {
            target: victim,
            attacker: Some(trap.owner),
            amount: TRAP_DAMAGE,
            armor_mult: 1.0,
            weapon: None,
            emp: false,
            cause: Some(Skill::Trap.name()),
        });
        outbox.fast.push(GameEvent::TrapSprung { pos: pos.0 });
        commands.entity(entity).despawn();
    }
}

/// Turrets take damage here (never from their owner); the rest is left for other resolvers.
fn resolve_turret_damage(
    mut commands: Commands,
    mut queue: ResMut<DamageQueue>,
    mut outbox: ResMut<Outbox>,
    mut turrets: Query<(&Pos, &mut Vitals, &TurretBrain)>,
    players: Query<&Player>,
) {
    queue.0.retain(|dmg| {
        let Ok((pos, mut vitals, brain)) = turrets.get_mut(dmg.target) else {
            return true;
        };
        if !vitals.alive || dmg.attacker == Some(brain.owner) {
            return false;
        }
        let amount = dmg.amount.min(vitals.hp);
        vitals.hp -= amount;
        outbox.fast.push(GameEvent::Hit {
            pos: pos.0,
            amount,
            armor: true,
            attacker: dmg
                .attacker
                .and_then(|a| players.get(a).ok())
                .map_or(0, |p| p.client_id),
        });
        if vitals.hp <= 0.0 {
            vitals.alive = false;
            commands.entity(dmg.target).despawn();
            outbox.fast.push(GameEvent::Boom {
                pos: pos.0,
                radius: 60.0,
                fire: true,
            });
        }
        false
    });
}

/// While stealthed, stop replicating a combatant to clients farther away than
/// `STEALTH_SIGHT` (they keep the last state they saw, without updates).
fn stealth_visibility(
    mut commands: Commands,
    mut sneaks: Query<(Entity, &Player, &Pos, &Vitals, &mut HiddenFrom)>,
    viewers: Query<(Entity, &Pos, &ControlledBy), With<Human>>,
    bodies: Query<(&Player, &Pos)>,
    specs: Query<(&crate::spectate::SpectatorSession, &Spectator)>,
) {
    for (entity, player, pos, vitals, mut hidden) in &mut sneaks {
        let settled =
            Skill::Stealth.duration().saturating_sub(vitals.stealth) >= STEALTH_HIDE_DELAY;
        let mut want: Vec<Entity> = if vitals.alive && vitals.stealth > 0 && settled {
            let mut links: Vec<Entity> = viewers
                .iter()
                .filter(|(viewer, viewer_pos, _)| {
                    *viewer != entity && !visible(vitals, pos.0, viewer_pos.0)
                })
                .map(|(_, _, control)| control.owner)
                .collect();
            // A watcher sees with the followed combatant's eyes, including this sneak.
            for (session, spec) in &specs {
                if session.disconnected_ticks != 0 || spec.following == 0 {
                    continue;
                }
                if spec.following == player.client_id || session.link == Entity::PLACEHOLDER {
                    continue;
                }
                let Some(eye) = bodies.iter().find_map(|(body, body_pos)| {
                    (body.client_id == spec.following).then_some(body_pos.0)
                }) else {
                    continue;
                };
                if !visible(vitals, pos.0, eye) {
                    links.push(session.link);
                }
            }
            links
        } else {
            Vec::new()
        };
        want.sort();
        want.dedup();
        if want == hidden.0 {
            continue;
        }
        for link in hidden.0.iter().filter(|l| !want.contains(l)).copied() {
            commands.queue(move |world: &mut World| world.gain_visibility(entity, link));
        }
        for link in want.iter().filter(|l| !hidden.0.contains(l)).copied() {
            commands.queue(move |world: &mut World| world.lose_visibility_retained(entity, link));
        }
        hidden.0 = want;
    }
}

#[cfg(test)]
mod tests {
    use evoclash_game::evolution::{Evolution, Perk};
    use evoclash_game::shop::Wallet;

    use super::*;
    use crate::evolution::{Dealt, LifeLog, Recovery, XpQueue};
    use crate::gameplay::{Cooldowns, Outbox};

    fn world_app() -> App {
        let mut app = App::new();
        app.init_resource::<DamageQueue>()
            .init_resource::<Outbox>()
            .init_resource::<XpQueue>()
            .init_resource::<Dealt>()
            .init_resource::<Arena>()
            .init_resource::<crate::gameplay::SimTick>();
        app.world_mut().spawn(MatchState {
            phase: Phase::Playing,
            ..default()
        });
        app
    }

    fn spawn_player(app: &mut App, id: u64, pos: Vec2) -> Entity {
        app.world_mut()
            .spawn((
                Player {
                    client_id: id,
                    name: format!("p{id}"),
                    color: [1.0; 3],
                    bot: false,
                },
                Pos(pos),
                Vitals::default(),
                Loadout::default(),
                Stats::default(),
                Evolution::default(),
                SkillState::default(),
                Recovery::default(),
                CurrentInput::default(),
                Cooldowns::default(),
                PosHistory::default(),
                (Wallet::default(), LifeLog::default()),
            ))
            .id()
    }

    #[test]
    fn skill_cooldowns() {
        let mut app = world_app();
        app.add_systems(Update, (tick_skills, activate_skills).chain());
        let p = spawn_player(&mut app, 1, Vec2::ZERO);
        let skill = Skill::EnergyShield;
        app.world_mut().get_mut::<SkillState>(p).unwrap().skill_a = Some(skill);
        app.world_mut()
            .get_mut::<CurrentInput>(p)
            .unwrap()
            .0
            .skill_a = true;
        app.update();
        let s = *app.world().get::<SkillState>(p).unwrap();
        assert_eq!(
            (s.cd_a, s.active_a, s.shield_hp),
            (skill.cooldown(), 256, SHIELD_HP)
        );
        // Holding the key does nothing until the cooldown is over.
        for _ in 0..skill.cooldown() - 1 {
            app.update();
        }
        let s = *app.world().get::<SkillState>(p).unwrap();
        assert_eq!((s.cd_a, s.active_a, s.shield_hp), (1, 0, 0.0));
        app.update();
        let s = *app.world().get::<SkillState>(p).unwrap();
        assert_eq!((s.cd_a, s.active_a), (skill.cooldown(), 256));
    }

    #[test]
    fn trap_kill_credits_the_owner() {
        let mut app = world_app();
        app.add_systems(Update, (spring_traps, resolve_damage).chain());
        let owner = spawn_player(&mut app, 1, Vec2::new(-500.0, 0.0));
        let victim = spawn_player(&mut app, 2, Vec2::ZERO);
        app.world_mut().get_mut::<Vitals>(victim).unwrap().hp = 10.0;
        app.world_mut().spawn((
            Pos(Vec2::new(20.0, 0.0)),
            TrapState {
                owner,
                ttl: TRAP_TICKS,
            },
        ));
        app.update();
        assert!(!app.world().get::<Vitals>(victim).unwrap().alive);
        assert_eq!(app.world().get::<Stats>(owner).unwrap().kills, 1);
        let xp = &app.world().resource::<XpQueue>().0;
        assert_eq!(xp.len(), 1);
        assert_eq!((xp[0].to, xp[0].base), (owner, 80));
    }

    #[test]
    fn turret_kill_credits_the_owner() {
        let mut app = world_app();
        app.add_systems(
            Update,
            (crate::combat::step_projectiles, resolve_damage).chain(),
        );
        let owner = spawn_player(&mut app, 1, Vec2::new(-500.0, 0.0));
        let victim = spawn_player(&mut app, 2, Vec2::ZERO);
        app.world_mut().get_mut::<Vitals>(victim).unwrap().hp = 5.0;
        app.init_resource::<crate::gameplay::SimTick>();
        app.world_mut().spawn((
            Projectile {
                kind: ProjectileKind::Shot(WeaponKind::Rifle),
                vel: Vec2::X * TURRET_SHOT_SPEED,
            },
            Pos(Vec2::new(-30.0, 0.0)),
            Flight::turret_shot(owner, 20, TURRET_DAMAGE, Skill::AutoTurret.name()),
        ));
        app.update();
        assert!(!app.world().get::<Vitals>(victim).unwrap().alive);
        assert_eq!(app.world().get::<Stats>(owner).unwrap().kills, 1);
        assert_eq!(app.world().resource::<XpQueue>().0[0].to, owner);
    }

    #[test]
    fn perks_survive_death() {
        let mut app = world_app();
        app.add_systems(Update, resolve_damage);
        let p = spawn_player(&mut app, 1, Vec2::ZERO);
        let mut evo = Evolution::default();
        evo.add_xp(200);
        evo.ranks[Perk::Claws.index()] = 1;
        let before = evo.clone();
        app.world_mut().entity_mut(p).insert(evo);
        app.world_mut()
            .resource_mut::<DamageQueue>()
            .0
            .push(Damage {
                target: p,
                attacker: None,
                amount: 500.0,
                armor_mult: 1.0,
                weapon: None,
                emp: false,
                cause: Some("辐射"),
            });
        app.update();
        assert!(!app.world().get::<Vitals>(p).unwrap().alive);
        assert_eq!(app.world().get::<Evolution>(p).unwrap(), &before);
    }
}

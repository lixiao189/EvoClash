//! Server side of in-match evolution: XP and scrap awards, level-ups, the perk/shop/deploy
//! commands players send, lifesteal, regeneration and armor self-repair. The numbers live in
//! `evoclash_game::evolution` and `evoclash_game::shop`.

use bevy::prelude::*;
use evoclash_game::evolution::*;
use evoclash_game::protocol::*;
use evoclash_game::shop::{PlayerCommand, ShopItem, Wallet, can_deploy};
use lightyear::prelude::server::*;
use lightyear::prelude::*;

use crate::gameplay::{Outbox, SimSet, resolve_damage};

/// 再生组织 starts this long after the last hit.
const REGEN_DELAY: u32 = 4 * 64;
/// 自修护甲 starts this long after the last hit.
const REPAIR_DELAY: u32 = 5 * 64;
const DT: f32 = 1.0 / 64.0;

pub struct EvolutionPlugin;

impl Plugin for EvolutionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<XpQueue>().init_resource::<Dealt>();
        app.add_systems(
            FixedUpdate,
            (
                read_commands.in_set(SimSet::Input),
                (lifesteal, grant_xp, recover)
                    .chain()
                    .in_set(SimSet::Resolve)
                    .after(resolve_damage),
            ),
        );
    }
}

/// XP earned this tick, before catch-up and perk multipliers.
pub struct XpAward {
    pub to: Entity,
    pub base: u32,
    /// Where it was earned (for the floating "+XP").
    pub pos: Vec2,
    /// From a crate or safe (撬锁专家 bonus applies).
    pub loot: bool,
    /// 废料 earned along with it (no catch-up or perk multiplier).
    pub scrap: u32,
}

#[derive(Resource, Default)]
pub struct XpQueue(pub Vec<XpAward>);

/// Damage someone dealt this tick, for lifesteal.
pub struct Hit {
    pub attacker: Entity,
    pub amount: f32,
    pub melee: bool,
}

#[derive(Resource, Default)]
pub struct Dealt(pub Vec<Hit>);

/// Server-only record of the current life (for the death recap) and of the perks held at
/// the last death (for 沿用上次配置).
#[derive(Component, Default)]
pub struct LifeLog {
    /// `SimTick` when this life started.
    pub born: u32,
    pub scrap: u32,
    /// Players killed this life.
    pub kills: u32,
    pub last_ranks: [u8; 12],
    pub last_first_pick: [u8; 3],
    /// 沿用上次配置 was pressed before deploying was allowed: go as soon as it is.
    pub deploy_asap: bool,
}

/// Ticks since this combatant last took damage.
#[derive(Component, Default)]
pub struct Recovery {
    pub since_hurt: u32,
}

/// Bring max HP and skill slots in line with the perks.
pub fn apply_evolution(evo: &Evolution, skills: &mut SkillState, vitals: &mut Vitals) {
    let max = evo.max_hp();
    if vitals.max_hp != max {
        if vitals.alive {
            vitals.hp = (vitals.hp + max - vitals.max_hp).max(1.0);
        }
        vitals.max_hp = max;
    }
    let (a, b) = evo.skills();
    if skills.skill_a != a {
        if skills.skill_a == Some(Skill::Stealth) {
            vitals.stealth = 0;
        }
        skills.skill_a = a;
        skills.active_a = 0;
        skills.shield_hp = 0.0;
    }
    if skills.skill_b != b {
        skills.skill_b = b;
        skills.active_b = 0;
    }
}

/// Apply one `PlayerCommand`. Outside Playing the shop, perk panel and death-deploy are
/// refused, so a lobby death stays on its 2s timer. Refunds and deploying also require
/// the body to be dead. Returns true if the perks changed (call `apply_evolution`).
pub fn handle_command(
    cmd: &PlayerCommand,
    evo: &mut Evolution,
    wallet: &mut Wallet,
    vitals: &mut Vitals,
    life: &mut LifeLog,
    allow_shop: bool,
) -> bool {
    if !allow_shop {
        return false;
    }
    if let PlayerCommand::Spend(perk) = cmd {
        return evo.spend(*perk);
    }
    if vitals.alive {
        return false;
    }
    let changed = match cmd {
        PlayerCommand::Spend(_) => false,
        PlayerCommand::Refund(perk) => evo.refund(*perk),
        PlayerCommand::ResetPerks => {
            let had = evo.total_ranks() > 0;
            evo.reset();
            had
        }
        PlayerCommand::Buy(item) => {
            wallet.buy(*item, evo);
            false
        }
        PlayerCommand::Unbuy(item) => {
            wallet.unbuy(*item);
            false
        }
        PlayerCommand::Deploy => {
            if can_deploy(vitals.respawn_in) {
                vitals.respawn_in = 1;
            }
            false
        }
        PlayerCommand::DeployLast => {
            let changed = evo.restore(life.last_ranks, life.last_first_pick);
            wallet.rebuy_last(evo);
            // Too early: `respawn` sends them off the moment it's allowed.
            if can_deploy(vitals.respawn_in) {
                vitals.respawn_in = 1;
            } else {
                life.deploy_asap = true;
            }
            changed
        }
    };
    // Fewer EMP slots after a refund: give back the ones that no longer fit.
    while wallet.cart.iter().filter(|c| **c == ShopItem::Emp).count() > ShopItem::Emp.limit(evo) {
        wallet.unbuy(ShopItem::Emp);
    }
    changed
}

/// Commands from each client link, applied to the body it controls.
#[allow(clippy::type_complexity)]
fn read_commands(
    state: Query<&MatchState>,
    mut links: Query<(Entity, &mut MessageReceiver<PlayerCommand>), With<ClientOf>>,
    mut players: Query<(
        &ControlledBy,
        &mut Evolution,
        &mut Wallet,
        &mut Vitals,
        &mut SkillState,
        &mut LifeLog,
    )>,
) {
    let allow_shop = state.iter().any(|s| s.phase == Phase::Playing);
    for (link, mut receiver) in &mut links {
        if !receiver.has_messages() {
            continue;
        }
        let cmds: Vec<PlayerCommand> = receiver.receive().collect();
        let Some((_, mut evo, mut wallet, mut vitals, mut skills, mut life)) =
            players.iter_mut().find(|(c, ..)| c.owner == link)
        else {
            continue;
        };
        for cmd in &cmds {
            if handle_command(
                cmd,
                &mut evo,
                &mut wallet,
                &mut vitals,
                &mut life,
                allow_shop,
            ) {
                apply_evolution(&evo, &mut skills, &mut vitals);
            }
        }
    }
}

pub(crate) fn grant_xp(
    state: Query<&MatchState>,
    mut queue: ResMut<XpQueue>,
    mut outbox: ResMut<Outbox>,
    mut players: Query<(
        &Player,
        &mut Evolution,
        &mut Stats,
        &mut Wallet,
        &mut LifeLog,
    )>,
) {
    if state.iter().any(|s| s.phase != Phase::Playing) {
        queue.0.clear();
        return;
    }
    if queue.0.is_empty() {
        return;
    }
    let top = players.iter().map(|(_, e, ..)| e.level).max().unwrap_or(1);
    for award in queue.0.drain(..) {
        let Ok((player, mut evo, mut stats, mut wallet, mut life)) = players.get_mut(award.to)
        else {
            continue;
        };
        wallet.scrap += award.scrap;
        life.scrap += award.scrap;
        let mut mult = catch_up_mult(top, evo.level);
        if award.loot {
            mult *= evo.loot_xp_mult();
        }
        let amount = scaled_xp(award.base, mult);
        if amount == 0 && award.scrap == 0 {
            continue;
        }
        let gained = evo.add_xp(amount);
        outbox.direct.push((
            award.to,
            GameEvent::XpGain {
                amount,
                pos: award.pos,
                scrap: award.scrap,
            },
        ));
        if gained > 0 {
            stats.level = evo.level;
            outbox.reliable.push(GameEvent::LevelUp {
                client: player.client_id,
                level: evo.level,
            });
        }
    }
}

/// 嗜血 (doubled while 狂暴).
fn lifesteal(mut dealt: ResMut<Dealt>, mut players: Query<(&Evolution, &SkillState, &mut Vitals)>) {
    for hit in dealt.0.drain(..) {
        let Ok((evo, skills, mut vitals)) = players.get_mut(hit.attacker) else {
            continue;
        };
        let frac = evo.lifesteal(hit.melee) * skills.lifesteal_mult();
        if frac <= 0.0 || !vitals.alive || vitals.hp >= vitals.max_hp {
            continue;
        }
        vitals.hp = (vitals.hp + hit.amount * frac).min(vitals.max_hp);
    }
}

/// 再生组织 and 自修护甲 kick in after a while without getting hurt.
fn recover(mut players: Query<(&Evolution, &mut Recovery, &mut Vitals)>) {
    for (evo, mut recovery, mut vitals) in &mut players {
        recovery.since_hurt = recovery.since_hurt.saturating_add(1);
        if !vitals.alive {
            continue;
        }
        let regen = evo.regen_per_sec();
        if regen > 0.0 && recovery.since_hurt >= REGEN_DELAY && vitals.hp < vitals.max_hp {
            vitals.hp = (vitals.hp + regen * DT).min(vitals.max_hp);
        }
        let repair = evo.armor_repair_per_sec();
        let cap = evo.armor_cap(vitals.armor);
        if repair > 0.0 && recovery.since_hurt >= REPAIR_DELAY && vitals.armor_points < cap {
            vitals.armor_points = (vitals.armor_points + repair * DT).min(cap);
        }
    }
}

#[cfg(test)]
mod tests {
    use evoclash_game::shop::RESPAWN_MAX_TICKS;

    use super::*;

    fn dead() -> Vitals {
        Vitals {
            alive: false,
            respawn_in: RESPAWN_MAX_TICKS,
            ..default()
        }
    }

    #[test]
    fn points_are_spent_through_commands() {
        let mut evo = Evolution::default();
        evo.add_xp(xp_to_reach(3));
        let (mut wallet, mut vitals, mut life) =
            (Wallet::default(), Vitals::default(), LifeLog::default());
        let mut run = |cmd, evo: &mut Evolution| {
            handle_command(&cmd, evo, &mut wallet, &mut vitals, &mut life, true)
        };
        assert!(run(PlayerCommand::Spend(Perk::Claws), &mut evo));
        assert!(run(PlayerCommand::Spend(Perk::Overclock), &mut evo));
        assert!(
            !run(PlayerCommand::Spend(Perk::Claws), &mut evo),
            "no points left"
        );
        assert_eq!((evo.points, evo.total_ranks()), (0, 2));
    }

    #[test]
    fn respec_and_shop_only_while_dead() {
        let mut evo = Evolution::default();
        evo.add_xp(xp_to_reach(3));
        evo.spend(Perk::Claws);
        evo.spend(Perk::Claws);
        let mut wallet = Wallet {
            scrap: 100,
            ..default()
        };
        let mut life = LifeLog::default();
        let mut alive = Vitals::default();
        for cmd in [
            PlayerCommand::Refund(Perk::Claws),
            PlayerCommand::ResetPerks,
            PlayerCommand::Buy(ShopItem::Bow),
            PlayerCommand::Deploy,
        ] {
            handle_command(&cmd, &mut evo, &mut wallet, &mut alive, &mut life, true);
        }
        assert_eq!((evo.rank(Perk::Claws), wallet.scrap), (2, 100));
        assert!(wallet.cart.is_empty() && !life.deploy_asap);

        let mut vitals = dead();
        let refund = PlayerCommand::Refund(Perk::Claws);
        assert!(handle_command(
            &refund,
            &mut evo,
            &mut wallet,
            &mut vitals,
            &mut life,
            true,
        ));
        assert_eq!((evo.rank(Perk::Claws), evo.points), (1, 1));
        let reset = PlayerCommand::ResetPerks;
        assert!(handle_command(
            &reset,
            &mut evo,
            &mut wallet,
            &mut vitals,
            &mut life,
            true,
        ));
        assert_eq!((evo.total_ranks(), evo.points), (0, 2));
        let buy = PlayerCommand::Buy(ShopItem::Bow);
        handle_command(&buy, &mut evo, &mut wallet, &mut vitals, &mut life, true);
        assert_eq!(
            (wallet.scrap, wallet.cart.clone()),
            (65, vec![ShopItem::Bow])
        );
    }

    #[test]
    fn refunding_emp_slots_returns_extra_emps() {
        let mut evo = Evolution::default();
        evo.add_xp(xp_to_reach(2));
        evo.spend(Perk::EmpExpert);
        let mut wallet = Wallet {
            scrap: 100,
            ..default()
        };
        let (mut vitals, mut life) = (dead(), LifeLog::default());
        for _ in 0..4 {
            let buy = PlayerCommand::Buy(ShopItem::Emp);
            handle_command(&buy, &mut evo, &mut wallet, &mut vitals, &mut life, true);
        }
        assert_eq!(wallet.cart.len(), 4);
        let refund = PlayerCommand::Refund(Perk::EmpExpert);
        handle_command(&refund, &mut evo, &mut wallet, &mut vitals, &mut life, true);
        assert_eq!((wallet.cart.len(), wallet.scrap), (3, 55));
    }

    #[test]
    fn grant_xp_credits_scrap_without_multipliers() {
        let mut app = App::new();
        app.init_resource::<XpQueue>().init_resource::<Outbox>();
        app.add_systems(Update, grant_xp);
        let p = app
            .world_mut()
            .spawn((
                Player {
                    client_id: 1,
                    name: "p".into(),
                    color: [1.0; 3],
                    bot: false,
                },
                Evolution::default(),
                Stats::default(),
                Wallet::default(),
                LifeLog::default(),
            ))
            .id();
        for (base, scrap) in [(20, 4), (0, 6)] {
            app.world_mut().resource_mut::<XpQueue>().0.push(XpAward {
                to: p,
                base,
                pos: Vec2::ZERO,
                loot: true,
                scrap,
            });
        }
        app.update();
        let world = app.world();
        assert_eq!(world.get::<Wallet>(p).unwrap().scrap, 10);
        assert_eq!(world.get::<LifeLog>(p).unwrap().scrap, 10);
        // A scrap-only award still shows its floating number.
        let gains: Vec<_> = world
            .resource::<Outbox>()
            .direct
            .iter()
            .map(|(_, e)| match e {
                GameEvent::XpGain { amount, scrap, .. } => (*amount, *scrap),
                _ => panic!("{e:?}"),
            })
            .collect();
        assert_eq!(gains, vec![(20, 4), (0, 6)]);
    }

    #[test]
    fn constitution_raises_max_hp() {
        let mut evo = Evolution::default();
        evo.ranks[Perk::Constitution.index()] = 2;
        let mut skills = SkillState::default();
        let mut vitals = Vitals {
            hp: 50.0,
            ..default()
        };
        apply_evolution(&evo, &mut skills, &mut vitals);
        assert_eq!((vitals.hp, vitals.max_hp), (74.0, 124.0));
    }
}

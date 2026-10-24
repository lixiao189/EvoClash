//! Server side of in-match evolution: XP awards and level-ups, perk offers and picks,
//! lifesteal, regeneration and armor self-repair. The numbers live in
//! `evoclash_game::evolution`; offers use the server's RNG (not part of the shared sim).

use bevy::prelude::*;
use evoclash_game::evolution::*;
use evoclash_game::protocol::*;
use rand::Rng;

use crate::bots::Bot;
use crate::gameplay::{CurrentInput, Outbox, SimSet, read_human_inputs, resolve_damage};

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
                choose_perks.in_set(SimSet::Input).after(read_human_inputs),
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

/// Server-only pick bookkeeping.
#[derive(Component, Default)]
pub struct Evolving {
    /// `PlayerInput::pick` last tick, for edge detection.
    pub last_pick: u8,
}

/// Ticks since this combatant last took damage.
#[derive(Component, Default)]
pub struct Recovery {
    pub since_hurt: u32,
}

/// Three distinct perks that aren't maxed; perks of the leading path weigh double.
pub fn roll_offer(evo: &Evolution, rng: &mut impl Rng) -> Option<[Perk; 3]> {
    let lead = evo.lead_path();
    let mut pool: Vec<(Perk, u32)> = evo
        .open_perks()
        .map(|p| (p, if Some(p.evo_path()) == lead { 2 } else { 1 }))
        .collect();
    let mut offer = [Perk::ALL[0]; 3];
    for slot in &mut offer {
        let total: u32 = pool.iter().map(|(_, w)| w).sum();
        if total == 0 {
            return None;
        }
        let mut roll = rng.random_range(0..total);
        let i = pool
            .iter()
            .position(|(_, w)| {
                if roll < *w {
                    true
                } else {
                    roll -= w;
                    false
                }
            })
            .unwrap_or(0);
        *slot = pool.swap_remove(i).0;
    }
    Some(offer)
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

/// Which slot to take this tick, if any: a human's key press, a bot's preference after a
/// second, or a random one once the offer has been ignored for 45 s.
fn decide(
    offer: &[Perk; 3],
    age: u16,
    pressed: Option<u8>,
    bot: Option<EvoPath>,
    rng: &mut impl Rng,
) -> Option<usize> {
    if let Some(path) = bot {
        return (age >= BOT_PICK_TICKS)
            .then(|| offer.iter().position(|p| p.evo_path() == path).unwrap_or(0));
    }
    if let Some(key) = pressed {
        return Some(key as usize - 1);
    }
    (age >= AUTO_PICK_TICKS).then(|| rng.random_range(0..3))
}

#[allow(clippy::type_complexity)]
pub(crate) fn choose_perks(
    mut players: Query<(
        &mut Evolution,
        &mut Evolving,
        &mut SkillState,
        &mut Vitals,
        &CurrentInput,
        Option<&Bot>,
    )>,
) {
    let mut rng = rand::rng();
    for (mut evo, mut evolving, mut skills, mut vitals, input, bot) in &mut players {
        let key = input.0.pick;
        let pressed = (key != evolving.last_pick && (1..=3).contains(&key)).then_some(key);
        evolving.last_pick = key;
        if evo.pending == 0 {
            continue;
        }
        let Some(offer) = evo.offer else {
            evo.offer = roll_offer(&evo, &mut rng);
            evo.offer_ticks_left = if evo.offer.is_some() {
                AUTO_PICK_TICKS
            } else {
                0
            };
            continue;
        };
        evo.offer_ticks_left = evo.offer_ticks_left.saturating_sub(1);
        let Some(slot) = decide(
            &offer,
            AUTO_PICK_TICKS - evo.offer_ticks_left,
            pressed,
            bot.map(|b| b.path),
            &mut rng,
        ) else {
            continue;
        };
        if evo.pick(slot).is_some() {
            apply_evolution(&evo, &mut skills, &mut vitals);
        }
    }
}

fn grant_xp(
    mut queue: ResMut<XpQueue>,
    mut outbox: ResMut<Outbox>,
    mut players: Query<(&Player, &mut Evolution, &mut Stats)>,
) {
    if queue.0.is_empty() {
        return;
    }
    let top = players.iter().map(|(_, e, _)| e.level).max().unwrap_or(1);
    for award in queue.0.drain(..) {
        let Ok((player, mut evo, mut stats)) = players.get_mut(award.to) else {
            continue;
        };
        let mut mult = catch_up_mult(top, evo.level);
        if award.loot {
            mult *= evo.loot_xp_mult();
        }
        let amount = scaled_xp(award.base, mult);
        if amount == 0 {
            continue;
        }
        let gained = evo.add_xp(amount);
        outbox.direct.push((
            award.to,
            GameEvent::XpGain {
                amount,
                pos: award.pos,
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
    use super::*;

    #[test]
    fn offers_never_contain_maxed_perks() {
        let mut rng = rand::rng();
        let mut evo = Evolution::default();
        for perk in [Perk::Claws, Perk::LightFoot, Perk::Overclock] {
            evo.ranks[perk.index()] = MAX_RANK;
        }
        evo.ranks[Perk::Constitution.index()] = 2;
        for _ in 0..2000 {
            let offer = roll_offer(&evo, &mut rng).expect("enough open perks");
            for perk in offer {
                assert!(evo.rank(perk) < MAX_RANK, "{perk:?} is maxed");
            }
            assert!(offer[0] != offer[1] && offer[1] != offer[2] && offer[0] != offer[2]);
        }
        // Only two open perks left: no offer.
        evo.ranks = [MAX_RANK; 12];
        evo.ranks[0] = 0;
        evo.ranks[1] = 0;
        assert!(roll_offer(&evo, &mut rng).is_none());
    }

    #[test]
    fn leading_path_is_offered_more() {
        let mut rng = rand::rng();
        let mut evo = Evolution::default();
        evo.ranks[Perk::Overclock.index()] = 1;
        evo.first_pick = [0, 1, 0];
        let mut cyborg = 0;
        for _ in 0..3000 {
            let offer = roll_offer(&evo, &mut rng).unwrap();
            cyborg += offer
                .iter()
                .filter(|p| p.evo_path() == EvoPath::Cyborg)
                .count();
        }
        // Uniform would be 1/3 of 9000; double weight gives about half.
        assert!(cyborg > 4000, "{cyborg}");
    }

    fn picker(evo: Evolution) -> (App, Entity) {
        let mut app = App::new();
        app.add_systems(Update, choose_perks);
        let entity = app
            .world_mut()
            .spawn((
                evo,
                Evolving::default(),
                SkillState::default(),
                Vitals::default(),
                CurrentInput::default(),
            ))
            .id();
        (app, entity)
    }

    #[test]
    fn auto_pick_after_45_seconds() {
        let mut evo = Evolution::default();
        evo.add_xp(60);
        let (mut app, entity) = picker(evo);
        app.update(); // the offer appears
        let evo = app.world().get::<Evolution>(entity).unwrap();
        assert!(evo.offer.is_some());
        assert_eq!(evo.offer_ticks_left, AUTO_PICK_TICKS);
        for _ in 0..AUTO_PICK_TICKS - 1 {
            app.update();
        }
        let evo = app.world().get::<Evolution>(entity).unwrap();
        assert_eq!((evo.pending, evo.total_ranks()), (1, 0));
        app.update();
        let evo = app.world().get::<Evolution>(entity).unwrap();
        assert_eq!((evo.pending, evo.total_ranks()), (0, 1));
        assert!(evo.offer.is_none());
    }

    #[test]
    fn key_press_picks_once_and_picks_queue_up() {
        let mut evo = Evolution::default();
        evo.add_xp(150);
        assert_eq!(evo.pending, 2);
        let (mut app, entity) = picker(evo);
        app.update();
        let offered = app.world().get::<Evolution>(entity).unwrap().offer.unwrap();
        app.world_mut()
            .get_mut::<CurrentInput>(entity)
            .unwrap()
            .0
            .pick = 2;
        app.update();
        let evo = app.world().get::<Evolution>(entity).unwrap();
        assert_eq!(evo.rank(offered[1]), 1);
        assert_eq!(evo.pending, 1);
        // Still holding 2: the next offer is shown but not taken.
        app.update();
        app.update();
        let evo = app.world().get::<Evolution>(entity).unwrap();
        assert_eq!((evo.pending, evo.total_ranks()), (1, 1));
        assert!(evo.offer.is_some());
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

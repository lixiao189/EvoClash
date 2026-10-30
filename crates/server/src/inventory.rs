//! The backpack on the server: hotbar selection, moves made in the backpack panel, granting
//! items (`take`), walk-over pickups and the bag a fallen combatant leaves behind.

use std::collections::HashSet;

use bevy::prelude::*;
use evoclash_game::evolution::Evolution;
use evoclash_game::items::{ArmorKind, Item, MAX_EMPS, MELEE_SLOT};
use evoclash_game::protocol::*;
use lightyear::prelude::server::ClientOf;
use lightyear::prelude::*;

use crate::gameplay::{
    BunkerDrop, Cooldowns, CurrentInput, Expires, Outbox, SimSet, grant_bunker_bounty, spawn_pickup,
};
use crate::inbox::Inbox;
use crate::journal::{Injected, Trace};

const PICKUP_RADIUS: f32 = 46.0;
/// Ticks between deliberate (E) pickups, so one press swaps once.
const INTERACT_TICKS: u16 = 20;
/// Whatever is put down to make room lies around this long.
const PUT_DOWN_TTL: u32 = 30 * 64;

pub struct InventoryPlugin;

impl Plugin for InventoryPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            FixedUpdate,
            (select_slots, apply_moves).in_set(SimSet::Simulate),
        );
        app.add_systems(
            FixedUpdate,
            pickups
                .after(crate::combat::step_projectiles)
                .in_set(SimSet::Combat),
        );
    }
}

/// The last `PlayerInput::select` acted on.
#[derive(Component, Default)]
pub struct Selecting(u8);

/// What an `Item::Bag` pickup holds. Server-only: clients just see a bag.
#[derive(Component)]
pub struct BagContents(pub Vec<Item>);

fn select_slots(mut players: Query<(&CurrentInput, &mut Selecting, &mut Loadout)>) {
    for (input, mut selecting, mut loadout) in &mut players {
        let request = input.0.select;
        if request != selecting.0 {
            selecting.0 = request;
            loadout.select(request & 3);
        }
    }
}

/// Rearranging in the backpack panel arrives as reliable messages on the client's link.
pub(crate) fn apply_moves(
    mut links: Query<(Entity, &mut Inbox<InventoryMove>), With<ClientOf>>,
    mut players: Query<(&Player, Option<&ControlledBy>, &mut Loadout)>,
    specs: Query<&crate::spectate::SpectatorSession>,
    mut injected: Option<ResMut<Injected>>,
    mut trace: Option<ResMut<Trace>>,
) {
    if let Some(injected) = injected.as_mut() {
        let moves = std::mem::take(&mut injected.moves);
        for (id, m) in moves {
            if let Some(trace) = trace.as_mut() {
                trace.push(format!("inv {id} {} {}", m.from, m.to));
            }
            if crate::spectate::is_spectator(&specs, id) {
                continue;
            }
            let Some((_, _, mut loadout)) = players
                .iter_mut()
                .find(|(player, ..)| player.client_id == id)
            else {
                continue;
            };
            loadout.move_slots(m.from, m.to);
            loadout.auto_switch();
        }
    }
    for (link, mut inbox) in &mut links {
        let moves = inbox.take();
        if moves.is_empty() {
            continue;
        }
        let Some((player, _, mut loadout)) = players
            .iter_mut()
            .find(|(_, control, _)| control.is_some_and(|c| c.owner == link))
        else {
            continue;
        };
        let id = player.client_id;
        for m in moves {
            if let Some(trace) = trace.as_mut() {
                trace.push(format!("inv {id} {} {}", m.from, m.to));
            }
            loadout.move_slots(m.from, m.to);
        }
        loadout.auto_switch();
    }
}

/// Would a combatant grab this just by walking over it?
pub fn auto_wants(item: &Item, vitals: &Vitals, loadout: &Loadout, evo: &Evolution) -> bool {
    match *item {
        Item::Weapon(kind) => !loadout.owns(kind) && loadout.has_room(),
        Item::Ammo { kind, .. } => loadout.rounds(kind) < evo.ammo_cap(kind),
        Item::Armor { points, .. } => points > vitals.armor_points,
        Item::Emp => loadout.emps < evo.emp_cap(MAX_EMPS),
        Item::Medkit => vitals.hp < vitals.max_hp,
        // Opened item by item in `pickups`.
        Item::Bag => false,
    }
}

/// Grant `item` to a combatant. Everything that enters a backpack goes through here: pickups,
/// death bags, respawn kits and shop purchases. Returns what didn't fit or had to be put down
/// to make room, for the caller to drop.
pub fn take(
    item: Item,
    vitals: &mut Vitals,
    loadout: &mut Loadout,
    evo: &Evolution,
) -> Option<Item> {
    let could_fire = loadout.best_slot() != MELEE_SLOT;
    let rest = match item {
        Item::Weapon(kind) => loadout.add_weapon(kind).map(Item::Weapon),
        Item::Ammo { kind, rounds } => {
            let rounds = loadout.add_ammo(kind, rounds, evo);
            (rounds > 0).then_some(Item::Ammo { kind, rounds })
        }
        Item::Armor { kind, points } => {
            let old = (vitals.armor != ArmorKind::None && vitals.armor_points > 0.0).then_some(
                Item::Armor {
                    kind: vitals.armor,
                    points: vitals.armor_points,
                },
            );
            vitals.armor = kind;
            // 复合装甲 raises the cap and tops fresh armor up to it.
            let bonus = evo.armor_cap(kind) - kind.max_points();
            vitals.armor_points = (points + bonus).min(evo.armor_cap(kind)).max(points);
            old
        }
        Item::Emp => {
            if loadout.emps < evo.emp_cap(MAX_EMPS) {
                loadout.emps += 1;
                None
            } else {
                Some(Item::Emp)
            }
        }
        Item::Medkit => {
            vitals.hp = (vitals.hp + 50.0).min(vitals.max_hp);
            None
        }
        Item::Bag => None,
    };
    // Stuck with the stick for want of a gun that fires: reach for the one that now does.
    if !could_fire && loadout.selected == MELEE_SLOT {
        loadout.selected = loadout.best_slot();
    }
    rest
}

/// Drop what a fallen combatant carried as one bag (or the lone item itself).
pub fn spawn_bag(commands: &mut Commands, mut items: Vec<Item>, pos: Vec2, ttl: u32) {
    match items.len() {
        0 => {}
        1 => {
            spawn_pickup(commands, items.remove(0), pos, Some(ttl));
        }
        _ => {
            let bag = spawn_pickup(commands, Item::Bag, pos, Some(ttl));
            commands.entity(bag).insert(BagContents(items));
        }
    }
}

/// Walking over items takes the ones you'd want; E takes the nearest one regardless, swapping
/// it for the selected gun when the backpack is full. A bag gives up whatever you want from it
/// on contact, and E takes its next item.
fn pickups(
    mut commands: Commands,
    state: Query<&MatchState>,
    mut outbox: ResMut<Outbox>,
    mut players: Query<(
        Entity,
        &Player,
        &Pos,
        &CurrentInput,
        &mut Vitals,
        &mut Loadout,
        &mut Cooldowns,
        &Evolution,
        &mut Stats,
    )>,
    mut items: Query<(
        Entity,
        &Pos,
        &Pickup,
        Option<&mut BagContents>,
        Option<&BunkerDrop>,
    )>,
) {
    let playing = state.iter().any(|s| s.phase == Phase::Playing);
    let mut taken = HashSet::new();
    for (who, player, pos, input, mut vitals, mut loadout, mut cd, evo, mut stats) in &mut players {
        if !vitals.alive {
            continue;
        }
        let mut near: Vec<(f32, Entity)> = items
            .iter()
            .filter(|(e, ..)| !taken.contains(e))
            .map(|(e, p, ..)| (p.0.distance(pos.0), e))
            .filter(|(d, _)| *d <= PICKUP_RADIUS)
            .collect();
        if near.is_empty() {
            continue;
        }
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut forced = input.0.interact && cd.interact == 0;
        let mut put_down = Vec::new();
        for (_, entity) in near {
            let Ok((_, _, pickup, bag, bunker)) = items.get_mut(entity) else {
                continue;
            };
            if let Some(mut bag) = bag {
                let mut kept = Vec::new();
                for item in std::mem::take(&mut bag.0) {
                    let wanted = auto_wants(&item, &vitals, &loadout, evo);
                    if !wanted && !forced {
                        kept.push(item);
                        continue;
                    }
                    if !wanted {
                        forced = false;
                        cd.interact = INTERACT_TICKS;
                    }
                    // Whatever makes room goes into the bag in its place.
                    kept.extend(take(item, &mut vitals, &mut loadout, evo));
                }
                if kept.is_empty() {
                    taken.insert(entity);
                    commands.entity(entity).despawn();
                } else {
                    bag.0 = kept;
                }
                continue;
            }
            let item = pickup.item;
            let wanted = auto_wants(&item, &vitals, &loadout, evo);
            if !wanted && !forced {
                continue;
            }
            let from_bunker = bunker.is_some() && item.tier() >= 4;
            taken.insert(entity);
            commands.entity(entity).despawn();
            put_down.extend(take(item, &mut vitals, &mut loadout, evo));
            if from_bunker {
                grant_bunker_bounty(
                    &mut commands,
                    who,
                    &player.name,
                    &mut stats,
                    playing,
                    &mut outbox,
                );
            }
            if !wanted {
                // One deliberate swap per press.
                forced = false;
                cd.interact = INTERACT_TICKS;
            }
        }
        for item in put_down {
            spawn_pickup(
                &mut commands,
                item,
                pos.0 + Vec2::new(0.0, -18.0),
                Some(PUT_DOWN_TTL),
            );
        }
    }
}

/// Loot dropped by the world: a gun always comes with a box of its ammo beside it.
pub fn spawn_loot(commands: &mut Commands, item: Item, pos: Vec2, ttl: Option<u32>) -> Entity {
    let entity = spawn_pickup(commands, item, pos, ttl);
    if let Some(ammo) = item.matching_ammo() {
        let ammo = spawn_pickup(commands, ammo, pos + Vec2::new(22.0, 10.0), ttl);
        // Next to a loot spot's gun (which never expires), the box still does eventually.
        if ttl.is_none() {
            commands.entity(ammo).insert(Expires(PUT_DOWN_TTL * 2));
        }
    }
    entity
}

#[cfg(test)]
mod tests {
    use evoclash_game::evolution::SkillState;
    use evoclash_game::items::{AmmoKind, ArmorKind, BAG_SLOTS, GUN_SLOTS, Item, WeaponKind};
    use evoclash_game::sim::Arena;

    use super::*;
    use evoclash_game::shop::Wallet;

    use crate::evolution::{Dealt, LifeLog, Recovery, XpQueue};
    use crate::gameplay::{
        Damage, DamageQueue, Outbox, PosHistory, Rewind, SimTick, resolve_damage,
    };

    fn app() -> App {
        let mut app = App::new();
        app.init_resource::<DamageQueue>()
            .init_resource::<Outbox>()
            .init_resource::<XpQueue>()
            .init_resource::<Dealt>()
            .init_resource::<SimTick>()
            .init_resource::<Arena>()
            .insert_resource(crate::rng::SimRng::from_match_seed(1));
        app.world_mut().spawn(MatchState {
            phase: Phase::Playing,
            ..default()
        });
        app
    }

    fn spawn_player(app: &mut App, id: u64, pos: Vec2, loadout: Loadout) -> Entity {
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
                loadout,
                Stats::default(),
                Evolution::default(),
                SkillState::default(),
                Recovery::default(),
                (Wallet::default(), LifeLog::default()),
                (
                    CurrentInput::default(),
                    Cooldowns::default(),
                    PosHistory::default(),
                    Rewind::default(),
                ),
            ))
            .id()
    }

    fn armed(guns: &[WeaponKind], ammo: &[(AmmoKind, u16)]) -> Loadout {
        let evo = Evolution::default();
        let mut loadout = Loadout::default();
        for gun in guns {
            loadout.add_weapon(*gun);
        }
        for (kind, rounds) in ammo {
            loadout.add_ammo(*kind, *rounds, &evo);
        }
        loadout.selected = loadout.best_slot();
        loadout
    }

    fn pickups_on_ground(app: &mut App) -> Vec<Item> {
        let world = app.world_mut();
        world
            .query::<&Pickup>()
            .iter(world)
            .map(|p| p.item)
            .collect()
    }

    fn fire(app: &mut App, who: Entity) {
        let world = app.world_mut();
        world.get_mut::<CurrentInput>(who).unwrap().0.fire = true;
        world.get_mut::<Cooldowns>(who).unwrap().fire = 0;
        app.update();
    }

    #[test]
    fn a_dry_gun_is_kept_and_the_best_gun_with_ammo_comes_out() {
        let mut app = app();
        app.add_systems(Update, crate::combat::fire_weapons);
        let loadout = armed(
            &[WeaponKind::Bow, WeaponKind::Rifle],
            &[(AmmoKind::Bullet, 2), (AmmoKind::Arrow, 1)],
        );
        let p = spawn_player(&mut app, 1, Vec2::ZERO, loadout);
        assert_eq!(app.world().get::<Loadout>(p).unwrap().selected, 1);
        fire(&mut app, p);
        fire(&mut app, p);
        let loadout = app.world().get::<Loadout>(p).unwrap();
        assert_eq!(loadout.rounds(AmmoKind::Bullet), 0);
        assert_eq!(
            loadout.guns,
            [Some(WeaponKind::Bow), Some(WeaponKind::Rifle)]
        );
        assert_eq!(loadout.selected, 0, "switched to the bow");
        fire(&mut app, p);
        let loadout = app.world().get::<Loadout>(p).unwrap();
        assert_eq!(loadout.rounds(AmmoKind::Arrow), 0);
        assert_eq!(loadout.selected, MELEE_SLOT, "nothing left but the stick");
        assert_eq!(loadout.weapon(), WeaponKind::Stick);
    }

    #[test]
    fn picking_up_rounds_brings_out_a_dry_gun() {
        let mut app = app();
        app.add_systems(Update, pickups);
        let p = spawn_player(&mut app, 1, Vec2::ZERO, armed(&[WeaponKind::Rifle], &[]));
        assert_eq!(
            app.world().get::<Loadout>(p).unwrap().weapon(),
            WeaponKind::Stick
        );
        let mut commands = app.world_mut().commands();
        spawn_pickup(&mut commands, Item::ammo(AmmoKind::Arrow), Vec2::X, None);
        spawn_pickup(&mut commands, Item::ammo(AmmoKind::Bullet), Vec2::Y, None);
        app.update();
        let loadout = app.world().get::<Loadout>(p).unwrap();
        assert_eq!(
            loadout.rounds(AmmoKind::Arrow),
            AmmoKind::Arrow.box_rounds()
        );
        assert_eq!(loadout.weapon(), WeaponKind::Rifle);
        assert!(pickups_on_ground(&mut app).is_empty());
    }

    #[test]
    fn a_full_backpack_only_swaps_on_e() {
        let mut app = app();
        app.add_systems(Update, pickups);
        let mut loadout = armed(&[WeaponKind::Bow, WeaponKind::Rifle], &[]);
        loadout.bag = [Some(WeaponKind::Laser); BAG_SLOTS];
        loadout.selected = 1;
        assert_eq!(loadout.guns.len(), GUN_SLOTS);
        let p = spawn_player(&mut app, 1, Vec2::ZERO, loadout);
        let mut commands = app.world_mut().commands();
        spawn_pickup(
            &mut commands,
            Item::weapon(WeaponKind::Railgun),
            Vec2::X,
            None,
        );
        app.update();
        assert_eq!(
            pickups_on_ground(&mut app),
            [Item::weapon(WeaponKind::Railgun)],
            "walking over it leaves it"
        );
        app.world_mut()
            .get_mut::<CurrentInput>(p)
            .unwrap()
            .0
            .interact = true;
        app.update();
        let loadout = app.world().get::<Loadout>(p).unwrap();
        assert_eq!(loadout.guns[1], Some(WeaponKind::Railgun));
        assert_eq!(
            pickups_on_ground(&mut app),
            [Item::weapon(WeaponKind::Rifle)],
            "the selected gun is put down"
        );
    }

    /// A held seat is out of the map: it walks over a gun without taking it.
    #[test]
    fn a_withdrawn_body_picks_nothing_up() {
        let mut app = app();
        app.add_systems(Update, pickups);
        let p = spawn_player(&mut app, 1, Vec2::ZERO, Loadout::default());
        {
            let mut vitals = app.world_mut().get_mut::<Vitals>(p).unwrap();
            // What `gameplay::seats` does to a dropped body.
            vitals.alive = false;
        }
        let mut commands = app.world_mut().commands();
        spawn_pickup(
            &mut commands,
            Item::weapon(WeaponKind::Railgun),
            Vec2::X,
            None,
        );
        app.update();
        assert_eq!(
            pickups_on_ground(&mut app),
            [Item::weapon(WeaponKind::Railgun)],
            "a withdrawn body takes nothing"
        );
    }

    #[test]
    fn the_fallen_drop_one_bag_and_the_killer_salvages_ammo() {
        let mut app = app();
        app.add_systems(Update, resolve_damage);
        let mut victim_kit = armed(
            &[WeaponKind::Laser, WeaponKind::Rifle, WeaponKind::Bow],
            &[(AmmoKind::Cell, 20), (AmmoKind::Bullet, 30)],
        );
        victim_kit.emps = 2;
        let victim = spawn_player(&mut app, 1, Vec2::ZERO, victim_kit);
        let killer = spawn_player(
            &mut app,
            2,
            Vec2::new(300.0, 0.0),
            armed(&[WeaponKind::Rifle], &[(AmmoKind::Bullet, 1)]),
        );
        app.world_mut()
            .resource_mut::<DamageQueue>()
            .0
            .push(Damage {
                target: victim,
                attacker: Some(killer),
                amount: 1000.0,
                armor_mult: 1.0,
                weapon: Some(WeaponKind::Rifle),
                emp: false,
                cause: None,
            });
        app.update();

        assert_eq!(
            app.world().get::<Loadout>(victim).unwrap(),
            &Loadout::default()
        );
        assert_eq!(
            app.world()
                .get::<Loadout>(killer)
                .unwrap()
                .rounds(AmmoKind::Bullet),
            1 + AmmoKind::Bullet.box_rounds() / 4
        );
        let world = app.world_mut();
        let bags: Vec<(Item, Vec<Item>)> = world
            .query::<(&Pickup, &BagContents)>()
            .iter(world)
            .map(|(p, b)| (p.item, b.0.clone()))
            .collect();
        assert_eq!(bags.len(), 1, "one bag, not a pile");
        assert_eq!(pickups_on_ground(&mut app).len(), 1);
        let (item, contents) = &bags[0];
        assert_eq!(*item, Item::Bag);
        assert_eq!(contents.len(), 3 + 2 + 2, "guns, ammo stacks and EMPs");

        // The killer walks over it and takes everything that fits.
        let mut app2 = self::app();
        app2.add_systems(Update, pickups);
        let taker = spawn_player(&mut app2, 3, Vec2::ZERO, Loadout::default());
        let mut commands = app2.world_mut().commands();
        spawn_bag(&mut commands, contents.clone(), Vec2::X, 100);
        app2.update();
        let loadout = app2.world().get::<Loadout>(taker).unwrap();
        assert_eq!(loadout.guns().count(), 3);
        assert_eq!(loadout.rounds(AmmoKind::Cell), 20);
        assert_eq!(loadout.emps, 2);
        assert!(loadout.weapon().ranged().is_some(), "a gun comes out");
        assert!(
            pickups_on_ground(&mut app2).is_empty(),
            "the bag is emptied"
        );
    }

    #[test]
    fn walking_over_bunker_t4_forces_a_bounty_and_a_loose_t4_does_not() {
        use crate::gameplay::BunkerBounty;

        let mut app = app();
        app.add_systems(Update, pickups);
        let who = spawn_player(&mut app, 1, Vec2::ZERO, Loadout::default());
        {
            let mut commands = app.world_mut().commands();
            let dropped = spawn_pickup(
                &mut commands,
                Item::armor(ArmorKind::PowerArmor),
                Vec2::ZERO,
                None,
            );
            commands.entity(dropped).insert(BunkerDrop);
        }
        app.update();
        let stats = app.world().get::<Stats>(who).unwrap();
        assert_eq!(stats.streak, 3);
        assert!(app.world().get::<BunkerBounty>(who).is_some());
        assert_eq!(
            app.world().get::<Vitals>(who).unwrap().armor,
            ArmorKind::PowerArmor
        );

        let mut plain = self::app();
        plain.add_systems(Update, pickups);
        let who = spawn_player(&mut plain, 1, Vec2::ZERO, Loadout::default());
        {
            let mut commands = plain.world_mut().commands();
            spawn_pickup(
                &mut commands,
                Item::armor(ArmorKind::PowerArmor),
                Vec2::ZERO,
                None,
            );
        }
        plain.update();
        assert_eq!(plain.world().get::<Stats>(who).unwrap().streak, 0);
        assert!(plain.world().get::<BunkerBounty>(who).is_none());
    }
}

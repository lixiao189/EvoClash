use super::*;

/// Keep the two best guns that can fire on the hotbar and hold the best of them. Bots skip the
/// backpack panel and rearrange directly.
pub(super) fn arrange(loadout: &mut Loadout) {
    let rank = |l: &Loadout, w: Option<WeaponKind>| {
        w.map_or(0, |w| u8::from(l.usable(w)) * 10 + w.spec().tier() + 1)
    };
    for hot in 0..GUN_SLOTS as u8 {
        let best = (HOTBAR_SLOTS..evoclash_game::items::SLOT_COUNT)
            .max_by_key(|&id| rank(loadout, loadout.slot(id)))
            .expect("bag slots");
        if rank(loadout, loadout.slot(best)) > rank(loadout, loadout.slot(hot)) {
            loadout.move_slots(best, hot);
        }
    }
    let best = loadout.best_slot();
    if loadout.selected != best {
        loadout.selected = best;
    }
}

/// Where each path's nine points go, in order: skill A by the 2nd point, skill B by the 5th.
pub fn build_order(path: EvoPath) -> [Perk; 9] {
    use Perk::*;
    match path {
        EvoPath::Mutant => [
            Constitution,
            Claws,
            Regeneration,
            Bloodthirst,
            Constitution,
            Claws,
            Bloodthirst,
            Constitution,
            Regeneration,
        ],
        EvoPath::Cyborg => [
            Overclock,
            CompositeArmor,
            SelfRepair,
            Overclock,
            CompositeArmor,
            Overclock,
            SelfRepair,
            CompositeArmor,
            EnergyFocus,
        ],
        EvoPath::Scavenger => [
            LightFoot,
            AmmoRecycler,
            Lockpick,
            LightFoot,
            EmpExpert,
            AmmoRecycler,
            LightFoot,
            AmmoRecycler,
            EmpExpert,
        ],
    }
}

/// The next perk `path`'s build order still wants.
pub(super) fn next_perk(evo: &Evolution, path: EvoPath) -> Option<Perk> {
    let mut want = [0u8; 12];
    build_order(path).into_iter().find(|perk| {
        want[perk.index()] += 1;
        evo.rank(*perk) < want[perk.index()]
    })
}

/// Shop picks, best first.
pub(super) const GUNS: [ShopItem; 3] = [ShopItem::Rifle, ShopItem::PipeShotgun, ShopItem::Bow];
pub(super) const ARMORS: [ShopItem; 2] = [ShopItem::Kevlar, ShopItem::ScrapArmor];

/// A bot's death prep, through the same commands a human sends: spend points on the build,
/// and once it may deploy, buy the best gun, armor and EMPs it can afford and go.
/// Returns true if the perks changed.
pub fn prep_bot(
    path: EvoPath,
    evo: &mut Evolution,
    wallet: &mut Wallet,
    vitals: &mut Vitals,
    life: &mut LifeLog,
) -> bool {
    let mut changed = false;
    while let Some(perk) = next_perk(evo, path) {
        if !handle_command(&PlayerCommand::Spend(perk), evo, wallet, vitals, life, true) {
            break;
        }
        changed = true;
    }
    if vitals.alive || !can_deploy(vitals.respawn_in) {
        return changed;
    }
    for group in [&GUNS[..], &ARMORS[..]] {
        if let Some(item) = group
            .iter()
            .copied()
            .find(|item| wallet.can_buy(*item, evo))
        {
            handle_command(&PlayerCommand::Buy(item), evo, wallet, vitals, life, true);
        }
    }
    while wallet.can_buy(ShopItem::Emp, evo) {
        handle_command(
            &PlayerCommand::Buy(ShopItem::Emp),
            evo,
            wallet,
            vitals,
            life,
            true,
        );
    }
    handle_command(&PlayerCommand::Deploy, evo, wallet, vitals, life, true);
    changed
}

pub(super) fn prepare(
    state: Query<&MatchState>,
    mut bots: Query<(
        &Bot,
        &mut Evolution,
        &mut Wallet,
        &mut Vitals,
        &mut SkillState,
        &mut LifeLog,
    )>,
) {
    // Death prep (shop, deploy) is a Playing rule. Lobby respawns on its own 2s timer.
    if !state.iter().any(|s| s.phase == Phase::Playing) {
        return;
    }
    for (bot, mut evo, mut wallet, mut vitals, mut skills, mut life) in &mut bots {
        if prep_bot(bot.path, &mut evo, &mut wallet, &mut vitals, &mut life) {
            apply_evolution(&evo, &mut skills, &mut vitals);
        }
    }
}

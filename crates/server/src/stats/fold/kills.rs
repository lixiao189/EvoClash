use evoclash_game::items::WeaponKind;
use evoclash_game::shop::player_kill_scrap;

use super::super::{Note, PvpHit};
use super::*;

impl Fold {
    pub(super) fn apply_hits(&mut self, stamp: u32, hits: &[PvpHit]) -> Vec<u64> {
        let mut combat = Vec::new();
        for hit in hits {
            let slot = self
                .fights
                .entry((hit.attacker, hit.victim))
                .or_insert((stamp, stamp));
            if stamp.saturating_sub(slot.1) > FIGHT_GAP {
                slot.0 = stamp;
            }
            slot.1 = stamp;
            combat.push(hit.attacker);
            combat.push(hit.victim);
        }
        combat.sort_unstable();
        combat.dedup();
        combat
    }

    pub(super) fn note_quiet(&mut self, stamp: u32, combat: &[u64]) {
        for id in combat {
            if let Some(life) = self.open_life.get_mut(id) {
                let gap = stamp.saturating_sub(life.last_combat);
                life.quiet = life.quiet.max(gap);
                life.last_combat = stamp;
            }
        }
    }

    pub(super) fn apply_notes(&mut self, stamp: u32, notes: &[Note]) {
        for note in notes {
            match note {
                Note::Announce(text) if text.contains("地堡开启") => {
                    self.vaults.push(stamp);
                    self.last_vault = Some(stamp);
                }
                Note::Boom { pos, radius, fire } if !fire && *radius >= 250.0 => {
                    self.boss_boom = Some(*pos);
                }
                Note::BossDown { killer } => {
                    self.boss_end_tick = Some(stamp);
                    match killer {
                        Some(name) => {
                            self.boss_killed = true;
                            let max_score =
                                self.roster.values().map(|b| b.score).max().unwrap_or(0);
                            let theirs = self
                                .roster
                                .values()
                                .find(|b| b.name == *name)
                                .map(|b| b.score)
                                .unwrap_or(0);
                            self.boss_top_not_leader = Some(theirs < max_score);
                        }
                        None => self.boss_left = true,
                    }
                }
                Note::Kill {
                    killer,
                    victim,
                    how,
                    bounty,
                } => self.on_kill(stamp, killer, victim, how, *bounty),
                _ => {}
            }
        }
    }

    fn on_kill(&mut self, stamp: u32, killer: &str, victim: &str, how: &str, bounty: bool) {
        let victim_id = self.id_named(victim);
        let killer_id = self.id_named(killer);
        let victim_gear = victim_id
            .and_then(|id| self.roster.get(&id))
            .map(|b| b.gear)
            .unwrap_or(0);
        let (weapon, killer_tier) = weapon_of(how).map_or_else(
            || {
                let tier = killer_id
                    .and_then(|id| self.roster.get(&id))
                    .map(|b| b.weapon.spec().tier());
                (None, tier)
            },
            |w| (Some(w.spec().name().to_string()), Some(w.spec().tier())),
        );
        let underdog = killer_id.is_some()
            && killer_tier.is_some_and(|t| victim_gear >= t.saturating_add(GAP));
        let sourced = killer_id
            .and_then(|id| self.roster.get(&id))
            .is_some_and(|b| b.sourced);
        let cause = classify(killer, how, killer_id.is_some());
        if killer_id.is_some() {
            self.scrap_player += player_kill_scrap(bounty);
        }
        if let Some(id) = killer_id
            && let Some((start, _)) = self.fights.remove(&(id, victim_id.unwrap_or(id)))
        {
            self.fight_ticks.push(stamp.saturating_sub(start));
        }
        if let Some(id) = victim_id {
            if bounty {
                let start = self.bounty_at.remove(&id).unwrap_or(stamp);
                self.bounties += 1;
                if stamp.saturating_sub(start) <= 60 * TICKS_PER_SEC {
                    self.bounties_killed_60s += 1;
                }
            }
            self.close_life(id, stamp);
            self.take_autopsy(id, stamp, how);
        }
        self.kills.push(KillRec {
            tick: stamp,
            killer_id,
            victim_id: victim_id.unwrap_or(0),
            how: how.to_string(),
            cause: cause.to_string(),
            weapon,
            killer_tier,
            victim_gear,
            underdog,
            bounty,
            sourced,
        });
    }

    fn id_named(&self, name: &str) -> Option<u64> {
        self.roster
            .iter()
            .find(|(_, b)| b.name == name)
            .map(|(id, _)| *id)
    }
}

fn weapon_of(how: &str) -> Option<WeaponKind> {
    Some(match how {
        "木棍" => WeaponKind::Stick,
        "猎弓" => WeaponKind::Bow,
        "管式霰弹枪" => WeaponKind::PipeShotgun,
        "制式步枪" => WeaponKind::Rifle,
        "脉冲激光" => WeaponKind::Laser,
        "轨道炮" => WeaponKind::Railgun,
        _ => return None,
    })
}

fn classify(killer: &str, how: &str, killer_is_player: bool) -> &'static str {
    if how == "油桶爆炸" || (how == "辐射" && !killer.contains("辐射巨兽")) {
        return "environment";
    }
    if killer.contains("辐射巨兽") || how == "重踏" || how == "冲撞" {
        return "boss";
    }
    if killer_is_player {
        return "player";
    }
    if killer == "废土" {
        return "environment";
    }
    "monster"
}

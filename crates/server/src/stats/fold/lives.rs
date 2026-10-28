use evoclash_game::evolution::Evolution;
use evoclash_game::protocol::*;
use evoclash_game::sim::gear_tier;

use crate::bots::Bot;
use crate::evolution::LifeLog;

use super::*;

impl Fold {
    pub(super) fn close_life(&mut self, id: u64, end: u32) {
        let Some(mut life) = self.open_life.remove(&id) else {
            return;
        };
        life.quiet = life.quiet.max(end.saturating_sub(life.last_combat));
        self.lives.push(LifeRec {
            client_id: id,
            start: life.start,
            end,
            opens: life.opens,
            first_t1: life.first_t1.map(|t| t.saturating_sub(life.start)),
            first_t2: life.first_t2.map(|t| t.saturating_sub(life.start)),
            quiet_ticks: life.quiet,
            scrap: life.scrap,
        });
    }

    pub(super) fn open_for(&mut self, id: u64, stamp: u32) {
        self.open_life.entry(id).or_insert(OpenLife {
            start: stamp,
            opens: 0,
            first_t1: None,
            first_t2: None,
            last_combat: stamp,
            quiet: 0,
            scrap: 0,
        });
    }

    pub(super) fn store_bodies(&mut self, bodies: Vec<Sample>) {
        self.roster = bodies
            .into_iter()
            .map(|b| {
                (
                    b.id,
                    Body {
                        name: b.name,
                        alive: b.alive,
                        gear: b.gear,
                        weapon: b.weapon,
                        selected: b.selected,
                        holds_t4: b.holds_t4,
                        score: b.score,
                        kills: b.kills,
                        deaths: b.deaths,
                        last_score_tick: b.last_score_tick,
                        level: b.level,
                        streak: b.streak,
                        skill: b.skill,
                        path: b.path,
                    },
                )
            })
            .collect();
    }
}

pub(super) fn read_bodies(world: &mut World) -> Vec<Sample> {
    let mut q = world.query::<(
        &Player,
        &Pos,
        &Vitals,
        &Loadout,
        &Stats,
        &Evolution,
        &LifeLog,
        Option<&Bot>,
    )>();
    let mut out: Vec<Sample> = q
        .iter(world)
        .map(|(p, pos, vitals, loadout, stats, evo, life, bot)| {
            let gear = gear_tier(vitals, loadout);
            let weapon = loadout.weapon();
            let holds_t4 =
                loadout.best_tier() >= 4 || (vitals.armor.tier() >= 4 && vitals.armor_points > 0.0);
            Sample {
                id: p.client_id,
                name: p.name.clone(),
                pos: pos.0,
                alive: vitals.alive,
                gear,
                weapon,
                selected: loadout.selected,
                slots: carried(loadout),
                holds_t4,
                score: stats.score,
                kills: stats.kills,
                deaths: stats.deaths,
                last_score_tick: stats.last_score_tick,
                level: evo.level,
                streak: stats.streak,
                life_scrap: life.scrap,
                skill: bot
                    .map(|b| b.skill.as_str().to_string())
                    .unwrap_or_else(|| "default".into()),
                path: bot.map(|b| b.path.name().to_string()).unwrap_or_else(|| {
                    evo.lead_path()
                        .map(|p| p.name().to_string())
                        .unwrap_or_default()
                }),
            }
        })
        .collect();
    out.sort_by_key(|s| s.id);
    out
}

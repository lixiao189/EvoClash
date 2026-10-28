use evoclash_game::protocol::*;

use super::*;

impl Fold {
    pub(super) fn diff_mons(&mut self, stamp: u32, mons: HashMap<Entity, (bool, f32)>) {
        let mut next = HashMap::new();
        for (e, (elite, hp)) in &mons {
            if let Some(prev) = self.mons.get(e) {
                let mut first = prev.first;
                if *hp < prev.hp && first.is_none() {
                    first = Some(stamp);
                }
                next.insert(
                    *e,
                    Mon {
                        elite: *elite,
                        hp: *hp,
                        first,
                    },
                );
            } else {
                next.insert(
                    *e,
                    Mon {
                        elite: *elite,
                        hp: *hp,
                        first: None,
                    },
                );
            }
        }
        for (e, prev) in &self.mons {
            if mons.contains_key(e) {
                continue;
            }
            let dt = stamp.saturating_sub(prev.first.unwrap_or(stamp));
            if prev.elite {
                self.elite_ttk.push(dt);
            } else {
                self.monster_ttk.push(dt);
            }
        }
        self.mons = next;
    }

    pub(super) fn note_boss(&mut self, stamp: u32, stage: Stage) {
        if stage == Stage::Alive && self.prev_stage != Stage::Alive {
            self.boss_appeared = true;
            self.boss_appear_tick = Some(stamp);
        }
        if stage == Stage::Done && self.prev_stage == Stage::Alive && self.boss_end_tick.is_none() {
            self.boss_end_tick = Some(stamp);
            self.boss_left = true;
        }
    }
}

pub(super) fn read_mons(world: &mut World) -> HashMap<Entity, (bool, f32)> {
    let mut q = world.query::<(Entity, &Monster, &Vitals)>();
    q.iter(world)
        .filter(|(_, m, _)| m.kind != MonsterKind::Behemoth)
        .map(|(e, m, v)| (e, (m.elite, v.hp)))
        .collect()
}

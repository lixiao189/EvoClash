use super::*;

impl Fold {
    pub(super) fn diff_bodies(
        &mut self,
        stamp: u32,
        bodies: &[Sample],
        inner: impl Fn(Vec2) -> bool,
    ) {
        let mut snap = Vec::new();
        for b in bodies {
            let prev_alive = self.roster.get(&b.id).map(|p| p.alive);
            let was_t4 = self.roster.get(&b.id).is_some_and(|p| p.holds_t4);
            let was_bounty = self
                .roster
                .get(&b.id)
                .is_some_and(|p| p.streak >= evoclash_game::sim::BOUNTY_STREAK);
            if b.alive && prev_alive != Some(true) {
                self.open_for(b.id, stamp);
                self.births += 1;
                if b.weapon != WeaponKind::Stick {
                    self.births_armed += 1;
                }
            }
            if let Some(life) = self.open_life.get_mut(&b.id) {
                if b.gear >= 1 && life.first_t1.is_none() {
                    life.first_t1 = Some(stamp);
                }
                if b.gear >= 2 && life.first_t2.is_none() {
                    life.first_t2 = Some(stamp);
                }
                if b.life_scrap > life.scrap {
                    let gain = b.life_scrap - life.scrap;
                    life.scrap = b.life_scrap;
                    self.scrap_earned += gain;
                }
            }
            if b.holds_t4 && !was_t4 {
                self.holds.insert(b.id, stamp);
                self.t4_gains.push(stamp);
                self.had_t4.insert(b.id, true);
            } else if was_t4
                && !b.holds_t4
                && let Some(start) = self.holds.remove(&b.id)
            {
                self.t4_hold_ticks.push(stamp.saturating_sub(start));
            }
            if b.holds_t4 {
                self.had_t4.insert(b.id, true);
            }
            if b.streak >= evoclash_game::sim::BOUNTY_STREAK && !was_bounty {
                self.bounty_at.insert(b.id, stamp);
            }
            self.max_streak = self.max_streak.max(b.streak);
            if b.alive && stamp >= FINALE_AT {
                self.finale_alive += 1;
                if inner(b.pos) {
                    self.finale_inner += 1;
                }
            }
            snap.push(SnapRow {
                client_id: b.id,
                score: b.score,
                kills: b.kills,
                deaths: b.deaths,
                last_score_tick: b.last_score_tick,
                gear: b.gear,
                level: b.level,
            });
        }
        snap.sort_by_key(|r| r.client_id);
        if stamp == SNAP_2 && self.snap_2m.is_none() {
            self.snap_2m = Some(snap.clone());
        }
        if stamp == SNAP_5 && self.snap_5m.is_none() {
            self.snap_5m = Some(snap.clone());
        }
        if stamp == FINALE_AT && self.snap_6m.is_none() {
            self.snap_6m = Some(snap);
        }
    }
}

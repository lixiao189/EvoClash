//! Evolution-UI probes. Logs only when a scoped call, or the click-to-replication wait, exceeds 1ms.
//!
//! `std::time::Instant::now` panics on `wasm32-unknown-unknown`. Bevy's clock is that type
//! natively and `performance.now` in the browser.

use std::time::Duration;

use bevy::prelude::*;

pub use bevy::platform::time::Instant;

const SLOW: Duration = Duration::from_millis(1);

/// Click time of Spend / Refund / ResetPerks, cleared when `update_tree` applies `Evolution`.
#[derive(Resource, Default)]
pub struct EvoSelectSent(pub Option<Instant>);

/// On drop, warns `evo-ui {name} {us}us` if the scope took longer than 1ms.
#[must_use]
pub struct SlowWarn {
    name: &'static str,
    start: Instant,
}

impl SlowWarn {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            start: Instant::now(),
        }
    }
}

impl Drop for SlowWarn {
    fn drop(&mut self) {
        let elapsed = self.start.elapsed();
        if elapsed > SLOW {
            let us = elapsed.as_micros();
            warn!("evo-ui {} {us}us", self.name);
        }
    }
}

/// Wait from the perk click until the replicated `Evolution` is applied. Clears the mark either way.
pub fn take_select_rtt(sent: &mut EvoSelectSent) {
    let Some(start) = sent.0.take() else {
        return;
    };
    let elapsed = start.elapsed();
    if elapsed > SLOW {
        let us = elapsed.as_micros();
        warn!("evo-ui select-rtt {us}us");
    }
}

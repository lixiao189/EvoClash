//! The scaling policy: pure functions, no cluster, no registry.
//!
//! Ordinals are always compared as numbers: a StatefulSet pod is `<name>-<ordinal>`, and the
//! scaler deletes the highest ordinals first, so `server-10` is "above" `server-2`.

use std::collections::BTreeSet;

use super::ScaleConfig;

/// One match as the scaler sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Slot {
    pub(super) id: String,
    /// `None` when the match id does not belong to a pod of this StatefulSet.
    pub(super) ordinal: Option<u32>,
    /// A human, a watcher or a live seat reservation is in it. Bots alone are not people.
    pub(super) in_use: bool,
    /// Already told to take no new players (it is on the way out).
    pub(super) draining: bool,
}

/// What the scaler should do this tick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Plan {
    /// Leave the StatefulSet alone.
    Hold,
    /// Mark these matches draining first; the resize happens on a later tick, once they are out
    /// of the registry's joinable set.
    Drain(Vec<String>),
    /// PATCH `spec.replicas`.
    Resize(u32),
}

/// Match ids are `<pod>-<start suffix>` and pod names are `<statefulset>-<ordinal>`, so the
/// digits right after the StatefulSet name are the ordinal, whatever the suffix looks like.
/// `pub(crate)`: the registry orders lobby picking by ordinal too.
pub(crate) fn ordinal_of(match_id: &str, statefulset: &str) -> Option<u32> {
    let rest = match_id.strip_prefix(statefulset)?.strip_prefix('-')?;
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let after = &rest[digits..];
    if !(after.is_empty() || after.starts_with('-')) {
        return None;
    }
    rest.get(..digits)?.parse().ok()
}

/// `matches with people in them + buffer`, clamped to `[min, max]`.
pub(super) fn target_replicas(in_use: u32, buffer: u32, min: u32, max: u32) -> u32 {
    in_use.saturating_add(buffer).clamp(min, max.max(min))
}

/// Decide one tick. `allow_shrink` is false right after this replica took over.
pub(super) fn plan(current: u32, cfg: &ScaleConfig, allow_shrink: bool, slots: &[Slot]) -> Plan {
    let occupied: BTreeSet<u32> = slots
        .iter()
        .filter(|slot| slot.in_use)
        .filter_map(|slot| slot.ordinal)
        .collect();
    let target = target_replicas(occupied.len() as u32, cfg.buffer, cfg.min, cfg.max);
    if target > current {
        // Always safe: the new pods start empty matches.
        return Plan::Resize(target);
    }
    if target == current || !allow_shrink {
        return Plan::Hold;
    }
    // A match that is not on a pod of this StatefulSet cannot be reasoned about: never shrink
    // while one might still be running elsewhere.
    if slots
        .iter()
        .any(|slot| slot.in_use && slot.ordinal.is_none())
    {
        return Plan::Hold;
    }
    // The StatefulSet deletes the highest ordinals first, so we may not go below the highest
    // ordinal that still has someone in it (a `BTreeSet<u32>` compares ordinals as numbers).
    let floor = occupied
        .iter()
        .next_back()
        .map(|last| last + 1)
        .unwrap_or_default()
        .max(cfg.min)
        .min(cfg.max);
    let wanted = target.max(floor);
    if wanted >= current {
        return Plan::Hold;
    }
    let going: Vec<&Slot> = slots
        .iter()
        .filter(|slot| slot.ordinal.is_some_and(|o| o >= wanted && o < current))
        .collect();
    // The floor already rules this out; keep the guard so a stale ordinal can never cost a
    // player their match.
    if going.iter().any(|slot| slot.in_use) {
        return Plan::Hold;
    }
    let drain: Vec<String> = going
        .iter()
        .filter(|slot| !slot.draining)
        .map(|slot| slot.id.clone())
        .collect();
    if !drain.is_empty() {
        return Plan::Drain(drain);
    }
    Plan::Resize(wanted)
}

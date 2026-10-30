//! Policy, ordinal and timestamp tests. Nothing here touches a cluster.

use super::plan::{Plan, Slot, ordinal_of, plan, target_replicas};
use super::time::{micro_time, parse_rfc3339};
use super::*;

use evoclash_game::api::Heartbeat;

use crate::registry::Registry;

fn cfg() -> ScaleConfig {
    ScaleConfig {
        statefulset: "evoclash-server".into(),
        namespace: "evoclash".into(),
        buffer: 3,
        min: 3,
        max: 300,
    }
}

fn slot(id: &str, in_use: bool) -> Slot {
    Slot {
        id: id.into(),
        ordinal: ordinal_of(id, "evoclash-server"),
        in_use,
        draining: false,
    }
}

fn draining(mut slot: Slot) -> Slot {
    slot.draining = true;
    slot
}

/// The scaler's view of a registry, with the StatefulSet it belongs to.
fn slots_of(registry: &mut Registry) -> Vec<Slot> {
    registry
        .scaler_slots()
        .into_iter()
        .map(|view| Slot {
            ordinal: ordinal_of(&view.id, "evoclash-server"),
            id: view.id,
            in_use: view.in_use,
            draining: view.draining,
        })
        .collect()
}

fn heartbeat(match_id: &str, humans: u32) -> Heartbeat {
    Heartbeat {
        match_id: match_id.into(),
        public_addr: "10.0.0.7:7000".into(),
        cert_digest: "00".repeat(32),
        players: humans,
        max_players: 8,
        accepting: true,
        tick_us: 100,
        phase: "lobby".into(),
        phase_left_secs: None,
        humans,
        bots: 8 - humans,
        spectators: 0,
        is_private: false,
        room_code: None,
        room_seed: 7,
        match_index: 0,
        join_mode: "lobby".into(),
        exiting: false,
    }
}

#[test]
fn reads_the_ordinal_from_the_match_id() {
    let sts = "evoclash-server";
    assert_eq!(ordinal_of("evoclash-server-0-00ab12", sts), Some(0));
    assert_eq!(ordinal_of("evoclash-server-12-00ab12", sts), Some(12));
    assert_eq!(ordinal_of("evoclash-server-7", sts), Some(7));
    // Another StatefulSet's pods, and ids that are not pod names at all.
    assert_eq!(ordinal_of("other-server-3-abcdef", sts), None);
    assert_eq!(ordinal_of("evoclash-server-abc", sts), None);
    assert_eq!(ordinal_of("evoclash-server", sts), None);
    assert_eq!(ordinal_of("local", sts), None);
    // `-1` must not read as ordinal 1 of another prefix.
    assert_eq!(ordinal_of("evoclash-server-x1-abcdef", sts), None);
}

#[test]
fn ordinals_above_ten_are_compared_as_numbers() {
    let sts = "evoclash-server";
    assert_eq!(ordinal_of("evoclash-server-10-00ab12", sts), Some(10));
    assert_eq!(ordinal_of("evoclash-server-100-00ab12", sts), Some(100));
    // Numeric order, not the string order that would put `-10` before `-2`.
    let slots = [
        slot("evoclash-server-10-aaaaaa", false),
        slot("evoclash-server-2-bbbbbb", false),
    ];
    let mut cfg = cfg();
    cfg.buffer = 0;
    cfg.min = 1;
    // Highest busy ordinal is 10, so the floor is 11 and nothing shrinks from 11.
    let mut busy = slots.clone();
    busy[0].in_use = true;
    assert_eq!(plan(11, &cfg, true, &busy), Plan::Hold);
    // With 10 free, the drain list stops at the highest free ordinal, 10, not at 2.
    assert_eq!(
        plan(
            11,
            &cfg,
            true,
            &[draining(slots[0].clone()), draining(slots[1].clone())]
        ),
        Plan::Resize(1)
    );
}

#[test]
fn the_registry_reports_slots_in_ordinal_order() {
    let mut registry = Registry::default();
    registry.set_statefulset("evoclash-server");
    // Heartbeats arrive in whatever order; the scaler's drain list must run low to high.
    for id in [
        "evoclash-server-10-aaaaaa",
        "evoclash-server-2-bbbbbb",
        "evoclash-server-1-cccccc",
    ] {
        registry.heartbeat(&heartbeat(id, 0));
    }
    let ids: Vec<String> = registry.scaler_slots().into_iter().map(|v| v.id).collect();
    assert_eq!(
        ids,
        [
            "evoclash-server-1-cccccc",
            "evoclash-server-2-bbbbbb",
            "evoclash-server-10-aaaaaa",
        ]
    );
}

#[test]
fn target_is_in_use_plus_buffer_clamped() {
    assert_eq!(target_replicas(0, 3, 3, 300), 3);
    assert_eq!(target_replicas(2, 3, 3, 300), 5);
    assert_eq!(target_replicas(0, 0, 1, 10), 1);
    assert_eq!(target_replicas(9, 5, 0, 10), 10);
    assert_eq!(target_replicas(500, 3, 3, 300), 300);
    // A nonsense max must not lock the fleet below min.
    assert_eq!(target_replicas(0, 0, 4, 2), 4);
}

#[test]
fn scales_up_to_keep_the_buffer() {
    let slots = [
        slot("evoclash-server-0-aaaaaa", true),
        slot("evoclash-server-1-bbbbbb", true),
    ];
    assert_eq!(plan(3, &cfg(), true, &slots), Plan::Resize(5));
}

#[test]
fn holds_when_the_target_matches_the_current_size() {
    let slots = [slot("evoclash-server-0-aaaaaa", true)];
    assert_eq!(plan(4, &cfg(), true, &slots), Plan::Hold);
}

#[test]
fn a_player_on_the_top_pod_blocks_scale_down() {
    // Ordinal 6 is playing and it is the highest pod: the StatefulSet would delete it first,
    // so nothing shrinks even though the target is much smaller.
    let mut cfg = cfg();
    cfg.buffer = 0;
    cfg.min = 1;
    let slots = [
        slot("evoclash-server-5-aaaaaa", false),
        slot("evoclash-server-6-bbbbbb", true),
    ];
    assert_eq!(plan(7, &cfg, true, &slots), Plan::Hold);
}

#[test]
fn scale_down_stops_above_the_highest_busy_pod() {
    // Buffer 1, min 1, ordinal 2 in use: shrink to 3, not to the target of 2.
    let mut cfg = cfg();
    cfg.buffer = 1;
    cfg.min = 1;
    let slots = [
        slot("evoclash-server-2-aaaaaa", true),
        draining(slot("evoclash-server-3-bbbbbb", false)),
        draining(slot("evoclash-server-4-cccccc", false)),
    ];
    assert_eq!(plan(5, &cfg, true, &slots), Plan::Resize(3));
}

#[test]
fn empty_pods_above_a_busy_one_still_go() {
    // Ordinal 5 plays, so the floor is 6; the empty ordinal 6 can still be reclaimed.
    let slots = [
        slot("evoclash-server-5-aaaaaa", true),
        draining(slot("evoclash-server-6-bbbbbb", false)),
    ];
    assert_eq!(plan(7, &cfg(), true, &slots), Plan::Resize(6));
}

#[test]
fn empty_pods_are_drained_before_they_are_removed() {
    let mut cfg = cfg();
    cfg.buffer = 0;
    cfg.min = 2;
    let slots: Vec<Slot> = (0..5)
        .map(|ordinal| slot(&format!("evoclash-server-{ordinal}-aaaaaa"), false))
        .collect();
    // First tick: stop handing seats to the pods that are about to go.
    assert_eq!(
        plan(5, &cfg, true, &slots),
        Plan::Drain(vec![
            "evoclash-server-2-aaaaaa".into(),
            "evoclash-server-3-aaaaaa".into(),
            "evoclash-server-4-aaaaaa".into(),
        ])
    );
    // Once they are marked, the shrink goes through.
    let marked: Vec<Slot> = slots.into_iter().map(draining).collect();
    assert_eq!(plan(5, &cfg, true, &marked), Plan::Resize(2));
}

#[test]
fn nothing_shrinks_before_the_registry_has_heartbeats() {
    // Freshly elected: an empty registry must not look like an empty fleet.
    assert_eq!(plan(9, &cfg(), false, &[]), Plan::Hold);
}

#[test]
fn matches_outside_the_statefulset_hold_a_shrink() {
    let slots = [Slot {
        id: "m7000-00ab12".into(),
        ordinal: None,
        in_use: true,
        draining: false,
    }];
    assert_eq!(plan(9, &cfg(), true, &slots), Plan::Hold);
}

#[test]
fn a_drained_match_takes_no_more_players() {
    let mut registry = Registry::default();
    registry.set_statefulset("evoclash-server");
    registry.heartbeat(&heartbeat("evoclash-server-0-00ab12", 0));
    // A warm lobby full of bots has nobody in it: scale-down may reclaim it.
    let views = registry.scaler_slots();
    assert_eq!(views.len(), 1);
    assert!(!views[0].in_use);
    assert!(!views[0].draining);

    assert_eq!(
        registry.drain_matches(&["evoclash-server-0-00ab12".to_string()]),
        1
    );
    assert!(
        registry.reserve_seat().is_none(),
        "drained match got a seat"
    );
    assert!(registry.scaler_slots()[0].draining);

    // A human in the room keeps the pod from being scaled away, even once drained.
    registry.heartbeat(&heartbeat("evoclash-server-1-00ab12", 1));
    let slots = slots_of(&mut registry);
    assert!(slots.iter().any(|slot| slot.in_use));
    let mut cfg = cfg();
    cfg.buffer = 0;
    cfg.min = 1;
    // Ordinal 1 is the highest pod and has a human: the shrink waits for the match to end.
    assert_eq!(plan(2, &cfg, true, &slots), Plan::Hold);
}

#[test]
fn quick_start_prefers_the_lowest_ordinal() {
    let mut registry = Registry::default();
    registry.set_statefulset("evoclash-server");
    // Equal humans and countdown, so only the ordinal can break the tie.
    registry.heartbeat(&heartbeat("evoclash-server-10-aaaaaa", 0));
    registry.heartbeat(&heartbeat("evoclash-server-2-bbbbbb", 0));
    registry.heartbeat(&heartbeat("evoclash-server-9-cccccc", 0));
    let (id, _, _) = registry.reserve_seat().expect("an idle lobby");
    assert_eq!(
        id, "evoclash-server-2-bbbbbb",
        "string order would pick -10"
    );
}

#[test]
fn timestamps_round_trip() {
    // MicroTime: the API server rejects a Lease time without the six fractional digits.
    assert_eq!(micro_time(0), "1970-01-01T00:00:00.000000Z");
    assert_eq!(micro_time(1_700_000_000), "2023-11-14T22:13:20.000000Z");
    for secs in [0, 1, 1_700_000_000, 951_782_400, 4_102_444_800] {
        assert_eq!(parse_rfc3339(&micro_time(secs)), Some(secs));
    }
}

#[test]
fn parses_the_timestamps_the_api_server_writes() {
    // MicroTime keeps microseconds; a numeric offset must not shift the answer.
    assert_eq!(
        parse_rfc3339("2024-05-06T07:08:09.123456Z"),
        Some(1_714_979_289)
    );
    assert_eq!(
        parse_rfc3339("2024-05-06T09:08:09+02:00"),
        Some(1_714_979_289)
    );
    assert_eq!(parse_rfc3339(""), None);
    assert_eq!(parse_rfc3339("2024-05-06"), None);
    assert_eq!(parse_rfc3339("not a time"), None);
    assert_eq!(parse_rfc3339("2024-05-06T07:08:09"), None);
}

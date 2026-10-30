//! `?fault=` link faults: a debug-only way to break the client's connection on purpose.
//!
//! `blackout:<secs>@<at>` drops every incoming packet for `<secs>` seconds, `<at>` seconds after
//! the match session started; `close@<at>` tears the transport down at that point. Only the debug
//! client parses the parameter (`main.rs`), so players never reach this.
//!
//! The steps are replayed front to back by [`apply_faults`], which runs right after the session's
//! reconnect loop: that way the entity it cuts is the one the loop just spawned.

use bevy::prelude::*;
use lightyear::link::prelude::{LinkConditionerConfig, RecvLinkConditioner};
use lightyear::prelude::client::Disconnect;
use lightyear::prelude::*;

use crate::ClientArgs;
use crate::session::link_conditioner;

/// `?fault=` step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fault {
    /// Drop every incoming packet for `secs`, `at` seconds after the match started.
    Blackout { at: f32, secs: f32 },
    /// Tear the transport down, `at` seconds after the match started.
    Close { at: f32 },
}

impl Fault {
    fn at(self) -> f32 {
        match self {
            Fault::Blackout { at, .. } | Fault::Close { at } => at,
        }
    }
}

/// Parsed `?fault=` value, sorted by start time.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FaultSpec(pub Vec<Fault>);

impl FaultSpec {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

fn parse_seconds(text: &str) -> Option<f32> {
    let value: f32 = text.trim().parse().ok()?;
    (value.is_finite() && value >= 0.0).then_some(value)
}

/// `blackout:15@5,close@40`. Unknown pieces are dropped: a typo must not break the page.
/// `at` counts from the moment the match session started, not from page load.
pub fn parse_fault(text: &str) -> FaultSpec {
    let mut steps: Vec<Fault> = Vec::new();
    for part in text.split(',') {
        let Some((kind, at)) = part.split_once('@') else {
            continue;
        };
        let Some(at) = parse_seconds(at) else {
            continue;
        };
        match kind.trim().split_once(':') {
            Some((name, secs)) if name.trim() == "blackout" => {
                if let Some(secs) = parse_seconds(secs).filter(|secs| *secs > 0.0) {
                    steps.push(Fault::Blackout { at, secs });
                }
            }
            Some(_) => {}
            None if kind.trim() == "close" => steps.push(Fault::Close { at }),
            None => {}
        }
    }
    steps.sort_by(|a, b| a.at().total_cmp(&b.at()));
    FaultSpec(steps)
}

/// A conditioner that drops every incoming packet: for a while the client hears nothing, like a
/// severed link. Restored by [`apply_faults`] when the blackout ends.
fn blackout_conditioner() -> RecvLinkConditioner {
    RecvLinkConditioner::new(LinkConditionerConfig::default().with_fixed_loss(1.0))
}

/// `?fault=` playback: which steps already fired, and when the current blackout ends.
#[derive(Resource, Default)]
pub(crate) struct FaultRun {
    /// `Time::elapsed_secs` when the current match session started (first client entity).
    started: Option<f32>,
    fired: usize,
    blackout_until: Option<f32>,
}

/// Each step fires once, counted from the first client entity, so a reconnect in between does
/// not restart the clock.
pub(crate) fn apply_faults(
    mut commands: Commands,
    time: Res<Time>,
    args: Res<ClientArgs>,
    mut run: ResMut<FaultRun>,
    mut clients: Query<(Entity, &mut Link), With<Client>>,
) {
    let spec = &args.fault;
    if spec.is_empty() {
        return;
    }
    let now = time.elapsed_secs();
    // A blackout holds the drop-everything conditioner on whatever link exists (a long one
    // reconnects underneath it), and puts the real one back once it is over.
    if let Some(until) = run.blackout_until {
        if now < until {
            for (_, mut link) in &mut clients {
                link.recv.conditioner = Some(blackout_conditioner());
            }
            return;
        }
        run.blackout_until = None;
        let conditioner = link_conditioner(&args);
        for (_, mut link) in &mut clients {
            link.recv.conditioner = conditioner.clone();
        }
    }
    // There is only ever one client entity: the one the session's join spawned.
    let Ok((entity, mut link)) = clients.single_mut() else {
        return;
    };
    let started = *run.started.get_or_insert(now);
    let elapsed = now - started;
    while let Some(step) = spec
        .0
        .get(run.fired)
        .copied()
        .filter(|step| step.at() <= elapsed)
    {
        run.fired += 1;
        match step {
            Fault::Blackout { at, secs } => {
                info!("fault: blackout for {secs}s at {at}s");
                run.blackout_until = Some(now + secs);
                link.recv.conditioner = Some(blackout_conditioner());
            }
            Fault::Close { at } => {
                info!("fault: closing the client link at {at}s");
                commands.trigger(Disconnect { entity });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fault_parameter_parses_blackout_and_close() {
        assert!(parse_fault("").is_empty());
        assert_eq!(
            parse_fault("blackout:3@20"),
            FaultSpec(vec![Fault::Blackout {
                at: 20.0,
                secs: 3.0
            }])
        );
        assert_eq!(
            parse_fault("close@30"),
            FaultSpec(vec![Fault::Close { at: 30.0 }])
        );
        // Sorted by start time, so the spec can be replayed front to back.
        assert_eq!(
            parse_fault("close@30,blackout:15@5"),
            FaultSpec(vec![
                Fault::Blackout {
                    at: 5.0,
                    secs: 15.0
                },
                Fault::Close { at: 30.0 },
            ])
        );
        assert_eq!(
            parse_fault(" blackout:25@1 , close@2 ").0.len(),
            2,
            "whitespace is trimmed"
        );
        // Typos and nonsense drop out instead of breaking the page.
        for bad in [
            "blackout",
            "blackout@5",
            "blackout:0@5",
            "blackout:-2@5",
            "blackout:3@-1",
            "blackout:3@x",
            "close",
            "close@",
            "explode@5",
            "",
            ",",
        ] {
            assert!(parse_fault(bad).is_empty(), "{bad}");
        }
        // A good step next to a bad one still runs.
        assert_eq!(
            parse_fault("nonsense,close@7"),
            FaultSpec(vec![Fault::Close { at: 7.0 }])
        );
    }
}

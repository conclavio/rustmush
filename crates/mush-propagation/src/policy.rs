//! E7 cut 7 B3 (propagation's half) — the [`mush_core::SensorPolicy`]
//! implementation the **tactics-owned** sensor ingress reads.
//!
//! This is the whole of propagation's participation in the contact lifecycle:
//! it **states what is due** and nothing else. It holds no `ContactState`
//! handle, registers no contact family and no contact guard validator, and
//! could not write a contact if it tried — `ActionTxn::checked_key` refuses a
//! cross-owner key, which makes "propagation cannot forge tactics' state" a
//! kernel property rather than a convention.
//!
//! It is installed at `server_modules_with` through
//! `TacticsModule::with_sensor_policy`, so **`mush-tactics` gains no
//! `mush-propagation` edge**: the trait it satisfies is `mush-core`'s.

use mush_core::{ClockStamp, ContactLedgerState, ObservationIntent, SensorPolicy, World};
use mush_sim::SpatialState;

use crate::pass::detection_pass;
use crate::record::PropagationStore;

/// The observation policy, read by `mush-tactics`' sensor schedule adapters.
///
/// Holds nothing: it reads the per-world `PropagationStore`, `SpatialState` and
/// the leak-safe `ContactLedgerState`, so two calls on one unchanged world
/// answer the **same ordered set** — the determinism the deterministic key
/// depends on.
#[derive(Clone, Copy, Debug, Default)]
pub struct PropagationSensorPolicy;

impl SensorPolicy for PropagationSensorPolicy {
    fn due(&self, world: &World, at: &ClockStamp) -> Vec<ObservationIntent> {
        let (Some(store), Some(state)) = (
            world.module_state::<PropagationStore>(),
            world.module_state::<SpatialState>(),
        ) else {
            return Vec::new();
        };
        let ledger = world
            .module_state::<ContactLedgerState>()
            .cloned()
            .unwrap_or_default();
        // The pass records sightings, which is what makes it idempotent; the
        // policy is a READ seam, so it works on a scratch copy and the durable
        // sighting book is written only by the command/tick path that arms.
        let mut scratch = store.clone();
        detection_pass(
            &mut scratch,
            state,
            world,
            &ledger,
            at,
            None,
            mush_core::MAX_OBSERVATIONS_PER_PASS,
        )
        .intents
    }
}

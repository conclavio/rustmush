#![cfg(test)]

//! An identity-only control on an already settled arena, never an execution mode.
//! Capture the real custody seals before opening, retain both actual digests,
//! and hash a final-state clone after changing only the folded seal identities.

use std::collections::BTreeMap;

use mush_core::ObjId;
use tacsim::arena::state::{Arena, CustodyUnit};

/// The actual API5 custody and the reviewed four-field API4 seal projection.
pub struct Api4SealHashControl {
    seals: BTreeMap<ObjId, ([u8; 32], [u8; 32])>,
}

impl Api4SealHashControl {
    pub fn capture(units: &[CustodyUnit]) -> Self {
        assert!(!units.is_empty(), "the control must capture actual custody");
        assert_eq!(tacsim::minds::MIND_ABI, "rhai=1.26.0;api=5;stock=2");
        let api4_source = tacsim::arena::snapshot::digest_from_text(
            "9b330c0f5ccd861078d4928fbc30dda7a132df6c8f1b027d4a5a00ca452cf69b",
        )
        .expect("the retained API4 Standard source digest");
        let api5_source = tacsim::arena::snapshot::digest_from_text(
            "d318e09bd821a0d933e273309f32608fe422041c059e762377cae0551673b2af",
        )
        .expect("the observed API5 Standard source digest");
        assert!(
            tacsim::minds::standard_digest().0 == api5_source,
            "this control covers exactly the reviewed Standard source migration"
        );
        let mut seals = BTreeMap::new();
        for unit in units {
            let current = &unit.seal;
            assert!(
                current.unit == unit.id
                    && current.abi == tacsim::minds::MIND_ABI
                    && current.source_sha256 == api5_source
                    && current.requested.source_sha256 == api5_source
                    && current.rom_identity.source_sha256 == api5_source,
                "capture only actual custody with all reviewed current identities"
            );
            let mut prior = current.clone();
            prior.abi = "rhai=1.26.0;api=4;stock=2".into();
            prior.source_sha256 = api4_source;
            prior.requested.source_sha256 = api4_source;
            prior.rom_identity.source_sha256 = api4_source;
            // Use the actual MindSeal serializer/digest; every other byte,
            // including complete cards, classes, channels and custody, is kept.
            let pair = (current.digest(), prior.digest());
            assert!(
                pair.0 != pair.1,
                "the identity control must change the seal"
            );
            assert!(
                seals.insert(unit.id, pair).is_none(),
                "each real custody unit occurs once"
            );
        }
        Self { seals }
    }

    /// Hash a clone of the fully settled current run under only its old folded
    /// seal identities. This method never steps either arena, changes a unit's
    /// actual seal, or returns the altered clone to any execution path.
    pub fn hash_of_settled(&self, arena: &Arena) -> u64 {
        assert_eq!(
            arena.units.len(),
            self.seals.len(),
            "exact captured custody"
        );
        for unit in &arena.units {
            let (current, _) = self.seals.get(&unit.id).expect("the captured unit");
            assert!(
                unit.seal.digest == *current,
                "the settled arena retains the captured actual custody seal"
            );
        }
        assert!(
            !arena.minds.is_empty(),
            "the control must cover folded minds"
        );
        let mut prior = arena.clone();
        for (unit, mind) in &mut prior.minds {
            let (current, historical) = self.seals.get(unit).expect("the captured mind");
            assert!(
                mind.seal_digest == *current,
                "the real current run retains its captured folded seal"
            );
            mind.seal_digest = *historical;
        }
        tacsim::arena::hash::state_hash(&prior)
    }
}

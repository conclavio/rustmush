#![cfg(test)]

//! An identity-only control on an already settled arena, never an execution mode.
//! Capture the real custody seals before opening, retain both actual digests,
//! and hash a final-state clone after changing only the folded seal identities.

use std::collections::BTreeMap;

use mush_core::ObjId;
use tacsim::arena::state::{Arena, CustodyUnit};

/// The actual API6 custody and the reviewed API4 seal projection. The projection
/// restores only ABI, source identities and stock provenance revisions.
pub struct Api4SealHashControl {
    seals: BTreeMap<ObjId, ([u8; 32], [u8; 32])>,
}

impl Api4SealHashControl {
    pub fn capture(units: &[CustodyUnit]) -> Self {
        assert!(!units.is_empty(), "the control must capture actual custody");
        assert_eq!(tacsim::minds::MIND_ABI, "rhai=1.26.0;api=7;stock=3");
        let api4_source = tacsim::arena::snapshot::digest_from_text(
            "9b330c0f5ccd861078d4928fbc30dda7a132df6c8f1b027d4a5a00ca452cf69b",
        )
        .expect("the retained API4 Standard source digest");
        let api6_source = tacsim::arena::snapshot::digest_from_text(
            "3837c3456fbf1e3b8e012375a3d4e4cfcc2645d024fe203eef9359de1473a316",
        )
        .expect("the observed API6 Standard source digest");
        assert!(
            tacsim::minds::standard_digest().0 == api6_source,
            "this control covers exactly the reviewed Standard source migration"
        );
        let mut seals = BTreeMap::new();
        for unit in units {
            let current = &unit.seal;
            assert!(
                current.unit == unit.id
                    && current.abi == tacsim::minds::MIND_ABI
                    && current.source_sha256 == api6_source
                    && current.requested.source_sha256 == api6_source
                    && current.rom_identity.source_sha256 == api6_source,
                "capture only actual custody with all reviewed current identities"
            );
            let mut prior = current.clone();
            prior.abi = "rhai=1.26.0;api=4;stock=2".into();
            prior.source_sha256 = api4_source;
            prior.requested.source_sha256 = api4_source;
            prior.rom_identity.source_sha256 = api4_source;
            for origin in [
                &mut prior.origin,
                &mut prior.requested.origin,
                &mut prior.rom_identity.origin,
            ] {
                match origin {
                    tacsim::arena::profile_seal::MindOrigin::Doctrine { revision } => {
                        assert_eq!(*revision, tacsim::minds::STOCK_VERSION);
                        *revision = 2;
                    }
                    other => panic!("the captured baseline has stock provenance: {other:?}"),
                }
            }
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

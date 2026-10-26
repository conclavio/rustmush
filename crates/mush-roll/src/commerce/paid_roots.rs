//! Funding reserves the complete Roll-side terminal footprint. These roots
//! retain the immutable allocation history even after refund; they are never
//! evidence that a paid licence or term exists.

use super::sale_reader::OriginalSaleFacts;
use super::{TermPhase, WorldIcSeconds};
use mush_core::participant::OwnerPrepareContext;
use mush_core::{CommitToken, World};
use mush_economy::managed_purchase::{PurchaseOutcome, TerminalOutput};

use super::purchase::{PaidAllocation, PurchasePhase};
use super::records::{PublicationError, Record};
use super::roots::{RequiredRoot, RootAnchor};

pub(crate) struct PaidRoots {
    commit: CommitToken,
    before: Vec<RequiredRoot>,
    after: Vec<RequiredRoot>,
    fresh: Option<OriginalSaleFacts>,
}

pub(crate) fn required(row: &PaidAllocation) -> Result<Vec<RequiredRoot>, PublicationError> {
    row.validate()?;
    Ok(vec![
        RequiredRoot {
            anchor: RootAnchor::PaidOperation(row.operation),
            identity: row.revision,
        },
        RequiredRoot {
            anchor: RootAnchor::Term(row.licence, row.term),
            identity: row.revision,
        },
    ])
}

impl PaidRoots {
    pub(crate) fn funding(
        row: &PaidAllocation,
        commit: CommitToken,
        facts: &OriginalSaleFacts,
    ) -> Result<Self, PublicationError> {
        if row.operation != commit || row.phase != (PurchasePhase::Funded {}) {
            return Err(PublicationError::Mismatch);
        }
        Ok(Self {
            commit,
            before: Vec::new(),
            after: required(row)?,
            fresh: Some(facts.clone()),
        })
    }

    pub(crate) fn terminal(
        before: &PaidAllocation,
        after: &PaidAllocation,
        output: &TerminalOutput,
        facts: Option<&OriginalSaleFacts>,
    ) -> Result<Self, PublicationError> {
        before.validate()?;
        after.validate()?;
        if before.phase != (PurchasePhase::Funded {})
            || output.operation() != before.operation
            || output.contract() != before.funding.contract()
            || output.hold() != before.funding.hold()
        {
            return Err(PublicationError::Mismatch);
        }
        let mut expected = before.clone();
        expected.updated_by = output.commit();
        expected.phase = match output.outcome() {
            PurchaseOutcome::Granted => PurchasePhase::Granted {
                commit: output.commit(),
            },
            PurchaseOutcome::Refunded => PurchasePhase::Refunded {
                commit: output.commit(),
            },
        };
        if &expected != after || (output.outcome() == PurchaseOutcome::Granted) != facts.is_some() {
            return Err(PublicationError::Mismatch);
        }
        Ok(Self {
            commit: output.commit(),
            before: required(before)?,
            after: required(after)?,
            fresh: facts.cloned(),
        })
    }
    pub(crate) fn fresh(&self) -> Option<&OriginalSaleFacts> {
        self.fresh.as_ref()
    }
    pub(crate) const fn commit(&self) -> CommitToken {
        self.commit
    }
    pub(crate) fn before(&self) -> &[RequiredRoot] {
        &self.before
    }
    pub(crate) fn after(&self) -> &[RequiredRoot] {
        &self.after
    }
}

/// This is deliberately a pre-custody World check. The typed transaction keeps
/// the exact term/status/rating rows stable; Roll's generation keeps its source
/// audit and retained roots stable. The server freezes World time while the
/// exact writer intent is pending. No World callback runs after acknowledgement.
pub(crate) fn check_fresh(
    facts: Option<&OriginalSaleFacts>,
    context: &OwnerPrepareContext<'_>,
) -> Result<(), PublicationError> {
    let Some(facts) = facts else {
        return Ok(());
    };
    if context.invocation().actor() != facts.requester() {
        return Err(PublicationError::Mismatch);
    }
    check_window(facts, context.world())
}

fn check_window(facts: &OriginalSaleFacts, world: &World) -> Result<(), PublicationError> {
    let now = world.world_ic_now();
    if now.clock != facts.observed_ic().clock || now.at < facts.observed_ic().at {
        return Err(PublicationError::Mismatch);
    }
    let now = WorldIcSeconds::from_stamp(&now).map_err(|_| PublicationError::Invalid)?;
    if !matches!(
        facts.owner_window().phase_at(now),
        TermPhase::Active | TermPhase::Grace
    ) {
        return Err(PublicationError::Unavailable);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commerce::publication_tests::{open_gate_for_owner_unit_test, saved};
    use crate::commerce::{LicenceId, RevisionIdentity, TermGeneration};
    use mush_core::ModuleWrite;
    use std::collections::BTreeMap;

    #[test]
    fn terminal_republishes_exact_funded_root_images_without_capacity_growth() {
        let (mut store, _, _, revision) = saved();
        // Isolate the existing Roll owner/generation unit contract; this gate
        // does not claim full paired business boot or a paid grant.
        open_gate_for_owner_unit_test(&mut store);
        let identity = RevisionIdentity::capture(&revision).unwrap();
        let roots = vec![
            RequiredRoot {
                anchor: RootAnchor::PaidOperation(CommitToken::from_bytes([3; 16])),
                identity,
            },
            RequiredRoot {
                anchor: RootAnchor::Term(
                    LicenceId::new(2).unwrap(),
                    TermGeneration::new(1).unwrap(),
                ),
                identity,
            },
        ];
        let terminal = PaidRoots {
            commit: CommitToken::from_bytes([4; 16]),
            before: roots.clone(),
            after: roots.clone(),
            fresh: None,
        };
        assert!(
            store.prepare_paid_roots(&terminal).is_err(),
            "funded roots must already exist"
        );
        let funding = PaidRoots {
            commit: CommitToken::from_bytes([3; 16]),
            before: Vec::new(),
            after: roots,
            fresh: None,
        };
        let prepared = store.prepare_paid_roots(&funding).unwrap();
        let images = |change: &crate::PreparedChange| {
            change
                .operations()
                .iter()
                .map(|op| {
                    assert_eq!(op.ns, crate::NS);
                    match &op.op {
                        ModuleWrite::JsonSet { key, value } => (key.clone(), value.clone()),
                        _ => panic!("paid roots are retained images"),
                    }
                })
                .collect::<BTreeMap<_, _>>()
        };
        let funded = images(&prepared);
        assert_eq!(funded.len(), 2);
        store.bind_commercial_roots(prepared).unwrap().apply();
        let usage = store.current_usage().unwrap();
        let prepared = store.prepare_paid_roots(&terminal).unwrap();
        assert_eq!(images(&prepared), funded);
        let change = prepared.usage().unwrap();
        assert_eq!(change.before(), change.after());
        store.bind_commercial_roots(prepared).unwrap().apply();
        assert_eq!(store.current_usage(), Some(usage));
        for root in terminal.after() {
            store.check_required_root(root).unwrap();
        }
    }
}

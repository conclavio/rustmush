//! Exact new-action retention requests. Their only constructors consume the
//! private current sale observation and/or actual closed E4 output.

use mush_core::participant::OwnerPrepareContext;
use mush_core::{CommitToken, DefinitionRef, ObjId};
use mush_economy::managed_entitlement::{EntitlementProtocol, EntitlementTerminalOutput};
use mush_economy::managed_purchase::PurchaseOutcome;

use super::super::purchase::PurchasePhase;
use super::super::records::{PublicationError, Record};
use super::super::roots::{RequiredRoot, RootAnchor};
use super::EntitlementAllocationV1;
use super::reader::EntitlementSaleFacts;

#[derive(Clone, Copy)]
pub(crate) enum Transition {
    List,
    Fund,
    Grant,
    Refund,
}
pub(crate) fn action_id(protocol: EntitlementProtocol, transition: Transition) -> &'static str {
    match (protocol, transition) {
        (EntitlementProtocol::OriginalV3, Transition::List) => "list_original_mind",
        (EntitlementProtocol::OriginalV3, Transition::Fund) => "purchase_original_mind",
        (EntitlementProtocol::OriginalV3, Transition::Grant) => "settle_original_mind",
        (EntitlementProtocol::OriginalV3, Transition::Refund) => "refund_original_mind",
        (EntitlementProtocol::DerivedV1, Transition::List) => "list_derived_mind",
        (EntitlementProtocol::DerivedV1, Transition::Fund) => "purchase_derived_mind",
        (EntitlementProtocol::DerivedV1, Transition::Grant) => "settle_derived_mind",
        (EntitlementProtocol::DerivedV1, Transition::Refund) => "refund_derived_mind",
    }
}

#[derive(Clone)]
pub(crate) struct EntitlementFence {
    action: DefinitionRef,
    commit: CommitToken,
    requester: ObjId,
    fresh: Option<EntitlementSaleFacts>,
}
impl EntitlementFence {
    fn new(
        action: &DefinitionRef,
        protocol: EntitlementProtocol,
        transition: Transition,
        commit: CommitToken,
        requester: ObjId,
        fresh: Option<&EntitlementSaleFacts>,
    ) -> Result<Self, PublicationError> {
        if action.key().owner.as_str() != "market"
            || action.key().kind.as_str() != "action"
            || action.key().id.as_str() != action_id(protocol, transition)
            || action.version() != protocol.version()
            || fresh.is_some() != !matches!(transition, Transition::Refund)
            || fresh
                .is_some_and(|facts| facts.protocol() != protocol || facts.requester() != requester)
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(Self {
            action: action.clone(),
            commit,
            requester,
            fresh: fresh.cloned(),
        })
    }
    pub(crate) fn check(&self, context: &OwnerPrepareContext<'_>) -> Result<(), PublicationError> {
        if context.action() != &self.action
            || context.commit_token() != self.commit
            || context.invocation().actor() != self.requester
        {
            return Err(PublicationError::Mismatch);
        }
        if let Some(facts) = &self.fresh {
            facts.check_fresh(context)?;
        }
        Ok(())
    }
}

pub(crate) struct EntitlementRoots {
    fence: EntitlementFence,
    before: Vec<RequiredRoot>,
    after: Vec<RequiredRoot>,
}
impl EntitlementRoots {
    pub(crate) fn listing(
        action: &DefinitionRef,
        commit: CommitToken,
        facts: &EntitlementSaleFacts,
    ) -> Result<Self, PublicationError> {
        if facts.requester() != facts.seller()
            || facts
                .materialization()
                .creates()
                .iter()
                .any(|row| row.publication() != commit)
        {
            return Err(PublicationError::Mismatch);
        }
        let root = RequiredRoot {
            anchor: RootAnchor::Filed(facts.filed()),
            identity: facts.revision(),
        };
        Ok(Self {
            fence: EntitlementFence::new(
                action,
                facts.protocol(),
                Transition::List,
                commit,
                facts.requester(),
                Some(facts),
            )?,
            // Listing establishes only typed immutable economics. The actual
            // retained filed root is prevalidated and re-published unchanged,
            // giving this participant its existing physical owner image fence.
            before: vec![root.clone()],
            after: vec![root],
        })
    }
    pub(crate) fn funding(
        action: &DefinitionRef,
        row: &EntitlementAllocationV1,
        commit: CommitToken,
        facts: &EntitlementSaleFacts,
    ) -> Result<Self, PublicationError> {
        if row.operation != commit
            || row.phase != (PurchasePhase::Funded {})
            || !matches_facts(row, facts)
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(Self {
            fence: EntitlementFence::new(
                action,
                row.protocol,
                Transition::Fund,
                commit,
                facts.requester(),
                Some(facts),
            )?,
            before: Vec::new(),
            after: super::audit::required(row)?,
        })
    }
    pub(crate) fn terminal(
        action: &DefinitionRef,
        requester: ObjId,
        before: &EntitlementAllocationV1,
        after: &EntitlementAllocationV1,
        output: &EntitlementTerminalOutput,
        facts: Option<&EntitlementSaleFacts>,
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
        let transition = match output.outcome() {
            PurchaseOutcome::Granted => Transition::Grant,
            PurchaseOutcome::Refunded => Transition::Refund,
        };
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
        if &expected != after || facts.is_some_and(|facts| !matches_facts(before, facts)) {
            return Err(PublicationError::Mismatch);
        }
        Ok(Self {
            fence: EntitlementFence::new(
                action,
                before.protocol,
                transition,
                output.commit(),
                requester,
                facts,
            )?,
            before: super::audit::required(before)?,
            after: super::audit::required(after)?,
        })
    }
    pub(crate) fn commit(&self) -> CommitToken {
        self.fence.commit
    }
    pub(crate) fn fence(&self) -> &EntitlementFence {
        &self.fence
    }
    pub(crate) fn before(&self) -> &[RequiredRoot] {
        &self.before
    }
    pub(crate) fn after(&self) -> &[RequiredRoot] {
        &self.after
    }
}

fn matches_facts(row: &EntitlementAllocationV1, facts: &EntitlementSaleFacts) -> bool {
    row.protocol == facts.protocol()
        && row.holder.acting_character() == facts.requester()
        && row.charter == facts.charter()
        && row.line == facts.line()
        && row.filed == facts.filed()
        && row.revision == facts.revision()
        && row.rating == facts.rating()
        && row.terms.class() >= facts.minimum()
        && facts
            .royalty()
            .reference()
            .is_ok_and(|reference| reference == row.economics.grant().royalty_chain())
}

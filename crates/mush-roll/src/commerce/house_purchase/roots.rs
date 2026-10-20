//! Exact new-action retention requests. Their only constructors consume the
//! private current sale observation and/or actual closed E4 output.

use super::HouseProtocol;
use mush_core::participant::OwnerPrepareContext;
use mush_core::{CommitToken, DefinitionRef, ObjId};
use mush_economy::managed_house_purchase::HouseTerminalOutput;
use mush_economy::managed_purchase::PurchaseOutcome;

use super::super::purchase::PurchasePhase;
use super::super::records::{PublicationError, Record};
use super::super::roots::{RequiredRoot, RootAnchor};
use super::HouseAllocationV1;
use crate::commerce::house_catalogue::HouseSaleFacts;

#[derive(Clone, Copy)]
pub(crate) enum Transition {
    List,
    Fund,
    Grant,
    Refund,
}
pub(crate) fn action_id(protocol: HouseProtocol, transition: Transition) -> &'static str {
    match (protocol, transition) {
        (HouseProtocol::HouseV1 | HouseProtocol::HouseV2, Transition::List) => "list_house_mind",
        (HouseProtocol::HouseV1 | HouseProtocol::HouseV2, Transition::Fund) => {
            "purchase_house_mind"
        }
        (HouseProtocol::HouseV1 | HouseProtocol::HouseV2, Transition::Grant) => "settle_house_mind",
        (HouseProtocol::HouseV1 | HouseProtocol::HouseV2, Transition::Refund) => {
            "refund_house_mind"
        }
    }
}

#[derive(Clone)]
pub(crate) struct HouseFence {
    action: DefinitionRef,
    commit: CommitToken,
    requester: ObjId,
    fresh: Option<HouseSaleFacts>,
    publication: bool,
    buyer_admission: Option<std::sync::Arc<dyn super::HouseBuyerAdmission>>,
}
impl HouseFence {
    fn new(
        action: &DefinitionRef,
        protocol: HouseProtocol,
        transition: Transition,
        commit: CommitToken,
        requester: ObjId,
        fresh: Option<&HouseSaleFacts>,
    ) -> Result<Self, PublicationError> {
        if action.key().owner.as_str() != "market"
            || action.key().kind.as_str() != "action"
            || action.key().id.as_str() != action_id(protocol, transition)
            || action.version() != protocol.version()
            || fresh.is_some() != !matches!(transition, Transition::Refund)
            || fresh.is_some_and(|facts| facts.requester() != requester)
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(Self {
            action: action.clone(),
            commit,
            requester,
            fresh: fresh.cloned(),
            publication: matches!(transition, Transition::List),
            buyer_admission: None,
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
            facts.check_fresh(context, self.publication)?;
            if !self.publication {
                if let Some(policy) = &self.buyer_admission {
                    policy.check(context.world(), facts.issuer(), self.requester)?;
                }
            }
        }
        Ok(())
    }
}

pub(crate) struct HouseRoots {
    fence: HouseFence,
    before: Vec<RequiredRoot>,
    after: Vec<RequiredRoot>,
}
impl HouseRoots {
    pub(crate) fn listing(
        action: &DefinitionRef,
        protocol: HouseProtocol,
        commit: CommitToken,
        facts: &HouseSaleFacts,
    ) -> Result<Self, PublicationError> {
        let root = RequiredRoot {
            anchor: RootAnchor::Filed(facts.filed()),
            identity: facts.revision(),
        };
        Ok(Self {
            fence: HouseFence::new(
                action,
                protocol,
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
        row: &HouseAllocationV1,
        commit: CommitToken,
        facts: &HouseSaleFacts,
        buyer_admission: Option<std::sync::Arc<dyn super::HouseBuyerAdmission>>,
    ) -> Result<Self, PublicationError> {
        if row.operation != commit
            || row.phase != (PurchasePhase::Funded {})
            || !matches_facts(row, facts)
        {
            return Err(PublicationError::Mismatch);
        }
        let mut fence = HouseFence::new(
            action,
            row.protocol,
            Transition::Fund,
            commit,
            facts.requester(),
            Some(facts),
        )?;
        fence.buyer_admission = buyer_admission;
        Ok(Self {
            fence,
            before: Vec::new(),
            after: super::audit::required(row)?,
        })
    }
    pub(crate) fn terminal(
        action: &DefinitionRef,
        requester: ObjId,
        before: &HouseAllocationV1,
        after: &HouseAllocationV1,
        output: &HouseTerminalOutput,
        facts: Option<&HouseSaleFacts>,
        buyer_admission: Option<std::sync::Arc<dyn super::HouseBuyerAdmission>>,
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
        let mut fence = HouseFence::new(
            action,
            before.protocol,
            transition,
            output.commit(),
            requester,
            facts,
        )?;
        if matches!(transition, Transition::Grant) {
            fence.buyer_admission = buyer_admission;
        }
        Ok(Self {
            fence,
            before: super::audit::required(before)?,
            after: super::audit::required(after)?,
        })
    }
    pub(crate) fn commit(&self) -> CommitToken {
        self.fence.commit
    }
    pub(crate) fn fence(&self) -> &HouseFence {
        &self.fence
    }
    pub(crate) fn before(&self) -> &[RequiredRoot] {
        &self.before
    }
    pub(crate) fn after(&self) -> &[RequiredRoot] {
        &self.after
    }
}

fn matches_facts(row: &HouseAllocationV1, facts: &HouseSaleFacts) -> bool {
    row.validate().is_ok()
        && row.holder == facts.requester()
        && row.issuer == facts.issuer()
        && row.charter == facts.charter()
        && row.line == facts.line()
        && row.filed == facts.filed()
        && row.revision == facts.revision()
        && row.rating == facts.rating()
        && row.terms.class() >= facts.minimum()
}

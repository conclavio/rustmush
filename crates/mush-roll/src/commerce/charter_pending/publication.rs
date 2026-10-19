//! Private Roll retention request and repeated admission/terminal fences.
use super::{
    Readiness, bad,
    records::ClaimObservation,
    service::{Intent, Runtime, live},
};
use crate::commerce::{CharterId, records::PublicationError, roots::RequiredRoot};
use mush_core::participant::OwnerPrepareContext;
use mush_core::{CommitToken, DefinitionRef, ObjId, World};

pub(crate) struct PendingRoots {
    pub(super) commit: CommitToken,
    pub(super) old: Vec<RequiredRoot>,
    pub(super) added: Vec<RequiredRoot>,
    pub(super) fence: PendingFence,
}
impl PendingRoots {
    pub(crate) fn commit(&self) -> CommitToken {
        self.commit
    }
    pub(crate) fn old(&self) -> &[RequiredRoot] {
        &self.old
    }
    pub(crate) fn added(&self) -> &[RequiredRoot] {
        &self.added
    }
    pub(crate) fn fence(&self) -> &PendingFence {
        &self.fence
    }
}
#[derive(Clone)]
pub(crate) struct Admission {
    pub(super) charter: CharterId,
    pub(super) seller: ObjId,
    pub(super) buyer: ObjId,
    pub(super) title: mush_economy::EconomicAsset,
    pub(super) payout: mush_economy::Account,
    pub(super) deadline: i64,
    pub(super) claims: Option<ClaimObservation>,
}
impl Admission {
    fn check(&self, w: &World) -> Result<(), PublicationError> {
        if !live(w, self.seller)
            || !live(w, self.buyer)
            || self.seller == self.buyer
            || w.now() >= self.deadline
        {
            return Err(PublicationError::Unavailable);
        }
        crate::commerce::owner_qualification::current_title(w, self.seller, self.charter)?;
        let e = w
            .module_state::<mush_economy::EconomyStore>()
            .ok_or(PublicationError::Unavailable)?;
        if e.is_frozen()
            || e.snapshot().asset(self.charter.object()) != Some(&self.title)
            || !self.title.encumbrances.is_empty()
            || self.payout.owner != mush_org::PrincipalId::Object(self.seller)
            || self.payout.scope.is_some()
            || self.payout.status != mush_economy::AccountStatus::Active
            || e.snapshot().account(self.payout.id) != Some(&self.payout)
            || e.snapshot()
                .instrument(&self.payout.instrument)
                .is_none_or(|i| i.status != mush_economy::InstrumentStatus::Active)
        {
            return Err(PublicationError::Mismatch);
        }
        if let Some(claims) = &self.claims {
            let roll = w
                .module_state::<crate::RollStore>()
                .ok_or(PublicationError::Unavailable)?;
            if &roll.charter_pending_claims(self.charter).map_err(bad)? != claims {
                return Err(PublicationError::Mismatch);
            }
        }
        Ok(())
    }
}
#[derive(Clone)]
pub(super) enum Mode {
    Admission(Admission),
    RegistryRefund(crate::commerce::registry_revocation::RevocationFence),
    Service { runtime: Runtime, intent: Intent },
}
#[derive(Clone)]
pub(crate) struct PendingFence {
    pub(super) action: DefinitionRef,
    pub(super) commit: CommitToken,
    pub(super) actor: ObjId,
    pub(super) parameters: serde_json::Value,
    pub(super) at_ic: mush_core::ClockStamp,
    pub(super) at_utc: i64,
    pub(super) ready: Readiness,
    pub(super) mode: Mode,
}
impl PendingFence {
    pub(crate) fn check(&self, c: &OwnerPrepareContext<'_>) -> Result<(), PublicationError> {
        if c.action() != &self.action
            || c.commit_token() != self.commit
            || c.invocation().actor() != self.actor
            || c.invocation().parameters() != &self.parameters
            || c.invocation().principal()
                != &(mush_core::action::PrincipalId::Object { id: self.actor })
            || c.invocation().subject() != &mush_core::action::PerceivedRef::SelfActor
            || !c.invocation().targets().is_empty()
        {
            return Err(PublicationError::Mismatch);
        }
        if let Mode::RegistryRefund(authorization) = &self.mode {
            authorization.check(c)?;
        }
        self.check_world(c.world())
    }
    pub(super) fn check_world(&self, world: &World) -> Result<(), PublicationError> {
        self.ready.check()?;
        let now = world.world_ic_now();
        if now.clock != self.at_ic.clock || now.at < self.at_ic.at || world.now() < self.at_utc {
            return Err(PublicationError::Unavailable);
        }
        world
            .module_state::<crate::RollStore>()
            .ok_or(PublicationError::Unavailable)?
            .commerce_ready()
            .map_err(bad)?;
        match &self.mode {
            Mode::Admission(a) => a.check(world),
            Mode::RegistryRefund(_) => Ok(()),
            Mode::Service { runtime, intent } => {
                runtime.require(intent)?;
                intent.fresh(world)
            }
        }
    }
}

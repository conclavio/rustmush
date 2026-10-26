//! Closed personal charter title consideration. No term or source mutation.
pub(crate) mod audit;
pub(crate) mod economic;
pub(crate) mod history;
pub(crate) mod protocol;
mod provider;
pub(crate) mod records;
#[cfg(test)]
mod tests;
use crate::commerce::{CharterId, records::PublicationError, roots::RequiredRoot};
use mush_core::mechanics::{DefinitionRegistry, OwnerId};
use mush_core::participant::OwnerPrepareContext;
use mush_core::{CommitToken, DefinitionRef, ModError, ObjId, World};
use records::*;
use std::sync::{Arc, OnceLock};

fn bad<T>(_: T) -> PublicationError {
    PublicationError::Mismatch
}
#[derive(Clone, Default)]
pub(crate) struct Readiness(Arc<OnceLock<DefinitionRegistry>>);
impl Readiness {
    pub(crate) fn capture(&self, ctx: &mush_core::ModuleCtx<'_>) -> Result<(), ModError> {
        let defs = ctx
            .reg
            .definitions()
            .ok_or_else(|| ModError::msg("charter transfer definitions missing"))?;
        self.0
            .set(defs.clone())
            .map_err(|_| ModError::msg("charter transfer readiness already captured"))
    }
    pub(crate) fn check(&self) -> Result<(), PublicationError> {
        let defs = self.0.get().ok_or(PublicationError::Unavailable)?;
        for owner in ["roll", "economy"] {
            defs.require_owner_ready(&OwnerId::new(owner).map_err(bad)?)
                .map_err(bad)?;
        }
        Ok(())
    }
}

/// Only the actual Roll provider can create one of these private requests.
pub(crate) struct TransferRoots {
    commit: CommitToken,
    old: Vec<RequiredRoot>,
    added: RequiredRoot,
    fence: TransferFence,
}
impl TransferRoots {
    pub(crate) fn commit(&self) -> CommitToken {
        self.commit
    }
    pub(crate) fn old(&self) -> &[RequiredRoot] {
        &self.old
    }
    pub(crate) fn added(&self) -> &RequiredRoot {
        &self.added
    }
    pub(crate) fn fence(&self) -> &TransferFence {
        &self.fence
    }
}
#[derive(Clone)]
pub(crate) struct TransferFence {
    action: DefinitionRef,
    commit: CommitToken,
    actor: ObjId,
    parameters: serde_json::Value,
    charter: CharterId,
    seller: ObjId,
    buyer: ObjId,
    title: mush_economy::EconomicAsset,
    payout: mush_economy::Account,
    at_ic: mush_core::ClockStamp,
    at_utc: i64,
    deadline: i64,
    claims: Option<Option<(u64, u64)>>,
    readiness: Readiness,
}
impl TransferFence {
    pub(crate) fn check(&self, c: &OwnerPrepareContext<'_>) -> Result<(), PublicationError> {
        self.readiness.check()?;
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
        self.check_world(c.world())
    }
    fn check_world(&self, w: &World) -> Result<(), PublicationError> {
        self.readiness.check()?;
        let now = w.world_ic_now();
        if now.clock != self.at_ic.clock
            || now.at < self.at_ic.at
            || w.now() < self.at_utc
            || w.now() >= self.deadline
            || [self.seller, self.buyer].into_iter().any(|actor| {
                w.get(actor).is_none_or(|o| {
                    o.kind != mush_core::ObjKind::Player
                        || o.flags.contains(mush_core::Flags::GOING)
                })
            })
        {
            return Err(PublicationError::Mismatch);
        }
        crate::commerce::owner_qualification::current_title(w, self.seller, self.charter)?;
        let e = w
            .module_state::<mush_economy::EconomyStore>()
            .ok_or(PublicationError::Unavailable)?;
        if e.is_frozen()
            || e.snapshot().asset(self.charter.object()) != Some(&self.title)
            || e.snapshot().account(self.payout.id) != Some(&self.payout)
            || e.snapshot()
                .instrument(&self.payout.instrument)
                .is_none_or(|i| i.status != mush_economy::InstrumentStatus::Active)
        {
            return Err(PublicationError::Mismatch);
        }
        if let Some(expected) = self.claims {
            let roll = w
                .module_state::<crate::RollStore>()
                .ok_or(PublicationError::Unavailable)?;
            if roll
                .charter_transfer_claim_stamp(self.charter)
                .map_err(bad)?
                != expected
            {
                return Err(PublicationError::Mismatch);
            }
        }
        Ok(())
    }
}
pub(crate) use provider::register;

/// Explicit optional composition; default worlds declare no transfer actions.
#[derive(Clone, Debug, Default)]
pub struct CharterTransferConfig;
impl CharterTransferConfig {
    pub fn new() -> Self {
        Self
    }
    pub fn actions(&self) -> Result<[DefinitionRef; 2], ModError> {
        Ok([protocol::offer_action()?, protocol::accept_action()?])
    }
}

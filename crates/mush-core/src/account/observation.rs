//! Retained account observations for composed transactions. The real identity
//! backend captures under its map lock; later checks use core-owned latches and
//! never invoke an account oracle after writer acknowledgement.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use super::{AccountId, AccountIdentity};
use crate::action::{ActionInvocation, PrincipalId};
use crate::participant::ParticipantError;
use crate::{Flags, ObjId, ObjKind, World};

const MAX_ACCOUNT_OBSERVATIONS: usize = 64;

/// Identity-backend generation. Call `invalidate` while holding the same write
/// lock as every map mutation, before changing the map. Capture accounts under its
/// read lock. This mechanism is not an account oracle or a commerce authority.
#[derive(Debug, Default)]
pub struct AccountIdentityEpoch(Arc<AtomicU64>);

impl AccountIdentityEpoch {
    /// Invalidate every outstanding observation. Exhaustion permanently refuses
    /// new observations rather than wrapping and reviving an old one.
    pub fn invalidate(&self) {
        let _ = self
            .0
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                Some(value.saturating_add(1))
            });
    }

    fn capture(&self) -> Option<EpochCapture> {
        let value = self.0.load(Ordering::SeqCst);
        (value != u64::MAX).then(|| EpochCapture {
            epoch: Arc::clone(&self.0),
            value,
        })
    }

    /// Capture one actual account while the backend's map read lock is held.
    /// Unknown accounts must be refused by the backend before this call.
    #[must_use]
    pub fn personal_account(
        &self,
        requester: ObjId,
        account: AccountId,
    ) -> Option<PersonalAccountSnapshot> {
        Some(PersonalAccountSnapshot {
            requester,
            account,
            epoch: self.capture()?,
        })
    }

    /// Capture one actual pair while the backend's map read lock is held.
    /// Unknown accounts must be refused by the backend before this call.
    #[must_use]
    pub fn distinct_pair(
        &self,
        requester: ObjId,
        requester_account: AccountId,
        other: ObjId,
        other_account: AccountId,
    ) -> Option<AccountPairSnapshot> {
        if requester == other || requester_account == other_account {
            return None;
        }
        Some(AccountPairSnapshot {
            requester,
            other,
            epoch: self.capture()?,
        })
    }
}

#[derive(Debug)]
struct EpochCapture {
    epoch: Arc<AtomicU64>,
    value: u64,
}
impl EpochCapture {
    fn current(&self) -> bool {
        self.value != u64::MAX && self.epoch.load(Ordering::SeqCst) == self.value
    }
}

/// A backend-created pair with exact map-generation custody. It has no
/// serialization, clone or raw account-id getters. Transaction preparation
/// accepts it only from the Engine's installed account oracle.
#[derive(Debug)]
pub struct AccountPairSnapshot {
    requester: ObjId,
    other: ObjId,
    epoch: EpochCapture,
}

/// A backend-created single account with exact map-generation custody.
/// Only the Engine's installed oracle can supply this to preparation. It has
/// no serialization, clone or raw account-id getter.
#[derive(Debug)]
pub struct PersonalAccountSnapshot {
    requester: ObjId,
    account: AccountId,
    epoch: EpochCapture,
}

/// The actual invocation actor's captured personal account. This preparation
/// retains its backend and oracle-placement fences; this value is not a
/// transferable permission or a substitute for those retained checks.
/// A trusted participant may compare its independently resolved account, but
/// cannot extract or serialize the captured identity.
#[derive(Debug)]
pub struct PersonalAccountObservation {
    requester: ObjId,
    account: AccountId,
    preparation: crate::CommitToken,
}
impl PersonalAccountObservation {
    #[must_use]
    pub const fn requester(&self) -> ObjId {
        self.requester
    }
    #[must_use]
    pub fn matches_account(&self, account: AccountId) -> bool {
        self.account == account
    }
    /// Require the actual current owner-preparation token before using this
    /// proof. Keeping an unused proof cannot capture a later preparation.
    #[must_use]
    pub fn matches_preparation(&self, preparation: crate::CommitToken) -> bool {
        self.preparation == preparation
    }
}

/// An observation of the actual invocation actor and a different personal
/// account. It is not a transferable permission and cannot be deserialized.
#[derive(Debug)]
pub struct DistinctPersonalAccounts {
    requester: ObjId,
    other: ObjId,
}
impl DistinctPersonalAccounts {
    #[must_use]
    pub const fn requester(&self) -> ObjId {
        self.requester
    }
    #[must_use]
    pub const fn other(&self) -> ObjId {
        self.other
    }
}

/// Engine-owned placement fence. A replacement oracle invalidates captures
/// even if the replacement happens to return the same account values.
#[derive(Default)]
pub(crate) struct AccountOracleBinding {
    oracle: Option<Arc<dyn AccountIdentity>>,
    epoch: AccountIdentityEpoch,
}
impl AccountOracleBinding {
    pub(crate) fn install(&mut self, oracle: Arc<dyn AccountIdentity>) {
        self.epoch.invalidate();
        self.oracle = Some(oracle);
    }
    pub(crate) fn as_ref(&self) -> Option<&Arc<dyn AccountIdentity>> {
        self.oracle.as_ref()
    }
}

#[derive(Debug)]
struct Observation {
    requester: ObjId,
    other: Option<ObjId>,
    backend: EpochCapture,
    binding: EpochCapture,
}

#[derive(Debug, Default)]
pub(crate) struct AccountObservations(Vec<Observation>);
impl AccountObservations {
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub(crate) fn require_current(&self) -> Result<(), ParticipantError> {
        if self
            .0
            .iter()
            .all(|row| row.backend.current() && row.binding.current())
        {
            Ok(())
        } else {
            Err(ParticipantError::Authority)
        }
    }
    pub(crate) fn recheck(&self, world: &World) -> Result<(), ParticipantError> {
        self.require_current()?;
        if self.0.iter().all(|row| {
            personal(world, row.requester) && row.other.is_none_or(|other| personal(world, other))
        }) {
            Ok(())
        } else {
            Err(ParticipantError::Authority)
        }
    }
}

fn personal(world: &World, id: ObjId) -> bool {
    world.get(id).is_some_and(|object| {
        object.kind == ObjKind::Player && !object.flags.contains(Flags::GOING)
    })
}

pub(crate) struct AccountPrepareContext<'a> {
    binding: &'a AccountOracleBinding,
    world: &'a World,
    requester: ObjId,
    personal_principal: bool,
    observations: &'a mut AccountObservations,
}
impl<'a> AccountPrepareContext<'a> {
    pub(crate) fn new(
        binding: &'a AccountOracleBinding,
        world: &'a World,
        invocation: &ActionInvocation,
        observations: &'a mut AccountObservations,
    ) -> Self {
        Self {
            binding,
            world,
            requester: invocation.actor(),
            personal_principal: invocation.principal()
                == &PrincipalId::Object {
                    id: invocation.actor(),
                },
            observations,
        }
    }
    pub(crate) fn distinct(
        &mut self,
        other: ObjId,
    ) -> Result<DistinctPersonalAccounts, ParticipantError> {
        if !self.personal_principal
            || self.requester == other
            || !personal(self.world, self.requester)
            || !personal(self.world, other)
            || self.observations.0.len() >= MAX_ACCOUNT_OBSERVATIONS
        {
            return Err(ParticipantError::Authority);
        }
        let binding = self
            .binding
            .epoch
            .capture()
            .ok_or(ParticipantError::Authority)?;
        let pair = self
            .binding
            .as_ref()
            .ok_or(ParticipantError::Unavailable)?
            .observe_distinct_accounts(self.requester, other)
            .ok_or(ParticipantError::Authority)?;
        if pair.requester != self.requester
            || pair.other != other
            || !pair.epoch.current()
            || !binding.current()
        {
            return Err(ParticipantError::Authority);
        }
        self.observations.0.push(Observation {
            requester: pair.requester,
            other: Some(pair.other),
            backend: pair.epoch,
            binding,
        });
        self.observations.recheck(self.world)?;
        Ok(DistinctPersonalAccounts {
            requester: self.requester,
            other,
        })
    }

    pub(crate) fn personal_account(
        &mut self,
        preparation: crate::CommitToken,
    ) -> Result<PersonalAccountObservation, ParticipantError> {
        if !self.personal_principal
            || !personal(self.world, self.requester)
            || self.observations.0.len() >= MAX_ACCOUNT_OBSERVATIONS
        {
            return Err(ParticipantError::Authority);
        }
        let binding = self
            .binding
            .epoch
            .capture()
            .ok_or(ParticipantError::Authority)?;
        let captured = self
            .binding
            .as_ref()
            .ok_or(ParticipantError::Unavailable)?
            .capture_personal_account(self.requester)
            .ok_or(ParticipantError::Authority)?;
        if captured.requester != self.requester || !captured.epoch.current() || !binding.current() {
            return Err(ParticipantError::Authority);
        }
        self.observations.0.push(Observation {
            requester: captured.requester,
            other: None,
            backend: captured.epoch,
            binding,
        });
        self.observations.recheck(self.world)?;
        Ok(PersonalAccountObservation {
            requester: self.requester,
            account: captured.account,
            preparation,
        })
    }
}

#[cfg(test)]
mod tests;

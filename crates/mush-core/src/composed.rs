//! One retained installation for typed and registered owner post-images.
//!
//! This is the synchronous half of the existing authoritative writer protocol.
//! It adds neither a second receipt nor a callback after acknowledgement.

use crate::World;
use crate::action_state::{
    ActionStateError, ActionStateStore, PreparedInstallation, PreparedTxn, StateFamilyId,
};
use crate::authoritative::{
    AuthoritativeBundleError, MAX_AUTHORITATIVE_BUNDLE_BYTES, ReplayWrite,
    canonicalize_replay_writes,
};
use crate::mechanics::{OwnerId, canonical_json_bytes_with_limit};
use crate::participant::{OwnerImages, ParticipantError};
use smol_str::SmolStr;

/// A complete immutable union, with the typed reservation retaining every owner
/// patch until it is either refused before custody or installed after one ack.
#[derive(Debug)]
#[must_use]
pub struct PreparedComposedTxn {
    typed: PreparedTxn,
    owners: OwnerImages,
    shell: Option<crate::core_output::image::BuiltinObjects>,
    reads: crate::read_service::ReadObservations,
    accounts: crate::account::observation::AccountObservations,
    writes: Vec<ReplayWrite>,
}

#[derive(Debug, thiserror::Error)]
pub enum ComposedPrepareError {
    #[error("registered owner preparation failed: {0}")]
    Participant(#[from] ParticipantError),
    #[error("the complete replay write union is invalid: {0}")]
    Union(#[from] AuthoritativeBundleError),
    #[error("the complete replay write union exceeds its encoding bound")]
    Encoding,
}

impl PreparedComposedTxn {
    pub(crate) fn new(
        typed: PreparedTxn,
        mut owners: OwnerImages,
        shell: Option<crate::core_output::image::BuiltinObjects>,
        reads: crate::read_service::ReadObservations,
        accounts: crate::account::observation::AccountObservations,
    ) -> Result<Self, ComposedPrepareError> {
        if shell.as_ref().is_some_and(|shell| shell.creates_object()) && !owners.is_empty() {
            return Err(ParticipantError::Authority.into());
        }
        owners.order();
        let mut writes = typed.authoritative_replay_writes();
        writes.extend(owners.writes().cloned());
        if let Some(shell) = &shell {
            writes.extend(shell.writes().iter().cloned());
        }
        // Reuse the writer's actual physical-key identity. In particular a
        // LedgerChunkSet and ModuleJsonSet may address the same stored key.
        canonicalize_replay_writes(&mut writes, false)?;
        let value = serde_json::to_value(&writes).map_err(|_| ComposedPrepareError::Encoding)?;
        canonical_json_bytes_with_limit(&value, MAX_AUTHORITATIVE_BUNDLE_BYTES)
            .map_err(|_| ComposedPrepareError::Encoding)?;
        Ok(Self {
            typed,
            owners,
            shell,
            reads,
            accounts,
            writes,
        })
    }

    #[must_use]
    pub fn owner(&self) -> &OwnerId {
        self.typed.owner()
    }

    #[must_use]
    pub fn authoritative_replay_writes(&self) -> Vec<ReplayWrite> {
        self.writes.clone()
    }

    pub(crate) fn recheck(&self, world: &World) -> Result<(), ParticipantError> {
        self.reads.recheck(world)?;
        self.accounts.recheck(world)?;
        self.owners.recheck(world)?;
        if let Some(shell) = &self.shell {
            shell.recheck(world)?;
        }
        self.reads.recheck(world)?;
        self.accounts.recheck(world)
    }

    /// Only the old typed-only registry wrappers may unwrap this. A provider
    /// cannot discard registered owner images and keep its typed writes.
    pub(crate) fn into_typed(self) -> Result<PreparedTxn, ParticipantError> {
        if !self.owners.is_empty()
            || self.shell.is_some()
            || !self.reads.is_empty()
            || !self.accounts.is_empty()
        {
            return Err(ParticipantError::Authority);
        }
        Ok(self.typed)
    }

    /// Moving to installation marks the existing typed reservation as possibly
    /// in writer custody. Dropping the returned installation therefore fails
    /// stop exactly like the pre-existing typed installation.
    pub fn into_parts(self) -> (Vec<ReplayWrite>, ComposedInstallation) {
        let (_, typed) = self.typed.into_parts();
        (
            self.writes,
            ComposedInstallation {
                typed,
                owners: self.owners,
                shell: self.shell,
                reads: self.reads,
                accounts: self.accounts,
            },
        )
    }
}

impl From<PreparedTxn> for PreparedComposedTxn {
    fn from(typed: PreparedTxn) -> Self {
        // Existing typed preparation already guarantees canonical unique keys;
        // preserve its exact zero-participant bytes and refusal behavior.
        let writes = typed.authoritative_replay_writes();
        Self {
            typed,
            owners: OwnerImages::default(),
            shell: None,
            reads: crate::read_service::ReadObservations::default(),
            accounts: crate::account::observation::AccountObservations::default(),
            writes,
        }
    }
}

/// Private owner patches and typed reservation carried by the existing pending
/// submission. No constructor, clone, serialization or independent settlement.
#[derive(Debug)]
#[must_use = "an acknowledged composition must install or fail stop"]
pub struct ComposedInstallation {
    typed: PreparedInstallation,
    owners: OwnerImages,
    shell: Option<crate::core_output::image::BuiltinObjects>,
    reads: crate::read_service::ReadObservations,
    accounts: crate::account::observation::AccountObservations,
}

#[derive(Debug, thiserror::Error)]
pub enum ComposedInstallError {
    #[error("typed installation could not bind: {0}")]
    Typed(#[from] ActionStateError),
    #[error("registered owner installation could not bind: {0}")]
    Participant(#[from] ParticipantError),
}

impl From<PreparedInstallation> for ComposedInstallation {
    fn from(typed: PreparedInstallation) -> Self {
        Self {
            typed,
            owners: OwnerImages::default(),
            shell: None,
            reads: crate::read_service::ReadObservations::default(),
            accounts: crate::account::observation::AccountObservations::default(),
        }
    }
}

impl ComposedInstallation {
    #[must_use]
    pub fn touched_families(&self) -> Vec<(OwnerId, StateFamilyId, Vec<SmolStr>)> {
        self.typed.touched_families()
    }

    pub(crate) fn recheck(&self, world: &World) -> Result<(), ParticipantError> {
        self.reads.recheck(world)?;
        self.accounts.recheck(world)?;
        self.owners.recheck(world)?;
        if let Some(shell) = &self.shell {
            shell.recheck(world)?;
        }
        self.reads.recheck(world)?;
        self.accounts.recheck(world)
    }

    pub(crate) fn install(
        self,
        typed: &mut ActionStateStore,
        world: &mut World,
    ) -> Result<(), ComposedInstallError> {
        // Pre-submit authority was rechecked under the pending mutation barrier.
        // Post-ack binding checks owner identities/generations only; it must not
        // rerun providers, serializers or external authority/read callbacks.
        let typed_bound = typed.bind_installation(self.typed)?;
        self.reads.recheck(world)?;
        self.accounts.recheck(world)?;
        // Every owner binds before the first typed/owner apply. A later bind
        // refusal drops prior bindings without writing; the typed reservation
        // remains poisoned for the driver to fail stop after the durable ack.
        if let Some(shell) = &self.shell {
            shell.local(world)?;
        }
        let (slots, objects) = world.composed_install_parts_mut();
        let owners_bound = self.owners.bind(slots)?;
        let shell_bound = self.shell.map(|shell| shell.bind(objects)).transpose()?;
        // Owner bind callbacks may close a distinct read-service dependency.
        // Recheck its latch after the last callback and before any apply.
        self.reads.require_ready()?;
        self.accounts.require_current()?;
        let unsettled = typed_bound.apply_images();
        owners_bound.apply();
        if let Some(shell) = shell_bound {
            shell.apply();
        }
        unsettled.complete();
        Ok(())
    }

    /// Only a proven pre-custody refusal or exact durable Refused disposition
    /// may release this reservation. Integrity/unknown outcomes must retain it.
    pub(crate) fn abort(self, typed: &mut ActionStateStore) -> Result<(), ActionStateError> {
        typed.abort(self.typed)
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod boot_tests;

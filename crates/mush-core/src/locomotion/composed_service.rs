//! Actual typed handles are erased only after registration, preserving their
//! composition nonce and associated request/output types. No caller data can
//! replace an adapter or choose a movement postimage.

use super::super::{
    LocomotionAdmission,
    completion::MovementManifestLease,
    transition::{LocomotionDraft, LocomotionIntent},
};
use super::record::{MovementEffectReceipt, Record};
use super::{
    ComposedLocomotionReader, ComposedLocomotionReceipt, MAX_COMPOSED_OWNERS, MovementDraft,
    MovementIntent, SIM_PARTICIPANT, bad, exact_action, refused,
};
use crate::action_state::{ActionReadBase, ActionTxn};
use crate::participant::{
    OwnerParticipant, ParticipantError, ParticipantHandle, ParticipantPreparation,
};
use crate::{
    ActionInvocation, CommitToken, DefinitionDigest, DefinitionRef, ModError, OwnerId, Registrar,
    World,
};
use std::collections::BTreeSet;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct ComposedMovementPermit {
    pub(crate) participants: Vec<DefinitionRef>,
    pub(crate) lease: MovementManifestLease,
}
impl ComposedMovementPermit {
    fn new(
        primary: &DefinitionRef,
        lease: MovementManifestLease,
    ) -> Result<Self, ParticipantError> {
        let manifest = lease.snapshot()?;
        let participants: Vec<_> = std::iter::once(primary.clone())
            .chain(
                manifest
                    .groups()
                    .iter()
                    .map(|group| group.participant().clone()),
            )
            .collect();
        if participants.len() > MAX_COMPOSED_OWNERS
            || participants.len() < 2
            || participants.iter().collect::<BTreeSet<_>>().len() != participants.len()
        {
            return Err(refused());
        }
        Ok(Self {
            participants,
            lease,
        })
    }
    pub(crate) fn check(&self, action: &DefinitionRef) -> Result<(), ParticipantError> {
        self.lease.check()?;
        exact_action(action)?;
        if self.lease.snapshot()?.action() != action {
            return Err(refused());
        }
        Ok(())
    }
    pub(crate) fn permits_empty(&self, participant: &DefinitionRef) -> bool {
        self.lease.snapshot().is_ok_and(|manifest| {
            manifest
                .groups()
                .iter()
                .any(|group| group.participant() == participant)
        })
    }
}

trait Primary: Send + Sync {
    fn prepare(
        &self,
        ctx: &mut ParticipantPreparation<'_>,
        commit: CommitToken,
        intent: LocomotionIntent,
        permit: &ComposedMovementPermit,
    ) -> Result<LocomotionDraft, ParticipantError>;
}
struct RegisteredPrimary<P: OwnerParticipant<Request = LocomotionIntent, Output = LocomotionDraft>>(
    ParticipantHandle<P>,
);
impl<P: OwnerParticipant<Request = LocomotionIntent, Output = LocomotionDraft>> Primary
    for RegisteredPrimary<P>
{
    fn prepare(
        &self,
        ctx: &mut ParticipantPreparation<'_>,
        commit: CommitToken,
        intent: LocomotionIntent,
        permit: &ComposedMovementPermit,
    ) -> Result<LocomotionDraft, ParticipantError> {
        ctx.prepare_composed_participant(&self.0, commit, intent, permit)
    }
}
trait PreparedOwner: Send + Sync {
    fn participant(&self) -> &DefinitionRef;
    fn prepare(
        &self,
        ctx: &mut ParticipantPreparation<'_>,
        commit: CommitToken,
        intent: MovementIntent,
        permit: &ComposedMovementPermit,
    ) -> Result<MovementDraft, ParticipantError>;
}
struct RegisteredOwner<P: OwnerParticipant<Request = MovementIntent, Output = MovementDraft>> {
    participant: DefinitionRef,
    handle: ParticipantHandle<P>,
}
impl<P: OwnerParticipant<Request = MovementIntent, Output = MovementDraft>> PreparedOwner
    for RegisteredOwner<P>
{
    fn participant(&self) -> &DefinitionRef {
        &self.participant
    }
    fn prepare(
        &self,
        ctx: &mut ParticipantPreparation<'_>,
        commit: CommitToken,
        intent: MovementIntent,
        permit: &ComposedMovementPermit,
    ) -> Result<MovementDraft, ParticipantError> {
        ctx.prepare_composed_participant(&self.handle, commit, intent, permit)
    }
}

/// Build only inside the actual Sim registration callback. An outer composition
/// can add typed owner handles without introducing dependencies between owners.
pub struct ComposedLocomotionBuilder {
    action: DefinitionRef,
    participant: DefinitionRef,
    primary: Box<dyn Primary>,
    owners: Vec<Box<dyn PreparedOwner>>,
}
impl ComposedLocomotionBuilder {
    pub fn new<P>(
        reg: &mut Registrar<'_>,
        action: DefinitionRef,
        participant: DefinitionRef,
    ) -> Result<Self, ModError>
    where
        P: OwnerParticipant<Request = LocomotionIntent, Output = LocomotionDraft>,
    {
        exact_action(&action).map_err(bad)?;
        if reg.module() != "sim"
            || participant.key().owner.as_str() != "sim"
            || participant.key().kind.as_str() != "owner_participant"
            || participant.key().id.as_str() != SIM_PARTICIPANT
            || participant.version() != 1
        {
            return Err(bad("composed locomotion requires its actual Sim primary"));
        }
        let handle = reg.request_owner_participant::<P>(action.clone())?;
        Ok(Self {
            action,
            participant,
            primary: Box::new(RegisteredPrimary(handle)),
            owners: Vec::new(),
        })
    }
    pub fn owner<P>(
        mut self,
        reg: &mut Registrar<'_>,
        participant: DefinitionRef,
        observers: Vec<DefinitionRef>,
    ) -> Result<Self, ModError>
    where
        P: OwnerParticipant<Request = MovementIntent, Output = MovementDraft>,
    {
        if reg.module() != "sim"
            || self.owners.len() + 1 >= MAX_COMPOSED_OWNERS
            || participant == self.participant
            || self
                .owners
                .iter()
                .any(|owner| owner.participant() == &participant)
        {
            return Err(bad(
                "composed locomotion owner list conflicts or exceeds its bound",
            ));
        }
        let handle = reg.request_owner_participant::<P>(self.action.clone())?;
        reg.require_moved_completion::<P>(self.action.clone(), participant.clone(), observers)?;
        self.owners.push(Box::new(RegisteredOwner {
            participant,
            handle,
        }));
        Ok(self)
    }
    pub fn finish(self, reg: &mut Registrar<'_>) -> Result<ComposedLocomotion, ModError> {
        if reg.module() != "sim" || self.owners.is_empty() {
            return Err(bad(
                "composed locomotion requires actual prepared observer owners",
            ));
        }
        let participants = self
            .owners
            .iter()
            .map(|owner| owner.participant().clone())
            .collect();
        let lease = reg.request_movement_manifest(self.action.clone(), participants)?;
        let readers: BTreeSet<OwnerId> = self
            .owners
            .iter()
            .map(|owner| owner.participant().key().owner.clone())
            .collect();
        let records = super::record::register_records(reg, &readers)?;
        Ok(ComposedLocomotion {
            inner: Arc::new(Service {
                action: self.action,
                participant: self.participant,
                primary: self.primary,
                owners: self.owners,
                lease,
                records,
            }),
        })
    }
}
struct Service {
    action: DefinitionRef,
    participant: DefinitionRef,
    primary: Box<dyn Primary>,
    owners: Vec<Box<dyn PreparedOwner>>,
    lease: MovementManifestLease,
    records: crate::StateFamilyHandle<CommitToken, Record>,
}
#[derive(Clone)]
pub struct ComposedLocomotion {
    inner: Arc<Service>,
}
impl ComposedLocomotion {
    pub fn owner_reader(
        &self,
        participant: &DefinitionRef,
    ) -> Result<super::ComposedOwnerReader, ParticipantError> {
        if !self
            .inner
            .owners
            .iter()
            .any(|owner| owner.participant() == participant)
        {
            return Err(refused());
        }
        let known_participants = self
            .inner
            .owners
            .iter()
            .map(|owner| owner.participant())
            .filter(|known| known.key().owner == participant.key().owner)
            .cloned()
            .collect();
        Ok(super::ComposedOwnerReader {
            reader: self.reader(),
            participant: participant.clone(),
            known_participants,
        })
    }
    pub fn reader(&self) -> ComposedLocomotionReader {
        ComposedLocomotionReader {
            action: self.inner.action.clone(),
            participant: self.inner.participant.clone(),
            records: self.inner.records.clone(),
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare(
        &self,
        ctx: &mut ParticipantPreparation<'_>,
        world: &World,
        invocation: &ActionInvocation,
        action: &DefinitionRef,
        commit: CommitToken,
        txn: &mut ActionTxn<'_>,
        admission: &LocomotionAdmission,
        base: &ActionReadBase,
        provider: &crate::action::ProviderRegistrationOrdinal,
    ) -> Result<
        (
            ComposedLocomotionReceipt,
            super::super::image::LocomotionObjects,
        ),
        ParticipantError,
    > {
        let service = &self.inner;
        if action != &service.action
            || invocation != &admission.binding.invocation
            || action != &admission.binding.action
            || commit != admission.binding.commit
            || txn
                .get(&service.records, &commit)
                .map_err(|_| refused())?
                .is_some()
        {
            return Err(refused());
        }
        let permit = ComposedMovementPermit::new(&service.participant, service.lease.clone())?;
        permit.check(action)?;
        txn.read_preimage(base, provider).map_err(|_| refused())?;
        let manifest = service.lease.snapshot()?.clone();
        let sim_intent =
            LocomotionIntent::new_composed(world, invocation, action, commit, admission)?;
        let sim = service.primary.prepare(ctx, commit, sim_intent, &permit)?;
        if !ctx.matches_preparation(&sim.binding, commit) {
            return Err(refused());
        }
        let mut effects = Vec::with_capacity(manifest.groups().len());
        for (index, group) in manifest.groups().iter().enumerate() {
            let owner = service
                .owners
                .iter()
                .find(|owner| owner.participant() == group.participant())
                .ok_or_else(refused)?;
            let intent = MovementIntent::new(world, invocation, action, commit, admission, group)?;
            let intent_digest = intent.digest();
            let draft = owner.prepare(ctx, commit, intent, &permit)?;
            if !ctx.matches_preparation(&draft.binding, commit)
                || draft.group_binding != *group.binding()
                || draft.intent_digest != intent_digest
            {
                return Err(refused());
            }
            let writes = ctx.composed_writes(group.participant())?;
            let writes_digest = DefinitionDigest::for_canonical_value_with_limit(
                "core.composed_locomotion.owner_writes.v1",
                &serde_json::to_value(writes).map_err(|_| ParticipantError::Encoding)?,
                crate::authoritative::MAX_AUTHORITATIVE_BUNDLE_BYTES,
            )
            .map_err(|_| ParticipantError::ImageLimit)?;
            let proof_digest = super::record::proof_digest(
                &draft.group_binding,
                &draft.capture_digest,
                &draft.proof,
            )?;
            effects.push(MovementEffectReceipt {
                group: u16::try_from(index).map_err(|_| refused())?,
                binding: draft.group_binding,
                participant: group.participant().clone(),
                intent_digest,
                owner_capture_digest: draft.owner_capture_digest,
                capture_digest: draft.capture_digest,
                proof: draft.proof,
                proof_digest,
                writes_digest,
            });
        }
        ctx.require_composed_images(&permit, action, commit)?;
        let mut image = super::super::image::LocomotionObjects::prepare(
            world,
            admission,
            &sim.dependencies,
            service.participant.clone(),
            action.clone(),
            commit,
        )?;
        image.composed = Some(permit);
        let row = Record {
            format: 1,
            commit,
            action: action.clone(),
            participant: service.participant.clone(),
            actor: admission.actor(),
            source: admission.source(),
            exit: admission.exit(),
            destination: admission.destination(),
            prepared_at: sim.prepared_at,
            at_utc: sim.at_utc,
            plan_digest: sim.plan_digest,
            capture_digest: sim.capture_digest,
            manifest: serde_json::to_value(&manifest).map_err(|_| ParticipantError::Encoding)?,
            manifest_digest: manifest.digest().map_err(|_| refused())?,
            effects,
            objects: image.digests()?,
        };
        row.validate()?;
        txn.insert(&service.records, &commit, row.clone())
            .map_err(|_| refused())?;
        Ok((ComposedLocomotionReceipt { row, manifest }, image))
    }
}

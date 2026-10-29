//! Actual typed handles are erased only after registration, preserving their
//! composition nonce and associated request/output types. No caller data can
//! replace an adapter or choose a movement postimage.

use super::super::{
    LocomotionAdmission,
    completion::MovementManifestLease,
    transition::{LocomotionDraft, LocomotionIntent},
};
use super::record::{MovementEffectReceipt, Record, Records};
use super::{
    ComposedLocomotionReader, ComposedLocomotionReceipt, MAX_COMPOSED_OWNERS, MovementDraft,
    MovementIntent, SIM_PARTICIPANT, WindowLocomotionDraft, WindowLocomotionIntent,
    WindowProjection, bad, exact_action, exact_composition_action, exact_window_action, refused,
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
        exact_composition_action(action)?;
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

struct PrimaryContext<'a> {
    world: &'a World,
    invocation: &'a ActionInvocation,
    action: &'a DefinitionRef,
    commit: CommitToken,
    admission: &'a LocomotionAdmission,
}
struct PreparedPrimary {
    base: LocomotionDraft,
    window: Option<Box<WindowProjection>>,
}
trait Primary: Send + Sync {
    fn prepare(
        &self,
        ctx: &mut ParticipantPreparation<'_>,
        input: &PrimaryContext<'_>,
        permit: &ComposedMovementPermit,
    ) -> Result<PreparedPrimary, ParticipantError>;
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
        input: &PrimaryContext<'_>,
        permit: &ComposedMovementPermit,
    ) -> Result<PreparedPrimary, ParticipantError> {
        let intent = LocomotionIntent::new_composed(
            input.world,
            input.invocation,
            input.action,
            input.commit,
            input.admission,
        )?;
        let base = ctx.prepare_composed_participant(&self.0, input.commit, intent, permit)?;
        Ok(PreparedPrimary { base, window: None })
    }
}
struct RegisteredWindowPrimary<
    P: OwnerParticipant<Request = WindowLocomotionIntent, Output = WindowLocomotionDraft>,
>(ParticipantHandle<P>);
impl<P: OwnerParticipant<Request = WindowLocomotionIntent, Output = WindowLocomotionDraft>> Primary
    for RegisteredWindowPrimary<P>
{
    fn prepare(
        &self,
        ctx: &mut ParticipantPreparation<'_>,
        input: &PrimaryContext<'_>,
        permit: &ComposedMovementPermit,
    ) -> Result<PreparedPrimary, ParticipantError> {
        let intent = WindowLocomotionIntent::new(
            input.world,
            input.invocation,
            input.action,
            input.commit,
            input.admission,
        )?;
        let output = ctx.prepare_composed_participant(&self.0, input.commit, intent, permit)?;
        Ok(PreparedPrimary {
            base: output.base,
            window: Some(Box::new(output.projection)),
        })
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
    window: bool,
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
            window: false,
        })
    }
    pub fn new_window<P>(
        reg: &mut Registrar<'_>,
        action: DefinitionRef,
        participant: DefinitionRef,
    ) -> Result<Self, ModError>
    where
        P: OwnerParticipant<Request = WindowLocomotionIntent, Output = WindowLocomotionDraft>,
    {
        exact_window_action(&action).map_err(bad)?;
        if reg.module() != "sim"
            || participant.key().owner.as_str() != "sim"
            || participant.key().kind.as_str() != "owner_participant"
            || participant.key().id.as_str() != SIM_PARTICIPANT
            || participant.version() != 2
        {
            return Err(bad("window locomotion requires its actual v2 Sim primary"));
        }
        let handle = reg.request_owner_participant::<P>(action.clone())?;
        Ok(Self {
            action,
            participant,
            primary: Box::new(RegisteredWindowPrimary(handle)),
            owners: Vec::new(),
            window: true,
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
        let records = super::record::register_records(reg, &readers, self.window)?;
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
    records: Records,
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
    /// Read explicitly retained older versions of this actual owner's same
    /// participant key. Retained definitions and the original manifest still
    /// validate each receipt; this grants no historical adapter execution.
    /// The existing owner bound also bounds the total known profile list.
    pub fn owner_reader_retaining(
        &self,
        participant: &DefinitionRef,
        retained: &[DefinitionRef],
    ) -> Result<super::ComposedOwnerReader, ParticipantError> {
        let mut reader = self.owner_reader(participant)?;
        if retained.len() > MAX_COMPOSED_OWNERS {
            return Err(refused());
        }
        for profile in retained {
            if profile.key() != participant.key()
                || profile.version() >= participant.version()
                || reader.known_participants.contains(profile)
                || reader.known_participants.len() >= MAX_COMPOSED_OWNERS
            {
                return Err(refused());
            }
            reader.known_participants.push(profile.clone());
        }
        Ok(reader)
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
            || service.records.contains(txn, &commit)?
        {
            return Err(refused());
        }
        let permit = ComposedMovementPermit::new(&service.participant, service.lease.clone())?;
        permit.check(action)?;
        txn.read_preimage(base, provider).map_err(|_| refused())?;
        let manifest = service.lease.snapshot()?.clone();
        let PreparedPrimary { base: sim, window } = service.primary.prepare(
            ctx,
            &PrimaryContext {
                world,
                invocation,
                action,
                commit,
                admission,
            },
            &permit,
        )?;
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
            let intent = match window.as_deref() {
                None => MovementIntent::new(world, invocation, action, commit, admission, group)?,
                Some(window) => MovementIntent::new_window(
                    world, invocation, action, commit, admission, group, window,
                )?,
            };
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
        let mut image = if let Some(window) = window.as_deref() {
            super::super::image::LocomotionObjects::prepare_window(
                world,
                admission,
                &sim.dependencies,
                window.projected_room(),
                service.participant.clone(),
                action.clone(),
                commit,
            )?
        } else {
            super::super::image::LocomotionObjects::prepare(
                world,
                admission,
                &sim.dependencies,
                service.participant.clone(),
                action.clone(),
                commit,
            )?
        };
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
        row.validate_for(window.as_deref())?;
        service
            .records
            .insert(txn, &commit, &row, window.as_deref())?;
        Ok((
            ComposedLocomotionReceipt {
                row,
                manifest,
                window,
            },
            image,
        ))
    }
}

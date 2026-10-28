//! Closed structural Sim transition. Core constructs the object projections;
//! the registered Sim participant owns the spatial patch in the same commit.
//! This receipt is not personnel presence, embarkation, or activation authority.
pub(crate) mod image;

use crate::action::{ActionInvocation, PerceivedRef, PrincipalId};
use crate::action_state::{ActionTxn, StateFamilyDescriptor, StateFamilyHandle, StateFamilyId};
use crate::mechanics::{
    CanonicalId, DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef, OwnerId,
    canonical_json_bytes_with_limit,
};
use crate::participant::{
    OwnerParticipant, OwnerPreparationBinding, OwnerPrepareContext, ParticipantError,
    ParticipantHandle,
};
use crate::{CommitToken, DefinitionDigest, ModError, ModuleCtx, ObjId, Registrar, World};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::sync::Arc;

pub const ACTION: &str = "sim_physical_transition";
pub const PARTICIPANT: &str = "physical_transition";
const FAMILY: &str = "physical_transitions";
pub const MAX_PHYSICAL_MOVES: usize = 256;
pub const MAX_PHYSICAL_READ_DEPENDENCIES: usize = 512;
const MAX_OBJECTS: usize = MAX_PHYSICAL_MOVES * 3;
const IMAGE_BYTES: usize = 1_048_576;
const RECORD_BYTES: usize = 262_144;
pub const MAX_PHYSICAL_RECEIPT_SCAN: usize = 128;

pub fn action_descriptor() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new("action").map_err(bad)?,
        DefinitionId::new(ACTION).map_err(bad)?,
        2,
        json!({"format":1,"protocol":"sim_structural_personnel_capture_v2",
            "moves":MAX_PHYSICAL_MOVES,"objects":MAX_OBJECTS,
            "read_dependencies":MAX_PHYSICAL_READ_DEPENDENCIES,
            "image_bytes":IMAGE_BYTES,"receipt_bytes":RECORD_BYTES,
            "capture":{"family":"sim.personnel_crossings","required":"same_commit",
                "evidence":"uncertified"}}),
    )
    .map(DefinitionDescriptor::as_current)
    .map_err(bad)
}

/// Exact retained definition for structural receipts created before capture.
/// It is historical metadata, never the current execution contract.
pub fn action_descriptor_v1() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new("action").map_err(bad)?,
        DefinitionId::new(ACTION).map_err(bad)?,
        1,
        json!({"format":1,"protocol":"sim_structural_transition_v1",
            "moves":MAX_PHYSICAL_MOVES,"objects":MAX_OBJECTS,
            "read_dependencies":MAX_PHYSICAL_READ_DEPENDENCIES,
            "image_bytes":IMAGE_BYTES,"receipt_bytes":RECORD_BYTES}),
    )
    .map_err(bad)
}
fn exact_action(action: &DefinitionRef) -> Result<(), ParticipantError> {
    let descriptor = match action.version() {
        1 => action_descriptor_v1(),
        2 => action_descriptor(),
        _ => return Err(refused()),
    }
    .map_err(|_| refused())?;
    let expected = crate::DefinitionRegistryBuilder::new()
        .register(OwnerId::new("sim").map_err(|_| refused())?, descriptor)
        .map_err(|_| refused())?;
    if action != &expected {
        return Err(refused());
    }
    Ok(())
}
fn exact_participant(participant: &DefinitionRef) -> Result<(), ParticipantError> {
    if participant.key().owner.as_str() != "sim"
        || participant.key().kind.as_str() != "owner_participant"
        || participant.key().id.as_str() != PARTICIPANT
        || !matches!(participant.version(), 1 | 2)
    {
        return Err(refused());
    }
    Ok(())
}

/// A projection proposal produced by the actual spatial participant, never a
/// caller-supplied object image. Core validates the complete merged batch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhysicalMove {
    pub object: ObjId,
    pub source: Option<ObjId>,
    pub destination: ObjId,
}

/// The unforgeable preparation binding prevents transplanting a participant's
/// result to another actor, action, intent, or commit. Only Core consumes it.
pub struct PhysicalTransitionDraft {
    pub(crate) binding: OwnerPreparationBinding,
    root: ObjId,
    structural_digest: DefinitionDigest,
    moves: Vec<PhysicalMove>,
    read_dependencies: BTreeSet<ObjId>,
}
impl PhysicalTransitionDraft {
    /// Include every existing World object used by the spatial plan, including
    /// dependencies of zero-projection changes. They are fenced read preimages,
    /// not caller-authored object writes; missing or oversized dependencies refuse.
    pub fn new(
        ctx: &OwnerPrepareContext<'_>,
        root: ObjId,
        structural_digest: DefinitionDigest,
        moves: Vec<PhysicalMove>,
        read_dependencies: BTreeSet<ObjId>,
    ) -> Result<Self, ParticipantError> {
        exact_action(ctx.action())?;
        if moves.len() > MAX_PHYSICAL_MOVES
            || read_dependencies.len() > MAX_PHYSICAL_READ_DEPENDENCIES
            || ctx.world().get(root).is_none()
        {
            return Err(refused());
        }
        Ok(Self {
            binding: ctx.preparation_binding(),
            root,
            structural_digest,
            moves,
            read_dependencies,
        })
    }
}

/// Borrowed attribution supplied only while Core evaluates this exact batch's
/// real carry hooks. It exempts no hook by itself.
#[derive(Debug)]
pub struct PhysicalTransitionAttempt {
    actor: ObjId,
    root: ObjId,
    action: DefinitionRef,
    commit: CommitToken,
    moves: Vec<PhysicalMove>,
}
impl PhysicalTransitionAttempt {
    pub fn matches_projection(&self, actor: ObjId, object: ObjId, destination: ObjId) -> bool {
        self.actor == actor
            && self
                .moves
                .iter()
                .any(|m| m.object == object && m.destination == destination)
    }
    /// MoveAttempt receives the actual moving actor, independently of the
    /// authenticated operator who requested this structural transition.
    pub fn matches_actor_projection(&self, actor: ObjId, destination: ObjId) -> bool {
        self.moves
            .iter()
            .any(|m| m.object == actor && m.destination == destination)
    }
    pub const fn root(&self) -> ObjId {
        self.root
    }
    pub const fn action(&self) -> &DefinitionRef {
        &self.action
    }
    pub const fn commit(&self) -> CommitToken {
        self.commit
    }
    pub(crate) fn require_move(
        &self,
        actor: ObjId,
        object: ObjId,
        destination: ObjId,
    ) -> Result<(), ParticipantError> {
        exact_action(&self.action)?;
        if !self.matches_projection(actor, object, destination) {
            return Err(refused());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhysicalObjectDigest {
    pub object: ObjId,
    pub before: DefinitionDigest,
    pub after: DefinitionDigest,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PhysicalRecord {
    format: u16,
    commit: CommitToken,
    action: DefinitionRef,
    participant: DefinitionRef,
    actor: ObjId,
    root: ObjId,
    structural_digest: DefinitionDigest,
    at_utc: i64,
    objects: Vec<PhysicalObjectDigest>,
}
impl PhysicalRecord {
    fn validate(&self) -> Result<(), ParticipantError> {
        exact_action(&self.action)?;
        exact_participant(&self.participant)?;
        if self.format != 1
            || self.action.version() != self.participant.version()
            || self.objects.len() > MAX_OBJECTS
            || self.objects.windows(2).any(|w| w[0].object >= w[1].object)
        {
            return Err(refused());
        }
        bounded(self)?;
        Ok(())
    }
}
/// An exact prepared or hydrated structural receipt. No personnel evidence is
/// inferred from these object digests, including transitions with no projections.
#[derive(Clone, Debug)]
pub struct PhysicalTransitionReceipt {
    row: PhysicalRecord,
}
impl PhysicalTransitionReceipt {
    pub const fn commit(&self) -> CommitToken {
        self.row.commit
    }
    pub const fn root(&self) -> ObjId {
        self.row.root
    }
    pub const fn actor(&self) -> ObjId {
        self.row.actor
    }
    pub const fn action(&self) -> &DefinitionRef {
        &self.row.action
    }
    pub const fn participant(&self) -> &DefinitionRef {
        &self.row.participant
    }
    pub const fn at_utc(&self) -> i64 {
        self.row.at_utc
    }
    pub const fn structural_digest(&self) -> &DefinitionDigest {
        &self.row.structural_digest
    }
    pub fn object_digests(&self) -> &[PhysicalObjectDigest] {
        &self.row.objects
    }
}

/// A bounded page of receipts checked against their canonical keys and exact
/// registered historical definitions. It exposes no mutation capability.
#[derive(Debug)]
pub struct PhysicalTransitionReceiptPage {
    entries: Vec<PhysicalTransitionReceipt>,
    next_after: Option<String>,
}
impl PhysicalTransitionReceiptPage {
    pub fn entries(&self) -> &[PhysicalTransitionReceipt] {
        &self.entries
    }
    pub fn next_after(&self) -> Option<&str> {
        self.next_after.as_deref()
    }
}

pub struct CorePhysicalTransitions<P: OwnerParticipant<Output = PhysicalTransitionDraft>> {
    pub(crate) participant: DefinitionRef,
    pub(crate) handle: ParticipantHandle<P>,
    action: DefinitionRef,
    retained: Vec<(DefinitionRef, DefinitionRef)>,
    records: StateFamilyHandle<CommitToken, PhysicalRecord>,
}
impl<P: OwnerParticipant<Output = PhysicalTransitionDraft>> Clone for CorePhysicalTransitions<P> {
    fn clone(&self) -> Self {
        Self {
            participant: self.participant.clone(),
            handle: self.handle.clone(),
            action: self.action.clone(),
            retained: self.retained.clone(),
            records: self.records.clone(),
        }
    }
}
impl<P: OwnerParticipant<Output = PhysicalTransitionDraft>> CorePhysicalTransitions<P> {
    pub fn register(
        reg: &mut Registrar<'_>,
        action: DefinitionRef,
        participant: DefinitionRef,
    ) -> Result<Self, ModError> {
        Self::register_with_history(reg, action, participant, Vec::new())
    }

    /// Register the current closed service and, when needed, the one exact
    /// retained v1 action/participant pair. Historical definitions must also be
    /// registered by the owner; they do not gain an executable participant.
    pub fn register_with_history(
        reg: &mut Registrar<'_>,
        action: DefinitionRef,
        participant: DefinitionRef,
        retained: Vec<(DefinitionRef, DefinitionRef)>,
    ) -> Result<Self, ModError> {
        exact_action(&action).map_err(bad)?;
        exact_participant(&participant).map_err(bad)?;
        if reg.module() != "sim" || action.version() != 2 || participant.version() != 2 {
            return Err(bad("physical transition requires the current Sim contract"));
        }
        if retained.len() > 1 {
            return Err(bad(
                "physical transition history exceeds supported versions",
            ));
        }
        for (old_action, old_participant) in &retained {
            exact_action(old_action).map_err(bad)?;
            exact_participant(old_participant).map_err(bad)?;
            if old_action.version() != 1 || old_participant.version() != 1 {
                return Err(bad(
                    "physical transition history requires the exact v1 pair",
                ));
            }
        }
        let handle = reg.request_owner_participant::<P>(action.clone())?;
        let id = |suffix| CanonicalId::new(format!("sim.{FAMILY}.{suffix}.v1")).map_err(bad);
        let descriptor = StateFamilyDescriptor::new(
            StateFamilyId::new(FAMILY).map_err(bad)?,
            id("key")?,
            id("value")?,
            1,
            id("decode")?,
            id("validate")?,
            id("canonical")?,
        )
        .and_then(|d| d.with_limits(128, RECORD_BYTES))
        .map_err(bad)?;
        let records = reg.state_family(
            descriptor,
            |key: &CommitToken| Ok(key.to_string()),
            |value: &Value| decode(value),
            |row: &PhysicalRecord| row.validate().map_err(|e| e.to_string()),
            |row: &PhysicalRecord| bounded(row).map_err(|e| e.to_string()),
        )?;
        Ok(Self {
            participant,
            handle,
            action,
            retained,
            records,
        })
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare(
        &self,
        world: &World,
        invocation: &ActionInvocation,
        action: &DefinitionRef,
        commit: CommitToken,
        txn: &mut ActionTxn<'_>,
        draft: PhysicalTransitionDraft,
        hooks: Arc<[crate::module::HookReg]>,
        move_hooks: Arc<[crate::module::HookReg]>,
        base: &crate::action_state::ActionReadBase,
        provider: &crate::action::ProviderRegistrationOrdinal,
    ) -> Result<(PhysicalTransitionReceipt, image::PhysicalTransitionObjects), ParticipantError>
    {
        if action != &self.action
            || invocation.principal()
                != &(PrincipalId::Object {
                    id: invocation.actor(),
                })
            || invocation.subject() != &PerceivedRef::SelfActor
            || !invocation.targets().is_empty()
            || txn
                .get(&self.records, &commit)
                .map_err(|_| refused())?
                .is_some()
        {
            return Err(refused());
        }
        let image = image::PhysicalTransitionObjects::prepare(
            world,
            txn.read_preimage(base, provider).map_err(|_| refused())?,
            invocation.actor(),
            &draft,
            hooks,
            move_hooks,
            self.participant.clone(),
            action.clone(),
            commit,
        )?;
        let row = PhysicalRecord {
            format: 1,
            commit,
            action: action.clone(),
            participant: self.participant.clone(),
            actor: invocation.actor(),
            root: draft.root,
            structural_digest: draft.structural_digest,
            at_utc: world.now(),
            objects: image.digests()?,
        };
        row.validate()?;
        txn.insert(&self.records, &commit, row.clone())
            .map_err(|_| refused())?;
        Ok((PhysicalTransitionReceipt { row }, image))
    }
    fn validate_registry(&self, ctx: &ModuleCtx<'_>) -> Result<(), ParticipantError> {
        let definitions = ctx.reg.definitions().ok_or_else(refused)?;
        for (action, participant) in std::iter::once((&self.action, &self.participant)).chain(
            self.retained
                .iter()
                .map(|(action, participant)| (action, participant)),
        ) {
            definitions.inspect_exact(action).map_err(|_| refused())?;
            definitions
                .inspect_exact(participant)
                .map_err(|_| refused())?;
        }
        Ok(())
    }

    fn validate_record(&self, key: &str, row: &PhysicalRecord) -> Result<(), ParticipantError> {
        row.validate()?;
        let known = (row.action == self.action && row.participant == self.participant)
            || self.retained.iter().any(|(action, participant)| {
                &row.action == action && &row.participant == participant
            });
        if key != row.commit.to_string() || !known {
            return Err(refused());
        }
        Ok(())
    }

    /// Read one canonical receipt through the owning module's typed-state lens.
    /// Absence returns None; a crossed key or unknown definition fails closed.
    pub fn read_receipt(
        &self,
        ctx: &ModuleCtx<'_>,
        commit: &CommitToken,
    ) -> Result<Option<PhysicalTransitionReceipt>, ParticipantError> {
        self.validate_registry(ctx)?;
        let Some(row) = ctx
            .action_state(&self.records, commit)
            .map_err(|_| refused())?
        else {
            return Ok(None);
        };
        self.validate_record(&commit.to_string(), row)?;
        Ok(Some(PhysicalTransitionReceipt { row: row.clone() }))
    }

    /// Scan a bounded canonical page for cross-family boot audits. This validates
    /// historical metadata, not readiness or personnel-presence authority.
    pub fn scan_receipts(
        &self,
        ctx: &ModuleCtx<'_>,
        after: Option<&str>,
        limit: usize,
    ) -> Result<PhysicalTransitionReceiptPage, ParticipantError> {
        if limit == 0 || limit > MAX_PHYSICAL_RECEIPT_SCAN {
            return Err(refused());
        }
        self.validate_registry(ctx)?;
        let page = ctx
            .scan_action_state(&self.records, after, limit)
            .map_err(|_| refused())?;
        let mut entries = Vec::with_capacity(page.entries().len());
        for entry in page.entries() {
            self.validate_record(entry.encoded_key(), entry.value())?;
            entries.push(PhysicalTransitionReceipt {
                row: entry.value().clone(),
            });
        }
        Ok(PhysicalTransitionReceiptPage {
            entries,
            next_after: page.next_after().map(ToString::to_string),
        })
    }

    /// Strict key/value and exact registered-definition audit at owner boot.
    pub fn audit(&self, ctx: &ModuleCtx<'_>) -> Result<(), ParticipantError> {
        let mut cursor = None;
        let mut count = 0usize;
        loop {
            let page = self.scan_receipts(ctx, cursor.as_deref(), MAX_PHYSICAL_RECEIPT_SCAN)?;
            for _ in page.entries() {
                count = count.checked_add(1).ok_or_else(refused)?;
                if count > crate::action_state::MAX_ACTION_STATE_BOOT_RECORDS {
                    return Err(refused());
                }
            }
            cursor = page.next_after().map(ToString::to_string);
            if cursor.is_none() {
                return Ok(());
            }
        }
    }
}
fn refused() -> ParticipantError {
    ParticipantError::Authority
}
fn bad(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
fn bounded<T: Serialize>(row: &T) -> Result<Value, ParticipantError> {
    let value = serde_json::to_value(row).map_err(|_| ParticipantError::Encoding)?;
    canonical_json_bytes_with_limit(&value, RECORD_BYTES)
        .map_err(|_| ParticipantError::ImageLimit)?;
    Ok(value)
}
fn decode(value: &Value) -> Result<PhysicalRecord, String> {
    canonical_json_bytes_with_limit(value, RECORD_BYTES).map_err(|e| e.to_string())?;
    let row: PhysicalRecord = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
    row.validate().map_err(|e| e.to_string())?;
    if serde_json::to_value(&row).map_err(|e| e.to_string())? != *value {
        return Err("noncanonical physical receipt".into());
    }
    Ok(row)
}

#[cfg(test)]
mod tests;

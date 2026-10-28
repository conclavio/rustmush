//! Immutable captures of covered ordinary movement, never continuous aboard evidence.
//!
//! A move overlays only the actor's actual Core location. It does not create a
//! placement row or repair a canonical projection. This separate family and its
//! digest domains leave structural personnel history byte-for-byte unchanged.
use std::collections::BTreeSet;

use mush_core::locomotion::transition::{
    ACTION, CoreLocomotion, LocomotionDraft, LocomotionIntent, LocomotionReceipt, PARTICIPANT,
    action_descriptor,
};
use mush_core::participant::{OwnerParticipant, ParticipantError};
use mush_core::{
    ActionTxn, CanonicalId, ClockRef, ClockStamp, CommitToken, DefinitionDigest, DefinitionRef,
    DefinitionRegistryBuilder, ModError, ModuleCtx, ObjId, ObjKind, OwnerId, Registrar,
    StateFamilyDescriptor, StateFamilyHandle, StateFamilyId, World,
    canonical_json_bytes_with_limit,
};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};

use crate::data::SpatialState;
use crate::personnel::Crossing;
use crate::personnel_capture::CaptureFacts;

const FAMILY: &str = "locomotion_crossings";
const MAX_BYTES: usize = 128 * 1024;
const MAX_CROSSINGS: usize = 256;
const MAX_ACTORS: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ActorMove {
    pub(crate) actor: ObjId,
    pub(crate) source: Option<ObjId>,
    pub(crate) exit: Option<ObjId>,
    pub(crate) destination: ObjId,
}

fn refused() -> ParticipantError {
    ParticipantError::refused("locomotion crossing capture is unavailable or inconsistent")
}
fn bad(error: impl std::fmt::Display) -> ModError {
    ModError::msg(error.to_string())
}

/// The provider combines this route digest with its current canonical Sim
/// dependency digest. The Core receipt retains that complete plan digest.
pub(crate) fn plan_digest(route: &ActorMove) -> Result<DefinitionDigest, ParticipantError> {
    DefinitionDigest::for_canonical_value("sim.locomotion.route.v1", &json!(route))
        .map_err(|_| ParticipantError::Encoding)
}

/// Derive actual pre/post ancestry at a previously validated preparation stamp.
/// A final recheck passes the original stamp after independently requiring the
/// same current clock generation and nondecreasing time; time never rewrites a
/// prepared historical fact. Authorization and unsupported-owner checks belong
/// to the live participant, not this pure bounded derivation.
pub(crate) fn derive(
    state: &SpatialState,
    world: &World,
    route: &ActorMove,
    stamp: &ClockStamp,
) -> Result<CaptureFacts, ParticipantError> {
    let actor = world.get(route.actor).ok_or_else(refused)?;
    if !actor.kind.is_actor()
        || actor.location != route.source
        || route.actor == route.destination
        || (route.exit.is_none() && route.source == Some(route.destination))
    {
        return Err(refused());
    }
    let mut dependencies = BTreeSet::from([route.actor, route.destination]);
    dependencies.extend(route.source);
    if let Some(exit) = route.exit {
        let object = world.get(exit).ok_or_else(refused)?;
        if object.kind != ObjKind::Exit || object.destination != Some(route.destination) {
            return Err(refused());
        }
        dependencies.insert(exit);
    }
    crate::personnel_capture::derive_actor_move(
        state,
        world,
        route.actor,
        route.destination,
        stamp,
        &dependencies,
    )
}

fn check_material(stamp: &ClockStamp, crossings: &[Crossing]) -> Result<(), ParticipantError> {
    if !matches!(stamp.clock, ClockRef::WorldIc { generation } if generation > 0)
        || crossings.len() > MAX_CROSSINGS
    {
        return Err(refused());
    }
    let mut actors = BTreeSet::new();
    let mut previous = None;
    for crossing in crossings {
        let key = (crossing.character, crossing.carrier);
        if crossing.character == crossing.carrier || previous.is_some_and(|p| p >= key) {
            return Err(refused());
        }
        actors.insert(crossing.character);
        if actors.len() > MAX_ACTORS {
            return Err(refused());
        }
        previous = Some(key);
    }
    Ok(())
}

pub(crate) fn capture_digest(
    prepared_at: &ClockStamp,
    before_ancestry_digest: &DefinitionDigest,
    after_ancestry_digest: &DefinitionDigest,
    crossings: &[Crossing],
) -> Result<DefinitionDigest, ParticipantError> {
    check_material(prepared_at, crossings)?;
    let value = json!({"prepared_at":prepared_at,
        "before_ancestry_digest":before_ancestry_digest,
        "after_ancestry_digest":after_ancestry_digest,"crossings":crossings});
    canonical_json_bytes_with_limit(&value, MAX_BYTES).map_err(|_| ParticipantError::ImageLimit)?;
    DefinitionDigest::for_canonical_value("sim.locomotion.capture.v1", &value)
        .map_err(|_| ParticipantError::Encoding)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Coverage {
    UncertifiedLegacyPaths,
}

fn required_optional<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(deserializer)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CrossingBatch {
    format: u16,
    commit: CommitToken,
    action: DefinitionRef,
    participant: DefinitionRef,
    actor: ObjId,
    #[serde(deserialize_with = "required_optional")]
    source: Option<ObjId>,
    #[serde(deserialize_with = "required_optional")]
    exit: Option<ObjId>,
    destination: ObjId,
    prepared_at: ClockStamp,
    at_utc: i64,
    plan_digest: DefinitionDigest,
    capture_digest: DefinitionDigest,
    before_ancestry_digest: DefinitionDigest,
    after_ancestry_digest: DefinitionDigest,
    coverage: Coverage,
    crossings: Vec<Crossing>,
}

fn exact_definitions(
    action: &DefinitionRef,
    participant: &DefinitionRef,
) -> Result<(), ParticipantError> {
    let expected = DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new("sim").map_err(|_| refused())?,
            action_descriptor().map_err(|_| refused())?,
        )
        .map_err(|_| refused())?;
    if action != &expected
        || action.key().id.as_str() != ACTION
        || participant.key().owner.as_str() != "sim"
        || participant.key().kind.as_str() != "owner_participant"
        || participant.key().id.as_str() != PARTICIPANT
        || participant.version() != 1
    {
        return Err(refused());
    }
    Ok(())
}

impl CrossingBatch {
    fn validate(&self) -> Result<(), ParticipantError> {
        exact_definitions(&self.action, &self.participant)?;
        if self.format != 1
            || self.actor == self.destination
            || (self.exit.is_none() && self.source == Some(self.destination))
            || self.capture_digest
                != capture_digest(
                    &self.prepared_at,
                    &self.before_ancestry_digest,
                    &self.after_ancestry_digest,
                    &self.crossings,
                )?
        {
            return Err(refused());
        }
        encoded(self)?;
        Ok(())
    }

    fn matches_receipt(&self, receipt: &LocomotionReceipt) -> Result<(), ParticipantError> {
        self.validate()?;
        if self.commit != receipt.commit()
            || &self.action != receipt.action()
            || &self.participant != receipt.participant()
            || self.actor != receipt.actor()
            || self.source != receipt.source()
            || self.exit != receipt.exit()
            || self.destination != receipt.destination()
            || &self.prepared_at != receipt.prepared_at()
            || self.at_utc != receipt.at_utc()
            || &self.plan_digest != receipt.plan_digest()
            || &self.capture_digest != receipt.capture_digest()
        {
            return Err(refused());
        }
        Ok(())
    }
}

fn encoded(batch: &CrossingBatch) -> Result<Value, ParticipantError> {
    if batch.crossings.len() > MAX_CROSSINGS {
        return Err(ParticipantError::ImageLimit);
    }
    let value = serde_json::to_value(batch).map_err(|_| ParticipantError::Encoding)?;
    canonical_json_bytes_with_limit(&value, MAX_BYTES).map_err(|_| ParticipantError::ImageLimit)?;
    Ok(value)
}
fn decode(value: &Value) -> Result<CrossingBatch, String> {
    canonical_json_bytes_with_limit(value, MAX_BYTES).map_err(|e| e.to_string())?;
    let batch: CrossingBatch = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
    batch.validate().map_err(|e| e.to_string())?;
    if encoded(&batch).map_err(|e| e.to_string())? != *value {
        return Err("noncanonical locomotion crossing capture".into());
    }
    Ok(batch)
}

#[derive(Clone)]
pub(crate) struct LocomotionCrossings {
    records: StateFamilyHandle<CommitToken, CrossingBatch>,
}
impl LocomotionCrossings {
    pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<Self, ModError> {
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
        .and_then(|descriptor| descriptor.with_limits(128, MAX_BYTES))
        .map_err(bad)?;
        let records = reg.state_family(
            descriptor,
            |key: &CommitToken| Ok(key.to_string()),
            decode,
            |batch: &CrossingBatch| batch.validate().map_err(|e| e.to_string()),
            |batch: &CrossingBatch| encoded(batch).map_err(|e| e.to_string()),
        )?;
        Ok(Self { records })
    }

    pub(crate) fn append(
        &self,
        txn: &mut ActionTxn<'_>,
        receipt: &LocomotionReceipt,
        facts: &CaptureFacts,
    ) -> Result<(), ParticipantError> {
        if facts.capture_digest
            != capture_digest(
                &facts.prepared_at,
                &facts.before_ancestry_digest,
                &facts.after_ancestry_digest,
                &facts.crossings,
            )?
            || txn
                .get(&self.records, &receipt.commit())
                .map_err(|_| refused())?
                .is_some()
        {
            return Err(refused());
        }
        let batch = CrossingBatch {
            format: 1,
            commit: receipt.commit(),
            action: receipt.action().clone(),
            participant: receipt.participant().clone(),
            actor: receipt.actor(),
            source: receipt.source(),
            exit: receipt.exit(),
            destination: receipt.destination(),
            prepared_at: facts.prepared_at.clone(),
            at_utc: receipt.at_utc(),
            plan_digest: *receipt.plan_digest(),
            capture_digest: facts.capture_digest,
            before_ancestry_digest: facts.before_ancestry_digest,
            after_ancestry_digest: facts.after_ancestry_digest,
            coverage: Coverage::UncertifiedLegacyPaths,
            crossings: facts.crossings.clone(),
        };
        batch.matches_receipt(receipt)?;
        txn.insert(&self.records, &receipt.commit(), batch)
            .map_err(|_| refused())?;
        Ok(())
    }

    /// Every current ordinary receipt requires its exact immutable capture,
    /// including commits with zero crossings. An orphan in either family fails
    /// boot closed. Structural receipts and captures belong to separate families.
    pub(crate) fn audit<P>(
        &self,
        ctx: &ModuleCtx<'_>,
        service: &CoreLocomotion<P>,
    ) -> Result<(), ParticipantError>
    where
        P: OwnerParticipant<Request = LocomotionIntent, Output = LocomotionDraft>,
    {
        let mut cursor = None;
        let mut count = 0usize;
        loop {
            let page = ctx
                .scan_action_state(&self.records, cursor.as_deref(), 128)
                .map_err(|_| refused())?;
            for entry in page.entries() {
                count = count.checked_add(1).ok_or_else(refused)?;
                if count > mush_core::action_state::MAX_ACTION_STATE_BOOT_RECORDS
                    || entry.encoded_key() != entry.value().commit.to_string()
                {
                    return Err(refused());
                }
                let receipt = service
                    .read_receipt(ctx, &entry.value().commit)?
                    .ok_or_else(refused)?;
                entry.value().matches_receipt(&receipt)?;
            }
            cursor = page.next_after().map(ToString::to_string);
            if cursor.is_none() {
                break;
            }
        }
        cursor = None;
        count = 0;
        loop {
            let page = service.scan_receipts(ctx, cursor.as_deref(), 128)?;
            for receipt in page.entries() {
                count = count.checked_add(1).ok_or_else(refused)?;
                if count > mush_core::action_state::MAX_ACTION_STATE_BOOT_RECORDS {
                    return Err(refused());
                }
                ctx.action_state(&self.records, &receipt.commit())
                    .map_err(|_| refused())?
                    .ok_or_else(refused)?
                    .matches_receipt(receipt)?;
            }
            cursor = page.next_after().map(ToString::to_string);
            if cursor.is_none() {
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests;

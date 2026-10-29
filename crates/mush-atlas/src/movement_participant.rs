//! Actual Atlas discovery write and checked absence of window effects. Sim
//! still refuses every open window; placement joins remain a separate cut.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use mush_core::ledger::BoundLedgerAppend;
use mush_core::locomotion::composed::{self, MovementDraft, MovementIntent};
use mush_core::participant::{
    BoundParticipant, OwnerParticipant, OwnerPreparation, OwnerPreparationBinding,
    OwnerPrepareContext, ParticipantError,
};
use mush_core::{
    ClockStamp, DefinitionDescriptor, DefinitionDigest, DefinitionId, DefinitionKind,
    DefinitionRegistryBuilder, DefinitionValidationContext, ModError, ObjId, OwnerId,
    PreparedLedgerAppend, Registrar, ReplayWrite,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::module::{AtlasState, NS};
use crate::movement::{Discovery, MovementCapture, SlotPresence, WindowEffect};
use crate::movement_history::{self as history, Mutation, Source, refused};
use crate::record::RecordKey;
use crate::travel::KnownDoc;

pub const PARTICIPANT: &str = "movement_discovery";
fn bad(error: impl std::fmt::Display) -> ModError {
    ModError::msg(error.to_string())
}

pub fn participant_descriptor() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(DefinitionKind::new("owner_participant").map_err(bad)?, DefinitionId::new(PARTICIPANT).map_err(bad)?, 1,
        json!({"state":"AtlasState","protocol":"composed_atlas_discovery_v1","capture_bytes":131072,"key_bytes":65536,
            "known_places":512,"known_policy":"player_fifo_duplicate_unchanged","window_policy":"checked_no_effect",
            "writes":["known_document","movement_knowledge_ledger"],"empty":"retained_owner_observation",
            "delivery":"same_commit","proof":"original_capture_and_knowledge_position_v1"})).map_err(bad)
}
pub fn observer_descriptors() -> Result<Vec<DefinitionDescriptor>, ModError> {
    ["travel_discovery", "window_arrival"]
        .into_iter()
        .map(|id| mush_core::moved_observer::descriptor(id, 1))
        .collect()
}
pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.definition_validator(
        DefinitionKind::new("owner_participant").map_err(bad)?,
        |ctx: &DefinitionValidationContext<'_>| {
            let expected = DefinitionRegistryBuilder::new()
                .register(
                    OwnerId::new(NS).map_err(|e| e.to_string())?,
                    participant_descriptor().map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
            if ctx.definition().reference() != &expected
                || !ctx.definition().references().is_empty()
            {
                return Err("unknown Atlas participant definition".into());
            }
            Ok(())
        },
    )?;
    let participant = reg.definition(participant_descriptor()?.as_current())?;
    let mut definitions = DefinitionRegistryBuilder::new();
    let action = definitions
        .register(
            OwnerId::new("sim").map_err(bad)?,
            composed::action_descriptor()?,
        )
        .map_err(bad)?;
    let observers = observer_descriptors()?
        .into_iter()
        .map(|descriptor| {
            definitions
                .register(OwnerId::new(NS).map_err(bad)?, descriptor)
                .map_err(bad)
        })
        .collect::<Result<Vec<_>, _>>()?;
    reg.owner_participant(
        participant.clone(),
        BTreeSet::from([action.clone()]),
        AtlasMovementParticipant,
    )?;
    reg.register_moved_completion::<AtlasMovementParticipant>(
        "atlas_movement_completion",
        1,
        action,
        participant,
        observers,
    )?;
    Ok(())
}

pub(crate) fn capture_digest(
    capture: &MovementCapture,
) -> Result<DefinitionDigest, ParticipantError> {
    let bytes = capture.encode().map_err(|_| refused())?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| ParticipantError::Encoding)?;
    DefinitionDigest::for_canonical_value_with_limit(
        "atlas.movement.capture.v1",
        &value,
        crate::movement::MAX_MOVEMENT_CAPTURE_BYTES,
    )
    .map_err(|_| ParticipantError::ImageLimit)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EventClaim {
    pub(crate) sequence: u64,
    pub(crate) digest: DefinitionDigest,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Proof {
    pub(crate) format: u16,
    pub(crate) capture: Value,
    pub(crate) history_next: u64,
    #[serde(deserialize_with = "required_option")]
    pub(crate) event: Option<EventClaim>,
}
fn required_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(deserializer)
}

pub struct AtlasMovementParticipant;
pub struct Patch {
    binding: OwnerPreparationBinding,
    stamp: ClockStamp,
    capture: MovementCapture,
    history_next: u64,
    history_fence: DefinitionDigest,
    append: Option<PreparedLedgerAppend>,
    known: Option<KnownDoc>,
}
pub struct BoundAtlasMovement<'a> {
    known: &'a mut BTreeMap<ObjId, Vec<RecordKey>>,
    actor: ObjId,
    update: Option<KnownDoc>,
    append: Option<BoundLedgerAppend<'a>>,
}
impl BoundParticipant for BoundAtlasMovement<'_> {
    fn apply(self) {
        if let Some(append) = self.append {
            let _ = append.apply();
        }
        if let Some(update) = self.update {
            self.known.insert(self.actor, update.places);
        }
    }
}
impl Patch {
    fn local(&self, state: &AtlasState) -> Result<(), ParticipantError> {
        self.capture.recheck_owner(state).map_err(|_| refused())?;
        if !state.movement_history.enabled
            || state.movement_history.ledger.next_seq(history::LOG) != self.history_next
            || state.movement_history.fence()? != self.history_fence
        {
            return Err(refused());
        }
        if let Some(append) = &self.append {
            state
                .movement_history
                .ledger
                .check_prepared_batch(std::slice::from_ref(append))
                .map_err(|_| refused())?;
        }
        Ok(())
    }
}
impl OwnerParticipant for AtlasMovementParticipant {
    type State = AtlasState;
    type Request = MovementIntent;
    type Output = MovementDraft;
    type Patch = Patch;
    type Bound<'a> = BoundAtlasMovement<'a>;
    fn prepare(
        &self,
        state: &AtlasState,
        ctx: &OwnerPrepareContext<'_>,
        intent: MovementIntent,
    ) -> Result<OwnerPreparation<Self::Output, Patch>, ParticipantError> {
        if !state.movement_history.enabled {
            return Err(refused());
        }
        let capture = crate::movement::capture(
            ctx.world(),
            intent.actor(),
            intent.source(),
            intent.destination(),
            intent.at_utc(),
        )
        .map_err(|_| refused())?;
        if capture.atlas_slot() != SlotPresence::Present
            || capture.window_effect() != WindowEffect::NoSelectedEffect
        {
            return Err(refused());
        }
        if let Discovery::Mapped {
            known_before: Some(before),
            ..
        } = capture.discovery()
        {
            history::canonical_known(before)?;
        }
        let digest = capture_digest(&capture)?;
        let known = capture.planned_known();
        let history_next = state.movement_history.ledger.next_seq(history::LOG);
        let history_fence = state.movement_history.fence()?;
        let (append, event) = if let Some(known) = &known {
            let event = Mutation {
                format: 1,
                actor: intent.actor(),
                at_utc: intent.at_utc(),
                source: Source::Prepared {
                    commit: intent.commit(),
                    prepared_at: intent.prepared_at().clone(),
                    capture_digest: digest,
                },
                known: known.clone(),
            };
            let append = state
                .movement_history
                .ledger
                .prepare_append_bounded(history::LOG, event.value()?, history::RESIDENT_BYTES)
                .map_err(|_| refused())?;
            let claim = EventClaim {
                sequence: append.seq(),
                digest: event.digest()?,
            };
            (Some(append), Some(claim))
        } else {
            (None, None)
        };
        let proof = Proof {
            format: 1,
            capture: serde_json::to_value(&capture).map_err(|_| ParticipantError::Encoding)?,
            history_next,
            event,
        };
        let output = MovementDraft::new(
            ctx,
            &intent,
            digest,
            serde_json::to_value(proof).map_err(|_| ParticipantError::Encoding)?,
        )?;
        let mut writes = Vec::new();
        if let Some(known) = &known {
            writes.push(ReplayWrite::ModuleJsonSet {
                namespace: OwnerId::new(NS).map_err(|_| refused())?,
                key: mush_core::AuthoritativeKey::parse_canonical(
                    crate::travel::known_key(intent.actor()).as_str(),
                )
                .map_err(|_| refused())?,
                value: serde_json::to_value(known).map_err(|_| ParticipantError::Encoding)?,
            });
        }
        if let Some(append) = &append {
            writes.push(
                append
                    .authoritative_replay_write(OwnerId::new(NS).map_err(|_| refused())?)
                    .map_err(|_| refused())?,
            );
        }
        let patch = Patch {
            binding: ctx.preparation_binding(),
            stamp: intent.prepared_at().clone(),
            capture,
            history_next,
            history_fence,
            append,
            known,
        };
        Ok(OwnerPreparation::new(output, patch, writes))
    }
    fn recheck(
        &self,
        state: &AtlasState,
        ctx: &OwnerPrepareContext<'_>,
        patch: &Patch,
    ) -> Result<(), ParticipantError> {
        let now = ctx.world().world_ic_now();
        if !ctx.matches_preparation(&patch.binding)
            || now.clock != patch.stamp.clock
            || now.at < patch.stamp.at
        {
            return Err(refused());
        }
        let current = crate::movement::capture(
            ctx.world(),
            patch.capture.actor(),
            patch.capture.from(),
            patch.capture.to(),
            patch.capture.at(),
        )
        .map_err(|_| refused())?;
        if current != patch.capture {
            return Err(refused());
        }
        patch.local(state)
    }
    fn bind<'a>(
        self: Arc<Self>,
        state: &'a mut AtlasState,
        patch: Patch,
    ) -> Result<Self::Bound<'a>, ParticipantError> {
        patch.local(state)?;
        let append = patch
            .append
            .map(|append| {
                state
                    .movement_history
                    .ledger
                    .bind_prepared(append)
                    .map_err(|_| refused())
            })
            .transpose()?;
        Ok(BoundAtlasMovement {
            known: &mut state.travel.known,
            actor: patch.capture.actor(),
            update: patch.known,
            append,
        })
    }
    fn accepts_write(&self, write: &ReplayWrite) -> bool {
        match write {
            ReplayWrite::ModuleJsonSet { namespace, key, .. } => {
                namespace.as_str() == NS
                    && key.as_str().strip_prefix("known:").is_some_and(|id| {
                        id.parse::<u32>()
                            .is_ok_and(|id| key.as_str() == format!("known:{id}"))
                    })
            }
            ReplayWrite::LedgerChunkSet { write } => {
                write.namespace().as_str() == NS && write.log_id() == history::LOG
            }
            _ => false,
        }
    }
}

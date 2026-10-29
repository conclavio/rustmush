//! Actual ordered knowledge journal and final known-book owner publication.

use std::collections::BTreeMap;

use mush_core::ledger::{BoundLedgerSequence, PreparedLedgerSequence};
use mush_core::locomotion::composed::{MovementDraft, MovementIntent};
use mush_core::participant::{
    BoundParticipant, OwnerPreparation, OwnerPreparationBinding, OwnerPrepareContext,
    ParticipantError,
};
use mush_core::{ClockStamp, DefinitionDigest, ObjId, OwnerId, ReplayWrite};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::module::{AtlasState, NS};
use crate::movement::Discovery;
use crate::movement_history::{self as history, Mutation, Source, refused};
use crate::movement_participant::EventClaim;
use crate::movement_window::WindowArrivalPlan;
use crate::movement_window_discovery::{self, WindowDiscoveryCapture};
use crate::record::RecordKey;
use crate::travel::KnownDoc;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LegClaim {
    pub(crate) history_next: u64,
    #[serde(deserialize_with = "required_event")]
    pub(crate) event: Option<EventClaim>,
}
fn required_event<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<EventClaim>, D::Error> {
    Option::deserialize(d)
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OrderedProof {
    pub(crate) format: u16,
    pub(crate) window: Value,
    pub(crate) discovery: Value,
    pub(crate) legs: Vec<LegClaim>,
}

struct Knowledge {
    capture: WindowDiscoveryCapture,
    sequence: PreparedLedgerSequence,
    fence: DefinitionDigest,
    known: Option<KnownDoc>,
}
impl Knowledge {
    fn prepare(
        state: &AtlasState,
        capture: WindowDiscoveryCapture,
        intent: &MovementIntent,
    ) -> Result<(Self, Vec<LegClaim>), ParticipantError> {
        let fence = state.movement_history.fence()?;
        let digest = capture.digest().map_err(|_| refused())?;
        let mut entries = Vec::new();
        let mut claims = Vec::new();
        let mut next = state.movement_history.ledger.next_seq(history::LOG);
        for leg in capture.legs() {
            if let Discovery::Mapped {
                known_before: Some(before),
                ..
            } = leg.discovery()
            {
                history::canonical_known(before)?;
            }
            let history_next = next;
            let event = if let Some(known) = leg.planned_known() {
                let mutation = Mutation {
                    format: 1,
                    actor: intent.actor(),
                    at_utc: intent.at_utc(),
                    source: Source::Prepared {
                        commit: intent.commit(),
                        prepared_at: intent.prepared_at().clone(),
                        capture_digest: digest,
                    },
                    known,
                };
                let claim = EventClaim {
                    sequence: next,
                    digest: mutation.digest()?,
                };
                entries.push((history::LOG.to_owned(), mutation.value()?));
                next = next.checked_add(1).ok_or_else(refused)?;
                Some(claim)
            } else {
                None
            };
            claims.push(LegClaim {
                history_next,
                event,
            });
        }
        let sequence = state
            .movement_history
            .ledger
            .prepare_sequence_bounded(entries, history::RESIDENT_BYTES)
            .map_err(|_| refused())?;
        // Claims retain actual prepared positions, including two appends in
        // the same chunk, rather than inferred positions after publication.
        {
            let mut positions = sequence.positions();
            for claim in claims.iter().filter_map(|leg| leg.event.as_ref()) {
                if positions.next() != Some((history::LOG, claim.sequence)) {
                    return Err(refused());
                }
            }
            if positions.next().is_some() {
                return Err(refused());
            }
        }
        let known = capture.planned_known();
        Ok((
            Self {
                capture,
                sequence,
                fence,
                known,
            },
            claims,
        ))
    }
    fn local(&self, state: &AtlasState) -> Result<(), ParticipantError> {
        self.capture.recheck_owner(state).map_err(|_| refused())?;
        if state.movement_history.fence()? != self.fence {
            return Err(refused());
        }
        state
            .movement_history
            .ledger
            .check_prepared_sequence(&self.sequence)
            .map_err(|_| refused())
    }
    fn writes(&self) -> Result<Vec<ReplayWrite>, ParticipantError> {
        let owner = OwnerId::new(NS).map_err(|_| refused())?;
        let mut writes = self
            .sequence
            .authoritative_replay_writes(owner.clone())
            .map_err(|_| refused())?;
        if let Some(known) = &self.known {
            writes.push(ReplayWrite::ModuleJsonSet {
                namespace: owner,
                key: mush_core::AuthoritativeKey::parse_canonical(
                    crate::travel::known_key(self.capture.actor()).as_str(),
                )
                .map_err(|_| refused())?,
                value: serde_json::to_value(known).map_err(|_| ParticipantError::Encoding)?,
            });
        }
        Ok(writes)
    }
}

pub struct WindowKnowledgePatch {
    binding: OwnerPreparationBinding,
    stamp: ClockStamp,
    window: Box<WindowArrivalPlan>,
    knowledge: Knowledge,
}
pub struct BoundWindowKnowledge<'a> {
    known: &'a mut BTreeMap<ObjId, Vec<RecordKey>>,
    actor: ObjId,
    update: Option<KnownDoc>,
    sequence: BoundLedgerSequence<'a>,
}
impl BoundParticipant for BoundWindowKnowledge<'_> {
    fn apply(self) {
        let _ = self.sequence.apply();
        if let Some(update) = self.update {
            self.known.insert(self.actor, update.places);
        }
    }
}
impl WindowKnowledgePatch {
    fn local(&self, state: &AtlasState) -> Result<(), ParticipantError> {
        self.window.recheck_owner(state).map_err(|_| refused())?;
        self.knowledge.local(state)
    }
    pub(super) fn recheck(
        &self,
        state: &AtlasState,
        ctx: &OwnerPrepareContext<'_>,
    ) -> Result<(), ParticipantError> {
        let now = ctx.world().world_ic_now();
        if !ctx.matches_preparation(&self.binding)
            || now.clock != self.stamp.clock
            || now.at < self.stamp.at
        {
            return Err(refused());
        }
        self.window.recheck(ctx.world()).map_err(|_| refused())?;
        self.knowledge
            .capture
            .recheck(ctx.world())
            .map_err(|_| refused())?;
        self.local(state)
    }
    pub(super) fn bind(
        self,
        state: &mut AtlasState,
    ) -> Result<BoundWindowKnowledge<'_>, ParticipantError> {
        self.local(state)?;
        let actor = self.knowledge.capture.actor();
        let sequence = state
            .movement_history
            .ledger
            .bind_prepared_sequence(self.knowledge.sequence)
            .map_err(|_| refused())?;
        Ok(BoundWindowKnowledge {
            known: &mut state.travel.known,
            actor,
            update: self.knowledge.known,
            sequence,
        })
    }
}

pub(super) fn prepare(
    state: &AtlasState,
    ctx: &OwnerPrepareContext<'_>,
    intent: MovementIntent,
    window: WindowArrivalPlan,
) -> Result<OwnerPreparation<MovementDraft, WindowKnowledgePatch>, ParticipantError> {
    let projection = intent.window_projection().ok_or_else(refused)?;
    window.check_projection(projection).map_err(|_| refused())?;
    let capture = movement_window_discovery::capture(
        ctx.world(),
        intent.actor(),
        intent.source(),
        intent.destination(),
        projection.projected_room(),
        intent.at_utc(),
    )
    .map_err(|_| refused())?;
    capture
        .check_first(window.capture())
        .map_err(|_| refused())?;
    let digest = capture.digest().map_err(|_| refused())?;
    let (knowledge, legs) = Knowledge::prepare(state, capture, &intent)?;
    let proof = serde_json::to_value(OrderedProof {
        format: 2,
        window: serde_json::to_value(&window).map_err(|_| ParticipantError::Encoding)?,
        discovery: serde_json::to_value(&knowledge.capture)
            .map_err(|_| ParticipantError::Encoding)?,
        legs,
    })
    .map_err(|_| ParticipantError::Encoding)?;
    let draft = MovementDraft::new(ctx, &intent, digest, proof)?;
    let writes = knowledge.writes()?;
    let patch = WindowKnowledgePatch {
        binding: ctx.preparation_binding(),
        stamp: intent.prepared_at().clone(),
        window: Box::new(window),
        knowledge,
    };
    Ok(OwnerPreparation::new(draft, patch, writes))
}

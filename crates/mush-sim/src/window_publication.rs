//! Actual Sim owner adapter. It requires a Core-minted window intent and
//! is registered only with composition-supplied real observer joins.

use super::*;
use mush_core::locomotion::composed::{
    self, WindowLocomotionDraft, WindowLocomotionIntent, WindowMetricTarget, WindowProjection,
};
use mush_core::participant::{
    BoundParticipant, OwnerParticipant, OwnerPreparation, OwnerPreparationBinding,
    OwnerPrepareContext,
};
use mush_core::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, ModError, OwnerId, PreparedLedgerAppend,
    ReplayWrite,
};
use std::collections::BTreeMap;
use std::sync::Arc;

const TAIL_BYTES: usize = 256 * 1024;
fn bad(error: impl std::fmt::Display) -> ModError {
    ModError::msg(error.to_string())
}

/// Proposal data only. The actual Atlas observer must independently attest
/// the exact selection through Core's shared window projection header.
#[derive(Clone, PartialEq, Eq)]
pub struct WindowSelection {
    room: ObjId,
    atlas_digest: DefinitionDigest,
    target: Option<WindowMetricTarget>,
}
impl WindowSelection {
    pub fn new(
        room: ObjId,
        atlas_digest: DefinitionDigest,
        target: Option<WindowMetricTarget>,
    ) -> Self {
        Self {
            room,
            atlas_digest,
            target,
        }
    }
    pub(crate) fn room(&self) -> ObjId {
        self.room
    }
    pub(crate) fn projection(&self, capture: DefinitionDigest) -> WindowProjection {
        WindowProjection::new(self.room, self.atlas_digest, capture, self.target)
    }
    pub(crate) fn plan_digest(
        &self,
        capture: DefinitionDigest,
    ) -> Result<DefinitionDigest, ParticipantError> {
        DefinitionDigest::for_canonical_value(
            "sim.window_movement.plan.v1",
            &json!({"capture":capture,"projection":self.projection(capture)}),
        )
        .map_err(|_| ParticipantError::Encoding)
    }
    pub(crate) fn native_target(&self) -> Option<WindowTarget> {
        self.target.map(|target| {
            WindowTarget::at(
                DomainId(target.domain()),
                target.x_cm(),
                target.y_cm(),
                target.z_cm(),
            )
        })
    }
}

pub type WindowSelector = Arc<
    dyn Fn(&World, ObjId, Option<ObjId>, ObjId, i64) -> Result<WindowSelection, ParticipantError>
        + Send
        + Sync,
>;

pub struct WindowMovementParticipant {
    selector: WindowSelector,
}
impl WindowMovementParticipant {
    pub(crate) fn prepare_journal(
        state: &SpatialState,
        record: &WindowMovementRecord,
    ) -> Result<PreparedLedgerAppend, ParticipantError> {
        append(state, record.journal_value())
    }
    pub fn new(selector: WindowSelector) -> Self {
        Self { selector }
    }
    pub fn descriptor() -> Result<DefinitionDescriptor, ModError> {
        DefinitionDescriptor::new(
            DefinitionKind::new("owner_participant").map_err(bad)?,
            DefinitionId::new(composed::SIM_PARTICIPANT).map_err(bad)?,
            2,
            json!({"state":"SpatialState","protocol":"sim_window_locomotion_v2",
                "capture":"sim.window_locomotion_crossings","ledger":WINDOW_MOVEMENT_LOG,
                "capture_bytes":MAX_WINDOW_MOVEMENT_BYTES,"tail_bytes":TAIL_BYTES,
                "publication":"final_native_placement_and_exact_journal",
                "bind":"canonical_native_fence_and_selected_append",
                "authority":"core_window_intent_and_actual_atlas_attestation",
                "personnel_eligibility":false}),
        )
        .map_err(bad)
    }
    fn select(
        &self,
        world: &World,
        intent: &WindowLocomotionIntent,
    ) -> Result<WindowSelection, ParticipantError> {
        (self.selector)(
            world,
            intent.actor(),
            intent.source(),
            intent.destination(),
            intent.at_utc(),
        )
    }
}

pub struct WindowMovementPatch {
    intent: WindowLocomotionIntent,
    binding: OwnerPreparationBinding,
    selection: WindowSelection,
    capture: OrderedWindowMovementCapture,
    append: PreparedLedgerAppend,
    journal_write: ReplayWrite,
}

pub struct BoundWindowMovement<'a> {
    placements: &'a mut BTreeMap<ObjId, crate::PlacementRecord>,
    derived: &'a mut crate::data::Derived,
    after_placements: BTreeMap<ObjId, crate::PlacementRecord>,
    after_derived: crate::data::Derived,
    journal: mush_core::ledger::BoundLedgerAppend<'a>,
}
impl BoundParticipant for BoundWindowMovement<'_> {
    fn apply(self) {
        *self.placements = self.after_placements;
        *self.derived = self.after_derived;
        let _ = self.journal.apply();
    }
}

pub(super) fn bind_capture<'a>(
    state: &'a mut SpatialState,
    capture: &OrderedWindowMovementCapture,
    append: PreparedLedgerAppend,
) -> Result<BoundWindowMovement<'a>, ParticipantError> {
    capture.recheck_native(state)?;
    capture.recheck_occupancy(state)?;
    let ReplayWrite::LedgerChunkSet { write } = journal_write(&append)? else {
        return Err(refused());
    };
    if write.log_id() != WINDOW_MOVEMENT_LOG {
        return Err(refused());
    }
    let mut after = planning_state(state)?;
    if let Some(record) = capture.placement_update()? {
        after.placements.insert(record.object, record);
    }
    after.derived = crate::graph::rebuild(&after);
    crate::physical::resident_bound(&after)?;
    let journal = state
        .recovery
        .bind_prepared(append)
        .map_err(|_| refused())?;
    Ok(BoundWindowMovement {
        placements: &mut state.placements,
        derived: &mut state.derived,
        after_placements: after.placements,
        after_derived: after.derived,
        journal,
    })
}

fn append(state: &SpatialState, value: Value) -> Result<PreparedLedgerAppend, ParticipantError> {
    state
        .recovery
        .prepare_append_bounded(WINDOW_MOVEMENT_LOG, value, TAIL_BYTES)
        .map_err(|_| refused())
}
fn journal_write(append: &PreparedLedgerAppend) -> Result<ReplayWrite, ParticipantError> {
    append
        .authoritative_replay_write(
            OwnerId::new(crate::NS).map_err(|_| ParticipantError::Authority)?,
        )
        .map_err(|_| ParticipantError::Encoding)
}

impl OwnerParticipant for WindowMovementParticipant {
    type State = SpatialState;
    type Request = WindowLocomotionIntent;
    type Output = WindowLocomotionDraft;
    type Patch = WindowMovementPatch;
    type Bound<'a> = BoundWindowMovement<'a>;

    fn prepare(
        &self,
        state: &SpatialState,
        ctx: &OwnerPrepareContext<'_>,
        intent: WindowLocomotionIntent,
    ) -> Result<OwnerPreparation<Self::Output, Self::Patch>, ParticipantError> {
        let selection = self.select(ctx.world(), &intent)?;
        let core = intent.preview_projection(ctx, &BTreeSet::new(), selection.room)?;
        let capture = capture_ordered(ctx.world(), core, selection.native_target())?;
        capture.recheck_native(state)?;
        let projection = selection.projection(capture.digest());
        let plan_digest = selection.plan_digest(capture.digest())?;
        let output = WindowLocomotionDraft::new(
            ctx,
            &intent,
            plan_digest,
            capture.digest(),
            capture.read_dependencies().clone(),
            projection.clone(),
        )?;
        let record = WindowMovementRecord::for_intent(
            ctx,
            &intent,
            &capture,
            plan_digest,
            projection,
            state.recovery.next_seq(WINDOW_MOVEMENT_LOG),
        )?;
        let append = Self::prepare_journal(state, &record)?;
        history::check_append(&record, &append)?;
        let journal_write = journal_write(&append)?;
        let mut writes = Vec::new();
        if let Some(record) = capture.placement_update()? {
            writes.push(ReplayWrite::ModuleJsonSet {
                namespace: OwnerId::new(crate::NS).map_err(|_| ParticipantError::Authority)?,
                key: mush_core::authoritative::AuthoritativeKey::parse_canonical(
                    crate::data::placement_key(record.object),
                )
                .map_err(|_| ParticipantError::Encoding)?,
                value: serde_json::to_value(record).map_err(|_| ParticipantError::Encoding)?,
            });
        }
        writes.push(journal_write.clone());
        Ok(OwnerPreparation::new(
            output,
            WindowMovementPatch {
                intent,
                binding: ctx.preparation_binding(),
                selection,
                capture,
                append,
                journal_write,
            },
            writes,
        ))
    }

    fn recheck(
        &self,
        state: &SpatialState,
        ctx: &OwnerPrepareContext<'_>,
        patch: &Self::Patch,
    ) -> Result<(), ParticipantError> {
        if !ctx.matches_preparation(&patch.binding)
            || self.select(ctx.world(), &patch.intent)? != patch.selection
        {
            return Err(refused());
        }
        patch.capture.recheck(ctx.world())?;
        patch.capture.recheck_native(state)?;
        let ReplayWrite::LedgerChunkSet { write } = &patch.journal_write else {
            return Err(refused());
        };
        let body = write
            .chunk()
            .entries
            .last()
            .ok_or_else(refused)?
            .value
            .clone();
        if journal_write(&append(state, body)?)? != patch.journal_write {
            return Err(refused());
        }
        Ok(())
    }

    fn bind<'a>(
        self: Arc<Self>,
        state: &'a mut SpatialState,
        patch: Self::Patch,
    ) -> Result<Self::Bound<'a>, ParticipantError> {
        bind_capture(state, &patch.capture, patch.append)
    }

    fn accepts_write(&self, write: &ReplayWrite) -> bool {
        match write {
            ReplayWrite::LedgerChunkSet { write } => {
                write.namespace().as_str() == crate::NS && write.log_id() == WINDOW_MOVEMENT_LOG
            }
            ReplayWrite::ModuleJsonSet {
                namespace,
                key,
                value,
            } if namespace.as_str() == crate::NS => {
                if record::preflight(value).is_err()
                    || crate::physical::encoded_size(value, RECORD_BYTES).is_err()
                {
                    return false;
                }
                serde_json::from_value::<crate::PlacementRecord>(value.clone()).is_ok_and(
                    |record| {
                        key.as_str() == crate::data::placement_key(record.object).as_str()
                            && serde_json::to_value(record).ok().as_ref() == Some(value)
                    },
                )
            }
            _ => false,
        }
    }
}

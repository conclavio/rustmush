//! Staff recovery is an attributed authoritative act. It never rewrites a lease,
//! refunds a deposit, creates ore, or clears inventory replay markers. A forced
//! home persists its intent before the surface tick moves the rig; its completion
//! is a second, ordinal-bound act, so a restart between the two is harmless.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, ActionTxn,
    AuthoritativeActionOutput, CanonicalId, CmdError, Ctx, DefinitionRef, IdempotencyKeyText,
    ObjId, OwnerId, PerceivedRef, ProjectedParameterKind, ProviderOfferDraft,
    ResolvedActionInvocation, RevisionKey, RevisionKind, RevisionValidator,
    RevisionValidatorContext, RulesValue, StateFamilyHandle, SubjectKey, World,
};
use mush_economy::VaultId;
use mush_inventory::{InventoryFamilies, record::EndpointRef};
use mush_roll::commerce::ic_claims::IcCloseKind;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::port::families::PortHandles;
use crate::port::ship::{Berth, berth_of, hull_view};
use crate::port::site_map::{EngineReads, SurfaceReads};
use crate::port_tick::port_error;
use crate::surface::act::{self, Where};
use crate::surface::cursor::{PocketCursor, PocketLock};
use crate::surface::excursion::{ActKind, Charge, RigKey, RigRecord, TvState};

/// Registered action and result; kept separate from player work permissions.
pub const RECOVERY_ACTION: &str = "tacsim_surface_recovery";
pub const RECOVERY_OUTCOME: &str = "tacsim_surface_recovery_result";
pub const RECOVERY_KEY_PREFIX: &str = "tacsim.recovery/";
/// Normal-world facts that must still hold when the recovery commits.
pub(crate) fn revision_kind() -> Result<RevisionKind, ActionError> {
    CanonicalId::new("surface_recovery_world")
        .map(RevisionKind::new)
        .map_err(|_| integrity("the recovery revision kind is invalid"))
}
/// Recovery visits share the surface fire's work budget.
const MAX_RECOVERIES: usize = 4;

/// The staff operation stored in the receipt, never a player act kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryOperation {
    /// Cancel work and close the excursion, leaving the rig where it stands.
    Close,
    /// Bring the rig into its assigned hull's bay, retaining its hopper.
    Home,
    /// Move the whole hopper into its assigned hull's hold.
    Unload,
}

impl RecoveryOperation {
    fn parse(word: &str) -> Option<Self> {
        match word {
            "close" => Some(Self::Close),
            "home" => Some(Self::Home),
            "unload" => Some(Self::Unload),
            _ => None,
        }
    }

    fn word(self) -> &'static str {
        match self {
            Self::Close => "close",
            Self::Home => "home",
            Self::Unload => "unload",
        }
    }
}

/// Last staff recovery and, for home, the durable placement intent. Its ordinal
/// is the rig's replay fence. A later ordinary act makes this receipt historical.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryReceipt {
    pub ordinal: u32,
    pub staff: ObjId,
    pub operator: ObjId,
    pub hull: ObjId,
    pub slot: u8,
    pub operation: RecoveryOperation,
    pub at_ic: i64,
    pub pending: bool,
    /// Frozen home destination; rechecked against the current assignment.
    pub bay_room: Option<ObjId>,
    pub bay_slot: Option<String>,
    pub moved: BTreeMap<CanonicalId, u64>,
}

impl RecoveryReceipt {
    pub(crate) fn check(&self, record: &RigRecord) -> Result<(), String> {
        if self.ordinal == 0
            || self.ordinal > record.ordinal
            || self.slot == 0
            || u32::from(self.slot) > crate::definitions::logistics::MAX_BAY_ITEMS
            || self.moved.len() > crate::surface::excursion::MAX_DUMP_GOODS
            || self.moved.values().any(|quantity| *quantity == 0)
            || self
                .moved
                .values()
                .try_fold(0_u64, |sum, quantity| sum.checked_add(*quantity))
                .is_none()
            || (self.operation != RecoveryOperation::Unload && !self.moved.is_empty())
        {
            return Err("the staff recovery receipt is out of bounds".to_owned());
        }
        if self.pending
            && (self.operation != RecoveryOperation::Home
                || self.ordinal != record.ordinal
                || self.bay_room.is_none()
                || self
                    .bay_slot
                    .as_ref()
                    .is_none_or(|slot| mush_sim::SlotId::new(slot).is_none())
                || record.act.is_some()
                || record.run.is_some()
                || record.queued.is_some()
                || record.plan.is_some()
                || record.excursion.is_some())
        {
            return Err("a pending recovery owns an otherwise idle rig".to_owned());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RigFacts {
    record_digest: String,
    operator: ObjId,
    hull: ObjId,
    slot: u8,
    assigned: bool,
    docked: bool,
    in_bay: bool,
    at_home: bool,
    bay_room: Option<ObjId>,
    bay_slot: Option<String>,
    hopper: Option<u64>,
    goods: Vec<CanonicalId>,
    hold: Option<u64>,
    hold_plan: Option<crate::port::market::HoldPlan>,
    /// Authenticated physical charge at offer time, never submitter-supplied.
    charge: Charge,
    staff: bool,
    finish: bool,
    /// **Doc 23 cut S.1** (L6): whether the rig, its epoch open, is parked short of its way
    /// home ([`act::parked_short`]) — a home then closes the epoch `RecoveredToBase`.
    parked: bool,
    /// **The S.1 fix round's F3**: whether the rig stands in any bay (the tick's placement of a
    /// leaving act puts it in none).
    in_any_bay: bool,
    /// **The S.1 fix round's F3**: what the act in flight's finish would open the epoch with,
    /// when it leaves the bay (`crate::surface::black_box::epoch_facts`) — a close that fences
    /// that finish out opens the epoch from it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    epoch: Option<crate::surface::black_box::EpochFacts>,
}

fn identity(w: &World, tv: ObjId, record: &RigRecord) -> Option<(ObjId, ObjId, u8)> {
    if let Some(lease) = crate::port::yard::rig_lease(w, tv) {
        return Some((lease.pilot, lease.hull, lease.slot));
    }
    if let Some(open) = &record.excursion {
        return Some((open.operator, open.hull, open.slot));
    }
    if let Some(act) = &record.act {
        return Some((act.operator, act.hull, act.slot));
    }
    if let Some(run) = &record.run {
        return Some((run.operator, run.hull, run.slot));
    }
    if let Some(queue) = &record.queued {
        return Some((queue.operator, queue.hull, queue.slot));
    }
    if let Some(plan) = &record.plan {
        return Some((plan.author, plan.hull, plan.slot));
    }
    record
        .recovery
        .as_ref()
        .map(|r| (r.operator, r.hull, r.slot))
}

fn facts(
    w: &World,
    actor: ObjId,
    tv: ObjId,
    record: &RigRecord,
    reads: crate::surface::black_box::EpochReads<'_>,
) -> Option<RigFacts> {
    let (operator, hull, slot) = identity(w, tv, record)?;
    let assigned = crate::port::yard::rig_lease(w, tv)
        .is_some_and(|lease| lease.hull == hull && lease.slot == slot);
    let view = hull_view(w, hull);
    let docked = matches!(berth_of(w, hull), Berth::Docked { .. });
    let bay = view.as_ref().and_then(|view| view.bay());
    let in_bay = act::where_is(w, tv) == (Where::Bay { hull });
    let at_home = bay.is_some_and(|(bay, room)| {
        w.module_state::<mush_sim::SpatialState>().is_some_and(|state| {
            matches!(state.parent_of(tv), mush_sim::SpatialParent::Interior { space, slot: Some(slot) }
                if space == room && slot.as_str() == bay.slot.id.as_str())
        })
    });
    let now = w.world_ic_now().at;
    let battery = w
        .module_state::<PortHandles>()
        .and_then(|handles| handles.surface_tools.as_ref())
        .and_then(|tools| act::rig_battery_id(w, tv).and_then(|id| act::battery_of(tools, &id)));
    let value = battery.map_or(record.charge.value_cu, |battery| {
        let settled_at = record
            .act
            .as_ref()
            .filter(|act| !in_bay && act.kind == crate::surface::excursion::ActKind::Launch)
            .map_or(now, |act| now.min(act.due_ic));
        crate::surface::excursion::charge_shown(record, settled_at, battery)
    });
    // A staff move may already have taken the rig out of the bay. Do not grant
    // recharge for an unknown interval on the ground.
    let known_launch = record.act.as_ref().is_some_and(|act| {
        act.kind == crate::surface::excursion::ActKind::Launch || act.params.launches()
    });
    let value = if !in_bay && record.charge.in_bay && !known_launch {
        record.charge.value_cu
    } else {
        value
    };
    let staff = crate::commands::is_staff(w, actor);
    let finish = record.recovery.as_ref().is_some_and(|r| {
        r.pending
            && r.ordinal == record.ordinal
            && r.staff == actor
            && r.hull == hull
            && r.slot == slot
            && r.operator == operator
            && assigned
            && docked
            && at_home
            && r.bay_room == bay.map(|(_, room)| room)
            && r.bay_slot.as_deref() == bay.map(|(bay, _)| bay.slot.id.as_str())
    });
    // Doc 23 cut S.1 (L6): read only for a rig whose epoch is open (a home then closes it).
    let parked = record.sortie.is_some() && act::parked_short(w, record, operator, battery);
    // The S.1 fix round's F3: the placement facts a close fencing out a finish reads.
    let in_any_bay = matches!(act::where_is(w, tv), Where::Bay { .. });
    let epoch = crate::surface::black_box::epoch_facts(w, reads, record);
    let hopper = crate::port::vehicle::hopper_of(w, tv);
    let hold = view
        .as_ref()
        .and_then(|view| crate::port::ship::hold_vault(w, view));
    let active = |vault: VaultId| {
        matches!(
            mush_inventory::provider::endpoint_status(w, EndpointRef(vault)),
            mush_economy::EndpointState::Live
        )
    };
    Some(RigFacts {
        record_digest: record_digest(record)?,
        operator,
        hull,
        slot,
        assigned,
        docked,
        in_bay,
        at_home,
        bay_room: bay.map(|(_, room)| room),
        bay_slot: bay.map(|(bay, _)| bay.slot.id.to_string()),
        hopper: hopper.filter(|vault| active(*vault)).map(|vault| vault.0),
        goods: crate::port::tv::hopper_goods_of(w, tv).map_or_else(Vec::new, |(goods, _)| goods),
        hold: hold.filter(|vault| active(*vault)).map(|vault| vault.0),
        hold_plan: view.as_ref().and_then(crate::port::market::hold_plan),
        charge: Charge {
            value_cu: value,
            at_ic: now,
            in_bay,
        },
        staff,
        finish,
        parked,
        in_any_bay,
        epoch,
    })
}

fn key(tv: ObjId, ordinal: u32, operation: &str) -> Option<IdempotencyKeyText> {
    IdempotencyKeyText::parse_canonical(format!(
        "{RECOVERY_KEY_PREFIX}{}/{ordinal}/{operation}",
        tv.0
    ))
    .ok()
}

fn record_digest(record: &RigRecord) -> Option<String> {
    mush_core::CanonicalHash::for_canonical_value(
        "tacsim.recovery_record.v1",
        1,
        &serde_json::to_value(record).ok()?,
        65_536,
    )
    .ok()
    .map(mush_core::CanonicalHash::to_hex)
}

/// Recheck the chosen rig's placement, assignment, tools and actor authority at
/// commit. Inventory and the full rig record also have their typed-family fences.
pub(crate) struct RecoveryRevisionValidator {
    pub excursions: StateFamilyHandle<RigKey, RigRecord>,
}

impl RevisionValidator for RecoveryRevisionValidator {
    fn current_generation(
        &self,
        context: &RevisionValidatorContext<'_>,
        key: &RevisionKey,
    ) -> Result<Option<u64>, ActionError> {
        let SubjectKey::Object { id: tv } = key.subject else {
            return Ok(None);
        };
        let Some(actor) = key
            .facet
            .as_ref()
            .and_then(|facet| facet.as_str().strip_prefix("staff_"))
            .and_then(crate::port::families::strict_decimal_u32)
            .map(ObjId)
        else {
            return Ok(None);
        };
        let record = context
            .action_state(&self.excursions, &RigKey(tv))
            .map_err(|_| integrity("the recovery revision cannot read its rig"))?
            .cloned()
            .unwrap_or_else(|| RigRecord::fresh_unread(tv));
        let w = context.world();
        let Some(fixed) = facts(w, actor, tv, &record, (&|_| None, &|_| None)) else {
            return Ok(None);
        };
        let hull = fixed.hull;
        let mut facts = serde_json::to_value(fixed)
            .map_err(|_| integrity("the recovery revision cannot encode its facts"))?;
        // Passage of time alone does not change authority or placement. Charge
        // was settled at the offer instant; its input record remains guarded.
        let map = facts
            .as_object_mut()
            .ok_or_else(|| integrity("the recovery revision facts are not a map"))?;
        map.remove("charge");
        // The S.1 fix round's F3: the epoch facts are what a fenced-out finish would record
        // (the seal, the class, the site's open epochs, read across other rigs); they carry no
        // authority, so another rig's epoch moving between the offer and the commit does not
        // fence this recovery. The placement they depend on is guarded above.
        map.remove("epoch");
        let spatial = w.module_state::<mush_sim::SpatialState>();
        let battery = w
            .module_state::<PortHandles>()
            .and_then(|handles| handles.surface_tools.as_ref())
            .and_then(|tools| {
                act::rig_battery_id(w, tv).and_then(|id| act::battery_of(tools, &id))
            });
        let hash = mush_core::CanonicalHash::for_canonical_value(
            "tacsim.recovery_world.v1",
            1,
            &json!({
                "facts": facts,
                "rig_parent": spatial.map(|state| state.parent_of(tv)),
                "hull_parent": spatial.map(|state| state.parent_of(hull)),
                "lease": crate::port::yard::rig_lease(w, tv),
                "battery": battery,
                "actor_gone": crate::port_tick::gone(w, actor),
                "enabled": w.module_state::<crate::store::TacsimStore>()
                    .is_some_and(crate::store::TacsimStore::enabled),
            }),
            65_536,
        )
        .map_err(|_| integrity("the recovery revision is out of bounds"))?;
        let mut bytes = [0; 8];
        bytes.copy_from_slice(
            hash.as_bytes()
                .get(..8)
                .ok_or_else(|| integrity("the recovery revision digest is too short"))?,
        );
        Ok(Some(u64::from_le_bytes(bytes)))
    }
}

fn invalid(message: &'static str) -> ActionError {
    port_error(ActionErrorCode::InvalidRequest, message)
}
fn rejected(message: &'static str) -> ActionError {
    port_error(ActionErrorCode::ProviderRejected, message)
}
fn integrity(message: &'static str) -> ActionError {
    port_error(ActionErrorCode::Integrity, message)
}

fn parameters(invocation: &ActionInvocation) -> Result<&Map<String, Value>, ActionError> {
    if invocation.subject() != &PerceivedRef::SelfActor || !invocation.targets().is_empty() {
        return Err(invalid("staff recovery is self-scoped and has no targets"));
    }
    invocation
        .parameters()
        .as_object()
        .ok_or_else(|| invalid("recovery needs parameters"))
}

fn request(p: &Map<String, Value>) -> Result<(ObjId, u32, &str), ActionError> {
    let tv = p
        .get("tv")
        .and_then(Value::as_str)
        .and_then(crate::port::families::strict_decimal_u32)
        .map(ObjId)
        .ok_or_else(|| invalid("recovery names a rig"))?;
    let ordinal = p
        .get("ordinal")
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n > 0)
        .ok_or_else(|| invalid("recovery names a positive ordinal"))?;
    let operation = p
        .get("operation")
        .and_then(Value::as_str)
        .filter(|word| *word == "finish" || RecoveryOperation::parse(word).is_some())
        .ok_or_else(|| invalid("recovery names close, home or unload"))?;
    Ok((tv, ordinal, operation))
}

/// Provider registered by the port composition with delegated inventory handles.
pub(crate) struct RecoveryProvider {
    pub action: DefinitionRef,
    pub outcome: DefinitionRef,
    pub excursions: StateFamilyHandle<RigKey, RigRecord>,
    pub pockets: StateFamilyHandle<mush_industry::OreDepositId, PocketCursor>,
    pub inventory: InventoryFamilies,
    /// **Doc 23 cut S.1** (L4, L6): the black box participant's handle for this action — a
    /// staff home's close line.
    pub black_box: Option<
        mush_core::participant::ParticipantHandle<crate::surface::black_box::BlackBoxParticipant>,
    >,
}

impl ActionProvider for RecoveryProvider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        request(parameters(invocation)?).map(|_| ())
    }

    fn resolved_revision_keys(
        &self,
        resolved: &ResolvedActionInvocation,
    ) -> Result<Vec<RevisionKey>, ActionError> {
        let invocation = resolved.invocation();
        let (tv, _, _) = request(parameters(invocation)?)?;
        Ok(vec![RevisionKey {
            owner: OwnerId::new("tacsim")
                .map_err(|_| integrity("the recovery owner is invalid"))?,
            kind: revision_kind()?,
            subject: SubjectKey::Object { id: tv },
            facet: Some(
                CanonicalId::new(format!("staff_{}", invocation.actor().0))
                    .map_err(|_| invalid("the recovery actor cannot be addressed"))?,
            ),
        }])
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<ProviderOfferDraft>, ActionError> {
        if !act::self_query(action, &self.action, query) {
            return Ok(None);
        }
        let w = &engine.world;
        if !w
            .module_state::<crate::store::TacsimStore>()
            .is_some_and(crate::store::TacsimStore::enabled)
        {
            return Ok(None);
        }
        let reads = EngineReads(engine);
        // The S.1 fix round's F3: the epoch facts' site count reads the world's rigs.
        let read_rig = |other: ObjId| reads.rig(other).ok().flatten();
        // Doc 23 cut S.2 (V7): the carrier's open epoch, its typed record.
        let read_carrier = |hull: ObjId| crate::port::epoch::open_id_in(engine, hull);
        let mut rigs = BTreeMap::new();
        for tv in act::world_rigs(w) {
            let Ok(record) = reads.rig(tv) else { continue };
            let record = record.unwrap_or_else(|| RigRecord::fresh_unread(tv));
            if let Some(fixed) = facts(w, query.actor(), tv, &record, (&read_rig, &read_carrier))
                && (fixed.staff || fixed.finish)
            {
                rigs.insert(tv.0.to_string(), fixed);
            }
        }
        if rigs.is_empty() {
            return Ok(None);
        }
        let choices = rigs.keys().cloned().collect();
        act::offer_draft(
            action,
            engine,
            json!({"rigs": rigs, "now": w.world_ic_now().at}),
            vec![
                act::parameter("tv", true, ProjectedParameterKind::Choice { choices })?,
                act::parameter(
                    "ordinal",
                    true,
                    ProjectedParameterKind::Integer {
                        minimum: 1,
                        maximum: i64::from(u32::MAX),
                    },
                )?,
                act::parameter(
                    "operation",
                    true,
                    ProjectedParameterKind::Choice {
                        choices: ["close", "home", "unload", "finish"]
                            .map(str::to_owned)
                            .to_vec(),
                    },
                )?,
            ],
            (
                "tacsim-surface-recovery",
                "Recover a rig",
                "Staff recovery of a stranded rig, preserving its ore and lease.",
            ),
        )
        .map(Some)
    }

    /// **Doc 23 cut S.1** (L4, L6): the recovery takes its preparation context, through which a
    /// staff home appends the epoch's close line in this act's own bundle.
    fn prepare_authoritative_with_context(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        context: &mut mush_core::PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let p = parameters(invocation)?;
        let (tv, ordinal, operation) = request(p)?;
        let now = p
            .get("now")
            .and_then(Value::as_i64)
            .ok_or_else(|| invalid("recovery fixes the IC instant"))?;
        let fixed: RigFacts = p
            .get("rigs")
            .and_then(Value::as_object)
            .and_then(|rigs| rigs.get(&tv.0.to_string()))
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .ok_or_else(|| {
                port_error(
                    ActionErrorCode::Unauthorized,
                    "that rig is not available for recovery",
                )
            })?;
        let found = act::read_record(txn, &self.excursions, tv)?;
        let existed = found.is_some();
        let mut record = found.unwrap_or_else(|| RigRecord::fresh_unread(tv));
        // Doc 23 cut S.1: the epoch's close line a staff home owes, appended below.
        let mut closing = None;
        record.check().map_err(|_| {
            integrity("the rig record cannot be recovered until its corruption is repaired")
        })?;
        if record_digest(&record).as_deref() != Some(fixed.record_digest.as_str()) {
            return Err(port_error(
                ActionErrorCode::Conflict,
                "the rig changed; request recovery again",
            ));
        }
        if operation == "finish" {
            let receipt = record
                .recovery
                .as_mut()
                .filter(|r| {
                    r.pending
                        && r.ordinal == ordinal
                        && r.staff == invocation.actor()
                        && r.hull == fixed.hull
                        && r.slot == fixed.slot
                        && r.operator == fixed.operator
                })
                .ok_or_else(|| rejected("that recovery is no longer pending"))?;
            if !fixed.finish || record.ordinal != ordinal {
                return Err(rejected("the rig has not reached its assigned bay"));
            }
            receipt.pending = false;
            receipt.at_ic = now;
            // Recharge starts when the durable finish confirms placement, never
            // while it was still stranded or waiting for room in the bay.
            record.charge = Charge {
                value_cu: record.charge.value_cu,
                at_ic: now,
                in_bay: true,
            };
        } else {
            if !fixed.staff {
                return Err(port_error(
                    ActionErrorCode::Unauthorized,
                    "only staff may start recovery",
                ));
            }
            if record.ordinal.checked_add(1) != Some(ordinal) {
                return Err(port_error(
                    ActionErrorCode::Conflict,
                    "the rig changed; request recovery again",
                ));
            }
            let operation =
                RecoveryOperation::parse(operation).ok_or_else(|| invalid("unknown recovery"))?;
            if operation != RecoveryOperation::Close
                && (!fixed.assigned
                    || !fixed.docked
                    || fixed.bay_room.is_none()
                    || fixed.bay_slot.is_none())
            {
                return Err(rejected(
                    "the rig's assigned hull must be docked with a readable bay",
                ));
            }
            if operation == RecoveryOperation::Unload && !fixed.in_bay {
                return Err(rejected(
                    "bring the rig into its assigned bay before unloading",
                ));
            }
            let moved = if operation == RecoveryOperation::Unload {
                self.unload(txn, &fixed)?
            } else {
                BTreeMap::new()
            };
            self.release_locks(txn, tv, &record)?;
            // Doc 23 cut S.1 (§6, D-23-1, D-23-4; L5, L6): the epoch at a staff recovery. A
            // close cancels the work and keeps the epoch, flagged abandoned — abandonment never
            // closes one; the close receipt's own line is the owner's one notice. A home closes
            // it at the receipt (the placement is the tick's, but the custody decision is the
            // staff's and is durable here): `RecoveredToBase` for a rig that was parked short of
            // its way home (the record's and the ground's facts, `RigFacts::parked`), else
            // `ReturnedToBase`, into the assigned hull. An unload, from the bay, moves neither.
            // `PoweredDownInBase` is unused (a rig's park is the bay) and `Destroyed` reserved.
            //
            // The S.1 fix round's F3: a close that lands between the tick's placement and the
            // finish (which it fences out) settles the custody fact the finish would have — an
            // act leaving the bay whose rig the tick already put out of every bay opens the
            // epoch here, flagged abandoned on the record (never on the open line); a recover
            // whose rig the tick already put in its bay closes it `ReturnedToBase`.
            match operation {
                RecoveryOperation::Close => {
                    let leaving = record
                        .act
                        .as_ref()
                        .filter(|act| act.kind == ActKind::Launch || act.params.launches());
                    let recovering = record
                        .act
                        .as_ref()
                        .filter(|act| act.kind == ActKind::Recover);
                    if let (Some(act), None, Some(excursion), false) = (
                        leaving,
                        record.sortie.as_ref(),
                        record.excursion.as_ref(),
                        fixed.in_any_bay,
                    ) {
                        let (mut opened, line) = crate::surface::black_box::open_epoch(
                            tv,
                            excursion,
                            act.due_ic,
                            fixed.epoch.as_ref(),
                        );
                        opened.abandoned = true;
                        record.sortie = Some(opened);
                        closing = Some(line);
                    } else if recovering.is_some() && fixed.in_bay {
                        if let Some(open) = record.sortie.take() {
                            closing = Some(crate::surface::black_box::BlackBoxLine::sortie_closed(
                                tv,
                                &open,
                                now,
                                IcCloseKind::ReturnedToBase { base: fixed.hull },
                            ));
                        }
                    } else if let Some(open) = record.sortie.as_mut() {
                        open.abandoned = true;
                    }
                }
                RecoveryOperation::Home => {
                    if let Some(open) = record.sortie.take() {
                        let base = fixed.hull;
                        closing = Some(crate::surface::black_box::BlackBoxLine::sortie_closed(
                            tv,
                            &open,
                            now,
                            if fixed.parked {
                                IcCloseKind::RecoveredToBase { base }
                            } else {
                                IcCloseKind::ReturnedToBase { base }
                            },
                        ));
                    }
                }
                RecoveryOperation::Unload => {}
            }
            record.ordinal = ordinal;
            record.charge = fixed.charge;
            if operation == RecoveryOperation::Home {
                record.charge.in_bay = false;
            }
            record.act = None;
            record.run = None;
            record.queued = None;
            record.plan = None;
            record.excursion = None;
            record.last = None;
            record.recovery = Some(Box::new(RecoveryReceipt {
                ordinal,
                staff: invocation.actor(),
                operator: fixed.operator,
                hull: fixed.hull,
                slot: fixed.slot,
                operation,
                at_ic: now,
                pending: operation == RecoveryOperation::Home,
                bay_room: fixed.bay_room,
                bay_slot: fixed.bay_slot,
                moved,
            }));
        }
        let receipt = record
            .recovery
            .as_ref()
            .ok_or_else(|| integrity("recovery has no receipt"))?;
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({
                "actor": invocation.actor().0, "tv": tv.0, "ordinal": ordinal,
                "operation": operation, "pending": receipt.pending, "moved": receipt.moved,
            }),
        )
        .map_err(|_| integrity("the recovery result is out of bounds"))?;
        // The line rides this bundle; an open whose line was not written is recorded on the
        // epoch in the same record write (the S.1 fix round's F2).
        if let Some(line) = closing {
            let opens = matches!(
                line,
                crate::surface::black_box::BlackBoxLine::MindSealed { .. }
            );
            let written =
                crate::surface::black_box::append(context, self.black_box.as_ref(), tv, line)?;
            if opens
                && !written
                && let Some(open) = record.sortie.as_mut()
            {
                open.unrecorded = true;
            }
        }
        act::write_record(txn, &self.excursions, record, existed)?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

impl RecoveryProvider {
    fn release_locks(
        &self,
        txn: &mut ActionTxn<'_>,
        tv: ObjId,
        record: &RigRecord,
    ) -> Result<(), ActionError> {
        let Some(run) = &record.run else {
            return Ok(());
        };
        let ours = PocketLock {
            tv,
            ordinal: run.ordinal,
        };
        for row in &run.lock_rows {
            let before = act::read_cursor(txn, &self.pockets, row.deposit)?
                .ok_or_else(|| integrity("a mining lock is missing; repair its cursor first"))?;
            if !before
                .rows
                .iter()
                .any(|cursor| cursor.pocket == row.pocket && cursor.lock.as_ref() == Some(&ours))
            {
                return Err(integrity(
                    "a mining lock changed owner; repair its cursor first",
                ));
            }
            let after = crate::surface::mine::cursor_released(&before, row.pocket, &ours);
            act::write_cursor(txn, &self.pockets, row.deposit, true, after)?;
        }
        Ok(())
    }

    fn unload(
        &self,
        txn: &mut ActionTxn<'_>,
        fixed: &RigFacts,
    ) -> Result<BTreeMap<CanonicalId, u64>, ActionError> {
        let source = EndpointRef(VaultId(
            fixed
                .hopper
                .ok_or_else(|| rejected("the hopper is unavailable"))?,
        ));
        let destination = EndpointRef(VaultId(
            fixed
                .hold
                .ok_or_else(|| rejected("the hull's hold is unavailable"))?,
        ));
        let plan = fixed
            .hold_plan
            .as_ref()
            .ok_or_else(|| rejected("the hold's capacity cannot be read"))?;
        let mut moved = BTreeMap::new();
        let mut pool_count = 0_u32;
        for good in &fixed.goods {
            if let Some(pool) = self
                .inventory
                .pool(txn, source, good)
                .map_err(|_| integrity("a hopper pool cannot be read"))?
            {
                pool_count += 1;
                if pool.available != pool.quantity {
                    return Err(rejected(
                        "the hopper contains reserved ore; resolve its reservations first",
                    ));
                }
                if pool.quantity > 0 {
                    moved.insert(good.clone(), pool.quantity);
                }
            }
        }
        let census = txn
            .get(&self.inventory.census, &source)
            .map_err(|_| integrity("the hopper's pool census cannot be read"))?
            .copied()
            .unwrap_or_default()
            .get();
        if census != pool_count {
            return Err(integrity(
                "the hopper contains unknown pools; repair its inventory first",
            ));
        }
        // Check every incoming good cumulatively, including all measures and
        // existing reserved stock, before staging a single inventory mutation.
        let mut additions: BTreeMap<CanonicalId, u64> = BTreeMap::new();
        for (good, quantity) in &moved {
            crate::port::market::plan_check(plan, good, *quantity, &mut |id| {
                let held = self
                    .inventory
                    .pool(txn, destination, id)
                    .map_err(|_| mush_market::CapacityReadError)?
                    .map_or(0, |pool| pool.quantity);
                held.checked_add(additions.get(id).copied().unwrap_or(0))
                    .ok_or(mush_market::CapacityReadError)
            })
            .map_err(|_| {
                rejected("the hold cannot take the whole hopper; make room before unloading")
            })?;
            additions.insert(good.clone(), *quantity);
        }
        if !moved.is_empty() {
            let moves: Vec<_> = moved
                .iter()
                .map(|(good, qty)| (good.clone(), *qty))
                .collect();
            self.inventory
                .transfer_many(txn, source, destination, &moves)
                .map_err(|_| rejected("the hopper transfer failed; nothing was changed"))?;
        }
        Ok(moved)
    }
}

/// Submit a request; confirmation is emitted by the tick only after its receipt
/// lands. The command must not promise success before authoritative prepare.
pub(crate) fn command(ctx: &mut Ctx<'_>, word: &str, tv: ObjId) -> Result<(), CmdError> {
    crate::commands::require_enabled(ctx)?;
    crate::commands::require_staff(ctx)?;
    let operation = RecoveryOperation::parse(word)
        .ok_or_else(|| CmdError::msg("Unknown recovery operation."))?;
    let handles = ctx
        .w
        .module_state::<PortHandles>()
        .cloned()
        .ok_or_else(|| CmdError::msg("Staff rig recovery is unavailable."))?;
    let action = handles
        .surface_recovery
        .ok_or_else(|| CmdError::msg("Staff rig recovery is unavailable."))?;
    if !act::world_rigs(ctx.w).contains(&tv) {
        return Err(CmdError::msg("No such terrain vehicle."));
    }
    let record = ctx
        .rig(tv)
        .map_err(CmdError::msg)?
        .unwrap_or_else(|| RigRecord::fresh_unread(tv));
    let fixed = facts(ctx.w, ctx.actor, tv, &record, (&|_| None, &|_| None))
        .ok_or_else(|| CmdError::msg("That rig has no recoverable assignment or excursion."))?;
    if operation != RecoveryOperation::Close
        && (!fixed.assigned
            || !fixed.docked
            || fixed.bay_room.is_none()
            || fixed.bay_slot.is_none())
    {
        return Err(CmdError::msg(
            "The rig's assigned hull must be docked with a readable bay.",
        ));
    }
    if operation == RecoveryOperation::Unload && !fixed.in_bay {
        return Err(CmdError::msg(
            "Bring the rig into its assigned bay before unloading.",
        ));
    }
    let ordinal = record
        .ordinal
        .checked_add(1)
        .ok_or_else(|| CmdError::msg("The rig's ordinal is exhausted."))?;
    let key =
        key(tv, ordinal, word).ok_or_else(|| CmdError::msg("The recovery key is unavailable."))?;
    ctx.submit_authoritative(
        action,
        PerceivedRef::SelfActor,
        Vec::new(),
        json!({"tv": tv.0.to_string(), "ordinal": ordinal, "operation": word}),
        key,
    );
    ctx.out.info(
        ctx.actor,
        format!(
            "Recovery requested: {} rig #{}. Its committed result will be reported.",
            operation.word(),
            tv.0
        ),
    );
    Ok(())
}

/// Durable recovery work, run by the surface tick. The stored staff identity is
/// the authority for this exact finish even if its role is subsequently revoked;
/// the provider grants that actor no permission to begin another recovery.
pub(crate) fn tick(
    ctx: &mut Ctx<'_>,
    handles: &PortHandles,
    records: &[RigRecord],
    stats: &mut crate::surface::tick::SurfaceFireStats,
) {
    let mut worked = 0_u32;
    let mut actors = BTreeSet::new();
    for record in records {
        let Some(receipt) = record.recovery.as_ref() else {
            continue;
        };
        if worked as usize >= MAX_RECOVERIES {
            break;
        }
        if receipt.pending {
            if ctx.authoritative_pending(receipt.staff) || actors.contains(&receipt.staff) {
                stats.deferred += 1;
                continue;
            }
            let fixed = facts(
                ctx.w,
                receipt.staff,
                record.tv,
                record,
                (&|_| None, &|_| None),
            );
            let reason = if crate::port_tick::gone(ctx.w, receipt.staff) {
                Some("the staff actor who requested it no longer exists")
            } else if fixed.as_ref().is_none_or(|fixed| {
                !fixed.assigned
                    || fixed.hull != receipt.hull
                    || fixed.slot != receipt.slot
                    || fixed.operator != receipt.operator
            }) {
                Some("the rig's assigned hull or operator changed")
            } else if fixed.as_ref().is_some_and(|fixed| !fixed.docked) {
                Some("the assigned hull is no longer docked")
            } else if fixed.as_ref().is_some_and(|fixed| {
                fixed.bay_room != receipt.bay_room || fixed.bay_slot != receipt.bay_slot
            }) {
                Some("the assigned bay changed")
            } else {
                None
            };
            if let Some(reason) = reason {
                if wait_notice(ctx, record, receipt, reason) {
                    worked += 1;
                    stats.told += 1;
                }
                continue;
            }
            let Some(action) = handles.surface_recovery.clone() else {
                continue;
            };
            let Some(fixed) = fixed else { continue };
            let Some(room) = receipt.bay_room else {
                continue;
            };
            let Some(slot) = receipt
                .bay_slot
                .as_ref()
                .and_then(|s| mush_sim::SlotId::new(s))
            else {
                continue;
            };
            if !fixed.at_home && mush_sim::place_interior(ctx, record.tv, room, Some(slot)).is_err()
            {
                if wait_notice(
                    ctx,
                    record,
                    receipt,
                    "the rig cannot be placed in its assigned bay",
                ) {
                    worked += 1;
                    stats.told += 1;
                }
                continue;
            }
            let Some(key) = key(record.tv, receipt.ordinal, "finish") else {
                continue;
            };
            // One closed typed call, not a general command dispatch. Unlike player
            // automation this may finish a recovery that GOD explicitly filed.
            let saved = ctx.actor;
            ctx.actor = receipt.staff;
            actors.insert(receipt.staff);
            ctx.submit_authoritative(action, PerceivedRef::SelfActor, Vec::new(),
                json!({"tv": record.tv.0.to_string(), "ordinal": receipt.ordinal, "operation": "finish"}), key);
            ctx.actor = saved;
            worked += 1;
            stats.submitted += 1;
        } else if project(ctx, record, receipt) {
            worked += 1;
            stats.told += 1;
        }
    }
    stats.worked += worked;
}

fn wait_notice(
    ctx: &mut Ctx<'_>,
    record: &RigRecord,
    receipt: &RecoveryReceipt,
    reason: &str,
) -> bool {
    let mut projection = crate::surface::tick::projection_for(
        ctx.w,
        record.tv,
        receipt.slot,
        receipt.operator,
        None,
    );
    if projection.recovery_wait_told == Some(receipt.ordinal) {
        return false;
    }
    let line = format!(
        "Rig #{} recovery is waiting: {reason}. Staff can retry @tacsim/tv home #{} or cancel with @tacsim/tv close #{}.",
        record.tv.0, record.tv.0, record.tv.0
    );
    projection.recovery_wait_told = Some(receipt.ordinal);
    projection.push_log(ctx.w.world_ic_now().at, &line);
    if !crate::surface::tick::write_projection(ctx, &projection) {
        return false;
    }
    if !crate::port_tick::gone(ctx.w, receipt.staff) {
        crate::port_tick::notify(ctx, receipt.staff, &line);
    }
    if receipt.operator != receipt.staff {
        crate::port_tick::notify(ctx, receipt.operator, &line);
    }
    true
}

fn project(ctx: &mut Ctx<'_>, record: &RigRecord, receipt: &RecoveryReceipt) -> bool {
    let mut projection = if receipt.ordinal == record.ordinal {
        crate::surface::tick::projection_for(ctx.w, record.tv, receipt.slot, receipt.operator, None)
    } else {
        // Do not run the excursion privacy reset against a newer act's view.
        match ctx
            .w
            .module_state::<crate::store::TacsimStore>()
            .and_then(|store| store.tv_rec(record.tv))
        {
            Some(crate::surface::excursion::TvRecState::Present(rec)) => rec.clone(),
            Some(crate::surface::excursion::TvRecState::Unreadable) | None => {
                crate::surface::excursion::TvRec::new(record.tv, receipt.slot)
            }
        }
    };
    if projection.recovery_told == Some(receipt.ordinal) {
        return false;
    }
    let line = match receipt.operation {
        RecoveryOperation::Close => format!(
            "Staff closed rig #{}'s excursion and cancelled its work. Its location and hopper are unchanged.",
            record.tv.0
        ),
        RecoveryOperation::Home => format!(
            "Staff brought rig #{} into hull #{}'s bay. Its hopper is unchanged; unload it before departure.",
            record.tv.0, receipt.hull.0
        ),
        RecoveryOperation::Unload => format!(
            "Staff unloaded rig #{} into hull #{}'s hold ({} t). Its pending work is cleared.",
            record.tv.0,
            receipt.hull.0,
            receipt.moved.values().sum::<u64>()
        ),
    };
    // An ordinary act may already have started after a completed close/unload.
    // Report the durable receipt without erasing that newer projection.
    if receipt.ordinal == record.ordinal {
        projection.operator = Some(receipt.operator);
        projection.hull = Some(receipt.hull);
        projection.excursion = None;
        projection.site = None;
        projection.state = if matches!(act::where_is(ctx.w, record.tv), Where::Bay { .. }) {
            TvState::Bay
        } else {
            TvState::Parked
        };
        projection.pose = crate::surface::tick::pose_of(ctx.w, record.tv);
        projection.act = None;
        projection.leg = None;
        projection.run = None;
        projection.plan = None;
        projection.plan_told = None;
        projection.applied = receipt.ordinal;
        projection.told = receipt.ordinal;
        projection.told_cycle = 0;
    }
    projection.recovery_told = Some(receipt.ordinal);
    projection.recovery_wait_told = None;
    if projection
        .operator
        .is_none_or(|operator| operator == receipt.operator)
    {
        projection.push_log(receipt.at_ic, &line);
    }
    if !crate::surface::tick::write_projection(ctx, &projection) {
        return false;
    }
    crate::port_tick::notify(ctx, receipt.staff, &line);
    if receipt.operator != receipt.staff {
        crate::port_tick::notify(ctx, receipt.operator, &line);
    }
    true
}

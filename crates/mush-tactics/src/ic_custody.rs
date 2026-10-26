//! Dedicated IC custody owned by Tactics. Old OOC documents and marker
//! restoration remain separate. Only the fixed four-owner provider may ask
//! this adapter to publish; decoded history is never a host close capability.
mod protocol;
pub use protocol::participant_descriptor;
use protocol::{MAX_HISTORY, MAX_HISTORY_BYTES, MAX_OPEN, MAX_UNIT_BYTES};

use crate::module::{UnitRecordStatus, UnitStore, encode_unit_document};
use crate::{TacticalUnit, UnitMutationOp, UnitMutationPlan};
use mush_core::action::PrincipalId;
use mush_core::authoritative::{AuthoritativeKey, ReplayWrite};
use mush_core::mechanics::{DefinitionRegistry, DefinitionRegistryBuilder, MechanicalState};
use mush_core::participant::{
    BoundParticipant, OwnerParticipant, OwnerPreparation, OwnerPrepareContext, ParticipantError,
};
use mush_core::{
    CommitToken, DefinitionDescriptor, DefinitionDigest, DefinitionId, DefinitionKind,
    DefinitionRef, Flags, ModError, ModuleBootData, ModuleCtx, ObjId, ObjKind, OwnerId, Registrar,
    World,
};
use mush_sim::ic_exclusion::IcEventRef;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, OnceLock};

const PREFIX: &str = "ic_custody:";
fn refused(_: impl std::fmt::Display) -> ParticipantError {
    ParticipantError::refused("IC hull custody is unavailable or changed")
}
fn bad(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Opening {
    operation: CommitToken,
    action: DefinitionRef,
    unit: ObjId,
    sortie: u64,
    dispatcher: ObjId,
    seal_digest: DefinitionDigest,
    record_schema: DefinitionRef,
    definition: DefinitionRef,
    revision: u64,
    unit_digest: DefinitionDigest,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Closing {
    operation: CommitToken,
    action: DefinitionRef,
    event: IcEventRef,
    unit_digest: DefinitionDigest,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    format: u16,
    open: Opening,
    #[serde(deserialize_with = "required_close")]
    closed: Option<Closing>,
}
fn required_close<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Closing>, D::Error> {
    Option::deserialize(d)
}
fn action(reference: &DefinitionRef, id: &str) -> bool {
    reference.key().owner.as_str() == "tacsim"
        && reference.key().kind.as_str() == "action"
        && reference.key().id.as_str() == id
        && reference.version() == 1
}
impl Record {
    fn key(&self) -> String {
        format!("{PREFIX}{}", self.open.operation.to_hex())
    }
    fn validate(&self) -> Result<(), ParticipantError> {
        if self.format != 1
            || self.open.unit.0 == 0
            || self.open.dispatcher.0 == 0
            || self.open.sortie == 0
            || !action(&self.open.action, "open_ic_mind")
            || self.open.revision == u64::MAX
            || self.open.record_schema.key().owner.as_str() != "tactics"
            || self.open.record_schema.key().kind.as_str() != "record"
            || [self.open.seal_digest, self.open.unit_digest]
                .iter()
                .any(|d| d.as_bytes() == &[0; 32])
        {
            return Err(refused("open"));
        }
        if let Some(c) = &self.closed
            && (c.operation == self.open.operation
                || !action(&c.action, "close_ic_mind")
                || c.event.sequence == 0
                || [c.event.digest, c.unit_digest]
                    .iter()
                    .any(|d| d.as_bytes() == &[0; 32]))
        {
            return Err(refused("close"));
        }
        mush_core::canonical_json_bytes_with_limit(
            &serde_json::to_value(self).map_err(refused)?,
            MAX_HISTORY_BYTES,
        )
        .map_err(refused)?;
        Ok(())
    }
    fn digest(&self) -> Result<DefinitionDigest, ParticipantError> {
        self.validate()?;
        DefinitionDigest::for_canonical_value(
            "tactics.ic_custody.open.v1",
            &serde_json::to_value(&self.open).map_err(refused)?,
        )
        .map_err(refused)
    }
}
/// Private complete historical book, kept in the existing actual UnitStore slot.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct IcCustodies {
    loaded: bool,
    generation: u64,
    rows: BTreeMap<CommitToken, Arc<Record>>,
    active: BTreeMap<ObjId, CommitToken>,
}
impl IcCustodies {
    fn reconstruct(rows: impl IntoIterator<Item = Record>) -> Result<Self, ParticipantError> {
        let mut s = Self {
            loaded: true,
            ..Self::default()
        };
        let mut sorties = BTreeSet::new();
        let mut closes = BTreeSet::new();
        for r in rows {
            r.validate()?;
            if s.rows.len() >= MAX_HISTORY
                || s.rows.contains_key(&r.open.operation)
                || !sorties.insert(r.open.sortie)
            {
                return Err(refused("history"));
            }
            if let Some(c) = &r.closed {
                if !closes.insert(c.operation) {
                    return Err(refused("close duplicate"));
                }
            } else if s.active.len() >= MAX_OPEN
                || s.active.insert(r.open.unit, r.open.operation).is_some()
            {
                return Err(refused("active duplicate"));
            }
            s.rows.insert(r.open.operation, Arc::new(r));
        }
        if closes.iter().any(|c| s.rows.contains_key(c)) {
            return Err(refused("crossed operation"));
        }
        Ok(s)
    }
    fn ready(&self) -> Result<(), ParticipantError> {
        if self.loaded {
            Ok(())
        } else {
            Err(refused("not loaded"))
        }
    }
    pub(crate) fn holds(&self, unit: ObjId) -> bool {
        self.active.contains_key(&unit)
    }
}
/// Read-only facts for the mandatory full four-owner final audit.
pub struct IcCustodyView<'a> {
    record: &'a Record,
}
impl IcCustodyView<'_> {
    /// Compare an immutable companion seal's captured hull to this actual
    /// owner's retained opening document. This read grants no mutation or
    /// custody and does not require today's hull to retain its old condition.
    pub fn verify_open_unit(&self, unit: &TacticalUnit) -> Result<(), ParticipantError> {
        let open = &self.record.open;
        if unit.object() != open.unit
            || unit.schema() != &open.definition
            || unit.revision() != open.revision
            || encode_unit_document(&open.record_schema, unit, None)
                .map_err(refused)?
                .1
                != open.unit_digest
        {
            return Err(refused("captured opening hull differs"));
        }
        Ok(())
    }
    /// Pure close projection through this owner's actual condition vocabulary
    /// and persisted unit codec. A digest is comparison data, never a mutation
    /// capability; the real participant must still prepare the same result.
    pub fn project_close_unit_digest(
        &self,
        unit: &TacticalUnit,
        conditions: &UnitMutationPlan,
        at: i64,
    ) -> Result<DefinitionDigest, ParticipantError> {
        self.verify_open_unit(unit)?;
        let after = close_unit(unit, conditions, at)?;
        encode_unit_document(&self.record.open.record_schema, &after, None)
            .map(|v| v.1)
            .map_err(refused)
    }
    pub fn open(&self) -> CommitToken {
        self.record.open.operation
    }
    pub fn unit(&self) -> ObjId {
        self.record.open.unit
    }
    pub fn sortie(&self) -> u64 {
        self.record.open.sortie
    }
    pub fn seal_digest(&self) -> DefinitionDigest {
        self.record.open.seal_digest
    }
    pub fn digest(&self) -> Result<DefinitionDigest, ParticipantError> {
        self.record.digest()
    }
    pub fn unit_digest(&self) -> DefinitionDigest {
        self.record.open.unit_digest
    }
    pub fn closed(&self) -> Option<(CommitToken, &DefinitionRef, &IcEventRef, DefinitionDigest)> {
        self.record
            .closed
            .as_ref()
            .map(|c| (c.operation, &c.action, &c.event, c.unit_digest))
    }
}
impl UnitStore {
    pub fn ic_custody_inventory(&self) -> Result<Vec<IcCustodyView<'_>>, ParticipantError> {
        self.ic.ready()?;
        Ok(self
            .ic
            .rows
            .values()
            .map(|r| IcCustodyView { record: r })
            .collect())
    }
    pub fn is_ic_custodied(&self, unit: ObjId) -> bool {
        self.ic.holds(unit)
    }
}
/// The owner-local marker remains a mutation fence even if a companion
/// projection is unavailable. The complete four-owner audit still refuses
/// that incomplete world; absence of Sim data never releases Tactics custody.
pub(crate) fn refuse_mutation(world: &World, unit: ObjId) -> Result<(), String> {
    if world
        .module_state::<UnitStore>()
        .is_some_and(|s| s.ic.holds(unit))
    {
        return Err(format!("Unit #{} is held by an IC sortie.", unit.0));
    }
    mush_sim::deployed::refuse_if_custodied(world, unit).map_err(|e| e.to_string())
}
#[derive(Clone, Debug)]
pub struct IcCustodyConfig {
    manifest: DefinitionRef,
    participants: [DefinitionRef; 4],
    actions: [DefinitionRef; 2],
    definitions: Arc<OnceLock<DefinitionRegistry>>,
}
impl IcCustodyConfig {
    pub fn new(
        manifest: DefinitionRef,
        participants: [DefinitionRef; 4],
    ) -> Result<Self, ModError> {
        if manifest.key().owner.as_str() != "tacsim"
            || manifest.key().kind.as_str() != "registry_manifest"
            || manifest.key().id.as_str() != "combat"
            || manifest.version() != 1
        {
            return Err(bad("IC manifest differs"));
        }
        for (r, (owner, id)) in participants.iter().zip([
            ("roll", "commercial_roots"),
            ("tacsim", "ic_sortie"),
            ("tactics", "ic_custody"),
            ("sim", "ic_exclusion"),
        ]) {
            if r.key().owner.as_str() != owner
                || r.key().kind.as_str() != "owner_participant"
                || r.key().id.as_str() != id
                || r.version() != 1
            {
                return Err(bad("IC participant differs"));
            }
        }
        let own = DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new("tactics").map_err(bad)?,
                participant_descriptor()?,
            )
            .map_err(bad)?;
        if participants.get(2) != Some(&own) {
            return Err(bad("IC custody descriptor differs"));
        }
        let build = |transition: &str| -> Result<DefinitionRef, ModError> {
            let refs = std::iter::once(manifest.clone())
                .chain(participants.iter().cloned())
                .collect();
            let d=DefinitionDescriptor::new(DefinitionKind::new("action").map_err(bad)?,DefinitionId::new(format!("{transition}_ic_mind")).map_err(bad)?,1,serde_json::json!({"format":1,"protocol":"ic_mind_custody_v1","transition":transition})).map_err(bad)?.with_references(refs);
            DefinitionRegistryBuilder::new()
                .register(OwnerId::new("tacsim").map_err(bad)?, d)
                .map_err(bad)
        };
        let actions = [build("open")?, build("close")?];
        Ok(Self {
            manifest,
            participants,
            actions,
            definitions: Arc::new(OnceLock::new()),
        })
    }
    pub(crate) fn register(
        &self,
        reg: &mut Registrar<'_>,
        record: &DefinitionRef,
    ) -> Result<(), ModError> {
        let definition = reg.definition(participant_descriptor()?)?;
        reg.owner_participant(
            definition,
            BTreeSet::from(self.actions.clone()),
            IcCustodyParticipant {
                config: self.clone(),
                record: record.clone(),
            },
        )
    }
    pub(crate) fn validate_registration(
        &self,
        ctx: &mush_core::DefinitionValidationContext<'_>,
    ) -> Result<(), String> {
        for r in std::iter::once(&self.manifest)
            .chain(self.participants.iter())
            .chain(self.actions.iter())
        {
            ctx.get_exact(r).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    pub(crate) fn loaded(&self, ctx: &ModuleCtx<'_>) -> Result<(), ModError> {
        self.definitions
            .set(
                ctx.reg
                    .definitions()
                    .ok_or_else(|| bad("IC definitions missing"))?
                    .clone(),
            )
            .map_err(|_| bad("IC custody already initialized"))
    }
    fn check(
        &self,
        ctx: &OwnerPrepareContext<'_>,
        index: usize,
        record: &DefinitionRef,
        unit: &TacticalUnit,
    ) -> Result<(), ParticipantError> {
        if self.actions.get(index) != Some(ctx.action()) {
            return Err(refused("action"));
        }
        let defs = self
            .definitions
            .get()
            .ok_or_else(|| refused("definitions"))?;
        for r in std::iter::once(&self.manifest)
            .chain(self.participants.iter())
            .chain(self.actions.iter())
            .chain(std::iter::once(record))
            .chain(unit.definition_references().iter())
        {
            defs.get_exact(r).map_err(refused)?;
        }
        Ok(())
    }
}
/// Inputs carry no TacticalUnit postimage. Close conditions must come from the
/// fixed Tacsim provider's genuine opaque event, not a public decoded receipt.
pub enum IcCustodyRequest {
    Open {
        unit: ObjId,
        sortie: u64,
        seal_digest: DefinitionDigest,
    },
    Close {
        open: CommitToken,
        event: IcEventRef,
        conditions: UnitMutationPlan,
    },
}
pub struct IcCustodyOutput {
    action: DefinitionRef,
    commit: CommitToken,
    open: CommitToken,
    unit: ObjId,
    sortie: u64,
    digest: DefinitionDigest,
    unit_digest: DefinitionDigest,
    event: Option<IcEventRef>,
}
impl IcCustodyOutput {
    pub fn action(&self) -> &DefinitionRef {
        &self.action
    }
    pub fn commit(&self) -> CommitToken {
        self.commit
    }
    pub fn open(&self) -> CommitToken {
        self.open
    }
    pub fn unit(&self) -> ObjId {
        self.unit
    }
    pub fn sortie(&self) -> u64 {
        self.sortie
    }
    pub fn digest(&self) -> DefinitionDigest {
        self.digest
    }
    pub fn unit_digest(&self) -> DefinitionDigest {
        self.unit_digest
    }
    pub fn event(&self) -> Option<&IcEventRef> {
        self.event.as_ref()
    }
}
pub struct IcCustodyParticipant {
    config: IcCustodyConfig,
    record: DefinitionRef,
}
pub struct IcCustodyPatch {
    action: DefinitionRef,
    commit: CommitToken,
    actor: ObjId,
    opening: bool,
    before_book: u64,
    index: usize,
    before: TacticalUnit,
    before_record: UnitRecordStatus,
    after: TacticalUnit,
    after_record: UnitRecordStatus,
    book: IcCustodies,
}
pub struct BoundIcCustody<'a> {
    unit: &'a mut TacticalUnit,
    row: &'a mut UnitRecordStatus,
    book: &'a mut IcCustodies,
    after: TacticalUnit,
    after_record: UnitRecordStatus,
    after_book: IcCustodies,
}
impl BoundParticipant for BoundIcCustody<'_> {
    fn apply(self) {
        *self.unit = self.after;
        *self.row = self.after_record;
        *self.book = self.after_book;
    }
}
fn active<'a>(
    state: &'a UnitStore,
    unit: ObjId,
    record: &DefinitionRef,
) -> Result<(usize, &'a TacticalUnit, &'a UnitRecordStatus), ParticipantError> {
    if state.record_definition.as_ref() != Some(record)
        || state.deployed.contains_key(&unit)
        || state.boot_deployed.iter().any(|(u, _)| *u == unit)
        || state.binding_repairs.iter().any(|r| r.object == unit)
    {
        return Err(refused("unit held"));
    }
    let u = state.units.get(&unit).ok_or_else(|| refused("unit"))?;
    let mut rows = state
        .records
        .iter()
        .enumerate()
        .filter(|(_, r)| r.claims_object(unit));
    let (i, r) = rows.next().ok_or_else(|| refused("row"))?;
    if rows.next().is_some()
        || r.state != MechanicalState::Active
        || r.key != format!("unit:{}", unit.0)
        || r.key_object != Some(unit)
        || r.object != Some(unit)
        || r.unit.as_ref() != Some(u)
        || r.deployed.is_some()
        || r.migration_plan.is_some()
    {
        return Err(refused("row identity"));
    }
    if r.payload_digest != Some(encode_unit_document(record, u, None).map_err(refused)?.1) {
        return Err(refused("row digest"));
    }
    Ok((i, u, r))
}
fn admission(world: &World, unit: ObjId, actor: ObjId) -> Result<(), ParticipantError> {
    for kind in [
        std::any::TypeId::of::<mush_sim::DeployedUnits>(),
        std::any::TypeId::of::<mush_sim::SpatialState>(),
        std::any::TypeId::of::<mush_sim::JourneyBook>(),
        std::any::TypeId::of::<mush_sim::ic_exclusion::IcExclusions>(),
    ] {
        if world
            .module_state_owner(kind)
            .is_none_or(|owner| owner.as_str() != "sim")
        {
            return Err(refused("physical owner unavailable"));
        }
    }
    if world
        .get(unit)
        .is_none_or(|o| o.kind != ObjKind::Thing || o.flags.contains(Flags::GOING))
        || world
            .get(actor)
            .is_none_or(|o| o.kind != ObjKind::Player || o.flags.contains(Flags::GOING))
    {
        return Err(refused("actor/hull"));
    }
    let binding = mush_core::BehaviorId::new(crate::module::UNIT_BEHAVIOR).map_err(refused)?;
    if world
        .get(unit)
        .and_then(|o| o.behaviors.get(&binding))
        .is_none_or(|b| b.schema_version != crate::UNIT_BINDING_VERSION)
    {
        return Err(refused("binding"));
    }
    mush_sim::deployed::refuse_if_custodied(world, unit).map_err(refused)?;
    Ok(())
}
fn capacity(schema: &DefinitionRef, unit: &TacticalUnit) -> Result<(), ParticipantError> {
    // Maximum same-shape closing condition; never installed or exposed as an
    // event. Conditions cannot add/remove assets, capability entries or cells.
    let (mut value, _) = encode_unit_document(schema, unit, None).map_err(refused)?;
    let u = value.get_mut("unit").ok_or_else(|| refused("unit wire"))?;
    *u.get_mut("revision")
        .ok_or_else(|| refused("revision wire"))? = serde_json::json!(u64::MAX);
    for pair in u
        .get_mut("cells")
        .and_then(serde_json::Value::as_array_mut)
        .ok_or_else(|| refused("cells wire"))?
    {
        let cell = pair
            .as_array_mut()
            .and_then(|p| p.get_mut(1))
            .ok_or_else(|| refused("cell wire"))?;
        *cell
            .get_mut("current")
            .ok_or_else(|| refused("current wire"))? = serde_json::json!(i64::MIN);
    }
    for family in ["sections", "components"] {
        for row in u
            .get_mut(family)
            .and_then(serde_json::Value::as_object_mut)
            .ok_or_else(|| refused("condition wire"))?
            .values_mut()
        {
            *row.get_mut("condition")
                .ok_or_else(|| refused("condition wire"))? =
                serde_json::json!("x".repeat(crate::unit::TACTICAL_ID_MAX));
        }
    }
    mush_core::canonical_json_bytes_with_limit(&value, MAX_UNIT_BYTES).map_err(refused)?;
    Ok(())
}
fn close_unit(
    before: &TacticalUnit,
    plan: &UnitMutationPlan,
    at: i64,
) -> Result<TacticalUnit, ParticipantError> {
    if plan.expected_revision() != Some(before.revision())
        || plan.ops().iter().any(|op| {
            !matches!(
                op,
                UnitMutationOp::SetCell { .. }
                    | UnitMutationOp::AdjustCell { .. }
                    | UnitMutationOp::SetSectionCondition { .. }
                    | UnitMutationOp::SetComponentCondition { .. }
            )
        })
    {
        return Err(refused("condition vocabulary"));
    }
    crate::unit::apply(before, plan, at)
        .map(|v| v.0)
        .map_err(refused)
}
impl OwnerParticipant for IcCustodyParticipant {
    type State = UnitStore;
    type Request = IcCustodyRequest;
    type Output = IcCustodyOutput;
    type Patch = IcCustodyPatch;
    type Bound<'a> = BoundIcCustody<'a>;
    fn prepare(
        &self,
        state: &UnitStore,
        ctx: &OwnerPrepareContext<'_>,
        request: Self::Request,
    ) -> Result<OwnerPreparation<Self::Output, Self::Patch>, ParticipantError> {
        state.ic.ready()?;
        let unit = match &request {
            IcCustodyRequest::Open { unit, .. } => *unit,
            IcCustodyRequest::Close { open, .. } => {
                state
                    .ic
                    .rows
                    .get(open)
                    .ok_or_else(|| refused("missing open"))?
                    .open
                    .unit
            }
        };
        let (index, before, before_record) = active(state, unit, &self.record)?;
        let opening = matches!(&request, IcCustodyRequest::Open { .. });
        self.config
            .check(ctx, if opening { 0 } else { 1 }, &self.record, before)?;
        let (row, after) = match request {
            IcCustodyRequest::Open {
                sortie,
                seal_digest,
                ..
            } => {
                admission(ctx.world(), unit, ctx.invocation().actor())?;
                if ctx.invocation().principal()
                    != &(PrincipalId::Object {
                        id: ctx.invocation().actor(),
                    })
                    || state.ic.holds(unit)
                    || state.ic.rows.len() >= MAX_HISTORY
                    || state.ic.active.len() >= MAX_OPEN
                    || state.ic.rows.contains_key(&ctx.commit_token())
                    || state.ic.rows.values().any(|r| {
                        r.open.sortie == sortie
                            || r.closed
                                .as_ref()
                                .is_some_and(|c| c.operation == ctx.commit_token())
                    })
                {
                    return Err(refused("open collision"));
                }
                capacity(&self.record, before)?;
                (
                    Record {
                        format: 1,
                        open: Opening {
                            operation: ctx.commit_token(),
                            action: ctx.action().clone(),
                            unit,
                            sortie,
                            dispatcher: ctx.invocation().actor(),
                            seal_digest,
                            record_schema: self.record.clone(),
                            definition: before.schema().clone(),
                            revision: before.revision(),
                            unit_digest: before_record
                                .payload_digest
                                .ok_or_else(|| refused("digest"))?,
                        },
                        closed: None,
                    },
                    before.clone(),
                )
            }
            IcCustodyRequest::Close {
                open,
                event,
                conditions,
            } => {
                let old = state
                    .ic
                    .rows
                    .get(&open)
                    .ok_or_else(|| refused("open missing"))?;
                if old.closed.is_some()
                    || state.ic.active.get(&unit) != Some(&open)
                    || old.open.unit_digest
                        != before_record
                            .payload_digest
                            .ok_or_else(|| refused("digest"))?
                    || ctx.commit_token() == open
                    || state.ic.rows.contains_key(&ctx.commit_token())
                    || state.ic.rows.values().any(|r| {
                        r.closed
                            .as_ref()
                            .is_some_and(|c| c.operation == ctx.commit_token())
                    })
                {
                    return Err(refused("close identity"));
                }
                let after = close_unit(before, &conditions, ctx.world().now())?;
                let mut row = (**old).clone();
                row.closed = Some(Closing {
                    operation: ctx.commit_token(),
                    action: ctx.action().clone(),
                    event,
                    unit_digest: encode_unit_document(&self.record, &after, None)
                        .map_err(refused)?
                        .1,
                });
                (row, after)
            }
        };
        row.validate()?;
        if opening {
            let mut future = row.clone();
            future.closed = Some(Closing {
                operation: CommitToken::from_bytes([255; 16]),
                action: self
                    .config
                    .actions
                    .get(1)
                    .ok_or_else(|| refused("close action"))?
                    .clone(),
                event: IcEventRef {
                    sequence: u64::MAX,
                    digest: DefinitionDigest::from_bytes([255; 32]),
                },
                unit_digest: DefinitionDigest::from_bytes([255; 32]),
            });
            mush_core::canonical_json_bytes_with_limit(
                &serde_json::to_value(future).map_err(refused)?,
                MAX_HISTORY_BYTES,
            )
            .map_err(refused)?;
        }
        let (document, unit_digest) =
            encode_unit_document(&self.record, &after, None).map_err(refused)?;
        let mut after_record = before_record.clone();
        after_record.unit = Some(after.clone());
        after_record.payload_digest = Some(unit_digest);
        let mut book = state.ic.clone();
        book.generation = book
            .generation
            .checked_add(1)
            .ok_or_else(|| refused("generation"))?;
        if opening {
            book.active.insert(unit, row.open.operation);
        } else {
            book.active.remove(&unit);
        }
        let output = IcCustodyOutput {
            action: ctx.action().clone(),
            commit: ctx.commit_token(),
            open: row.open.operation,
            unit,
            sortie: row.open.sortie,
            digest: row.digest()?,
            unit_digest,
            event: row.closed.as_ref().map(|c| c.event.clone()),
        };
        let write = ReplayWrite::ModuleJsonSet {
            namespace: OwnerId::new("tactics").map_err(refused)?,
            key: AuthoritativeKey::parse_canonical(row.key()).map_err(refused)?,
            value: serde_json::to_value(&row).map_err(refused)?,
        };
        book.rows.insert(row.open.operation, Arc::new(row));
        Ok(OwnerPreparation::new(
            output,
            IcCustodyPatch {
                action: ctx.action().clone(),
                commit: ctx.commit_token(),
                actor: ctx.invocation().actor(),
                opening,
                before_book: state.ic.generation,
                index,
                before: before.clone(),
                before_record: before_record.clone(),
                after,
                after_record,
                book,
            },
            vec![
                write,
                ReplayWrite::ModuleJsonSet {
                    namespace: OwnerId::new("tactics").map_err(refused)?,
                    key: AuthoritativeKey::parse_canonical(format!("unit:{}", unit.0))
                        .map_err(refused)?,
                    value: document,
                },
            ],
        ))
    }
    fn recheck(
        &self,
        state: &UnitStore,
        ctx: &OwnerPrepareContext<'_>,
        p: &Self::Patch,
    ) -> Result<(), ParticipantError> {
        state.ic.ready()?;
        self.config
            .check(ctx, if p.opening { 0 } else { 1 }, &self.record, &p.before)?;
        if ctx.action() != &p.action
            || ctx.commit_token() != p.commit
            || ctx.invocation().actor() != p.actor
            || state.ic.generation != p.before_book
        {
            return Err(refused("affinity"));
        }
        let (i, u, r) = active(state, p.before.object(), &self.record)?;
        if i != p.index || u != &p.before || r != &p.before_record {
            return Err(refused("stale"));
        }
        if p.opening {
            admission(ctx.world(), u.object(), p.actor)?;
        }
        Ok(())
    }
    fn bind<'a>(
        self: Arc<Self>,
        state: &'a mut UnitStore,
        p: Self::Patch,
    ) -> Result<Self::Bound<'a>, ParticipantError> {
        let (i, u, r) = active(state, p.before.object(), &self.record)?;
        if state.ic.generation != p.before_book
            || i != p.index
            || u != &p.before
            || r != &p.before_record
        {
            return Err(refused("stale bind"));
        }
        Ok(BoundIcCustody {
            unit: state
                .units
                .get_mut(&p.before.object())
                .ok_or_else(|| refused("unit"))?,
            row: state.records.get_mut(i).ok_or_else(|| refused("row"))?,
            book: &mut state.ic,
            after: p.after,
            after_record: p.after_record,
            after_book: p.book,
        })
    }
    fn accepts_write(&self, w: &ReplayWrite) -> bool {
        let ReplayWrite::ModuleJsonSet {
            namespace,
            key,
            value,
        } = w
        else {
            return false;
        };
        if namespace.as_str() != "tactics" {
            return false;
        }
        if key.as_str().starts_with("unit:") {
            return crate::module::replacement_write_is_exact(key.as_str(), value, &self.record);
        }
        let Ok(r) = serde_json::from_value::<Record>(value.clone()) else {
            return false;
        };
        r.validate().is_ok()
            && key.as_str() == r.key()
            && serde_json::to_value(&r).ok().as_ref() == Some(value)
            && self
                .config
                .actions
                .get(if r.closed.is_some() { 1 } else { 0 })
                == Some(
                    r.closed
                        .as_ref()
                        .map(|c| &c.action)
                        .unwrap_or(&r.open.action),
                )
    }
}
/// Extract before unit boot can repair anything. All missing companions are
/// checked by the final four-owner audit, even when this inventory is empty.
pub(crate) fn load(data: &mut ModuleBootData, enabled: bool) -> Result<IcCustodies, ModError> {
    let mut rows = Vec::new();
    let mut ordinary = Vec::new();
    for (k, v) in std::mem::take(&mut data.kv) {
        if k.starts_with(PREFIX) {
            if !enabled {
                return Err(bad(
                    "IC custody history requires its actual owner configuration",
                ));
            }
            if rows.len() >= MAX_HISTORY {
                return Err(bad("IC custody history bound exceeded"));
            }
            mush_core::canonical_json_bytes_with_limit(&v, MAX_HISTORY_BYTES).map_err(bad)?;
            let r: Record = serde_json::from_value(v.clone()).map_err(bad)?;
            r.validate().map_err(bad)?;
            if k.as_str() != r.key() || serde_json::to_value(&r).map_err(bad)? != v {
                return Err(bad("IC custody key or shape differs"));
            }
            rows.push(r);
        } else {
            ordinary.push((k, v));
        }
    }
    data.kv = ordinary;
    IcCustodies::reconstruct(rows).map_err(bad)
}
pub(crate) fn check_boot(state: &UnitStore) -> Result<(), ModError> {
    state.ic.ready().map_err(bad)?;
    for r in state.ic.rows.values().filter(|r| r.closed.is_none()) {
        let (_, u, row) = active(state, r.open.unit, &r.open.record_schema).map_err(bad)?;
        if u.schema() != &r.open.definition
            || u.revision() != r.open.revision
            || row.payload_digest != Some(r.open.unit_digest)
        {
            return Err(bad("IC held hull differs from its actual Open image"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;

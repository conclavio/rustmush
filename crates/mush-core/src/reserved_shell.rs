//! Closed, acknowledged inert object reservations for Roll's initial filing.
//!
//! Object allocation, the typed reservation and its per-requester open index
//! share one authoritative write union. The object has no location, parent,
//! behavior or player-controlled owner. Filing later consumes the reservation
//! and issues its E4 title; it never changes World fields after acknowledgement.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::World;
use crate::action::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, AuthoritativeActionOutput,
    CommitToken, PerceivedRef, PrincipalId, RequestUid,
};
use crate::action_state::{ActionTxn, StateFamilyDescriptor, StateFamilyHandle, StateFamilyId};
use crate::authoritative::ReplayWrite;
use crate::flags::Flags;
use crate::mechanics::{
    CanonicalId, DefinitionDigest, DefinitionRef, RulesValue, canonical_json_bytes_with_limit,
};
use crate::module::{ModError, Registrar};
use crate::obj::{GOD, Obj, ObjId, ObjKind};
use crate::participant::ParticipantError;
use crate::resolution::PrepareCtx;

pub const RESERVE_ACTION: &str = "reserve_initial_filing_shell";
pub const RESERVE_OUTCOME: &str = "reserved_initial_filing_shell";
pub const INITIALIZE_ACTION: &str = "initialize_commerce";
const FILING_ACTION: &str = "initial_filing";
const PURPOSE: &str = "roll.initial_filing";
const NAME: &str = "Reserved mind charter";
const FORMAT: u16 = 1;
const RECORD_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShellKey {
    requester: ObjId,
    request: RequestUid,
}
impl ShellKey {
    #[must_use]
    pub const fn requester(self) -> ObjId {
        self.requester
    }
    #[must_use]
    pub const fn request(self) -> RequestUid {
        self.request
    }
    fn encoded(self) -> String {
        format!("{}/{}", self.requester.0, self.request)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ShellState {
    Reserved {},
    Consumed { filing: CommitToken },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ShellRecord {
    format: u16,
    birth: u64,
    key: ShellKey,
    purpose: String,
    object: ObjId,
    created_at: i64,
    object_digest: DefinitionDigest,
    receipt: CommitToken,
    state: ShellState,
}
impl ShellRecord {
    fn object(&self) -> Obj {
        Obj::new(self.object, ObjKind::Thing, NAME, GOD, self.created_at)
    }
    fn validate(&self) -> Result<(), ParticipantError> {
        if self.format != FORMAT
            || self.birth == 0
            || self.purpose != PURPOSE
            || self.key.requester.0 == 0
            || self.object.0 < 2
            || object_digest(&self.object())? != self.object_digest
            || matches!(self.state, ShellState::Consumed { filing } if filing == self.receipt)
        {
            return Err(ParticipantError::Authority);
        }
        bounded(self)?;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenShell {
    format: u16,
    key: ShellKey,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ShellDeployment {
    format: u16,
    initialized_by: CommitToken,
    born: u64,
    consumed: u64,
}
impl ShellDeployment {
    fn validate(&self) -> Result<(), ParticipantError> {
        if self.format != FORMAT
            || self.consumed > self.born
            || self.born > crate::action_state::MAX_ACTION_STATE_BOOT_RECORDS as u64
        {
            return Err(ParticipantError::Authority);
        }
        Ok(())
    }
}

/// Opaque facts obtained from the actual committed typed record and World
/// object. This is not title authority, certification or a licence.
#[derive(Clone, Debug)]
pub struct ReservedShell {
    record: ShellRecord,
    before: Obj,
}

/// A verified consumed row, emitted only by the closed service's boot audit.
/// Roll must join it to its actual initial-filing operation in both directions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConsumedShell {
    key: ShellKey,
    object: ObjId,
    reservation: CommitToken,
    filing: CommitToken,
}
impl ConsumedShell {
    #[must_use]
    pub const fn key(self) -> ShellKey {
        self.key
    }
    #[must_use]
    pub const fn object(self) -> ObjId {
        self.object
    }
    #[must_use]
    pub const fn reservation(self) -> CommitToken {
        self.reservation
    }
    #[must_use]
    pub const fn filing(self) -> CommitToken {
        self.filing
    }
}
impl ReservedShell {
    #[must_use]
    pub const fn key(&self) -> ShellKey {
        self.record.key
    }
    #[must_use]
    pub const fn object(&self) -> ObjId {
        self.record.object
    }
    #[must_use]
    pub const fn requester(&self) -> ObjId {
        self.record.key.requester
    }
    #[must_use]
    pub const fn receipt(&self) -> CommitToken {
        self.record.receipt
    }
    #[must_use]
    pub const fn request(&self) -> RequestUid {
        self.record.key.request
    }
    #[must_use]
    pub const fn purpose(&self) -> &'static str {
        PURPOSE
    }
    #[must_use]
    pub const fn object_before(&self) -> &Obj {
        &self.before
    }
}

/// Handles never escape this closed service. Only Roll may register it, and
/// only the exact reserve/initial-filing actions may create/consume its rows.
#[derive(Clone, Debug)]
pub struct ShellReservations {
    records: StateFamilyHandle<ShellKey, ShellRecord>,
    open: StateFamilyHandle<ObjId, OpenShell>,
    deployment: StateFamilyHandle<(), ShellDeployment>,
}
impl ShellReservations {
    pub fn register(
        reg: &mut Registrar<'_>,
        action: DefinitionRef,
        outcome: DefinitionRef,
    ) -> Result<Self, ModError> {
        Self::register_mode(reg, action, outcome, true)
    }

    /// Register the same closed families for explicit setup, with no reserve
    /// action provider. Setup cannot begin allocating before an Active restart.
    pub fn register_for_setup(
        reg: &mut Registrar<'_>,
        action: DefinitionRef,
        outcome: DefinitionRef,
    ) -> Result<Self, ModError> {
        Self::register_mode(reg, action, outcome, false)
    }

    fn register_mode(
        reg: &mut Registrar<'_>,
        action: DefinitionRef,
        outcome: DefinitionRef,
        enable_reservation: bool,
    ) -> Result<Self, ModError> {
        if reg.module() != "roll"
            || !definition_is(&action, "action", RESERVE_ACTION)
            || !definition_is(&outcome, "outcome", RESERVE_OUTCOME)
        {
            return Err(ModError::msg("invalid closed shell registration"));
        }
        let records = reg.state_family(
            descriptor("initial_filing_shells")?,
            |key: &ShellKey| Ok(key.encoded()),
            |value: &Value| decode::<ShellRecord>(value),
            |row: &ShellRecord| row.validate().map_err(|e| e.to_string()),
            |row: &ShellRecord| encode(row),
        )?;
        let open = reg.state_family(
            descriptor("open_initial_filing_shells")?,
            |actor: &ObjId| Ok(actor.0.to_string()),
            |value: &Value| decode::<OpenShell>(value),
            |row: &OpenShell| {
                if row.format == FORMAT && row.key.requester.0 > 0 {
                    Ok(())
                } else {
                    Err("invalid open shell".into())
                }
            },
            |row: &OpenShell| encode(row),
        )?;
        let deployment = reg.state_family(
            descriptor("initial_filing_shell_deployment")?,
            |_: &()| Ok("registry".to_owned()),
            |value: &Value| decode::<ShellDeployment>(value),
            |row: &ShellDeployment| row.validate().map_err(|e| e.to_string()),
            |row: &ShellDeployment| encode(row),
        )?;
        let service = Self {
            records,
            open,
            deployment,
        };
        if enable_reservation {
            reg.action_provider(
                action.clone(),
                ShellProvider {
                    service: service.clone(),
                    action,
                    outcome,
                },
            )?;
        }
        Ok(service)
    }

    /// Used only by the explicit first-install action. Ordinary boot never
    /// interprets absent markers as a fresh deployment. Roll initializes its
    /// own deployment row in this same typed transaction and commit.
    pub(crate) fn initialize(
        &self,
        world: &World,
        invocation: &ActionInvocation,
        action: &DefinitionRef,
        commit: CommitToken,
        txn: &mut ActionTxn<'_>,
    ) -> Result<ShellObjects, ParticipantError> {
        require_action(invocation, action, INITIALIZE_ACTION)?;
        require_empty(invocation)?;
        require_actor(world, invocation.actor())?;
        if invocation.actor() != GOD
            || txn.owner().as_str() != "roll"
            || !txn.family_is_empty(&self.records).map_err(state_error)?
            || !txn.family_is_empty(&self.open).map_err(state_error)?
            || !txn.family_is_empty(&self.deployment).map_err(state_error)?
        {
            return Err(ParticipantError::Authority);
        }
        let image = ShellObjects::verify(
            world,
            GOD,
            world.get(GOD).cloned().ok_or(ParticipantError::Authority)?,
        )?;
        txn.insert(
            &self.deployment,
            &(),
            ShellDeployment {
                format: FORMAT,
                initialized_by: commit,
                born: 0,
                consumed: 0,
            },
        )
        .map_err(state_error)?;
        Ok(image)
    }

    /// Called only by the core-minted preparation context. It checks the
    /// per-requester mapping before inspecting/advancing the allocator. An
    /// abandoned reservation is reused for the next attempt, bounding each
    /// requester to one unconsumed inert object.
    pub(crate) fn prepare_reserve(
        &self,
        world: &World,
        invocation: &ActionInvocation,
        action: &DefinitionRef,
        commit: CommitToken,
        txn: &mut ActionTxn<'_>,
    ) -> Result<(ShellKey, ObjId, ShellObjects), ParticipantError> {
        require_action(invocation, action, RESERVE_ACTION)?;
        require_empty(invocation)?;
        if txn.owner().as_str() != "roll" {
            return Err(ParticipantError::Authority);
        }
        require_actor(world, invocation.actor())?;
        let mut deployment = txn
            .get(&self.deployment, &())
            .map_err(state_error)?
            .cloned()
            .ok_or(ParticipantError::Authority)?;
        deployment.validate()?;
        let old_open = txn
            .get(&self.open, &invocation.actor())
            .map_err(state_error)?
            .cloned();
        if let Some(open) = old_open {
            if open.format != FORMAT || open.key.requester != invocation.actor() {
                return Err(ParticipantError::Authority);
            }
            let record = txn
                .get(&self.records, &open.key)
                .map_err(state_error)?
                .cloned()
                .ok_or(ParticipantError::Authority)?;
            let shell = resolve_record(world, open.key, record)?;
            let image = ShellObjects::verify(world, invocation.actor(), shell.before.clone())?;
            // A new request acknowledges the same reservation under the typed
            // commit fence. Retain the exact mapping bytes: no alias record,
            // new object, birth count, or original receipt is introduced.
            txn.replace(&self.open, &invocation.actor(), open)
                .map_err(state_error)?;
            return Ok((shell.key(), shell.object(), image));
        }
        let key = ShellKey {
            requester: invocation.actor(),
            request: invocation.request(),
        };
        // A reused UID with a consumed/missing-open record must never allocate
        // again. The outer idempotency registry normally returns its old receipt.
        if txn.get(&self.records, &key).map_err(state_error)?.is_some() {
            return Err(ParticipantError::Authority);
        }
        let image = ShellObjects::create(world, invocation.actor())?;
        let object = image.object.clone();
        let record = ShellRecord {
            format: FORMAT,
            birth: deployment
                .born
                .checked_add(1)
                .ok_or(ParticipantError::ImageLimit)?,
            key,
            purpose: PURPOSE.into(),
            object: object.id,
            created_at: object.created,
            object_digest: object_digest(&object)?,
            receipt: commit,
            state: ShellState::Reserved {},
        };
        record.validate()?;
        deployment.born = record.birth;
        deployment.validate()?;
        txn.replace(&self.deployment, &(), deployment)
            .map_err(state_error)?;
        txn.insert(&self.records, &key, record)
            .map_err(state_error)?;
        txn.insert(
            &self.open,
            &invocation.actor(),
            OpenShell {
                format: FORMAT,
                key,
            },
        )
        .map_err(state_error)?;
        Ok((key, object.id, image))
    }

    /// Resolve the acknowledged reservation for this exact actor inside the
    /// first-filing transaction. The World object must already exist unchanged;
    /// a shell staged in the same unacknowledged transaction cannot pass.
    pub(crate) fn resolve(
        &self,
        world: &World,
        invocation: &ActionInvocation,
        action: &DefinitionRef,
        txn: &mut ActionTxn<'_>,
        key: ShellKey,
    ) -> Result<ReservedShell, ParticipantError> {
        // Both closed original-filing protocols consume the requester's same
        // acknowledged inert shell. This does not grant economic title.
        let id = match action.key().id.as_str() {
            FILING_ACTION => FILING_ACTION,
            "house_initial_filing" => "house_initial_filing",
            "dependent_initial_filing" => "dependent_initial_filing",
            _ => return Err(ParticipantError::Authority),
        };
        require_action(invocation, action, id)?;
        if key.requester != invocation.actor() || txn.owner().as_str() != "roll" {
            return Err(ParticipantError::Authority);
        }
        require_actor(world, invocation.actor())?;
        txn.get(&self.deployment, &())
            .map_err(state_error)?
            .ok_or(ParticipantError::Authority)?
            .validate()?;
        let open = txn
            .get(&self.open, &key.requester)
            .map_err(state_error)?
            .ok_or(ParticipantError::Authority)?;
        if open.format != FORMAT || open.key != key {
            return Err(ParticipantError::Authority);
        }
        let record = txn
            .get(&self.records, &key)
            .map_err(state_error)?
            .cloned()
            .ok_or(ParticipantError::Authority)?;
        resolve_record(world, key, record)
    }

    /// Consume once alongside the actual fee/title/filing post-images. No
    /// caller can turn an arbitrary object id into a reservation here.
    pub(crate) fn consume(
        &self,
        world: &World,
        invocation: &ActionInvocation,
        action: &DefinitionRef,
        txn: &mut ActionTxn<'_>,
        shell: ReservedShell,
        filing: CommitToken,
    ) -> Result<ShellObjects, ParticipantError> {
        let current = self.resolve(world, invocation, action, txn, shell.key())?;
        if current.record != shell.record || current.before != shell.before {
            return Err(ParticipantError::Authority);
        }
        let image = ShellObjects::verify(world, invocation.actor(), current.before)?;
        let mut deployment = txn
            .get(&self.deployment, &())
            .map_err(state_error)?
            .cloned()
            .ok_or(ParticipantError::Authority)?;
        deployment.consumed = deployment
            .consumed
            .checked_add(1)
            .ok_or(ParticipantError::ImageLimit)?;
        deployment.validate()?;
        let mut record = current.record;
        record.state = ShellState::Consumed { filing };
        record.validate()?;
        txn.replace(&self.deployment, &(), deployment)
            .map_err(state_error)?;
        txn.replace(&self.records, &record.key, record.clone())
            .map_err(state_error)?;
        txn.remove(&self.open, &invocation.actor())
            .map_err(state_error)?;
        Ok(image)
    }

    /// Both deployment markers must come from the same actual setup commit.
    pub fn verify_deployment(
        &self,
        ctx: &crate::module::ModuleCtx<'_>,
        initialized_by: CommitToken,
    ) -> Result<(), ParticipantError> {
        let row = ctx
            .action_state(&self.deployment, &())
            .map_err(|_| ParticipantError::Authority)?
            .ok_or(ParticipantError::Authority)?;
        row.validate()?;
        if row.initialized_by != initialized_by {
            return Err(ParticipantError::Authority);
        }
        Ok(())
    }

    /// Read-only setup-mode precondition. This never opens ordinary commerce;
    /// the explicit setup provider repeats the emptiness checks in its txn.
    pub fn empty_for_initialization(
        &self,
        ctx: &crate::module::ModuleCtx<'_>,
    ) -> Result<bool, ParticipantError> {
        let empty = ctx
            .scan_action_state(&self.records, None, 1)
            .map_err(|_| ParticipantError::Authority)?
            .entries()
            .is_empty()
            && ctx
                .scan_action_state(&self.open, None, 1)
                .map_err(|_| ParticipantError::Authority)?
                .entries()
                .is_empty()
            && ctx
                .scan_action_state(&self.deployment, None, 1)
                .map_err(|_| ParticipantError::Authority)?
                .entries()
                .is_empty();
        Ok(empty)
    }

    /// Read-only forward proof used by Roll's final boot audit. A receipt
    /// identifier alone is insufficient: the persisted row, exact inert object
    /// and initial-filing reference must all agree.
    pub fn verify_consumption(
        &self,
        ctx: &crate::module::ModuleCtx<'_>,
        key: ShellKey,
        object: ObjId,
        reservation: CommitToken,
        filing: CommitToken,
    ) -> Result<(), ParticipantError> {
        let row = ctx
            .action_state(&self.records, &key)
            .map_err(|_| ParticipantError::Authority)?
            .ok_or(ParticipantError::Authority)?;
        row.validate()?;
        if row.key != key
            || row.object != object
            || row.receipt != reservation
            || row.state != (ShellState::Consumed { filing })
            || ctx.w.get(object) != Some(&row.object())
        {
            return Err(ParticipantError::Authority);
        }
        Ok(())
    }

    /// Scan bounded pages of actual acknowledged rows and their reverse open
    /// index. The callback only receives immutable consumed facts and must join
    /// them to Roll's operation family; it cannot grant or install authority.
    pub fn audit(
        &self,
        ctx: &crate::module::ModuleCtx<'_>,
        mut consumed: impl FnMut(ConsumedShell) -> Result<(), ParticipantError>,
    ) -> Result<(), ParticipantError> {
        let deployment = ctx
            .action_state(&self.deployment, &())
            .map_err(|_| ParticipantError::Authority)?
            .ok_or(ParticipantError::Authority)?;
        deployment.validate()?;
        let markers = ctx
            .scan_action_state(&self.deployment, None, 2)
            .map_err(|_| ParticipantError::Authority)?;
        if markers.entries().len() != 1
            || markers.entries()[0].encoded_key() != "registry"
            || markers.next_after().is_some()
        {
            return Err(ParticipantError::Authority);
        }
        let mut births = std::collections::BTreeSet::new();
        let mut objects = std::collections::BTreeSet::new();
        let mut consumed_count = 0_u64;
        let mut after = None;
        loop {
            let page = ctx
                .scan_action_state(&self.records, after.as_deref(), 128)
                .map_err(|_| ParticipantError::Authority)?;
            for entry in page.entries() {
                let row = entry.value();
                row.validate()?;
                if entry.encoded_key() != row.key.encoded()
                    || ctx.w.get(row.object) != Some(&row.object())
                    || row.birth > deployment.born
                    || !births.insert(row.birth)
                    || !objects.insert(row.object)
                {
                    return Err(ParticipantError::Authority);
                }
                let open = ctx
                    .action_state(&self.open, &row.key.requester)
                    .map_err(|_| ParticipantError::Authority)?;
                match row.state {
                    ShellState::Reserved {} => {
                        if open.is_none_or(|o| o.format != FORMAT || o.key != row.key) {
                            return Err(ParticipantError::Authority);
                        }
                    }
                    ShellState::Consumed { filing } => {
                        consumed_count = consumed_count
                            .checked_add(1)
                            .ok_or(ParticipantError::ImageLimit)?;
                        if open.is_some_and(|o| o.key == row.key) {
                            return Err(ParticipantError::Authority);
                        }
                        consumed(ConsumedShell {
                            key: row.key,
                            object: row.object,
                            reservation: row.receipt,
                            filing,
                        })?;
                    }
                }
            }
            after = page.next_after().map(ToString::to_string);
            if after.is_none() {
                break;
            }
        }
        if births.len() as u64 != deployment.born || consumed_count != deployment.consumed {
            return Err(ParticipantError::Authority);
        }
        let mut after = None;
        loop {
            let page = ctx
                .scan_action_state(&self.open, after.as_deref(), 128)
                .map_err(|_| ParticipantError::Authority)?;
            for entry in page.entries() {
                let open = entry.value();
                if open.format != FORMAT || entry.encoded_key() != open.key.requester.0.to_string()
                {
                    return Err(ParticipantError::Authority);
                }
                let row = ctx
                    .action_state(&self.records, &open.key)
                    .map_err(|_| ParticipantError::Authority)?
                    .ok_or(ParticipantError::Authority)?;
                if row.key != open.key || row.state != (ShellState::Reserved {}) {
                    return Err(ParticipantError::Authority);
                }
            }
            after = page.next_after().map(ToString::to_string);
            if after.is_none() {
                break;
            }
        }
        Ok(())
    }
}

fn resolve_record(
    world: &World,
    key: ShellKey,
    record: ShellRecord,
) -> Result<ReservedShell, ParticipantError> {
    record.validate()?;
    if record.key != key || record.state != (ShellState::Reserved {}) {
        return Err(ParticipantError::Authority);
    }
    let before = record.object();
    if world.get(record.object) != Some(&before) {
        return Err(ParticipantError::Authority);
    }
    Ok(ReservedShell { record, before })
}

/// Builtin object leg; generic owner adapters still cannot emit object writes.
#[derive(Debug)]
pub(crate) struct ShellObjects {
    requester: ObjId,
    before_next: u32,
    object: Obj,
    create: bool,
    writes: Vec<ReplayWrite>,
}
impl ShellObjects {
    fn create(world: &World, requester: ObjId) -> Result<Self, ParticipantError> {
        require_actor(world, requester)?;
        let next = world.next_id();
        let after = next.checked_add(1).ok_or(ParticipantError::ImageLimit)?;
        if next < 2 || world.get(ObjId(next)).is_some() {
            return Err(ParticipantError::Authority);
        }
        let object = Obj::new(ObjId(next), ObjKind::Thing, NAME, GOD, world.now());
        object_digest(&object)?;
        Ok(Self {
            requester,
            before_next: next,
            object: object.clone(),
            create: true,
            writes: vec![
                ReplayWrite::ObjectSet { object },
                ReplayWrite::ObjectAllocatorSet { next_id: after },
            ],
        })
    }
    fn verify(world: &World, requester: ObjId, object: Obj) -> Result<Self, ParticipantError> {
        require_actor(world, requester)?;
        if world.get(object.id) != Some(&object) {
            return Err(ParticipantError::Authority);
        }
        Ok(Self {
            requester,
            before_next: world.next_id(),
            object,
            create: false,
            writes: Vec::new(),
        })
    }
    pub(crate) fn writes(&self) -> &[ReplayWrite] {
        &self.writes
    }
    pub(crate) const fn creates_object(&self) -> bool {
        self.create
    }
    pub(crate) fn recheck(&self, world: &World) -> Result<(), ParticipantError> {
        require_actor(world, self.requester)?;
        if world.next_id() != self.before_next
            || if self.create {
                world.get(self.object.id).is_some()
            } else {
                world.get(self.object.id) != Some(&self.object)
            }
        {
            return Err(ParticipantError::Authority);
        }
        Ok(())
    }
    pub(crate) fn bind(
        self,
        storage: ShellObjectStorage<'_>,
    ) -> Result<BoundShellObjects<'_>, ParticipantError> {
        let ShellObjectStorage { objects, next_id } = storage;
        if *next_id != self.before_next
            || objects.len() != self.before_next as usize
            || objects
                .get(GOD.0 as usize)
                .and_then(Option::as_ref)
                .is_none()
            || objects
                .get(self.requester.0 as usize)
                .and_then(Option::as_ref)
                .is_none_or(|o| o.kind != ObjKind::Player || o.flags.contains(Flags::GOING))
        {
            return Err(ParticipantError::Authority);
        }
        let after = if self.create {
            if self.object.id.0 != *next_id {
                return Err(ParticipantError::Authority);
            }
            objects
                .try_reserve(1)
                .map_err(|_| ParticipantError::ImageLimit)?;
            next_id.checked_add(1).ok_or(ParticipantError::ImageLimit)?
        } else {
            if objects
                .get(self.object.id.0 as usize)
                .and_then(Option::as_ref)
                != Some(&self.object)
            {
                return Err(ParticipantError::Authority);
            }
            *next_id
        };
        Ok(BoundShellObjects {
            objects,
            next_id,
            object: self.create.then_some(self.object),
            after,
        })
    }
}

/// World creates this field split; no owner callback receives mutable World.
pub(crate) struct ShellObjectStorage<'a> {
    pub(crate) objects: &'a mut Vec<Option<Obj>>,
    pub(crate) next_id: &'a mut u32,
}
pub(crate) struct BoundShellObjects<'a> {
    objects: &'a mut Vec<Option<Obj>>,
    next_id: &'a mut u32,
    object: Option<Obj>,
    after: u32,
}
impl BoundShellObjects<'_> {
    pub(crate) fn apply(self) {
        if let Some(object) = self.object {
            self.objects.push(Some(object));
            *self.next_id = self.after;
        }
        // No indexes change for this unlocated, unzoned Thing. It is already
        // durably written; adding it to legacy dirty tracking would write twice.
    }
}

fn require_actor(world: &World, actor: ObjId) -> Result<(), ParticipantError> {
    if world
        .get(actor)
        .is_none_or(|o| o.kind != ObjKind::Player || o.flags.contains(Flags::GOING))
        || world.get(GOD).is_none()
    {
        return Err(ParticipantError::Authority);
    }
    Ok(())
}
fn definition_is(def: &DefinitionRef, kind: &str, id: &str) -> bool {
    def.key().owner.as_str() == "roll"
        && def.key().kind.as_str() == kind
        && def.key().id.as_str() == id
        && def.version() == 1
}
fn require_action(
    inv: &ActionInvocation,
    action: &DefinitionRef,
    id: &str,
) -> Result<(), ParticipantError> {
    if !definition_is(action, "action", id)
        || inv.principal() != &(PrincipalId::Object { id: inv.actor() })
        || inv.subject() != &PerceivedRef::SelfActor
        || !inv.targets().is_empty()
    {
        return Err(ParticipantError::Authority);
    }
    Ok(())
}
fn require_empty(inv: &ActionInvocation) -> Result<(), ParticipantError> {
    if !inv
        .parameters()
        .as_object()
        .is_some_and(serde_json::Map::is_empty)
    {
        return Err(ParticipantError::Authority);
    }
    Ok(())
}
fn state_error(_: crate::action_state::ActionStateError) -> ParticipantError {
    ParticipantError::Authority
}
fn bounded<T: Serialize>(row: &T) -> Result<Value, ParticipantError> {
    let value = serde_json::to_value(row).map_err(|_| ParticipantError::Encoding)?;
    canonical_json_bytes_with_limit(&value, RECORD_BYTES)
        .map_err(|_| ParticipantError::ImageLimit)?;
    Ok(value)
}
fn encode<T: Serialize>(row: &T) -> Result<Value, String> {
    bounded(row).map_err(|e| e.to_string())
}
fn decode<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T, String> {
    canonical_json_bytes_with_limit(value, RECORD_BYTES).map_err(|e| e.to_string())?;
    serde_json::from_value(value.clone()).map_err(|e| e.to_string())
}
fn object_digest(object: &Obj) -> Result<DefinitionDigest, ParticipantError> {
    DefinitionDigest::for_canonical_value(
        "core.reserved_initial_filing_shell.v1",
        &bounded(object)?,
    )
    .map_err(|_| ParticipantError::Encoding)
}
fn descriptor(name: &str) -> Result<StateFamilyDescriptor, ModError> {
    let id = StateFamilyId::new(name).map_err(|e| ModError::msg(e.to_string()))?;
    let key = |suffix: &str| {
        CanonicalId::new(format!("roll.{name}.{suffix}.v1"))
            .map_err(|e| ModError::msg(e.to_string()))
    };
    StateFamilyDescriptor::new(
        id,
        key("key")?,
        key("value")?,
        FORMAT,
        key("decode")?,
        key("validate")?,
        key("encode")?,
    )
    .and_then(|d| d.with_limits(96, RECORD_BYTES + 1024))
    .map_err(|e| ModError::msg(e.to_string()))
}

struct ShellProvider {
    service: ShellReservations,
    action: DefinitionRef,
    outcome: DefinitionRef,
}
fn action_error() -> ActionError {
    ActionError::new(
        ActionErrorCode::ProviderRejected,
        false,
        "The filing reservation is unavailable.",
    )
    .expect("fixed bounded shell refusal")
}
impl ActionProvider for ShellProvider {
    fn validate_invocation(&self, inv: &ActionInvocation) -> Result<(), ActionError> {
        require_action(inv, &self.action, RESERVE_ACTION)
            .and_then(|()| require_empty(inv))
            .map_err(|_| action_error())
    }
    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &crate::LegalActionQuery,
        engine: &crate::Engine,
    ) -> Result<Option<crate::ProviderOfferDraft>, ActionError> {
        if action != &self.action
            || query.truth_subject() != &(crate::SubjectKey::Object { id: query.actor() })
            || query.projected_subject() != &PerceivedRef::SelfActor
            || require_actor(&engine.world, query.actor()).is_err()
        {
            return Ok(None);
        }
        let definitions = engine.registry.definitions().ok_or_else(action_error)?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(crate::RevisionSet::default())
            .map_err(|_| action_error())?;
        guards
            .add_definition(action.clone(), definitions)
            .map_err(|_| action_error())?;
        Ok(Some(crate::ProviderOfferDraft {
            fixed_parameters: json!({}),
            parameters: crate::ProjectedParameterSchema::new(Vec::new())
                .map_err(|_| action_error())?,
            targets: crate::ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: crate::ActionPresentation::new(
                "roll-reserve-mind-charter",
                "Reserve a mind charter",
                "Reserve the charter object for an initial mind filing.",
            )
            .map_err(|_| action_error())?,
            guards: guards.finish().map_err(|_| action_error())?,
        }))
    }
    fn prepare_authoritative_with_context(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        context: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.validate_invocation(inv)?;
        let (key, object) = context
            .reserve_initial_filing_shell(&self.service, txn)
            .map_err(|_| action_error())?;
        let result = RulesValue::new(
            self.outcome.clone(),
            json!({"reservation":key,"object":object}),
        )
        .map_err(|_| action_error())?;
        Ok(AuthoritativeActionOutput::new(result)
            .with_definitions(vec![self.outcome.clone()])
            .with_eligible_witnesses(vec![inv.principal().clone()]))
    }
}

#[cfg(test)]
mod tests;

//! # The tactics module — exact definitions, lifecycle, persist, and read
//!
//! The cut where M31 `MobilityProfile` and M32 `TacticalUnit` (pure libraries)
//! become LIVE: attached to real objects, persisted, surviving reboot, and
//! finally READ (a show command, and a panel that consumes the M32 damage
//! [`Journal`](crate::unit::Journal)). This module owns NS `tactics`; the
//! seam-test stub moved to `tacseam` so both can load in one Engine.
//!
//! ## Contracts (from the M33 design review)
//! - **The in-RAM slot [`UnitStore`] is the single source of truth.** Every
//!   mutation reads the slot, replaces the entry with the returned unit, THEN
//!   persists the whole `unit:<obj>` doc from that entry — never a detached
//!   clone (monolithic doc: a whole-unit write clobbers the same key).
//! - **Two orthogonal states on one object.** A tactical unit's section tree is
//!   entirely in mush-tactics; the same object may independently be a mush-sim
//!   placed participant, a journey traveler, or neither. Tactics boot consults
//!   ONLY `w.get(obj)` (never `SpatialState`) — create-eligibility and
//!   boot-eligibility use the same World-only predicate (the M30 lesson).
//! - **Boot is observational.** Binding/document disagreements become bounded
//!   recovery incidents and repair previews. Boot never rewrites a legacy
//!   record, deletes a corrupt record, or silently attaches/detaches a marker.
//! - **Panel: staff/self only, always [`Audience::Actor`].** No crew fan-out and
//!   no `SpatialState` read; every payload routes through [`project`], whose M33
//!   body ignores the observer and returns full state, so the epistemic
//!   projector later swaps the body without touching emit topology.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use mush_core::module::{
    GameModule, ModError, ModuleBootData, ModuleCtx, Registrar, WorldEvent, WorldEventKind,
};
use mush_core::output::display_name;
use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionPresentation, ActionProvider,
    ActionStateError, ActionTxn, ArgShape, Audience, AuthoritativeActionOutput, AuthorityGuardKind,
    BehaviorBinding, BehaviorId, BehaviorSpec, CanonicalId, CmdError, CommandSpec,
    ContactGuardValidator, ContactStateCodec, Ctx, Deferred, DefinitionDescriptor,
    DefinitionDigest, DefinitionId, DefinitionKind, DefinitionLookupError,
    DefinitionMigrationDescriptor, DefinitionRef, DefinitionValidationContext, DestroyGate,
    DetachGate, Engine, Flags, IncidentId, IncidentState, LegalActionQuery,
    MAX_BOOT_REPORT_DETAILS, MechanicalState, MigrationPlanSummary, ModuleBootReport, ModuleOp,
    ModuleWrite, ObjId, ObjKind, Outbox, OwnerId, PanelFeedHandle, PerceivedRef, Powers,
    PrincipalId, ProjectedParameterSchema, ProjectedTargetSchema, ProviderOfferDraft,
    RecoveryIncident, RecoveryReason, RecoveryReporter, RecoveryScope, RecoverySeverity,
    RecoverySubject, Registry, RevisionSet, Role, RulesValue, StateFamilyDescriptor,
    StateFamilyHandle, StateFamilyId, SubjectKey, World, canonical_json_bytes_with_limit,
    contact_state_family_descriptor, match_object, perm,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use smol_str::SmolStr;

use crate::unit::{
    self, TacticalUnit, UnitCatalog, UnitMutationOp, UnitMutationPlan, decode_legacy_unit_v1,
};

/// The module id and persistence namespace.
pub const NS: &str = "tactics";
/// The marker behavior: an object carrying it IS a tactical unit.
pub const UNIT_BEHAVIOR: &str = "tactics.unit";
/// The panel feed id (the reserved `tactics.unit` view).
pub const UNIT_FEED: &str = "tactics.unit";
/// Current persistence envelope version. Version 1 was the original untagged
/// `TacticalUnit` document with contextual string references.
pub const UNIT_RECORD_VERSION: u32 = 2;
/// Current object-behavior binding schema.
pub const UNIT_BINDING_VERSION: u32 = 2;
const UNIT_RECORD_NAME: &str = "mush-tactics.unit";
/// Maximum canonical bytes in one authoritative unit document.
pub const MAX_UNIT_RECORD_BYTES: usize = mush_core::MAX_RULES_VALUE_BYTES;
/// Maximum repair previews retained/rendered in diagnostics. The aggregate
/// count remains exact even when details are truncated.
pub const MAX_UNIT_BINDING_REPAIRS: usize = MAX_BOOT_REPORT_DETAILS;
/// Production action definition id for the first K3-migrated tactics action.
pub const BRACE_ACTION_ID: &str = "brace";
/// Semantic outcome definition id returned by [`BRACE_ACTION_ID`].
pub const BRACED_OUTCOME_ID: &str = "braced";
/// Engine-owned typed family mutated by the brace provider.
pub const STANCE_STATE_FAMILY: &str = "stance";
/// Private authoritative mapping for projected tactical contact IDs.
pub const CONTACT_STATE_FAMILY: &str = "contacts";

/// Minimal rules-neutral posture state used by the first production K3
/// provider. It intentionally lives beside, rather than inside,
/// [`TacticalUnit`]: concrete rulesets decide what mechanical modifiers a
/// braced posture grants.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticalStance {
    pub braced: bool,
    pub revision: u64,
}

/// A boot-discovered binding disagreement. K1 reports these without mutating
/// either the object or the authoritative module record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnitBindingRepair {
    /// Object whose binding requires an explicit repair transaction.
    pub object: ObjId,
    /// Authoritative unit-record key, or `None` for a stale binding/husk.
    pub record_key: Option<SmolStr>,
    /// Binding schema observed at boot (`None` means no binding).
    pub found_schema: Option<u32>,
    /// Schema a current unit record requires.
    pub expected_schema: u32,
}

/// Inspectable lifecycle metadata for every discovered unit record. Raw bytes
/// remain authoritative in the backing store and are deliberately not copied
/// into staff diagnostics.
#[derive(Clone, Debug, PartialEq)]
pub struct UnitRecordStatus {
    /// Exact raw persistence key that was discovered.
    pub key: SmolStr,
    /// Object identity claimed by a canonical `unit:<obj>` persistence key.
    /// This remains distinct from the embedded identity so a mis-keyed record
    /// keeps custody of both subjects and cannot be overwritten by live create.
    pub key_object: Option<ObjId>,
    /// Embedded object when the record decoded far enough to identify it.
    pub object: Option<ObjId>,
    /// Whether mechanics may interpret the record.
    pub state: MechanicalState,
    /// Decoded state retained for inspection while frozen/migrating.
    pub unit: Option<TacticalUnit>,
    /// Bounded digest of the authoritative JSON payload, when encodable.
    pub payload_digest: Option<DefinitionDigest>,
    /// Exact dry-run plan for an accepted legacy v1 document.
    pub migration_plan: Option<MigrationPlanSummary>,
}

impl UnitRecordStatus {
    fn claims_object(&self, object: ObjId) -> bool {
        self.key_object == Some(object) || self.object == Some(object)
    }
}

/// The in-RAM unit slot — the single source of truth; each unit rides a durable
/// `unit:<obj>` doc.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UnitStore {
    /// Active tactical units by object. Frozen, migrating, and quarantined
    /// records never enter this mechanics-facing index.
    pub units: BTreeMap<ObjId, TacticalUnit>,
    /// Ordered status for every unit document discovered at boot.
    pub records: Vec<UnitRecordStatus>,
    /// Binding repair previews; no repair is applied before K3.
    pub binding_repairs: Vec<UnitBindingRepair>,
    /// Total binding disagreements observed, including truncated previews.
    pub binding_repair_count: u64,
    /// Bounded diagnostics snapshot exposed through `@stats`.
    pub boot_report: Option<ModuleBootReport>,
}

impl UnitStore {
    fn push_binding_repair(&mut self, repair: UnitBindingRepair, report: &mut ModuleBootReport) {
        self.binding_repair_count = self.binding_repair_count.saturating_add(1);
        if self.binding_repairs.len() < MAX_UNIT_BINDING_REPAIRS {
            self.binding_repairs.push(repair);
        } else {
            report.details_truncated = true;
        }
    }

    /// Install a newly authored active unit into both mechanics and the
    /// lifecycle index. Keeping these views together prevents a boot-loaded
    /// status from becoming a stale detach/create gate after live mutation.
    fn install_active(&mut self, unit: TacticalUnit, payload_digest: DefinitionDigest) {
        let object = unit.object();
        let key = unit_key(object);
        let mut removed_active = 0_u64;
        self.records.retain(|record| {
            let remove = record.object == Some(object) && record.state == MechanicalState::Active;
            if remove {
                removed_active = removed_active.saturating_add(1);
            }
            !remove
        });
        self.binding_repairs
            .retain(|repair| repair.object != object);
        self.units.insert(object, unit.clone());
        self.records.push(UnitRecordStatus {
            key,
            key_object: Some(object),
            object: Some(object),
            state: MechanicalState::Active,
            unit: Some(unit),
            payload_digest: Some(payload_digest),
            migration_plan: None,
        });
        if let Some(report) = &mut self.boot_report {
            report.active = report
                .active
                .saturating_sub(removed_active)
                .saturating_add(1);
        }
    }

    fn update_active(&mut self, unit: TacticalUnit, payload_digest: DefinitionDigest) {
        let object = unit.object();
        self.units.insert(object, unit.clone());
        if let Some(record) = self
            .records
            .iter_mut()
            .find(|record| record.object == Some(object) && record.state == MechanicalState::Active)
        {
            record.key = unit_key(object);
            record.unit = Some(unit);
            record.payload_digest = Some(payload_digest);
            record.migration_plan = None;
        } else {
            self.records.push(UnitRecordStatus {
                key: unit_key(object),
                key_object: Some(object),
                object: Some(object),
                state: MechanicalState::Active,
                unit: Some(unit),
                payload_digest: Some(payload_digest),
                migration_plan: None,
            });
            if let Some(report) = &mut self.boot_report {
                report.active = report.active.saturating_add(1);
            }
        }
    }

    fn remove_active(&mut self, object: ObjId) -> bool {
        let removed = self.units.remove(&object).is_some();
        if !removed {
            return false;
        }
        let before = self.records.len();
        self.records.retain(|record| {
            !(record.object == Some(object) && record.state == MechanicalState::Active)
        });
        let removed_records = before.saturating_sub(self.records.len()) as u64;
        let before_repairs = self.binding_repairs.len();
        self.binding_repairs
            .retain(|repair| repair.object != object);
        let removed_repairs = before_repairs.saturating_sub(self.binding_repairs.len()) as u64;
        self.binding_repair_count = self.binding_repair_count.saturating_sub(removed_repairs);
        if let Some(report) = &mut self.boot_report {
            report.active = report.active.saturating_sub(removed_records.max(1));
        }
        true
    }

    fn hold_active(&mut self, object: ObjId, incident: IncidentId) -> bool {
        if self.units.remove(&object).is_none() {
            return false;
        }
        let mut held_records = 0_u64;
        for record in &mut self.records {
            if record.object == Some(object) && record.state == MechanicalState::Active {
                record.state = MechanicalState::Frozen { incident };
                held_records = held_records.saturating_add(1);
            }
        }
        if let Some(report) = &mut self.boot_report {
            let held_records = held_records.max(1);
            report.active = report.active.saturating_sub(held_records);
            report.frozen = report.frozen.saturating_add(held_records);
        }
        true
    }
}

/// The tactics [`GameModule`]. Construct with a frozen [`UnitCatalog`] (composed
/// at wiring time; empty by default — the engine seeds no concrete spec).
#[derive(Debug)]
pub struct TacticsModule {
    catalog: Arc<UnitCatalog>,
    legacy_record_definition: Option<DefinitionRef>,
    current_record_definition: Option<DefinitionRef>,
    /// E7c7 B3: the composition-installed observation policy the sensor ingress
    /// reads. `mush-propagation` supplies it; `mush-tactics` never learns which
    /// crate did, because the trait is `mush-core`'s. Unwired arms nothing.
    sensors: mush_core::SensorPolicyHandle,
}

impl Default for TacticsModule {
    fn default() -> Self {
        Self::new()
    }
}

impl TacticsModule {
    /// A module with an empty catalog (no unit schemas registered).
    #[must_use]
    pub fn new() -> TacticsModule {
        TacticsModule {
            catalog: Arc::new(UnitCatalog::default()),
            legacy_record_definition: None,
            current_record_definition: None,
            sensors: mush_core::SensorPolicyHandle::unwired(),
        }
    }
    /// A module with a frozen catalog (a ruleset / dev wiring supplies it).
    #[must_use]
    pub fn with_catalog(catalog: UnitCatalog) -> TacticsModule {
        TacticsModule {
            catalog: Arc::new(catalog),
            legacy_record_definition: None,
            current_record_definition: None,
            sensors: mush_core::SensorPolicyHandle::unwired(),
        }
    }

    /// **E7 cut 7 B3, OPEN 1 — the composition seam that keeps `mush-tactics`
    /// the sole `ContactState` authority.**
    ///
    /// The kernel's owner fence (`ScheduleSourceRegistryBuilder::register` and
    /// `scheduled_action_ingress`, both in `mush-core/src/schedule_source.rs`)
    /// forces source-owner == schedule-family-owner == fired-action-owner, and
    /// the six `contact_*` actions are this module's — so a
    /// `mush-propagation`-owned schedule source **cannot** fire them and the
    /// observation ingress has to be registered here. The *policy* is still
    /// propagation's: it arrives as an `Arc<dyn mush_core::SensorPolicy>` at
    /// `server_modules_with`, in the `IndustryModule::with_recipes` /
    /// `SimulationModule::with_accounts` shape, so **`mush-tactics` gains no
    /// `mush-propagation` edge**. Unwired, the ingress arms nothing at all.
    #[must_use]
    pub fn with_sensor_policy(mut self, policy: Arc<dyn mush_core::SensorPolicy>) -> TacticsModule {
        self.sensors = mush_core::SensorPolicyHandle::new(policy);
        self
    }
}

/// Who may arm the sensor ingress (E7c7 B3).
///
/// **Staff only.** A player who could arm it could author another actor's
/// picture, which is the exact hazard the K lens exists to close; the offer
/// simply does not exist for anyone else, so there is no refusal to probe with
/// either.
#[must_use]
pub fn sensor_arm_is_permitted(world: &World, actor: ObjId) -> bool {
    perm::has_power(world, actor, Powers::SIM_ADMIN) || perm::has_role(world, actor, Role::Royalty)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct UnitDocumentV2 {
    record: SmolStr,
    record_version: u32,
    /// Exact schema identity for this persisted envelope. The human-readable
    /// name/version fields are diagnostic; this digest-bound reference is the
    /// authority that decides whether these bytes may be interpreted.
    record_schema: DefinitionRef,
    unit: TacticalUnit,
}

#[derive(Debug, Eq, PartialEq)]
enum UnitDocumentV2DecodeError {
    Invalid(&'static str),
    DefinitionDigestMismatch {
        requested: Box<DefinitionRef>,
        registered: Box<DefinitionRef>,
    },
}

impl UnitDocumentV2 {
    fn new(record_schema: DefinitionRef, unit: TacticalUnit) -> Self {
        Self {
            record: SmolStr::new_static(UNIT_RECORD_NAME),
            record_version: UNIT_RECORD_VERSION,
            record_schema,
            unit,
        }
    }

    fn decode(
        value: serde_json::Value,
        expected_schema: &DefinitionRef,
    ) -> Result<Self, UnitDocumentV2DecodeError> {
        // Authenticate the small strict envelope before interpreting the
        // ruleset-shaped TacticalUnit body. A stale/forged schema identity may
        // be retained for recovery, but its payload never reaches typed decode.
        let object = value
            .as_object()
            .ok_or(UnitDocumentV2DecodeError::Invalid("unit_v2_malformed"))?;
        const FIELDS: [&str; 4] = ["record", "record_version", "record_schema", "unit"];
        if object.len() != FIELDS.len() || FIELDS.iter().any(|field| !object.contains_key(*field)) {
            return Err(UnitDocumentV2DecodeError::Invalid("unit_v2_malformed"));
        }
        if object.get("record").and_then(serde_json::Value::as_str) != Some(UNIT_RECORD_NAME)
            || object
                .get("record_version")
                .and_then(serde_json::Value::as_u64)
                != Some(u64::from(UNIT_RECORD_VERSION))
        {
            return Err(UnitDocumentV2DecodeError::Invalid("unit_schema_unknown"));
        }
        let record_schema: DefinitionRef = serde_json::from_value(
            object
                .get("record_schema")
                .cloned()
                .ok_or(UnitDocumentV2DecodeError::Invalid("unit_v2_malformed"))?,
        )
        .map_err(|_| UnitDocumentV2DecodeError::Invalid("unit_v2_malformed"))?;
        if &record_schema != expected_schema {
            if record_schema.locator() == expected_schema.locator()
                && record_schema.digest() != expected_schema.digest()
            {
                return Err(UnitDocumentV2DecodeError::DefinitionDigestMismatch {
                    requested: Box::new(record_schema),
                    registered: Box::new(expected_schema.clone()),
                });
            }
            return Err(UnitDocumentV2DecodeError::Invalid(
                "unit_record_schema_mismatch",
            ));
        }
        let unit: TacticalUnit = serde_json::from_value(
            object
                .get("unit")
                .cloned()
                .ok_or(UnitDocumentV2DecodeError::Invalid("unit_v2_malformed"))?,
        )
        .map_err(|_| UnitDocumentV2DecodeError::Invalid("unit_v2_malformed"))?;
        Ok(Self::new(record_schema, unit))
    }
}

fn mechanics_descriptor(
    kind: &str,
    id: &str,
    version: u32,
    payload: serde_json::Value,
) -> Result<DefinitionDescriptor, ModError> {
    let kind = DefinitionKind::new(kind).map_err(|error| ModError::msg(error.to_string()))?;
    let id = DefinitionId::new(id).map_err(|error| ModError::msg(error.to_string()))?;
    DefinitionDescriptor::new(kind, id, version, payload)
        .map_err(|error| ModError::msg(error.to_string()))
}

#[derive(Clone)]
struct ExpectedDefinitionShape {
    payload: serde_json::Value,
    references: Vec<DefinitionRef>,
    current: bool,
}

fn exact_definition_validator(
    descriptors: &[DefinitionDescriptor],
) -> impl Fn(&DefinitionValidationContext<'_>) -> Result<(), String> + Send + Sync + 'static {
    let expected: BTreeMap<_, _> = descriptors
        .iter()
        .map(|descriptor| {
            let mut references = descriptor.references().to_vec();
            references.sort();
            (
                (descriptor.id().clone(), descriptor.version()),
                ExpectedDefinitionShape {
                    payload: descriptor.payload().clone(),
                    references,
                    current: descriptor.is_current(),
                },
            )
        })
        .collect();
    move |context: &DefinitionValidationContext<'_>| {
        let entry = context.definition();
        let key = (
            entry.reference().key().id.clone(),
            entry.reference().version(),
        );
        let Some(shape) = expected.get(&key) else {
            return Err("unknown definition id or version for this tactics catalog".into());
        };
        if entry.payload() != &shape.payload
            || entry.references() != shape.references
            || entry.is_current() != shape.current
        {
            return Err("definition does not match its typed tactics descriptor".into());
        }
        Ok(())
    }
}

fn braced_outcome_descriptor() -> Result<DefinitionDescriptor, ModError> {
    Ok(mechanics_descriptor(
        "outcome",
        BRACED_OUTCOME_ID,
        1,
        json!({
            "fields": {
                "actor": "obj_id",
                "braced": "bool",
                "revision": "u64"
            }
        }),
    )?
    .as_current())
}

fn brace_action_descriptor(outcome: DefinitionRef) -> Result<DefinitionDescriptor, ModError> {
    Ok(mechanics_descriptor(
        "action",
        BRACE_ACTION_ID,
        1,
        json!({
            "subject": "self_actor",
            "targets": 0,
            "parameters": {},
            "state_family": STANCE_STATE_FAMILY
        }),
    )?
    .with_references(vec![outcome])
    .as_current())
}

#[derive(Debug)]
struct BraceProvider {
    action: DefinitionRef,
    stance: StateFamilyHandle<ObjId, TacticalStance>,
    outcome: DefinitionRef,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BracedOutcome {
    actor: u32,
    braced: bool,
    revision: u64,
}

impl BraceProvider {
    fn validate(invocation: &ActionInvocation) -> Result<(), ActionError> {
        if invocation.subject() != &PerceivedRef::SelfActor
            || !invocation.targets().is_empty()
            || invocation
                .parameters()
                .as_object()
                .is_none_or(|parameters| !parameters.is_empty())
        {
            return Err(ActionError::new(
                ActionErrorCode::InvalidRequest,
                false,
                "tactics brace requires self, no targets, and no parameters",
            )
            .expect("static tactics action error is bounded"));
        }
        Ok(())
    }

    fn state_error(_error: &ActionStateError) -> ActionError {
        ActionError::new(
            ActionErrorCode::Integrity,
            false,
            "tactics brace state preparation was refused",
        )
        .expect("static tactics state error is bounded")
    }

    fn prepare_stance(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<TacticalStance, ActionError> {
        Self::validate(invocation)?;
        let actor = invocation.actor();
        let prior = txn
            .get(&self.stance, &actor)
            .map_err(|error| Self::state_error(&error))?
            .cloned();
        let next = TacticalStance {
            braced: true,
            revision: prior
                .as_ref()
                .map_or(Ok(1), |state| state.revision.checked_add(1).ok_or(()))
                .map_err(|()| {
                    ActionError::new(
                        ActionErrorCode::BoundsExceeded,
                        false,
                        "tactics brace revision is exhausted",
                    )
                    .expect("static tactics bounds error is bounded")
                })?,
        };
        if prior.is_some() {
            txn.replace(&self.stance, &actor, next.clone())
        } else {
            txn.insert(&self.stance, &actor, next.clone())
        }
        .map_err(|error| Self::state_error(&error))?;
        Ok(next)
    }
}

impl ActionProvider for BraceProvider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        Self::validate(invocation)
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &LegalActionQuery,
        engine: &Engine,
    ) -> Result<Option<ProviderOfferDraft>, ActionError> {
        let SubjectKey::Object { id: subject } = query.truth_subject() else {
            return Ok(None);
        };
        if *subject != query.actor() || query.projected_subject() != &PerceivedRef::SelfActor {
            return Ok(None);
        }
        let definitions = engine.registry.definitions().ok_or_else(|| {
            ActionError::new(
                ActionErrorCode::Integrity,
                false,
                "tactics definition registry is unavailable",
            )
            .expect("static tactics projection error is bounded")
        })?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())
            .map_err(|_| {
                ActionError::new(
                    ActionErrorCode::Integrity,
                    false,
                    "tactics guard registry is unavailable",
                )
                .expect("static tactics projection error is bounded")
            })?;
        guards
            .add_definition(action.clone(), definitions)
            .map_err(|_| {
                ActionError::new(
                    ActionErrorCode::Integrity,
                    false,
                    "tactics brace definition guard is unavailable",
                )
                .expect("static tactics projection error is bounded")
            })?;
        Ok(Some(ProviderOfferDraft {
            fixed_parameters: json!({}),
            parameters: ProjectedParameterSchema::empty(),
            targets: ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: ActionPresentation::new(
                "brace",
                "Brace",
                "Adopt a braced tactical stance.",
            )
            .map_err(|_| {
                ActionError::new(
                    ActionErrorCode::Integrity,
                    false,
                    "tactics brace presentation is invalid",
                )
                .expect("static tactics projection error is bounded")
            })?,
            guards: guards.finish().map_err(|_| {
                ActionError::new(
                    ActionErrorCode::Integrity,
                    false,
                    "tactics brace offer guards are invalid",
                )
                .expect("static tactics projection error is bounded")
            })?,
        }))
    }

    fn project_receipt(
        &self,
        action: &DefinitionRef,
        outcome: &RulesValue,
        viewer: &PrincipalId,
    ) -> Result<Option<mush_proto::ProjectedActionResult>, ActionError> {
        if action != &self.action || outcome.schema() != &self.outcome {
            return Err(ActionError::new(
                ActionErrorCode::Integrity,
                false,
                "tactics brace receipt carries the wrong durable schema",
            )
            .expect("static tactics projection error is bounded"));
        }
        let decoded: BracedOutcome =
            serde_json::from_value(outcome.data().clone()).map_err(|_| {
                ActionError::new(
                    ActionErrorCode::Integrity,
                    false,
                    "tactics brace receipt outcome is malformed",
                )
                .expect("static tactics projection error is bounded")
            })?;
        if !decoded.braced
            || decoded.revision == 0
            || viewer
                != &(PrincipalId::Object {
                    id: ObjId(decoded.actor),
                })
        {
            return Err(ActionError::new(
                ActionErrorCode::Integrity,
                false,
                "tactics brace receipt outcome disagrees with its eligible viewer",
            )
            .expect("static tactics projection error is bounded"));
        }
        mush_proto::ProjectedActionResult::new(
            "brace",
            "Brace committed.",
            json!({
                "braced": decoded.braced,
                "revision": decoded.revision,
            }),
        )
        .map(Some)
        .map_err(|_| {
            ActionError::new(
                ActionErrorCode::BoundsExceeded,
                false,
                "tactics brace receipt projection exceeds public bounds",
            )
            .expect("static tactics projection error is bounded")
        })
    }

    fn prepare_invocation(
        &self,
        _invocation: &ActionInvocation,
        _txn: &mut ActionTxn<'_>,
    ) -> Result<(), ActionError> {
        Err(ActionError::new(
            ActionErrorCode::Unavailable,
            false,
            "tactics brace requires the authoritative action coordinator",
        )
        .expect("static tactics migration error is bounded"))
    }

    fn prepare_authoritative(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let next = self.prepare_stance(invocation, txn)?;
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({
                "actor": invocation.actor().0,
                "braced": next.braced,
                "revision": next.revision,
            }),
        )
        .map_err(|_| {
            ActionError::new(
                ActionErrorCode::BoundsExceeded,
                false,
                "tactics brace outcome exceeded canonical bounds",
            )
            .expect("static tactics outcome error is bounded")
        })?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

impl GameModule for TacticsModule {
    fn name(&self) -> &'static str {
        NS
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        reg.state(UnitStore::default());

        let stance = reg.state_family(
            StateFamilyDescriptor::new(
                StateFamilyId::new(STANCE_STATE_FAMILY)
                    .map_err(|error| ModError::msg(error.to_string()))?,
                CanonicalId::new("obj_id.decimal.v1")
                    .map_err(|error| ModError::msg(error.to_string()))?,
                CanonicalId::new("tactics.stance.v1")
                    .map_err(|error| ModError::msg(error.to_string()))?,
                1,
                CanonicalId::new("tactics.stance.decode_json.v1")
                    .map_err(|error| ModError::msg(error.to_string()))?,
                CanonicalId::new("tactics.stance.validate.v1")
                    .map_err(|error| ModError::msg(error.to_string()))?,
                CanonicalId::new("tactics.stance.json.v1")
                    .map_err(|error| ModError::msg(error.to_string()))?,
            )
            .map_err(|error| ModError::msg(error.to_string()))?,
            |key: &ObjId| Ok(key.0.to_string()),
            |value: &serde_json::Value| {
                serde_json::from_value(value.clone()).map_err(|error| error.to_string())
            },
            |value: &TacticalStance| {
                if value.revision == 0 {
                    Err("tactics stance revision must be non-zero".to_owned())
                } else {
                    Ok(())
                }
            },
            |value: &TacticalStance| serde_json::to_value(value).map_err(|error| error.to_string()),
        )?;
        let contact_codec = ContactStateCodec;
        let contacts = reg.state_family(
            contact_state_family_descriptor(
                StateFamilyId::new(CONTACT_STATE_FAMILY)
                    .map_err(|error| ModError::msg(error.to_string()))?,
            )
            .map_err(|error| ModError::msg(error.to_string()))?,
            contact_codec,
            contact_codec,
            contact_codec,
            contact_codec,
        )?;
        reg.authority_guard_validator(
            AuthorityGuardKind::Contact,
            ContactGuardValidator::new(contacts.clone()),
        )?;

        let outcome_descriptor = braced_outcome_descriptor()?;
        let contact_outcome_descriptor = crate::contact::outcome_descriptor()?;
        // E7c7 B3: the sensor ingress's own outcome joins the ONE exact
        // "outcome" validator — a second `definition_validator` for one kind is
        // not a thing the registrar allows, so the ingress contributes
        // descriptors here rather than registering its own.
        let sensor_outcome_descriptor = crate::sensor::arm_outcome_descriptor()?;
        reg.definition_validator(
            DefinitionKind::new("outcome").map_err(|error| ModError::msg(error.to_string()))?,
            exact_definition_validator(&[
                outcome_descriptor.clone(),
                contact_outcome_descriptor.clone(),
                sensor_outcome_descriptor.clone(),
            ]),
        )?;
        let outcome = reg.definition(outcome_descriptor)?;
        let contact_outcome = reg.definition(contact_outcome_descriptor)?;
        let sensor_outcome = reg.definition(sensor_outcome_descriptor)?;
        let action_descriptor = brace_action_descriptor(outcome.clone())?;
        let contact_action_descriptors = crate::contact::action_descriptors(&contact_outcome)?;
        let sensor_arm_descriptor = crate::sensor::arm_action_descriptor(&sensor_outcome)?;
        let mut action_descriptors = Vec::with_capacity(2 + contact_action_descriptors.len());
        action_descriptors.push(action_descriptor.clone());
        action_descriptors.extend(
            contact_action_descriptors
                .iter()
                .map(|(_, descriptor)| descriptor.clone()),
        );
        action_descriptors.push(sensor_arm_descriptor.clone());
        reg.definition_validator(
            DefinitionKind::new("action").map_err(|error| ModError::msg(error.to_string()))?,
            exact_definition_validator(&action_descriptors),
        )?;
        let action = reg.definition(action_descriptor)?;
        let contact_actions = contact_action_descriptors
            .into_iter()
            .map(|(operation, descriptor)| {
                reg.definition(descriptor).map(|action| (operation, action))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let sensor_arm = reg.definition(sensor_arm_descriptor)?;
        let sensor_actions: Vec<(mush_core::ObservationKind, DefinitionRef)> = contact_actions
            .iter()
            .map(|(operation, action)| (operation.kind(), action.clone()))
            .collect();
        reg.action_provider(
            action.clone(),
            BraceProvider {
                action,
                stance,
                outcome,
            },
        )?;
        crate::contact::register_providers(
            reg,
            contacts.clone(),
            contact_outcome,
            contact_actions,
        )?;
        // E7c7 B3: the trusted sensor-policy ingress. Registered here, under
        // THIS owner, because the kernel owner fence forbids any other module
        // from firing a `contact_*` action (see `crate::sensor`'s header).
        crate::sensor::register(
            reg,
            contacts.clone(),
            sensor_outcome,
            sensor_arm,
            &sensor_actions,
            self.sensors.clone(),
        )?;
        // E7c7 B3: the leak-safe contact-ledger projection. `scan_action_state`
        // is owner-fenced, so only THIS module can read `tactics/contacts` —
        // and a policy that cannot name a `ContactId` could only ever arm an
        // `Observe`. The projection publishes the holder's own view (opaque id,
        // counters, liveness, and propagation's own phantom id where the
        // subject is one) into `World` state; no real unit's `ObjId` crosses.
        reg.state(mush_core::ContactLedgerState::default());
        let ledger_contacts = contacts;
        let ledger_owner = OwnerId::new(NS).map_err(|error| ModError::msg(error.to_string()))?;
        let ledger_scan = std::sync::Mutex::new(crate::sensor::LedgerScan::default());
        reg.tick(
            crate::sensor::LEDGER_TICK_TAG,
            mush_core::module::TickCadence::Coarse(std::time::Duration::from_secs(
                crate::sensor::LEDGER_TICK_SECS,
            )),
            Arc::new(
                move |ctx: &mut Ctx<'_>, _tag: &str, _scene: Option<mush_core::SceneId>| {
                    // A poisoned lock is a bug in a handler that cannot panic; the
                    // projection simply does not advance this fire.
                    if let Ok(mut staging) = ledger_scan.lock() {
                        crate::sensor::ledger_tick(
                            ctx,
                            &ledger_contacts,
                            &ledger_owner,
                            &mut staging,
                        );
                    }
                },
            ),
        )?;

        // The record format itself is versioned so an old untagged unit can be
        // dry-run through the shared migration registry. The callback captures
        // only the frozen typed catalog; it has no World/store handle.
        let v1_descriptor = mechanics_descriptor(
            "record",
            "unit",
            1,
            json!({"encoding": "legacy_untagged_tactical_unit", "strict": true}),
        )?;
        let v2_descriptor = mechanics_descriptor(
            "record",
            "unit",
            UNIT_RECORD_VERSION,
            json!({
                "record": UNIT_RECORD_NAME,
                "record_version": UNIT_RECORD_VERSION,
                "strict": true
            }),
        )?
        .as_current();
        reg.definition_validator(
            DefinitionKind::new("record").map_err(|error| ModError::msg(error.to_string()))?,
            exact_definition_validator(&[v1_descriptor.clone(), v2_descriptor.clone()]),
        )?;
        let v1 = reg.definition(v1_descriptor)?;
        let v2 = reg.definition(v2_descriptor)?;
        let legacy_aliases: Vec<_> = self
            .catalog
            .legacy_v1_aliases()
            .map(|(id, version, target)| {
                json!({"legacy_id": id, "legacy_version": version, "target": target})
            })
            .collect();
        let migration_descriptor = mechanics_descriptor(
            "migration",
            "unit.v1_to_v2",
            1,
            json!({
                "adapter": "catalog_contextual",
                "from": 1,
                "to": 2,
                "legacy_aliases": legacy_aliases
            }),
        )?
        .with_references(vec![v1.clone(), v2.clone()]);
        reg.definition_validator(
            DefinitionKind::new("migration").map_err(|error| ModError::msg(error.to_string()))?,
            exact_definition_validator(std::slice::from_ref(&migration_descriptor)),
        )?;
        let migration_definition = reg.definition(migration_descriptor)?;
        let migration_catalog = Arc::clone(&self.catalog);
        let migration_target = v2.clone();
        reg.definition_migration(DefinitionMigrationDescriptor::new(
            migration_definition,
            v1.clone(),
            v2.clone(),
            move |legacy| {
                let decoded = decode_legacy_unit_v1(legacy, &migration_catalog)
                    .map_err(|error| error.to_string())?;
                serde_json::to_value(UnitDocumentV2::new(migration_target.clone(), decoded.unit))
                    .map_err(|error| error.to_string())
            },
        ))?;
        self.legacy_record_definition = Some(v1);
        self.current_record_definition = Some(v2.clone());
        reg.recovery_reporter(Arc::new(TacticsRecoveryReporter))?;

        let mut spec = BehaviorSpec::new(BehaviorId::new(UNIT_BEHAVIOR).unwrap())
            .with_binding_schema_version(UNIT_BINDING_VERSION);
        spec.allowed_kinds = vec![ObjKind::Thing, ObjKind::Player, ObjKind::Npc];
        reg.behavior(spec)?;
        reg.destroy_gate(Arc::new(TacticsDestroyGate));
        reg.detach_gate(Arc::new(TacticsDetachGate));
        let destroy_record_definition = v2.clone();
        reg.subscribe(
            WorldEventKind::ObjDestroyed,
            Arc::new(move |ctx: &mut Ctx<'_>, event: &WorldEvent| {
                on_obj_destroyed(&destroy_record_definition, ctx, event);
            }),
        );

        let feed = reg.panel_feed(UNIT_FEED)?;
        let catalog = self.catalog.clone();
        let current_record_definition = v2;
        reg.command(
            CommandSpec {
                name: "@unit".into(),
                shape: ArgShape::EqPair,
                switches: ["create", "show", "damage", "remove", "recovery"]
                    .iter()
                    .copied()
                    .map(SmolStr::new)
                    .collect(),
                help: "@unit/create <obj>=<schema-id>   make an object a tactical unit\n\
                       @unit/show <obj>                 inspect it\n\
                       @unit/damage <obj>={json plan}   (staff/debug) apply a UnitMutationPlan\n\
                       @unit/remove <obj>               strip the tactical unit\n\
                       @unit/recovery                   inspect K1 lifecycle/recovery state"
                    .into(),
                ..CommandSpec::default()
            },
            Arc::new(move |ctx: &mut Ctx<'_>| {
                cmd_unit(&catalog, &feed, &current_record_definition, ctx)
            }),
        )?;
        Ok(())
    }

    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        let legacy = self
            .legacy_record_definition
            .clone()
            .ok_or_else(|| ModError::msg("tactics legacy record definition was not registered"))?;
        let current = self
            .current_record_definition
            .clone()
            .ok_or_else(|| ModError::msg("tactics current record definition was not registered"))?;
        boot_units(ctx, data, &self.catalog, &legacy, &current)
    }
}

// ---- persistence ------------------------------------------------------

fn unit_key(obj: ObjId) -> SmolStr {
    SmolStr::new(format!("unit:{}", obj.0))
}
fn encode_unit_document(
    record_schema: &DefinitionRef,
    unit: &TacticalUnit,
) -> Result<(serde_json::Value, DefinitionDigest), String> {
    let document = UnitDocumentV2::new(record_schema.clone(), unit.clone());
    let value = serde_json::to_value(document).map_err(|error| error.to_string())?;
    let digest = DefinitionDigest::for_canonical_value_with_limit(
        "mush-tactics.unit.authoritative.v1",
        &value,
        MAX_UNIT_RECORD_BYTES,
    )
    .map_err(|error| format!("unit record exceeds its canonical encoding limit: {error}"))?;
    Ok((value, digest))
}
fn persist_encoded_unit(out: &mut Outbox, object: ObjId, value: serde_json::Value) {
    out.defer(Deferred::Persist(ModuleOp {
        ns: SmolStr::new_static(NS),
        op: ModuleWrite::JsonSet {
            key: unit_key(object),
            value,
        },
    }));
}
fn persist_unit_del(out: &mut Outbox, obj: ObjId) {
    persist_key_del(out, unit_key(obj));
}
fn persist_key_del(out: &mut Outbox, key: SmolStr) {
    out.defer(Deferred::Persist(ModuleOp {
        ns: SmolStr::new_static(NS),
        op: ModuleWrite::Del { key },
    }));
}

// ---- the panel projection (epistemic seam) ----------------------------

/// Project `unit` for `observer` at `now` into a panel payload. The M33 body
/// ignores `observer`/`now` and returns full state; the future epistemic
/// projector swaps this body per-observer without touching emit topology.
fn project(unit: &TacticalUnit, _observer: ObjId, _now: i64) -> serde_json::Value {
    serde_json::to_value(unit).unwrap_or(serde_json::Value::Null)
}

// ---- lifecycle --------------------------------------------------------

/// A destroyed unit-object drops its unit + doc — **and** every OTHER unit that
/// pointed a component at the husk as its economic asset drops that pointer
/// (E3c, doc 08b §5.2).
///
/// The M32/M33 deferral is over: `mush-economy` now titles objects, so a
/// component whose `asset` names a dead `ObjId` is a dangling reference into a
/// namespace that will never resolve again. Nulling goes through a real
/// [`UnitMutationPlan`], so the unit's revision bumps and a queued order that
/// captured `(subject, gen)` sees the change; the repaired doc rides the same
/// outbox as everything else this event produced.
///
/// If a dependency owner is frozen, this post-destroy observer is too late to
/// veto the core object's prior `GOING` transition. It removes the unit from the
/// active index but preserves its authoritative module record in Frozen state;
/// references from other units are still scrubbed when their asset object dies.
fn on_obj_destroyed(current_record_definition: &DefinitionRef, ctx: &mut Ctx<'_>, ev: &WorldEvent) {
    let WorldEvent::ObjDestroyed { obj } = ev else {
        return;
    };
    let obj = *obj;
    let dependency_freeze = ctx
        .w
        .module_state::<UnitStore>()
        .and_then(|store| store.units.get(&obj))
        .map(|unit| first_unit_dependency_freeze(ctx.reg, unit));
    match dependency_freeze {
        Some(Ok(Some(freeze))) => {
            ctx.w
                .module_state_mut::<UnitStore>()
                .expect("unit store")
                .hold_active(obj, freeze.incident);
            tracing::warn!(
                object = obj.0,
                incident = freeze.incident.0,
                "tactics: object shell was destroyed, but preserved and held its unit record because a dependency owner is frozen"
            );
            return;
        }
        Some(Err(error)) => {
            tracing::error!(
                object = obj.0,
                %error,
                "tactics: could not establish dependency readiness after object destruction; preserved unit record"
            );
            return;
        }
        Some(Ok(None)) | None => {}
    }
    let removed = ctx
        .w
        .module_state_mut::<UnitStore>()
        .is_some_and(|store| store.remove_active(obj));
    if removed {
        persist_unit_del(ctx.out, obj);
    }
    let now = ctx.w.now();
    let repaired = {
        let Some(store) = ctx.w.module_state::<UnitStore>() else {
            return;
        };
        scrub_component_assets(&store.units, now, &|a| a == obj)
    };
    for unit in repaired {
        match encode_unit_document(current_record_definition, &unit) {
            Ok((document, digest)) => {
                tracing::warn!(unit = %unit.object(), dead = %obj, "tactics: component asset pointer scrubbed (object destroyed)");
                persist_encoded_unit(ctx.out, unit.object(), document);
                if let Some(store) = ctx.w.module_state_mut::<UnitStore>() {
                    store.update_active(unit, digest);
                }
            }
            Err(error) => {
                tracing::error!(unit = %unit.object(), dead = %obj, %error, "tactics: could not encode component asset repair; live state left untouched");
            }
        }
    }
}

/// The one scrub: for every unit holding a component whose `asset` satisfies
/// `dead`, null those pointers through a single [`UnitMutationPlan`] and return
/// the repaired units (the caller persists and installs them).
///
/// Pure with respect to the store — it clones and folds, so a plan the engine
/// refuses (it cannot, but the type says it may) leaves the live unit exactly as
/// it was, pointer and all, with a loud log instead of a half-edit.
fn scrub_component_assets(
    units: &BTreeMap<ObjId, TacticalUnit>,
    now: i64,
    dead: &dyn Fn(ObjId) -> bool,
) -> Vec<TacticalUnit> {
    let mut repaired = Vec::new();
    for (id, unit) in units {
        let ops: Vec<UnitMutationOp> = unit
            .components()
            .filter(|(_, c)| c.asset.is_some_and(dead))
            .map(|(cid, _)| UnitMutationOp::SetComponentAsset {
                component: cid.clone(),
                asset: None,
            })
            .collect();
        if ops.is_empty() {
            continue;
        }
        let plan = match UnitMutationPlan::new(ops, None, Some(SmolStr::new_static("asset-scrub")))
        {
            Ok(plan) => plan,
            Err(error) => {
                tracing::error!(unit = %id, %error, "tactics: component asset scrub exceeded the bounded mutation-plan contract; pointers left unchanged");
                continue;
            }
        };
        match unit::apply(unit, &plan, now) {
            Ok((mut post, _journal)) => {
                post.settle_mobility(now);
                repaired.push(post);
            }
            Err(e) => {
                tracing::error!(unit = %id, error = %e, "tactics: component asset scrub refused; pointer left dangling");
            }
        }
    }
    repaired
}

/// Keep every authoritative unit record recoverable before the core object is
/// marked `GOING`. Active, owner-ready units may use the normal destroy path;
/// held records and units whose dependencies are frozen must be explicitly
/// recovered instead of losing their object shell at the next sweep.
struct TacticsDestroyGate;
impl DestroyGate for TacticsDestroyGate {
    fn check(&self, w: &World, reg: &Registry, _actor: ObjId, obj: ObjId) -> Result<(), String> {
        let Some(store) = w.module_state::<UnitStore>() else {
            return Ok(());
        };
        if store
            .records
            .iter()
            .any(|record| record.claims_object(obj) && record.state != MechanicalState::Active)
        {
            return Err(
                "That object is claimed by a held tactical unit record; recover the record before destruction."
                    .into(),
            );
        }
        if let Some(unit) = store.units.get(&obj) {
            return match first_unit_dependency_freeze(reg, unit) {
                Ok(None) => Ok(()),
                Ok(Some(freeze)) => Err(format!(
                    "That tactical unit is frozen for integrity recovery (incident {}); recover it before destruction.",
                    freeze.incident.0
                )),
                Err(error) => Err(format!(
                    "That tactical unit's mechanics readiness cannot be established; refusing destruction: {error}"
                )),
            };
        }
        if store.records.iter().any(|record| record.claims_object(obj)) {
            return Err(
                "That object is claimed by a held tactical unit record; recover the record before destruction."
                    .into(),
            );
        }
        Ok(())
    }
}

/// Refuse `@behavior/remove tactics.unit` while the object is a live unit —
/// `@unit/remove` is the sanctioned teardown (which detaches programmatically).
struct TacticsDetachGate;
impl DetachGate for TacticsDetachGate {
    fn check(
        &self,
        w: &World,
        _reg: &Registry,
        _actor: ObjId,
        obj: ObjId,
        behavior: &BehaviorId,
    ) -> Result<(), String> {
        if behavior.as_str() == UNIT_BEHAVIOR
            && w.module_state::<UnitStore>().is_some_and(|store| {
                store.units.contains_key(&obj)
                    || store.records.iter().any(|record| record.claims_object(obj))
            })
        {
            return Err(
                "That object has a tactical unit record; resolve its lifecycle or @unit/remove the active unit first."
                    .into(),
            );
        }
        Ok(())
    }
}

struct TacticsRecoveryReporter;

impl RecoveryReporter for TacticsRecoveryReporter {
    fn report(&self, world: &World) -> ModuleBootReport {
        world
            .module_state::<UnitStore>()
            .and_then(|store| store.boot_report.clone())
            .unwrap_or_else(|| ModuleBootReport::new(OwnerId::new(NS).expect("canonical owner")))
    }
}

#[derive(Clone)]
enum RecordOrigin {
    Current,
    Legacy(Box<MigrationPlanSummary>),
}

#[derive(Clone)]
enum FailedState {
    Frozen,
    Quarantined,
}

#[derive(Clone)]
struct RecordFailure {
    state: FailedState,
    severity: RecoverySeverity,
    reason: Box<RecoveryReason>,
}

struct PendingRecord {
    key: SmolStr,
    key_object: Option<ObjId>,
    payload_digest: Option<DefinitionDigest>,
    unit: Option<TacticalUnit>,
    origin: Option<RecordOrigin>,
    failure: Option<RecordFailure>,
    binding_repair: Option<UnitBindingRepair>,
}

fn canonical_unit_key_object(key: &str) -> Option<ObjId> {
    let suffix = key.strip_prefix("unit:")?;
    let value = suffix.parse::<u32>().ok()?;
    (key == format!("unit:{value}")).then_some(ObjId(value))
}

fn malformed(code: &str) -> RecoveryReason {
    RecoveryReason::MalformedRecord {
        code: mush_core::CanonicalId::new(code).expect("static recovery code is canonical"),
    }
}

fn integrity(code: &str) -> RecoveryReason {
    RecoveryReason::IntegrityViolation {
        code: mush_core::CanonicalId::new(code).expect("static recovery code is canonical"),
    }
}

fn record_failure(
    state: FailedState,
    severity: RecoverySeverity,
    reason: RecoveryReason,
) -> RecordFailure {
    RecordFailure {
        state,
        severity,
        reason: Box::new(reason),
    }
}

fn record_digest(value: &serde_json::Value) -> Result<DefinitionDigest, RecordFailure> {
    DefinitionDigest::for_canonical_value_with_limit(
        "mush-tactics.unit.authoritative.v1",
        value,
        MAX_UNIT_RECORD_BYTES,
    )
    .map_err(|_| {
        record_failure(
            FailedState::Quarantined,
            RecoverySeverity::Error,
            malformed("unit_record_oversize"),
        )
    })
}

fn incident_for(
    key: &str,
    object: Option<ObjId>,
    payload_digest: Option<DefinitionDigest>,
    failure: &RecordFailure,
    now: i64,
) -> RecoveryIncident {
    let identity = json!({
        "key": key,
        "object": object.map(|id| id.0),
        "payload_digest": payload_digest,
        "reason": &failure.reason,
    });
    let digest =
        DefinitionDigest::for_canonical_value("mush-tactics.recovery-incident.v1", &identity)
            .expect("bounded recovery identity encodes");
    let mut id_bytes = [0_u8; 8];
    id_bytes.copy_from_slice(&digest.as_bytes()[..8]);
    let subject_id = match object {
        Some(object) => format!("unit.{}", object.0),
        None => format!("record.{}", &digest.to_hex()[..16]),
    };
    RecoveryIncident::new(
        IncidentId(u64::from_be_bytes(id_bytes)),
        OwnerId::new(NS).expect("canonical owner"),
        if object.is_some() {
            RecoveryScope::Object
        } else {
            RecoveryScope::Record
        },
        failure.severity,
        (*failure.reason).clone(),
        vec![RecoverySubject {
            owner: OwnerId::new(NS).expect("canonical owner"),
            kind: mush_core::CanonicalId::new("tactical_unit").expect("canonical subject kind"),
            id: mush_core::CanonicalId::new(subject_id).expect("canonical subject id"),
        }],
        payload_digest,
        now,
        IncidentState::Open,
    )
    .expect("bounded recovery incident")
}

fn decode_pending_record(
    key: SmolStr,
    value: serde_json::Value,
    catalog: &UnitCatalog,
    registry: &Registry,
    legacy_record_definition: &DefinitionRef,
    current_record_definition: &DefinitionRef,
) -> PendingRecord {
    let key_object = canonical_unit_key_object(&key);
    let payload_digest = match record_digest(&value) {
        Ok(digest) => Some(digest),
        Err(failure) => {
            return PendingRecord {
                key,
                key_object,
                payload_digest: None,
                unit: None,
                origin: None,
                failure: Some(failure),
                binding_repair: None,
            };
        }
    };

    let is_versioned = value.as_object().is_some_and(|map| {
        map.contains_key("record") || map.contains_key("record_version") || map.contains_key("unit")
    });
    if is_versioned {
        let mut pending = match UnitDocumentV2::decode(value, current_record_definition) {
            Ok(document) => PendingRecord {
                key,
                key_object,
                payload_digest,
                unit: Some(document.unit),
                origin: Some(RecordOrigin::Current),
                failure: None,
                binding_repair: None,
            },
            Err(UnitDocumentV2DecodeError::Invalid(code)) => PendingRecord {
                key,
                key_object,
                payload_digest,
                unit: None,
                origin: None,
                failure: Some(record_failure(
                    FailedState::Quarantined,
                    RecoverySeverity::Error,
                    malformed(code),
                )),
                binding_repair: None,
            },
            Err(UnitDocumentV2DecodeError::DefinitionDigestMismatch {
                requested,
                registered,
            }) => PendingRecord {
                key,
                key_object,
                payload_digest,
                unit: None,
                origin: None,
                failure: Some(record_failure(
                    FailedState::Frozen,
                    RecoverySeverity::Critical,
                    RecoveryReason::DefinitionDigestMismatch {
                        requested: *requested,
                        registered: *registered,
                    },
                )),
                binding_repair: None,
            },
        };
        // Decode a valid envelope even under a bad key so the held record keeps
        // its embedded subject and participates in dependency/claim analysis.
        // The authoritative key remains untouched and is quarantined.
        if key_object.is_none() && pending.unit.is_some() {
            pending.failure = Some(record_failure(
                FailedState::Quarantined,
                RecoverySeverity::Error,
                malformed("unit_key_noncanonical"),
            ));
        }
        return pending;
    }

    if key_object.is_none() {
        return PendingRecord {
            key,
            key_object,
            payload_digest,
            unit: None,
            origin: None,
            failure: Some(record_failure(
                FailedState::Quarantined,
                RecoverySeverity::Error,
                malformed("unit_key_noncanonical"),
            )),
            binding_repair: None,
        };
    }

    // First run the contextual adapter so a missing catalog/provider can be
    // distinguished from malformed or ambiguous legacy bytes. Then run the
    // same adapter through the shared exact migration registry to obtain the
    // authoritative, hash-bound dry-run plan.
    let decoded = match decode_legacy_unit_v1(&value, catalog) {
        Ok(decoded) => decoded,
        Err(crate::unit::LegacyUnitV1Error::RootDefinitionUnavailable) => {
            return PendingRecord {
                key,
                key_object,
                payload_digest,
                unit: None,
                origin: None,
                failure: Some(record_failure(
                    FailedState::Frozen,
                    RecoverySeverity::Warning,
                    RecoveryReason::MigrationUnavailable {
                        from: legacy_record_definition.clone(),
                        to: current_record_definition.clone(),
                    },
                )),
                binding_repair: None,
            };
        }
        Err(error) => {
            tracing::error!(%key, error = %error, "tactics: legacy unit cannot be migrated");
            return PendingRecord {
                key,
                key_object,
                payload_digest,
                unit: None,
                origin: None,
                failure: Some(record_failure(
                    FailedState::Quarantined,
                    RecoverySeverity::Error,
                    RecoveryReason::MigrationUnavailable {
                        from: legacy_record_definition.clone(),
                        to: current_record_definition.clone(),
                    },
                )),
                binding_repair: None,
            };
        }
    };
    let source = match RulesValue::new(legacy_record_definition.clone(), value) {
        Ok(source) => source,
        Err(_) => {
            return PendingRecord {
                key,
                key_object,
                payload_digest,
                unit: None,
                origin: None,
                failure: Some(record_failure(
                    FailedState::Quarantined,
                    RecoverySeverity::Error,
                    malformed("unit_record_oversize"),
                )),
                binding_repair: None,
            };
        }
    };
    let Some(definitions) = registry.definitions() else {
        unreachable!("definition registry is frozen before module records load")
    };
    let Some(migrations) = registry.definition_migrations() else {
        unreachable!("migration registry is frozen before module records load")
    };
    let plan = match migrations.dry_run(&source, current_record_definition, definitions) {
        Ok(plan) => plan,
        Err(error) => {
            tracing::error!(%key, error = %error, "tactics: exact v1 migration dry-run failed");
            return PendingRecord {
                key,
                key_object,
                payload_digest,
                unit: None,
                origin: None,
                failure: Some(record_failure(
                    FailedState::Quarantined,
                    RecoverySeverity::Error,
                    RecoveryReason::MigrationUnavailable {
                        from: legacy_record_definition.clone(),
                        to: current_record_definition.clone(),
                    },
                )),
                binding_repair: None,
            };
        }
    };
    let migrated_document =
        UnitDocumentV2::decode(plan.output().data().clone(), current_record_definition);
    if migrated_document.as_ref().map(|doc| &doc.unit) != Ok(&decoded.unit) {
        return PendingRecord {
            key,
            key_object,
            payload_digest,
            unit: None,
            origin: None,
            failure: Some(record_failure(
                FailedState::Quarantined,
                RecoverySeverity::Critical,
                integrity("unit_migration_output_mismatch"),
            )),
            binding_repair: None,
        };
    }
    PendingRecord {
        key,
        key_object,
        payload_digest,
        unit: Some(decoded.unit),
        origin: Some(RecordOrigin::Legacy(Box::new(plan.summary()))),
        failure: None,
        binding_repair: None,
    }
}

#[derive(Default)]
struct ExactDefinitionValidation {
    failure: Option<RecordFailure>,
    digest_conflicts: Vec<RecordFailure>,
}

fn validate_exact_definitions(
    unit: &TacticalUnit,
    registry: &Registry,
) -> ExactDefinitionValidation {
    let definitions = registry
        .definitions()
        .expect("definition registry is frozen before module records load");
    let mut missing = None;
    let mut inherited_freeze = None;
    let mut digest_conflicts = Vec::new();
    for reference in unit.definition_references() {
        match definitions.get_exact(&reference) {
            Ok(_) => {}
            Err(DefinitionLookupError::Missing { requested }) => {
                missing.get_or_insert_with(|| {
                    record_failure(
                        FailedState::Frozen,
                        RecoverySeverity::Warning,
                        RecoveryReason::DefinitionUnavailable {
                            definition: requested,
                        },
                    )
                });
            }
            Err(DefinitionLookupError::MissingLocator { .. }) => {
                missing.get_or_insert_with(|| {
                    record_failure(
                        FailedState::Frozen,
                        RecoverySeverity::Warning,
                        RecoveryReason::DefinitionUnavailable {
                            definition: reference,
                        },
                    )
                });
            }
            Err(DefinitionLookupError::DigestMismatch {
                requested,
                registered,
            }) => {
                digest_conflicts.push(record_failure(
                    FailedState::Frozen,
                    RecoverySeverity::Critical,
                    RecoveryReason::DefinitionDigestMismatch {
                        requested,
                        registered,
                    },
                ));
            }
            Err(DefinitionLookupError::OwnerNotReady { source, .. }) => {
                let reason = match *source {
                    mush_core::MechanicalOwnerReadinessError::Frozen { reason, .. } => *reason,
                    mush_core::MechanicalOwnerReadinessError::RegistryUnavailable
                    | mush_core::MechanicalOwnerReadinessError::UnknownOwner { .. } => {
                        integrity("definition_owner_readiness_unavailable")
                    }
                };
                inherited_freeze.get_or_insert_with(|| {
                    record_failure(FailedState::Frozen, RecoverySeverity::Critical, reason)
                });
            }
        }
    }
    ExactDefinitionValidation {
        failure: digest_conflicts
            .first()
            .cloned()
            .or(inherited_freeze)
            .or(missing),
        digest_conflicts,
    }
}

fn latch_record_definition_conflicts(
    registry: &Registry,
    record: &PendingRecord,
    conflicts: &[RecordFailure],
    now: i64,
) -> Result<(), ModError> {
    for failure in conflicts {
        let RecoveryReason::DefinitionDigestMismatch { requested, .. } = failure.reason.as_ref()
        else {
            continue;
        };
        let owner = &requested.key().owner;
        match registry.mechanics_owner_freeze(owner) {
            Ok(Some(_)) => continue,
            Ok(None) => {}
            Err(error) => {
                return Err(ModError::msg(format!(
                    "cannot inspect mechanics readiness for digest-conflicted owner {owner}: {error}"
                )));
            }
        }
        let object = record
            .unit
            .as_ref()
            .map(TacticalUnit::object)
            .or(record.key_object);
        let incident = incident_for(&record.key, object, record.payload_digest, failure, now);
        registry
            .freeze_mechanics_owner(owner, incident.id, (*failure.reason).clone())
            .map_err(|error| {
                ModError::msg(format!(
                    "cannot freeze digest-conflicted mechanics owner {owner}: {error}"
                ))
            })?;
        tracing::error!(
            owner = %owner,
            incident = incident.id.0,
            "tactics: exact definition digest conflict froze owner mechanics"
        );
    }
    Ok(())
}

fn fan_out_owner_freezes(
    registry: &Registry,
    pending: &mut [PendingRecord],
) -> Result<(), ModError> {
    for record in pending {
        if matches!(
            record.failure.as_ref().map(|failure| &failure.state),
            Some(FailedState::Quarantined)
        ) {
            continue;
        }
        let Some(unit) = record.unit.as_ref() else {
            continue;
        };
        let owners = unit_dependency_owners(unit);
        let mut inherited_freeze = None;
        for owner in owners {
            match registry.mechanics_owner_freeze(&owner) {
                Ok(Some(freeze)) => {
                    inherited_freeze = Some(freeze);
                    break;
                }
                Ok(None) => {}
                // A self-describing record may legitimately outlive an optional,
                // currently unloaded definition owner. Its existing Missing result
                // remains a record-local freeze; there is no composed owner whose
                // callbacks need disabling.
                Err(mush_core::MechanicalOwnerReadinessError::UnknownOwner { .. })
                    if record.failure.is_some() => {}
                Err(error) => {
                    return Err(ModError::msg(format!(
                        "cannot inspect mechanics readiness for tactical owner {owner}: {error}"
                    )));
                }
            }
        }
        if let Some(freeze) = inherited_freeze {
            record.failure = Some(record_failure(
                FailedState::Frozen,
                RecoverySeverity::Critical,
                freeze.reason,
            ));
        }
    }
    Ok(())
}

/// Rebuild only the active mechanics index. Every authoritative input remains
/// untouched; disagreements and legacy conversions are represented as
/// lifecycle state, bounded incidents, and exact dry-run plans.
fn boot_units(
    ctx: &mut ModuleCtx<'_>,
    data: ModuleBootData,
    catalog: &UnitCatalog,
    legacy_record_definition: &DefinitionRef,
    current_record_definition: &DefinitionRef,
) -> Result<(), ModError> {
    let unit_behavior = BehaviorId::new(UNIT_BEHAVIOR).expect("static behavior id");
    let owner = OwnerId::new(NS).expect("canonical owner");
    let now = ctx.w.now();
    let mut report = ModuleBootReport::new(owner);
    let mut rows: Vec<(SmolStr, serde_json::Value)> = data
        .kv
        .into_iter()
        .filter(|(key, value)| {
            key.starts_with("unit:")
                || value.get("record").and_then(serde_json::Value::as_str) == Some(UNIT_RECORD_NAME)
        })
        .collect();
    rows.sort_by(|(left_key, left_value), (right_key, right_value)| {
        left_key.cmp(right_key).then_with(|| {
            let left = canonical_json_bytes_with_limit(left_value, MAX_UNIT_RECORD_BYTES)
                .unwrap_or_default();
            let right = canonical_json_bytes_with_limit(right_value, MAX_UNIT_RECORD_BYTES)
                .unwrap_or_default();
            left.cmp(&right)
        })
    });
    let mut pending: Vec<PendingRecord> = rows
        .into_iter()
        .map(|(key, value)| {
            decode_pending_record(
                key,
                value,
                catalog,
                ctx.reg,
                legacy_record_definition,
                current_record_definition,
            )
        })
        .collect();

    // Exact digest conflicts are owner-wide integrity evidence even when an
    // otherwise parsable record later fails a key, duplicate-subject, object,
    // or binding lifecycle check. Scan and latch them first, in the canonical
    // row/reference order above, so a secondary record defect cannot mask a
    // provider breach. Strictly malformed bodies have no trusted dependency
    // graph and remain record-local quarantine.
    let mut exact_validations = Vec::with_capacity(pending.len());
    for record in &pending {
        let validation = record
            .unit
            .as_ref()
            .map(|unit| validate_exact_definitions(unit, ctx.reg));
        if let Some(validation) = &validation {
            latch_record_definition_conflicts(ctx.reg, record, &validation.digest_conflicts, now)?;
        } else if let Some(failure) = &record.failure
            && matches!(
                failure.reason.as_ref(),
                RecoveryReason::DefinitionDigestMismatch { .. }
            )
        {
            // A same-locator digest conflict in the strict v2 envelope is
            // sufficient to freeze the tactics record-schema owner without
            // interpreting the unauthenticated unit body.
            latch_record_definition_conflicts(ctx.reg, record, std::slice::from_ref(failure), now)?;
        }
        exact_validations.push(validation);
    }

    let mut key_counts: BTreeMap<SmolStr, usize> = BTreeMap::new();
    let mut object_counts: BTreeMap<ObjId, usize> = BTreeMap::new();
    for record in &pending {
        *key_counts.entry(record.key.clone()).or_default() += 1;
        if let Some(unit) = &record.unit {
            *object_counts.entry(unit.object()).or_default() += 1;
        }
    }

    let mut claimed_objects: BTreeMap<ObjId, ()> = BTreeMap::new();
    for (record, validation) in pending.iter_mut().zip(exact_validations) {
        if let Some(object) = record.key_object {
            claimed_objects.insert(object, ());
        }
        if let Some(unit) = &record.unit {
            claimed_objects.insert(unit.object(), ());
        }
        if key_counts.get(&record.key).copied().unwrap_or_default() > 1 {
            record.failure = Some(record_failure(
                FailedState::Quarantined,
                RecoverySeverity::Critical,
                integrity("unit_duplicate_key"),
            ));
            continue;
        }
        let Some(unit) = record.unit.as_ref() else {
            continue;
        };
        let object = unit.object();
        if object_counts.get(&object).copied().unwrap_or_default() > 1 {
            record.failure = Some(record_failure(
                FailedState::Quarantined,
                RecoverySeverity::Critical,
                integrity("unit_duplicate_object"),
            ));
            continue;
        }
        if record.key_object != Some(object) {
            record.failure = Some(record_failure(
                FailedState::Quarantined,
                RecoverySeverity::Error,
                integrity("unit_key_object_mismatch"),
            ));
            continue;
        }
        match ctx.w.get(object) {
            None => {
                record.failure = Some(record_failure(
                    FailedState::Quarantined,
                    RecoverySeverity::Error,
                    integrity("unit_object_missing"),
                ));
                continue;
            }
            Some(object_record) if object_record.flags.contains(Flags::GOING) => {
                record.failure = Some(record_failure(
                    FailedState::Quarantined,
                    RecoverySeverity::Error,
                    integrity("unit_object_going"),
                ));
                continue;
            }
            Some(object_record)
                if !matches!(
                    object_record.kind,
                    ObjKind::Thing | ObjKind::Player | ObjKind::Npc
                ) =>
            {
                record.failure = Some(record_failure(
                    FailedState::Quarantined,
                    RecoverySeverity::Error,
                    integrity("unit_object_kind_invalid"),
                ));
                continue;
            }
            Some(_) => {}
        }
        let validation = validation.expect("decoded unit has exact-definition validation");
        if let Some(failure) = validation.failure {
            record.failure = Some(failure);
            continue;
        }
        let Some(spec) = catalog.get_exact(unit.schema()) else {
            record.failure = Some(record_failure(
                FailedState::Frozen,
                RecoverySeverity::Warning,
                RecoveryReason::ProviderUnavailable {
                    owner: unit.schema().key().owner.clone(),
                },
            ));
            continue;
        };
        if let Err(error) = spec.validate_instance(unit) {
            tracing::error!(key = %record.key, error = %error, "tactics: unit shape disagrees with exact definition");
            record.failure = Some(record_failure(
                FailedState::Quarantined,
                RecoverySeverity::Critical,
                integrity("unit_definition_shape_mismatch"),
            ));
            continue;
        }

        let binding_schema = ctx
            .w
            .get(object)
            .and_then(|object_record| object_record.behaviors.get(&unit_behavior))
            .map(|binding| binding.schema_version);
        if binding_schema != Some(UNIT_BINDING_VERSION) {
            record.binding_repair = Some(UnitBindingRepair {
                object,
                record_key: Some(record.key.clone()),
                found_schema: binding_schema,
                expected_schema: UNIT_BINDING_VERSION,
            });
            if matches!(record.origin, Some(RecordOrigin::Current)) {
                record.failure = Some(record_failure(
                    FailedState::Frozen,
                    RecoverySeverity::Warning,
                    integrity(if binding_schema.is_some() {
                        "unit_binding_schema_mismatch"
                    } else {
                        "unit_binding_missing"
                    }),
                ));
            }
        }
    }

    // Digest conflicts are owner integrity incidents, not record corruption.
    // Latch every implicated definition owner before admitting any active unit,
    // then mechanically hold every intact dependent record for that owner.
    fan_out_owner_freezes(ctx.reg, &mut pending)?;

    let mut store = UnitStore::default();
    for record in pending {
        if let Some(repair) = record.binding_repair.clone() {
            store.push_binding_repair(repair, &mut report);
        }
        let object = record
            .unit
            .as_ref()
            .map(TacticalUnit::object)
            .or(record.key_object);
        let (state, incident, migration_plan) = if let Some(failure) = &record.failure {
            let incident = incident_for(
                &record.key,
                object.or(record.key_object),
                record.payload_digest,
                failure,
                now,
            );
            let state = match failure.state {
                FailedState::Frozen => MechanicalState::Frozen {
                    incident: incident.id,
                },
                FailedState::Quarantined => MechanicalState::Quarantined {
                    incident: incident.id,
                },
            };
            (state, Some(incident), None)
        } else {
            match record.origin.clone().expect("decoded record has an origin") {
                RecordOrigin::Current => (MechanicalState::Active, None, None),
                RecordOrigin::Legacy(plan) => (
                    MechanicalState::Migrating {
                        from: Box::new(legacy_record_definition.clone()),
                        to: Box::new(current_record_definition.clone()),
                    },
                    None,
                    Some(*plan),
                ),
            }
        };
        report.record_state(&state);
        if let Some(incident) = incident {
            tracing::warn!(incident = incident.id.0, key = %record.key, ?state, "tactics unit held at boot");
            report.push_incident(incident.summary());
        }
        if let Some(plan) = &migration_plan {
            report.push_migration_plan(plan.clone());
        }
        if state == MechanicalState::Active {
            let unit = record.unit.clone().expect("active record has decoded unit");
            store.units.insert(unit.object(), unit);
        }
        store.records.push(UnitRecordStatus {
            key: record.key,
            key_object: record.key_object,
            object,
            state,
            unit: record.unit,
            payload_digest: record.payload_digest,
            migration_plan,
        });
    }

    // A binding with no corresponding key or decodable embedded subject is a
    // repair preview too. It is never detached during boot.
    for object in ctx.w.iter() {
        let Some(binding) = object.behaviors.get(&unit_behavior) else {
            continue;
        };
        if claimed_objects.contains_key(&object.id) {
            continue;
        }
        let repair = UnitBindingRepair {
            object: object.id,
            record_key: None,
            found_schema: Some(binding.schema_version),
            expected_schema: UNIT_BINDING_VERSION,
        };
        let failure = record_failure(
            FailedState::Frozen,
            RecoverySeverity::Warning,
            integrity("unit_binding_without_record"),
        );
        let incident = incident_for(
            "binding-without-record",
            Some(object.id),
            None,
            &failure,
            now,
        );
        report.push_incident(incident.summary());
        store.push_binding_repair(repair, &mut report);
    }

    // E3c: the boot half of the component-asset scrub. Only `@destroy` emits
    // `ObjDestroyed`, so a programmatic `World::destroy` (or an object lost
    // during downtime) leaves a component pointing at nothing; this is the pass
    // that catches it. World-only eligibility, like everything else here — the
    // question is "does that object still exist?", never "does it have a title?"
    // (`mush-tactics` takes no economy dependency).
    let now = ctx.w.now();
    let repaired = {
        let w: &World = ctx.w;
        scrub_component_assets(&store.units, now, &|a| {
            w.get(a).is_none_or(|o| o.flags.contains(Flags::GOING))
        })
    };
    for unit in repaired {
        match encode_unit_document(current_record_definition, &unit) {
            Ok((document, digest)) => {
                tracing::warn!(unit = %unit.object(), "tactics: component asset pointer(s) to a missing object scrubbed at boot");
                persist_encoded_unit(ctx.out, unit.object(), document);
                store.update_active(unit, digest);
            }
            Err(error) => {
                tracing::error!(unit = %unit.object(), %error, "tactics: could not encode component asset repair at boot; live state left untouched");
            }
        }
    }
    store.boot_report = Some(report);

    if let Some(slot) = ctx.w.module_state_mut::<UnitStore>() {
        *slot = store;
    }
    Ok(())
}

// ---- commands ---------------------------------------------------------

fn is_admin(w: &World, who: ObjId) -> bool {
    perm::has_power(w, who, Powers::SIM_ADMIN) || perm::has_role(w, who, Role::Royalty)
}
fn require_admin(ctx: &Ctx<'_>) -> Result<(), CmdError> {
    if is_admin(ctx.w, ctx.actor) {
        Ok(())
    } else {
        Err(CmdError::msg("Permission denied."))
    }
}

fn require_owner_ready(registry: &Registry, owner: &OwnerId) -> Result<(), CmdError> {
    registry
        .require_mechanics_owner_ready(owner)
        .map_err(|error| match error {
            mush_core::MechanicalOwnerReadinessError::Frozen { incident, .. } => {
                CmdError::msg(format!(
                    "Mechanics owned by {owner} are frozen for integrity recovery (incident {}).",
                    incident.0
                ))
            }
            _ => CmdError::msg(format!(
                "Mechanics owned by {owner} are not available: {error}"
            )),
        })
}

fn require_definition_owners_ready<'a>(
    registry: &Registry,
    references: impl IntoIterator<Item = &'a DefinitionRef>,
) -> Result<(), CmdError> {
    let owners: BTreeSet<_> = references
        .into_iter()
        .map(|reference| reference.key().owner.clone())
        .collect();
    for owner in owners {
        require_owner_ready(registry, &owner)?;
    }
    Ok(())
}

fn unit_dependency_owners(unit: &TacticalUnit) -> BTreeSet<OwnerId> {
    let mut owners: BTreeSet<_> = unit
        .definition_references()
        .into_iter()
        .map(|reference| reference.key().owner.clone())
        .collect();
    // The strict envelope, persistence adapter, lifecycle index, and mutation
    // implementation are themselves tactics-owned mechanics. A unit cannot
    // remain executable merely because all of its ruleset-owned definitions
    // are ready while the record-schema owner is frozen.
    owners.insert(OwnerId::new(NS).expect("static tactics owner is canonical"));
    owners
}

fn require_unit_ready(registry: &Registry, unit: &TacticalUnit) -> Result<(), CmdError> {
    for owner in unit_dependency_owners(unit) {
        require_owner_ready(registry, &owner)?;
    }
    Ok(())
}

fn first_unit_dependency_freeze(
    registry: &Registry,
    unit: &TacticalUnit,
) -> Result<Option<mush_core::MechanicalOwnerFreeze>, mush_core::MechanicalOwnerReadinessError> {
    let owners = unit_dependency_owners(unit);
    for owner in owners {
        if let Some(freeze) = registry.mechanics_owner_freeze(&owner)? {
            return Ok(Some(freeze));
        }
    }
    Ok(None)
}
fn info(ctx: &mut Ctx<'_>, msg: impl Into<String>) {
    let actor = ctx.actor;
    ctx.out.sys_to_actor(actor, "tactics", msg.into());
}
fn rhs_required(ctx: &Ctx<'_>, usage: &str) -> Result<String, CmdError> {
    ctx.args
        .rhs()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| CmdError::msg(format!("Usage: {usage}")))
}

fn cmd_unit(
    catalog: &UnitCatalog,
    feed: &PanelFeedHandle,
    current_record_definition: &DefinitionRef,
    ctx: &mut Ctx<'_>,
) -> Result<(), CmdError> {
    require_admin(ctx)?;
    if ctx.switches.has("create") {
        unit_create(catalog, feed, current_record_definition, ctx)
    } else if ctx.switches.has("recovery") {
        unit_recovery(ctx)
    } else if ctx.switches.has("show") {
        unit_show(ctx)
    } else if ctx.switches.has("damage") {
        unit_damage(feed, current_record_definition, ctx)
    } else if ctx.switches.has("remove") {
        unit_remove(ctx)
    } else {
        Err(CmdError::msg(
            "Usage: @unit/create|show|damage|remove <obj>[=...] or @unit/recovery",
        ))
    }
}

/// Why an object cannot become a tactical unit (parity with route_ineligible).
fn unit_ineligible(ctx: &Ctx<'_>, obj: ObjId) -> Option<String> {
    match ctx.w.get(obj) {
        None => return Some("No such object.".into()),
        Some(o) if o.flags.contains(Flags::GOING) => return Some("It is being destroyed.".into()),
        Some(o) if matches!(o.kind, ObjKind::Room | ObjKind::Exit) => {
            return Some(format!(
                "{} cannot be a tactical unit.",
                display_name(ctx.w, obj)
            ));
        }
        _ => {}
    }
    ctx.w
        .module_state::<UnitStore>()
        .is_some_and(|store| {
            store.units.contains_key(&obj)
                || store.records.iter().any(|record| record.claims_object(obj))
        })
        .then(|| {
            format!(
                "{} already has a tactical unit record (resolve or remove it first).",
                display_name(ctx.w, obj)
            )
        })
}

fn unit_create(
    catalog: &UnitCatalog,
    feed: &PanelFeedHandle,
    current_record_definition: &DefinitionRef,
    ctx: &mut Ctx<'_>,
) -> Result<(), CmdError> {
    let obj = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let def_id = rhs_required(ctx, "@unit/create <obj>=<schema-id>")?;
    if let Some(why) = unit_ineligible(ctx, obj) {
        return Err(CmdError::msg(why));
    }
    let definitions = ctx
        .reg
        .definitions()
        .ok_or_else(|| CmdError::msg("The mechanics definition registry is not ready."))?;
    let spec = catalog
        .get_current(def_id.trim(), definitions)
        .ok_or_else(|| {
            CmdError::msg(format!(
                "No unambiguous current unit schema `{}` is available.",
                def_id.trim()
            ))
        })?;
    let mut references = spec.body().definition_references();
    references.push(spec.definition.clone());
    references.push(current_record_definition.clone());
    require_definition_owners_ready(ctx.reg, &references)?;
    for reference in &references {
        definitions.get_exact(reference).map_err(|error| {
            CmdError::msg(format!("Unit schema dependency is unavailable: {error}"))
        })?;
    }
    definitions
        .get_exact(&spec.definition)
        .map_err(|error| CmdError::msg(format!("Unit schema is unavailable: {error}")))?;
    let now = ctx.w.now();
    let mut u = TacticalUnit::from_spec(spec, obj).map_err(|e| CmdError::msg(e.to_string()))?;
    u.settle_mobility(now);
    // Validate/encode before touching either the behavior binding or RAM. A
    // newly authored unit must always be acceptable to the next boot.
    let (document, payload_digest) =
        encode_unit_document(current_record_definition, &u).map_err(CmdError::msg)?;
    ctx.w
        .attach_behavior(
            obj,
            BehaviorId::new(UNIT_BEHAVIOR).unwrap(),
            BehaviorBinding {
                schema_version: UNIT_BINDING_VERSION,
                attached_by: ctx.actor,
                attached_at: now,
            },
        )
        .map_err(|e| CmdError::msg(format!("cannot mark unit: {e}")))?;
    let frame = project(&u, ctx.actor, now);
    persist_encoded_unit(ctx.out, obj, document);
    ctx.w
        .module_state_mut::<UnitStore>()
        .expect("unit store")
        .install_active(u, payload_digest);
    feed.keyframe_to(ctx, Audience::Actor(ctx.actor), frame);
    info(
        ctx,
        format!(
            "{} is now a tactical unit ({}).",
            display_name(ctx.w, obj),
            def_id.trim()
        ),
    );
    Ok(())
}

fn unit_show(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let obj = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let lines: Vec<String> = {
        let store = ctx.w.module_state::<UnitStore>().expect("unit store");
        let u = if let Some(unit) = store.units.get(&obj) {
            unit
        } else if let Some(record) = store
            .records
            .iter()
            .find(|record| record.claims_object(obj))
        {
            let mut lines = vec![format!(
                "{} — tactical record {} is {:?}; mechanics are disabled",
                display_name(ctx.w, obj),
                record.key,
                record.state
            )];
            if let Some(unit) = &record.unit {
                lines.push(format!(
                    "  schema {}; revision {}",
                    unit.schema(),
                    unit.revision()
                ));
            }
            return lines.into_iter().try_for_each(|line| {
                info(ctx, line);
                Ok::<(), CmdError>(())
            });
        } else {
            return Err(CmdError::msg("It is not a tactical unit."));
        };
        let mut v = vec![format!(
            "{} — schema {}, revision {}, mobility {}",
            display_name(ctx.w, obj),
            u.schema(),
            u.revision(),
            u.mobility()
                .and_then(|m| m.effective_mode(ctx.w.now()).map(|id| id.to_string()))
                .unwrap_or_else(|| "none".into()),
        )];
        for (id, s) in u.sections() {
            v.push(format!("  section {id}: {}", s.condition));
        }
        for (addr, cell) in u.cells() {
            v.push(format!("  cell {addr:?}: {}/{}", cell.current, cell.max));
        }
        for (id, c) in u.components() {
            v.push(format!(
                "  component {id}: {} ({})",
                c.condition, c.definition
            ));
        }
        for k in u.capabilities() {
            v.push(format!("  capability {} (source {:?})", k.id, k.source));
        }
        v
    };
    for l in lines {
        info(ctx, l);
    }
    Ok(())
}

fn unit_recovery(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let store = ctx.w.module_state::<UnitStore>().expect("unit store");
    let Some(report) = &store.boot_report else {
        info(ctx, "Tactics recovery has not run.");
        return Ok(());
    };
    let mut lines = vec![format!(
        "Tactics lifecycle: {} active, {} frozen, {} migrating, {} quarantined; {} incident(s), {} migration plan(s), {} binding repair(s){}.",
        report.active,
        report.frozen,
        report.migrating,
        report.quarantined,
        report.incident_count,
        report.migration_plan_count,
        store.binding_repair_count,
        if report.details_truncated {
            " (details truncated)"
        } else {
            ""
        },
    )];
    for incident in &report.incidents {
        lines.push(format!(
            "  incident {}: {:?} {:?} ({:?})",
            incident.id.0, incident.severity, incident.reason, incident.state
        ));
    }
    for plan in &report.migration_plans {
        lines.push(format!(
            "  migration {} -> {} ({} step(s), {} -> {})",
            plan.from, plan.to, plan.step_count, plan.before_digest, plan.after_digest
        ));
    }
    for repair in &store.binding_repairs {
        lines.push(format!(
            "  binding repair #{}: found {:?}, require v{}{}",
            repair.object.0,
            repair.found_schema,
            repair.expected_schema,
            repair.record_key.as_ref().map_or_else(
                || " (no unit record)".to_string(),
                |key| format!(" ({key})")
            ),
        ));
    }
    let _ = store;
    for line in lines {
        info(ctx, line);
    }
    Ok(())
}

fn unit_damage(
    feed: &PanelFeedHandle,
    current_record_definition: &DefinitionRef,
    ctx: &mut Ctx<'_>,
) -> Result<(), CmdError> {
    let obj = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let json = rhs_required(ctx, "@unit/damage <obj>={json UnitMutationPlan}")?;
    let plan: UnitMutationPlan = serde_json::from_str(json.trim())
        .map_err(|e| CmdError::msg(format!("bad plan JSON: {e}")))?;
    // Safe subset: GrantCapability mints an unreconciled opaque capability (no
    // projector yet to check it) — refuse it via this staff-debug path.
    if plan
        .ops()
        .iter()
        .any(|op| matches!(op, UnitMutationOp::GrantCapability { .. }))
    {
        return Err(CmdError::msg(
            "GrantCapability is not allowed via @unit/damage (needs a ruleset projector).",
        ));
    }
    let now = ctx.w.now();
    let unit = ctx
        .w
        .module_state::<UnitStore>()
        .and_then(|s| s.units.get(&obj).cloned())
        .ok_or_else(|| CmdError::msg("It is not a tactical unit."))?;
    require_unit_ready(ctx.reg, &unit)?;
    let (mut post, journal) =
        unit::apply(&unit, &plan, now).map_err(|e| CmdError::msg(e.to_string()))?;
    post.settle_mobility(now);
    let (from, to, n) = (
        journal.from_revision,
        journal.to_revision,
        journal.entries.len(),
    );
    let delta = serde_json::to_value(&journal).expect("journal serializes");
    let (document, payload_digest) =
        encode_unit_document(current_record_definition, &post).map_err(CmdError::msg)?;
    persist_encoded_unit(ctx.out, obj, document);
    ctx.w
        .module_state_mut::<UnitStore>()
        .expect("unit store")
        .update_active(post, payload_digest);
    feed.delta_to(ctx, Audience::Actor(ctx.actor), delta);
    info(
        ctx,
        format!(
            "Applied {n} op(s) to {} (revision {from} -> {to}).",
            display_name(ctx.w, obj)
        ),
    );
    Ok(())
}

fn unit_remove(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let obj = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let unit = ctx
        .w
        .module_state::<UnitStore>()
        .and_then(|store| store.units.get(&obj))
        .cloned()
        .ok_or_else(|| CmdError::msg("It is not a tactical unit."))?;
    require_unit_ready(ctx.reg, &unit)?;
    let removed = ctx
        .w
        .module_state_mut::<UnitStore>()
        .expect("unit store")
        .remove_active(obj);
    debug_assert!(removed, "unit was present immediately before removal");
    persist_unit_del(ctx.out, obj);
    // Programmatic detach bypasses the DetachGate (the store entry is already gone).
    let _ = ctx
        .w
        .detach_behavior(obj, &BehaviorId::new(UNIT_BEHAVIOR).unwrap());
    info(
        ctx,
        format!("{} is no longer a tactical unit.", display_name(ctx.w, obj)),
    );
    Ok(())
}

//! Gate-2 fixture: one component-provided attack prepares two tactical-unit
//! post-images and one deterministic resolution before either unit can install.
//!
//! This deliberately lives in the contract-test crate. Production
//! `mush-tactics` still owns its legacy unit-document authority, so proving the
//! K3/K6 transaction shape here does not create a second live authority for
//! production `TacticalUnit` records.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionInvocationDraft, ActionOfferId,
    ActionPresentation, ActionProvider, ActionStateError, ActionTxn, ActionWindow,
    ActionWindowDraft, ActionWindowError, ActionWindowGuardValidator, ActionWindowId,
    ActionWindowPhase, ActionWindowRecord, ActionWindowStateCodec, AuthoritativeActionOutput,
    AuthorityGuardKind, BridgeDescriptor, CanonicalHash, CanonicalId, ChannelGrant, ChannelId,
    ClockProviderFailure, ClockRef, ClockStamp, CommitToken, ContactGuardValidator, ContactId,
    ContactState, ContactStateCodec, ContactStateDraft, ControlHandoffDraft,
    ControlLeaseGuardValidator, ControlLeaseKey, ControlLeaseState, ControlLeaseStateCodec,
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef, DefinitionRegistryBuilder,
    DefinitionValidationContext, DefinitionValidator, Engine, GameModule, IdempotencyKey,
    IdempotencyScope, LegalActionQuery, ModError, ObjId, OfferKeyRing, OfferSecret, OwnerId,
    PerceivedRef, PrepareCtx, PrincipalId, ProjectedParameterSchema, ProjectedTargetSchema,
    ProviderOfferDraft, Registrar, RequestUid, ResolutionKeyRing, ResolutionSecret,
    ResolvedActionInvocation, RevisionKey, RevisionKind, RevisionSet, RevisionValidator,
    RevisionValidatorContext, RulesValue, ScopedIdempotencyKey, StateFamilyDescriptor,
    StateFamilyHandle, StateFamilyId, SubjectKey, WindowIntent, WindowMode, WindowOrderKey,
    WindowRestartPolicy, WindowTimeoutPolicy, World, action_window_state_family_descriptor,
    contact_state_family_descriptor, control_lease_state_family_descriptor,
};
use mush_tactics::{
    CellAddr, CellId, CellSpec, ComponentId, ComponentSpec, ConditionId, InstallPoint, SectionId,
    SectionSpec, TacticalUnit, UnitMutationOp, UnitMutationPlan, UnitSpec, UnitSpecBody, apply,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Owner used by the isolated Gate-2 module.
pub const TACTICAL_GATE_OWNER: &str = "toytactics";
/// Independent owner of the fixture's tactical placement truth.
pub const TACTICAL_PLACEMENT_OWNER: &str = "toyspace";
/// Component-provided action exercised by Gate 2.
pub const FIRE_ACTION_ID: &str = "fire";
/// Private durable outcome for the fire action.
pub const FIRED_OUTCOME_ID: &str = "fired";
/// Engine-owned family holding the fixture's unit records.
pub const TACTICAL_UNIT_FAMILY: &str = "units";
/// Engine-owned family holding the fixture's channel leases.
pub const CONTROL_LEASE_FAMILY: &str = "control_leases";
/// Engine-owned family holding the fixture's action window.
pub const ACTION_WINDOW_FAMILY: &str = "action_windows";
/// Engine-owned family holding the fixture's map placement records.
pub const PLACEMENT_FAMILY: &str = "placements";
/// Internal action that consumes one durably closed simultaneous fire set.
pub const RESOLVE_VOLLEY_ACTION_ID: &str = "resolve_volley";
/// Private durable outcome for simultaneous fire resolution.
pub const VOLLEY_RESOLVED_OUTCOME_ID: &str = "volley_resolved";
/// Component action that durably submits one fire intent into a volley window.
pub const SUBMIT_FIRE_ACTION_ID: &str = "submit_fire";
/// Private durable outcome for accepting or replacing a fire intent.
pub const FIRE_SUBMITTED_OUTCOME_ID: &str = "fire_submitted";

const UNIT_RULES_ID: &str = "unit_state";
const WEAPON_DEFINITION_ID: &str = "weapon";
const UNIT_DEFINITION_ID: &str = "skirmisher";
const WEAPON_COMPONENT_ID: &str = "primary_weapon";
const AMMO_CELL_ID: &str = "ammo";
const HEAT_CELL_ID: &str = "heat";
const STRUCTURE_CELL_ID: &str = "structure";
const READY_CONDITION_ID: &str = "ready";
#[cfg(test)]
const DISABLED_CONDITION_ID: &str = "disabled";
const SECTION_IDS: [&str; 3] = ["forward", "center", "aft"];

#[derive(Clone, Debug)]
pub struct TacticalGateDefinitions {
    pub unit: DefinitionRef,
    pub weapon: DefinitionRef,
    pub fire: DefinitionRef,
    pub fired: DefinitionRef,
    pub placement_bridge: DefinitionRef,
    pub resolve_volley: DefinitionRef,
    pub volley_resolved: DefinitionRef,
    pub submit_fire: DefinitionRef,
    pub fire_submitted: DefinitionRef,
    pub submit_placement_bridge: DefinitionRef,
}

#[derive(Clone, Debug)]
pub struct TacticalGateState {
    pub units: StateFamilyHandle<ObjId, TacticalUnit>,
    pub contacts: StateFamilyHandle<ContactId, ContactState>,
    pub control_leases: StateFamilyHandle<ControlLeaseKey, ControlLeaseState>,
    pub windows: StateFamilyHandle<ActionWindowId, ActionWindowRecord>,
    pub placements: StateFamilyHandle<ObjId, TacticalPlacement>,
}

#[derive(Clone)]
struct DefinitionFixture {
    descriptors: Vec<DefinitionDescriptor>,
    definitions: TacticalGateDefinitions,
    unit_spec: UnitSpec,
}

fn descriptor(kind: &str, id: &str, payload: Value) -> Result<DefinitionDescriptor, String> {
    DefinitionDescriptor::new(
        DefinitionKind::new(kind).map_err(|error| error.to_string())?,
        DefinitionId::new(id).map_err(|error| error.to_string())?,
        1,
        payload,
    )
    .map_err(|error| error.to_string())
}

fn definition_fixture() -> Result<DefinitionFixture, String> {
    let owner = OwnerId::new(TACTICAL_GATE_OWNER).map_err(|error| error.to_string())?;
    let mut registry = DefinitionRegistryBuilder::new();
    let mut descriptors = Vec::new();

    let rules_descriptor = descriptor(
        "schema",
        UNIT_RULES_ID,
        json!({ "fields": {}, "purpose": "gate_2_unit_state" }),
    )?
    .as_current();
    let rules = registry
        .register(owner.clone(), rules_descriptor.clone())
        .map_err(|error| error.to_string())?;
    descriptors.push(rules_descriptor);

    let weapon_descriptor = descriptor(
        "component",
        WEAPON_DEFINITION_ID,
        json!({ "provides_action": FIRE_ACTION_ID, "channel": "weapons" }),
    )?
    .as_current();
    let weapon = registry
        .register(owner.clone(), weapon_descriptor.clone())
        .map_err(|error| error.to_string())?;
    descriptors.push(weapon_descriptor);

    let body = UnitSpecBody {
        sections: SECTION_IDS
            .iter()
            .map(|section| SectionSpec {
                id: SectionId::new(section).expect("static section id"),
                condition: ConditionId::new(READY_CONDITION_ID).expect("static condition id"),
                facets: Vec::new(),
                slots: Vec::new(),
                cells: vec![CellSpec {
                    id: CellId::new(STRUCTURE_CELL_ID).expect("static cell id"),
                    min: 0,
                    max: 12,
                    init: 12,
                }],
            })
            .collect(),
        unit_cells: vec![CellSpec {
            id: CellId::new(HEAT_CELL_ID).expect("static cell id"),
            min: 0,
            max: 10,
            init: 0,
        }],
        components: vec![ComponentSpec {
            id: ComponentId::new(WEAPON_COMPONENT_ID).expect("static component id"),
            definition: weapon.clone(),
            install: InstallPoint::Section(SectionId::new("forward").expect("static section id")),
            condition: ConditionId::new(READY_CONDITION_ID).expect("static condition id"),
            cells: vec![CellSpec {
                id: CellId::new(AMMO_CELL_ID).expect("static cell id"),
                min: 0,
                max: 5,
                init: 2,
            }],
            asset: None,
        }],
        capabilities: Vec::new(),
        mobility: None,
        rules: RulesValue::new(rules, json!({})).map_err(|error| error.to_string())?,
    };
    let unit_descriptor = body
        .descriptor(
            DefinitionKind::new("unit").expect("static definition kind"),
            DefinitionId::new(UNIT_DEFINITION_ID).expect("static definition id"),
            1,
        )
        .map_err(|error| error.to_string())?
        .as_current();
    let unit = registry
        .register(owner.clone(), unit_descriptor.clone())
        .map_err(|error| error.to_string())?;
    descriptors.push(unit_descriptor);
    let unit_spec = body.with_definition(unit.clone());

    let fired_descriptor = descriptor(
        "outcome",
        FIRED_OUTCOME_ID,
        json!({
            "private_fields": [
                "source", "target", "source_revision", "target_revision", "section", "damage"
            ],
            "projected_result": "weapon_fired"
        }),
    )?
    .as_current();
    let fired = registry
        .register(owner.clone(), fired_descriptor.clone())
        .map_err(|error| error.to_string())?;
    descriptors.push(fired_descriptor);

    let fire_descriptor = descriptor(
        "action",
        FIRE_ACTION_ID,
        json!({
            "provided_by": WEAPON_COMPONENT_ID,
            "required_channel": "weapons",
            "source_mutations": ["ammo", "heat"],
            "target_mutation": "section_structure",
            "resolution_draws": ["to_hit", "location", "damage"]
        }),
    )?
    .with_references(vec![unit.clone(), weapon.clone(), fired.clone()])
    .as_current();
    let fire = registry
        .register(owner.clone(), fire_descriptor.clone())
        .map_err(|error| error.to_string())?;
    descriptors.push(fire_descriptor);

    let fire_submitted_descriptor = descriptor(
        "outcome",
        FIRE_SUBMITTED_OUTCOME_ID,
        json!({
            "private_fields": ["window", "revision"],
            "projected_result": "fire_intent_submitted"
        }),
    )?
    .as_current();
    let fire_submitted = registry
        .register(owner.clone(), fire_submitted_descriptor.clone())
        .map_err(|error| error.to_string())?;
    descriptors.push(fire_submitted_descriptor);

    let submit_fire_descriptor = descriptor(
        "action",
        SUBMIT_FIRE_ACTION_ID,
        json!({
            "provided_by": WEAPON_COMPONENT_ID,
            "required_channel": "weapons",
            "operation": "submit_or_replace_simultaneous_intent",
            "resolution_draws": []
        }),
    )?
    .with_references(vec![unit.clone(), weapon.clone(), fire_submitted.clone()])
    .as_current();
    let submit_fire = registry
        .register(owner.clone(), submit_fire_descriptor.clone())
        .map_err(|error| error.to_string())?;
    descriptors.push(submit_fire_descriptor);

    let placement_bridge_descriptor = descriptor(
        "bridge",
        "tactical_placement",
        json!({
            "source_owner": TACTICAL_GATE_OWNER,
            "target_owner": TACTICAL_PLACEMENT_OWNER,
            "interaction": FIRE_ACTION_ID,
            "purpose": "read_exact_selected_placement_revision"
        }),
    )?
    .with_references(vec![fire.clone()])
    .as_current();
    let placement_bridge = registry
        .register(owner.clone(), placement_bridge_descriptor.clone())
        .map_err(|error| error.to_string())?;
    descriptors.push(placement_bridge_descriptor);

    let submit_placement_bridge_descriptor = descriptor(
        "bridge",
        "submit_fire_placement",
        json!({
            "source_owner": TACTICAL_GATE_OWNER,
            "target_owner": TACTICAL_PLACEMENT_OWNER,
            "interaction": SUBMIT_FIRE_ACTION_ID,
            "purpose": "validate_submitted_fire_placements"
        }),
    )?
    .with_references(vec![submit_fire.clone()])
    .as_current();
    let submit_placement_bridge = registry
        .register(owner.clone(), submit_placement_bridge_descriptor.clone())
        .map_err(|error| error.to_string())?;
    descriptors.push(submit_placement_bridge_descriptor);

    let volley_resolved_descriptor = descriptor(
        "outcome",
        VOLLEY_RESOLVED_OUTCOME_ID,
        json!({
            "private_fields": ["window", "shots"],
            "projected_result": "volley_resolved"
        }),
    )?
    .as_current();
    let volley_resolved = registry
        .register(owner.clone(), volley_resolved_descriptor.clone())
        .map_err(|error| error.to_string())?;
    descriptors.push(volley_resolved_descriptor);

    let resolve_volley_descriptor = descriptor(
        "action",
        RESOLVE_VOLLEY_ACTION_ID,
        json!({
            "internal": true,
            "consumes": "closed_action_window",
            "resolution": "stable_window_order"
        }),
    )?
    .with_references(vec![
        unit.clone(),
        weapon.clone(),
        submit_fire.clone(),
        volley_resolved.clone(),
    ])
    .as_current();
    let resolve_volley = registry
        .register(owner, resolve_volley_descriptor.clone())
        .map_err(|error| error.to_string())?;
    descriptors.push(resolve_volley_descriptor);

    Ok(DefinitionFixture {
        descriptors,
        definitions: TacticalGateDefinitions {
            unit,
            weapon,
            fire,
            fired,
            placement_bridge,
            resolve_volley,
            volley_resolved,
            submit_fire,
            fire_submitted,
            submit_placement_bridge,
        },
        unit_spec,
    })
}

#[derive(Clone)]
struct ExpectedDefinition {
    payload: Value,
    references: Vec<DefinitionRef>,
    current: bool,
}

fn expected_by_kind(
    fixture: &DefinitionFixture,
) -> BTreeMap<DefinitionKind, BTreeMap<(DefinitionId, u32), ExpectedDefinition>> {
    let mut kinds: BTreeMap<_, BTreeMap<_, _>> = BTreeMap::new();
    for descriptor in &fixture.descriptors {
        let mut references = descriptor.references().to_vec();
        references.sort();
        kinds.entry(descriptor.kind().clone()).or_default().insert(
            (descriptor.id().clone(), descriptor.version()),
            ExpectedDefinition {
                payload: descriptor.payload().clone(),
                references,
                current: descriptor.is_current(),
            },
        );
    }
    kinds
}

fn validate_definition(
    expected: &BTreeMap<(DefinitionId, u32), ExpectedDefinition>,
    context: &DefinitionValidationContext<'_>,
) -> Result<(), String> {
    let entry = context.definition();
    let key = (
        entry.reference().key().id.clone(),
        entry.reference().version(),
    );
    let Some(shape) = expected.get(&key) else {
        return Err("unknown Gate-2 definition".to_owned());
    };
    if entry.payload() != &shape.payload
        || entry.references() != shape.references
        || entry.is_current() != shape.current
    {
        return Err("Gate-2 definition differs from its typed descriptor".to_owned());
    }
    Ok(())
}

struct FixtureDefinitionValidator {
    expected: BTreeMap<(DefinitionId, u32), ExpectedDefinition>,
}

impl DefinitionValidator for FixtureDefinitionValidator {
    fn validate(&self, context: &DefinitionValidationContext<'_>) -> Result<(), String> {
        validate_definition(&self.expected, context)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticalPlacement {
    object: ObjId,
    arena: CanonicalId,
    x: i32,
    y: i32,
    revision: u64,
}

impl TacticalPlacement {
    fn new(object: ObjId, x: i32, y: i32) -> Self {
        Self {
            object,
            arena: CanonicalId::new("arena").expect("static arena id"),
            x,
            y,
            revision: 1,
        }
    }

    fn moved(&self, x: i32, y: i32) -> Self {
        Self {
            x,
            y,
            revision: self.revision.checked_add(1).expect("fixture revision"),
            ..self.clone()
        }
    }
}

#[derive(Debug)]
struct PlacementRevisionValidator {
    placements: StateFamilyHandle<ObjId, TacticalPlacement>,
}

impl RevisionValidator for PlacementRevisionValidator {
    fn current_generation(
        &self,
        context: &RevisionValidatorContext<'_>,
        key: &RevisionKey,
    ) -> Result<Option<u64>, ActionError> {
        if key.owner != *self.placements.owner()
            || key.kind.as_id().as_str() != "placement"
            || key.facet.is_some()
        {
            return Ok(None);
        }
        let SubjectKey::Object { id } = key.subject else {
            return Ok(None);
        };
        let placement = context.action_state(&self.placements, &id).map_err(|_| {
            ActionError::new(
                ActionErrorCode::Integrity,
                false,
                "tactical placement state is unavailable",
            )
            .expect("static placement error is bounded")
        })?;
        Ok(placement
            .filter(|placement| placement.object == id)
            .map(|placement| placement.revision))
    }
}

#[derive(Debug)]
struct TacticalPlacementModule {
    captured: Arc<Mutex<Option<StateFamilyHandle<ObjId, TacticalPlacement>>>>,
}

impl GameModule for TacticalPlacementModule {
    fn name(&self) -> &'static str {
        TACTICAL_PLACEMENT_OWNER
    }

    fn register(&mut self, registrar: &mut Registrar<'_>) -> Result<(), ModError> {
        let placements = registrar.state_family(
            StateFamilyDescriptor::new(
                StateFamilyId::new(PLACEMENT_FAMILY)
                    .map_err(|error| ModError::msg(error.to_string()))?,
                CanonicalId::new("obj_id.decimal.v1")
                    .map_err(|error| ModError::msg(error.to_string()))?,
                CanonicalId::new("toyspace.placement.v1")
                    .map_err(|error| ModError::msg(error.to_string()))?,
                1,
                CanonicalId::new("toyspace.placement.decode_json.v1")
                    .map_err(|error| ModError::msg(error.to_string()))?,
                CanonicalId::new("toyspace.placement.validate.v1")
                    .map_err(|error| ModError::msg(error.to_string()))?,
                CanonicalId::new("toyspace.placement.json.v1")
                    .map_err(|error| ModError::msg(error.to_string()))?,
            )
            .map_err(|error| ModError::msg(error.to_string()))?,
            |key: &ObjId| Ok(key.0.to_string()),
            |value: &Value| {
                serde_json::from_value(value.clone()).map_err(|error| error.to_string())
            },
            |value: &TacticalPlacement| {
                if value.revision == 0 {
                    Err("a tactical placement revision must be nonzero".to_owned())
                } else {
                    Ok(())
                }
            },
            |value: &TacticalPlacement| {
                serde_json::to_value(value).map_err(|error| error.to_string())
            },
        )?;
        registrar.revision_validator(
            RevisionKind::new(CanonicalId::new("placement").expect("static revision kind")),
            PlacementRevisionValidator {
                placements: placements.clone(),
            },
        )?;
        *self.captured.lock().expect("placement capture poisoned") = Some(placements);
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct FireProvider {
    units: StateFamilyHandle<ObjId, TacticalUnit>,
    contacts: StateFamilyHandle<ContactId, ContactState>,
    #[cfg(test)]
    control_leases: StateFamilyHandle<ControlLeaseKey, ControlLeaseState>,
    windows: StateFamilyHandle<ActionWindowId, ActionWindowRecord>,
    placements: StateFamilyHandle<ObjId, TacticalPlacement>,
    window: ActionWindowId,
    action: DefinitionRef,
    outcome: DefinitionRef,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FireParameters {
    source_revision: u64,
    #[serde(default)]
    target_revision: Option<u64>,
    now: ClockStamp,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct FiredOutcome {
    event: String,
    source: ObjId,
    target: ObjId,
    source_revision: u64,
    target_revision: u64,
    section: SectionId,
    damage: i64,
    to_hit: u64,
}

#[cfg(test)]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ProjectedMeter {
    id: String,
    minimum: i64,
    maximum: i64,
    current: i64,
}

#[cfg(test)]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ProjectedSection {
    id: String,
    condition: String,
    meters: Vec<ProjectedMeter>,
}

#[cfg(test)]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ProjectedComponent {
    id: String,
    condition: String,
    meters: Vec<ProjectedMeter>,
}

#[cfg(test)]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ProjectedOwnUnit {
    subject: PerceivedRef,
    state_revision: u64,
    meters: Vec<ProjectedMeter>,
    sections: Vec<ProjectedSection>,
    components: Vec<ProjectedComponent>,
}

#[cfg(test)]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ProjectedContactUnit {
    contact: PerceivedRef,
    observed_at: ClockStamp,
    condition_band: String,
}

#[cfg(test)]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ProjectedTacticalSnapshot {
    observed_at: ClockStamp,
    own_unit: ProjectedOwnUnit,
    contacts: Vec<ProjectedContactUnit>,
}

impl FireProvider {
    fn invalid(message: &'static str) -> ActionError {
        ActionError::new(ActionErrorCode::InvalidRequest, false, message)
            .expect("static Gate-2 request error is bounded")
    }

    fn stale(message: &'static str) -> ActionError {
        ActionError::new(ActionErrorCode::Stale, false, message)
            .expect("static Gate-2 stale error is bounded")
    }

    fn rejected(message: &'static str) -> ActionError {
        ActionError::new(ActionErrorCode::ProviderRejected, false, message)
            .expect("static Gate-2 provider error is bounded")
    }

    fn integrity(message: &'static str) -> ActionError {
        ActionError::new(ActionErrorCode::Integrity, false, message)
            .expect("static Gate-2 integrity error is bounded")
    }

    fn state_error(error: &ActionStateError) -> ActionError {
        let code = match error {
            ActionStateError::OperationLimit { .. }
            | ActionStateError::BundleBytesExceeded { .. }
            | ActionStateError::CanonicalPostImage { .. }
            | ActionStateError::ArithmeticOverflow
            | ActionStateError::RevisionExhausted => ActionErrorCode::BoundsExceeded,
            ActionStateError::OwnerNotReady(_) => ActionErrorCode::Unavailable,
            ActionStateError::Missing { .. } | ActionStateError::AlreadyExists { .. } => {
                ActionErrorCode::Conflict
            }
            _ => ActionErrorCode::Integrity,
        };
        ActionError::new(code, false, "Gate-2 tactical state operation was refused")
            .expect("static Gate-2 state error is bounded")
    }

    fn decode_parameters(invocation: &ActionInvocation) -> Result<FireParameters, ActionError> {
        if !matches!(
            invocation.subject(),
            PerceivedRef::SelfActor | PerceivedRef::KnownObject { .. }
        ) || invocation.targets().len() != 1
        {
            return Err(Self::invalid(
                "fire requires one controlled unit subject and exactly one target",
            ));
        }
        let parameters: FireParameters = serde_json::from_value(invocation.parameters().clone())
            .map_err(|_| Self::invalid("fire parameters are malformed"))?;
        match &parameters.now.clock {
            ClockRef::DomainTick {
                owner,
                id,
                generation,
            } if owner.as_str() == TACTICAL_GATE_OWNER
                && id.as_str() == "arena"
                && *generation > 0 => {}
            _ => return Err(Self::invalid("fire requires the exact Gate-2 domain clock")),
        }
        Ok(parameters)
    }

    fn known_target(invocation: &ActionInvocation) -> Result<ObjId, ActionError> {
        let PerceivedRef::KnownObject { id: target, .. } = invocation.targets()[0] else {
            return Err(Self::invalid(
                "direct Gate-2 fire requires a known-object target",
            ));
        };
        Ok(target)
    }

    fn known_source(invocation: &ActionInvocation) -> Result<ObjId, ActionError> {
        match invocation.subject() {
            PerceivedRef::SelfActor => Ok(invocation.actor()),
            PerceivedRef::KnownObject { id, .. } => Ok(*id),
            PerceivedRef::Contact { .. } => Err(Self::invalid(
                "fire requires a self or known controlled unit subject",
            )),
        }
    }

    fn resolved_source(resolved: &ResolvedActionInvocation) -> Result<ObjId, ActionError> {
        let SubjectKey::Object { id } = resolved.truth_subject() else {
            return Err(Self::integrity("resolved fire subject is not an object"));
        };
        Ok(*id)
    }

    fn resolved_target(resolved: &ResolvedActionInvocation) -> Result<ObjId, ActionError> {
        let [SubjectKey::Object { id: target }] = resolved.truth_targets() else {
            return Err(Self::integrity(
                "resolved fire must name exactly one object target",
            ));
        };
        Ok(*target)
    }

    fn read_unit(&self, txn: &mut ActionTxn<'_>, id: ObjId) -> Result<TacticalUnit, ActionError> {
        let unit = txn
            .get(&self.units, &id)
            .map_err(|error| Self::state_error(&error))?
            .cloned()
            .ok_or_else(|| Self::stale("a tactical unit is no longer available"))?;
        if unit.object() != id {
            return Err(Self::integrity(
                "a Gate-2 unit record disagrees with its family key",
            ));
        }
        Ok(unit)
    }

    fn read_target_unit(
        &self,
        txn: &mut ActionTxn<'_>,
        id: ObjId,
    ) -> Result<TacticalUnit, ActionError> {
        let unit = txn
            .get(&self.units, &id)
            .map_err(|_| Self::stale("the selected target is unavailable"))?
            .cloned()
            .ok_or_else(|| Self::stale("the selected target is unavailable"))?;
        if unit.object() != id {
            return Err(Self::stale("the selected target is unavailable"));
        }
        Ok(unit)
    }

    #[cfg(test)]
    fn projected_meter(id: &CellId, cell: &mush_tactics::ScalarCell) -> ProjectedMeter {
        ProjectedMeter {
            id: id.as_str().to_owned(),
            minimum: cell.min,
            maximum: cell.max,
            current: cell.current,
        }
    }

    /// Build an observer DTO before any transport or renderer sees the data.
    /// Exact section/component detail is limited to the operated unit; contact
    /// truth is reduced to an opaque reference plus a ruleset-level band.
    #[cfg(test)]
    fn project_tactical_snapshot(
        &self,
        engine: &Engine,
        principal: &PrincipalId,
        source_id: ObjId,
    ) -> Result<ProjectedTacticalSnapshot, ActionError> {
        let PrincipalId::Object { id: actor } = principal else {
            return Err(Self::stale("the tactical status view is unavailable"));
        };
        let weapons = ChannelId::new("weapons").expect("static channel id");
        let mut grants = engine
            .world
            .command_subjects_of(*actor)
            .iter()
            .filter(|grant| {
                grant.subject == source_id
                    && grant.owner == TACTICAL_GATE_OWNER
                    && grant.channels.contains(&weapons)
            });
        let Some(grant) = grants.next() else {
            return Err(Self::stale("the tactical status view is unavailable"));
        };
        if grants.next().is_some() {
            return Err(Self::stale("the tactical status view is unavailable"));
        }
        let lease_key = ControlLeaseKey::new(source_id, weapons);
        let lease = engine
            .action_state()
            .get(&self.control_leases, &lease_key)
            .map_err(|error| Self::state_error(&error))?
            .ok_or_else(|| Self::stale("the tactical status view is unavailable"))?;
        if lease.holder() != Some(principal) || lease.epoch() != grant.epoch {
            return Err(Self::stale("the tactical status view is unavailable"));
        }
        let source = engine
            .action_state()
            .get(&self.units, &source_id)
            .map_err(|error| Self::state_error(&error))?
            .ok_or_else(|| Self::stale("the operated tactical unit is unavailable"))?;
        let source_placement = engine
            .action_state()
            .get(&self.placements, &source_id)
            .map_err(|error| Self::state_error(&error))?
            .ok_or_else(|| Self::stale("the operated tactical unit is unavailable"))?;
        if source.object() != source_id || source_placement.object != source_id {
            return Err(Self::integrity(
                "the operated tactical projection has inconsistent identity",
            ));
        }

        let meters = source
            .cells()
            .filter_map(|(address, cell)| match address {
                CellAddr::Unit { cell: id } => Some(Self::projected_meter(id, cell)),
                _ => None,
            })
            .collect();
        let sections = source
            .sections()
            .map(|(id, section)| ProjectedSection {
                id: id.as_str().to_owned(),
                condition: section.condition.as_str().to_owned(),
                meters: source
                    .cells()
                    .filter_map(|(address, cell)| match address {
                        CellAddr::Section {
                            section: owner,
                            cell: meter,
                        } if owner == id => Some(Self::projected_meter(meter, cell)),
                        _ => None,
                    })
                    .collect(),
            })
            .collect();
        let components = source
            .components()
            .map(|(id, component)| ProjectedComponent {
                id: id.as_str().to_owned(),
                condition: component.condition.as_str().to_owned(),
                meters: source
                    .cells()
                    .filter_map(|(address, cell)| match address {
                        CellAddr::Component {
                            component: owner,
                            cell: meter,
                        } if owner == id => Some(Self::projected_meter(meter, cell)),
                        _ => None,
                    })
                    .collect(),
            })
            .collect();

        let page = engine
            .action_state()
            .scan_family(
                &self.contacts,
                None,
                mush_core::MAX_STATE_FAMILY_SCAN_RECORDS
                    .min(mush_core::MAX_PROJECTED_ACTION_CHOICES),
            )
            .map_err(|error| Self::state_error(&error))?;
        let mut contacts = Vec::new();
        for entry in page.entries() {
            let contact = entry.value();
            if !contact.is_visible_to(principal) {
                continue;
            }
            let SubjectKey::Object { id: target_id } = contact.truth_subject() else {
                continue;
            };
            let Some(target) = engine
                .action_state()
                .get(&self.units, target_id)
                .map_err(|error| Self::state_error(&error))?
            else {
                continue;
            };
            let Some(target_placement) = engine
                .action_state()
                .get(&self.placements, target_id)
                .map_err(|error| Self::state_error(&error))?
            else {
                continue;
            };
            if target.object() != *target_id
                || target_placement.object != *target_id
                || target_placement.arena != source_placement.arena
            {
                continue;
            }
            let id = ContactId::from_hex(entry.encoded_key())
                .map_err(|_| Self::integrity("a contact family key is malformed"))?;
            let condition_band = if target.cells().any(|(_, cell)| cell.current < cell.max) {
                "damaged"
            } else {
                "nominal"
            };
            contacts.push(ProjectedContactUnit {
                contact: PerceivedRef::Contact {
                    provider: self.contacts.owner().clone(),
                    id,
                    generation: contact.generation(),
                },
                observed_at: contact.observed_at().clone(),
                condition_band: condition_band.to_owned(),
            });
        }
        Ok(ProjectedTacticalSnapshot {
            observed_at: engine
                .clock_now(&tactical_gate_clock())
                .map_err(|_| Self::stale("the tactical clock is unavailable"))?,
            own_unit: ProjectedOwnUnit {
                subject: PerceivedRef::SelfActor,
                state_revision: source.revision(),
                meters,
                sections,
                components,
            },
            contacts,
        })
    }

    fn prepare_fire(
        &self,
        invocation: &ActionInvocation,
        source_id: ObjId,
        target_id: ObjId,
        txn: &mut ActionTxn<'_>,
        context: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let parameters = Self::decode_parameters(invocation)?;
        if target_id == source_id {
            return Err(Self::invalid("a Gate-2 unit cannot fire on itself"));
        }
        let source = self.read_unit(txn, source_id)?;
        let target = self.read_target_unit(txn, target_id)?;
        if source.revision() != parameters.source_revision {
            return Err(Self::stale("a Gate-2 unit revision changed"));
        }
        if parameters
            .target_revision
            .is_some_and(|expected| target.revision() != expected)
        {
            return Err(Self::stale("the selected target is unavailable"));
        }

        let weapon_id = ComponentId::new(WEAPON_COMPONENT_ID).expect("static component id");
        let weapon = source
            .component(&weapon_id)
            .ok_or_else(|| Self::stale("the firing component is no longer installed"))?;
        if weapon.condition != ConditionId::new(READY_CONDITION_ID).expect("static condition id") {
            return Err(Self::stale("the firing component is not enabled"));
        }

        let ammo = CellAddr::Component {
            component: weapon_id,
            cell: CellId::new(AMMO_CELL_ID).expect("static cell id"),
        };
        let heat = CellAddr::Unit {
            cell: CellId::new(HEAT_CELL_ID).expect("static cell id"),
        };
        let ammo_state = source
            .cell(&ammo)
            .ok_or_else(|| Self::integrity("the firing component has no ammunition cell"))?;
        let heat_state = source
            .cell(&heat)
            .ok_or_else(|| Self::integrity("the firing unit has no heat cell"))?;
        if ammo_state.current <= ammo_state.min {
            return Err(Self::rejected("the firing component has no ammunition"));
        }
        if heat_state.current >= heat_state.max {
            return Err(Self::rejected("the firing unit cannot accept more heat"));
        }

        let inputs_hash = CanonicalHash::for_canonical_value(
            "toytactics.fire.inputs.v1",
            1,
            &json!({
                "action": self.action,
                "source": source_id,
                "target": target_id,
                "source_revision": source.revision(),
                "target_revision": target.revision(),
                "now": parameters.now,
            }),
            16 * 1024,
        )
        .map_err(|_| Self::integrity("the fire resolution inputs are not canonical"))?;
        let mut rng = context
            .start_resolution(
                &CanonicalId::new("toytactics.fire").expect("static resolution domain"),
                inputs_hash,
            )
            .map_err(|_| Self::integrity("the fire resolution RNG is unavailable"))?;
        let to_hit = rng
            .draw_bounded_u64(&CanonicalId::new("to_hit").expect("static draw label"), 12)
            .map_err(|_| Self::integrity("the to-hit draw was refused"))?;
        let location = rng
            .draw_bounded_u64(
                &CanonicalId::new("location").expect("static draw label"),
                SECTION_IDS.len() as u64,
            )
            .map_err(|_| Self::integrity("the location draw was refused"))?;
        let damage = rng
            .draw_bounded_u64(&CanonicalId::new("damage").expect("static draw label"), 4)
            .map_err(|_| Self::integrity("the damage draw was refused"))?
            + 1;
        let section = SectionId::new(SECTION_IDS[location as usize]).expect("static section id");
        let target_structure = CellAddr::Section {
            section: section.clone(),
            cell: CellId::new(STRUCTURE_CELL_ID).expect("static cell id"),
        };
        let structure = target
            .cell(&target_structure)
            .ok_or_else(|| Self::stale("the selected target is unavailable"))?;
        if structure.current <= structure.min {
            return Err(Self::stale("the selected target is unavailable"));
        }
        let applied_damage = i64::try_from(damage)
            .map_err(|_| Self::integrity("the damage draw exceeds the scalar domain"))?
            .min(structure.current - structure.min);

        // Build BOTH complete post-images first. No transaction operation is
        // staged until both independent UnitMutationPlans have succeeded.
        let source_plan = UnitMutationPlan::new(
            vec![
                UnitMutationOp::AdjustCell {
                    addr: ammo,
                    delta: -1,
                },
                UnitMutationOp::AdjustCell {
                    addr: heat,
                    delta: 1,
                },
            ],
            Some(source.revision()),
            None,
        )
        .map_err(|_| Self::integrity("the source mutation plan was refused"))?;
        let target_plan = UnitMutationPlan::new(
            vec![UnitMutationOp::AdjustCell {
                addr: target_structure,
                delta: -applied_damage,
            }],
            Some(target.revision()),
            None,
        )
        .map_err(|_| Self::integrity("the target mutation plan was refused"))?;
        let (source_post, _) = apply(&source, &source_plan, parameters.now.at)
            .map_err(|_| Self::stale("the source mutation plan became stale"))?;
        let (target_post, _) = apply(&target, &target_plan, parameters.now.at)
            .map_err(|_| Self::stale("the target mutation plan became stale"))?;

        txn.replace(&self.units, &source_id, source_post.clone())
            .map_err(|error| Self::state_error(&error))?;
        txn.replace(&self.units, &target_id, target_post.clone())
            .map_err(|error| Self::state_error(&error))?;

        let outcome = RulesValue::new(
            self.outcome.clone(),
            serde_json::to_value(FiredOutcome {
                event: "fired".to_owned(),
                source: source_id,
                target: target_id,
                source_revision: source_post.revision(),
                target_revision: target_post.revision(),
                section,
                damage: applied_damage,
                to_hit,
            })
            .map_err(|_| Self::integrity("the fire outcome could not be encoded"))?,
        )
        .map_err(|_| Self::integrity("the fire outcome exceeds canonical bounds"))?;
        let mut resolution_definitions = vec![
            self.action.clone(),
            source.schema().clone(),
            target.schema().clone(),
            weapon.definition.clone(),
        ];
        resolution_definitions.sort();
        resolution_definitions.dedup();
        let resolution = rng
            .into_prepared(
                resolution_definitions,
                RulesValue::new(
                    self.action.clone(),
                    json!({ "source_revision": source.revision(), "target_revision": target.revision() }),
                )
                .map_err(|_| Self::integrity("the fire modifiers exceed canonical bounds"))?,
                outcome.clone(),
            )
            .map_err(|_| Self::integrity("the fire resolution transcript is invalid"))?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_resolution(resolution)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

#[cfg(test)]
fn tactical_snapshot_keyframe(
    snapshot: &ProjectedTacticalSnapshot,
    generation: u64,
    sequence: u64,
) -> Result<mush_proto::ViewEnvelope, ActionError> {
    mush_proto::ViewEnvelope::keyframe(
        "unit.status",
        1,
        generation,
        sequence,
        serde_json::to_value(snapshot)
            .map_err(|_| FireProvider::integrity("the tactical snapshot could not be encoded"))?,
    )
    .map_err(|_| FireProvider::integrity("the tactical snapshot exceeds public bounds"))
}

#[cfg(test)]
fn render_tactical_snapshot_line(snapshot: &ProjectedTacticalSnapshot) -> String {
    let mut line = format!(
        "UNIT revision {}: {} section(s), {} component(s)",
        snapshot.own_unit.state_revision,
        snapshot.own_unit.sections.len(),
        snapshot.own_unit.components.len()
    );
    for contact in &snapshot.contacts {
        if let PerceivedRef::Contact { id, generation, .. } = &contact.contact {
            line.push_str(&format!(
                " | CONTACT {} generation {}: {}",
                id.to_hex(),
                generation,
                contact.condition_band
            ));
        }
    }
    line
}

impl ActionProvider for FireProvider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        Self::decode_parameters(invocation)?;
        if !matches!(
            invocation.targets()[0],
            PerceivedRef::KnownObject { .. } | PerceivedRef::Contact { .. }
        ) {
            return Err(Self::invalid("fire target reference is unsupported"));
        }
        Ok(())
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &LegalActionQuery,
        engine: &Engine,
    ) -> Result<Option<ProviderOfferDraft>, ActionError> {
        let SubjectKey::Object { id: source_id } = query.truth_subject() else {
            return Ok(None);
        };
        let projection_matches = match query.projected_subject() {
            PerceivedRef::SelfActor => *source_id == query.actor(),
            PerceivedRef::KnownObject { id, .. } => id == source_id,
            PerceivedRef::Contact { .. } => false,
        };
        if !projection_matches {
            return Ok(None);
        }
        let now = engine
            .clock_now(&tactical_gate_clock())
            .map_err(|_| Self::stale("the tactical clock is unavailable"))?;
        let Some(window) = engine
            .action_state()
            .get(&self.windows, &self.window)
            .map_err(|error| Self::state_error(&error))?
        else {
            return Ok(None);
        };
        if window.phase() != ActionWindowPhase::Open
            || !window
                .window()
                .contains(&now)
                .map_err(|_| Self::integrity("the tactical window clock is inconsistent"))?
        {
            return Ok(None);
        }
        let Some(source) = engine
            .action_state()
            .get(&self.units, source_id)
            .map_err(|error| Self::state_error(&error))?
        else {
            return Ok(None);
        };
        let Some(source_placement) = engine
            .action_state()
            .get(&self.placements, source_id)
            .map_err(|error| Self::state_error(&error))?
        else {
            return Ok(None);
        };
        if source_placement.object != *source_id {
            return Err(Self::integrity(
                "the source placement disagrees with its family key",
            ));
        }
        let weapon_id = ComponentId::new(WEAPON_COMPONENT_ID).expect("static component id");
        let ready = ConditionId::new(READY_CONDITION_ID).expect("static condition id");
        let ammo = CellAddr::Component {
            component: weapon_id.clone(),
            cell: CellId::new(AMMO_CELL_ID).expect("static cell id"),
        };
        let heat = CellAddr::Unit {
            cell: CellId::new(HEAT_CELL_ID).expect("static cell id"),
        };
        if source
            .component(&weapon_id)
            .is_none_or(|weapon| weapon.condition != ready)
            || source
                .cell(&ammo)
                .is_none_or(|cell| cell.current <= cell.min)
            || source
                .cell(&heat)
                .is_none_or(|cell| cell.current >= cell.max)
        {
            return Ok(None);
        }

        let page = engine
            .action_state()
            .scan_family(
                &self.contacts,
                None,
                mush_core::MAX_STATE_FAMILY_SCAN_RECORDS
                    .min(mush_core::MAX_PROJECTED_ACTION_CHOICES),
            )
            .map_err(|error| Self::state_error(&error))?;
        let mut choices = Vec::new();
        for entry in page.entries() {
            let contact = entry.value();
            let actionable_target = match contact.truth_subject() {
                SubjectKey::Object { id } => {
                    let unit_exists = engine
                        .action_state()
                        .get(&self.units, id)
                        .map_err(|error| Self::state_error(&error))?
                        .is_some();
                    let target_placement = engine
                        .action_state()
                        .get(&self.placements, id)
                        .map_err(|error| Self::state_error(&error))?;
                    unit_exists
                        && target_placement.is_some_and(|placement| {
                            placement.object == *id && placement.arena == source_placement.arena
                        })
                }
                SubjectKey::Scoped { .. } => false,
            };
            if actionable_target
                && contact.is_visible_to(query.principal())
                && contact.truth_subject() != query.truth_subject()
            {
                let id = ContactId::from_hex(entry.encoded_key())
                    .map_err(|_| Self::integrity("a contact family key is malformed"))?;
                choices.push(PerceivedRef::Contact {
                    provider: self.contacts.owner().clone(),
                    id,
                    generation: contact.generation(),
                });
            }
        }
        if choices.is_empty() {
            return Ok(None);
        }

        let definitions = engine
            .registry
            .definitions()
            .ok_or_else(|| Self::integrity("the definition registry is unavailable"))?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())
            .map_err(|_| Self::integrity("the guard registry is unavailable"))?;
        guards
            .add_definition(action.clone(), definitions)
            .map_err(|_| Self::integrity("the fire definition guard is unavailable"))?;
        guards
            .add(mush_core::AuthorityGuard::Window {
                owner: self.windows.owner().clone(),
                id: window.window().id(),
                generation: window.window().generation(),
            })
            .map_err(|_| Self::integrity("the fire window guard is unavailable"))?;
        Ok(Some(ProviderOfferDraft {
            fixed_parameters: json!({
                "source_revision": source.revision(),
                "now": now,
            }),
            parameters: ProjectedParameterSchema::empty(),
            targets: ProjectedTargetSchema::Choices {
                minimum: 1,
                maximum: 1,
                choices,
            },
            required_channels: vec![ChannelId::new("weapons").expect("static channel id")],
            expires: Some(window.window().closes().clone()),
            presentation: ActionPresentation::new(
                "fire",
                "Fire",
                "Fire the ready weapon at one currently perceived contact.",
            )
            .map_err(|_| Self::integrity("the fire presentation is invalid"))?,
            guards: guards
                .finish()
                .map_err(|_| Self::integrity("the fire guard set is invalid"))?,
        }))
    }

    fn delayed_decision_witness_keys(
        &self,
        _action: &DefinitionRef,
        query: &LegalActionQuery,
        offer: &mush_core::ProjectedLegalAction,
        engine: &Engine,
    ) -> Result<mush_core::DecisionWitnessKeys, ActionError> {
        let SubjectKey::Object { id: source } = query.truth_subject() else {
            return Err(Self::stale("the delayed fire source is unavailable"));
        };
        let placement_kind =
            RevisionKind::new(CanonicalId::new("placement").expect("static revision kind"));
        let common = vec![RevisionKey {
            owner: self.placements.owner().clone(),
            kind: placement_kind.clone(),
            subject: SubjectKey::Object { id: *source },
            facet: None,
        }];
        let choices = match offer.targets() {
            ProjectedTargetSchema::Choices { choices, .. } => choices,
            ProjectedTargetSchema::None => {
                return Err(Self::integrity("the delayed fire offer has no targets"));
            }
        };
        let mut targets = Vec::with_capacity(choices.len());
        for perceived in choices {
            let target = match perceived {
                PerceivedRef::KnownObject { id, .. } => *id,
                PerceivedRef::Contact {
                    provider,
                    id,
                    generation,
                } if provider == self.contacts.owner() => {
                    let contact = engine
                        .action_state()
                        .get(&self.contacts, id)
                        .map_err(|error| Self::state_error(&error))?
                        .filter(|contact| {
                            contact.generation() == *generation
                                && contact.is_visible_to(query.principal())
                        })
                        .ok_or_else(|| Self::stale("the delayed fire contact is unavailable"))?;
                    let SubjectKey::Object { id } = contact.truth_subject() else {
                        return Err(Self::stale("the delayed fire target is unavailable"));
                    };
                    *id
                }
                PerceivedRef::Contact { .. } => {
                    return Err(Self::stale("the delayed fire contact route changed"));
                }
                PerceivedRef::SelfActor => {
                    return Err(Self::stale("the delayed fire target cannot be the actor"));
                }
            };
            targets.push(mush_core::DecisionTargetWitnessKeys {
                perceived: perceived.clone(),
                revisions: vec![RevisionKey {
                    owner: self.placements.owner().clone(),
                    kind: placement_kind.clone(),
                    subject: SubjectKey::Object { id: target },
                    facet: None,
                }],
            });
        }
        Ok(mush_core::DecisionWitnessKeys { common, targets })
    }

    fn prepare_invocation(
        &self,
        _invocation: &ActionInvocation,
        _txn: &mut ActionTxn<'_>,
    ) -> Result<(), ActionError> {
        Err(ActionError::new(
            ActionErrorCode::Unavailable,
            false,
            "Gate-2 fire requires the authoritative action coordinator",
        )
        .expect("static Gate-2 migration error is bounded"))
    }

    fn prepare_authoritative_with_context(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        context: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let source = Self::known_source(invocation)?;
        let target = Self::known_target(invocation)?;
        self.prepare_fire(invocation, source, target, txn, context)
    }

    fn prepare_resolved_authoritative_with_context(
        &self,
        resolved: &ResolvedActionInvocation,
        txn: &mut ActionTxn<'_>,
        context: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let source = Self::resolved_source(resolved)?;
        let target = Self::resolved_target(resolved)?;
        self.prepare_fire(resolved.invocation(), source, target, txn, context)
    }

    fn resolved_revision_keys(
        &self,
        resolved: &ResolvedActionInvocation,
    ) -> Result<Vec<RevisionKey>, ActionError> {
        let kind = RevisionKind::new(CanonicalId::new("placement").expect("static revision kind"));
        Ok(vec![
            RevisionKey {
                owner: self.placements.owner().clone(),
                kind: kind.clone(),
                subject: SubjectKey::Object {
                    id: Self::resolved_source(resolved)?,
                },
                facet: None,
            },
            RevisionKey {
                owner: self.placements.owner().clone(),
                kind,
                subject: SubjectKey::Object {
                    id: Self::resolved_target(resolved)?,
                },
                facet: None,
            },
        ])
    }

    fn project_receipt(
        &self,
        action: &DefinitionRef,
        outcome: &RulesValue,
        _viewer: &PrincipalId,
    ) -> Result<Option<mush_proto::ProjectedActionResult>, ActionError> {
        if action != &self.action || outcome.schema() != &self.outcome {
            return Err(Self::integrity(
                "the fire receipt carries the wrong durable schema",
            ));
        }
        let decoded: FiredOutcome = serde_json::from_value(outcome.data().clone())
            .map_err(|_| Self::integrity("the durable fire outcome is malformed"))?;
        if decoded.event != "fired" || decoded.damage <= 0 {
            return Err(Self::integrity(
                "the durable fire outcome violates the Gate-2 contract",
            ));
        }
        mush_proto::ProjectedActionResult::new(
            "fire",
            "Weapon fired.",
            json!({ "event": "weapon_fired" }),
        )
        .map(Some)
        .map_err(|_| Self::integrity("the projected fire receipt is invalid"))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct FireSubmittedOutcome {
    event: String,
    window: ActionWindowId,
    revision: u64,
}

#[derive(Debug)]
struct VolleySubmitProvider {
    projector: FireProvider,
    action: DefinitionRef,
    outcome: DefinitionRef,
}

impl VolleySubmitProvider {
    fn submission_error(error: ActionWindowError) -> ActionError {
        let (code, message) = match error {
            ActionWindowError::CapacityReached { .. } => (
                ActionErrorCode::ProviderRejected,
                "the simultaneous fire window is full",
            ),
            ActionWindowError::InputOperationLimit
            | ActionWindowError::ReceiptLimit
            | ActionWindowError::RecordBytes { .. } => (
                ActionErrorCode::BoundsExceeded,
                "the simultaneous fire window reached its durable operation bound",
            ),
            ActionWindowError::OperationConflict { .. } => (
                ActionErrorCode::InvalidRequest,
                "the simultaneous fire request identity was reused",
            ),
            ActionWindowError::StaleRevision { .. }
            | ActionWindowError::NotOpen { .. }
            | ActionWindowError::OutsideWindow => (
                ActionErrorCode::Stale,
                "the simultaneous fire window changed",
            ),
            _ => (
                ActionErrorCode::Integrity,
                "the simultaneous fire window rejected a valid intent",
            ),
        };
        ActionError::new(code, false, message).expect("static submission error is bounded")
    }
}

impl ActionProvider for VolleySubmitProvider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        self.projector.validate_invocation(invocation)
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &LegalActionQuery,
        engine: &Engine,
    ) -> Result<Option<ProviderOfferDraft>, ActionError> {
        let Some(mut draft) = self.projector.project_offer(action, query, engine)? else {
            return Ok(None);
        };
        draft.presentation = ActionPresentation::new(
            "submit_fire",
            "Submit fire",
            "Commit or replace this unit's fire intent for simultaneous resolution.",
        )
        .map_err(|_| FireProvider::integrity("the fire submission presentation is invalid"))?;
        Ok(Some(draft))
    }

    fn prepare_invocation(
        &self,
        _invocation: &ActionInvocation,
        _txn: &mut ActionTxn<'_>,
    ) -> Result<(), ActionError> {
        Err(ActionError::new(
            ActionErrorCode::Unavailable,
            false,
            "simultaneous fire submission requires resolved authority",
        )
        .expect("static simultaneous submission error is bounded"))
    }

    fn prepare_resolved_authoritative_with_context(
        &self,
        resolved: &ResolvedActionInvocation,
        txn: &mut ActionTxn<'_>,
        _context: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let source_id = FireProvider::resolved_source(resolved)?;
        let target_id = FireProvider::resolved_target(resolved)?;
        if source_id == target_id {
            return Err(FireProvider::invalid(
                "a Gate-2 unit cannot submit fire on itself",
            ));
        }
        let parameters = FireProvider::decode_parameters(resolved.invocation())?;
        let source = self.projector.read_unit(txn, source_id)?;
        let target = self.projector.read_target_unit(txn, target_id)?;
        if source.revision() != parameters.source_revision
            || parameters
                .target_revision
                .is_some_and(|revision| target.revision() != revision)
        {
            return Err(FireProvider::stale(
                "a simultaneous fire participant changed",
            ));
        }
        let weapon_id = ComponentId::new(WEAPON_COMPONENT_ID).expect("static component id");
        let weapon = source
            .component(&weapon_id)
            .ok_or_else(|| FireProvider::stale("the firing component is no longer installed"))?;
        let ammo = CellAddr::Component {
            component: weapon_id,
            cell: CellId::new(AMMO_CELL_ID).expect("static cell id"),
        };
        let heat = CellAddr::Unit {
            cell: CellId::new(HEAT_CELL_ID).expect("static cell id"),
        };
        if weapon.condition != ConditionId::new(READY_CONDITION_ID).expect("static condition id")
            || source
                .cell(&ammo)
                .is_none_or(|cell| cell.current <= cell.min)
            || source
                .cell(&heat)
                .is_none_or(|cell| cell.current >= cell.max)
        {
            return Err(FireProvider::stale(
                "the firing component cannot submit an intent",
            ));
        }

        let record = txn
            .get(&self.projector.windows, &self.projector.window)
            .map_err(|error| FireProvider::state_error(&error))?
            .cloned()
            .ok_or_else(|| FireProvider::stale("the simultaneous fire window is unavailable"))?;
        if record.phase() != ActionWindowPhase::Open
            || record.window().mode() != WindowMode::CollectSimultaneous
            || !record
                .window()
                .contains(&parameters.now)
                .map_err(|_| FireProvider::integrity("the volley clock is inconsistent"))?
        {
            return Err(FireProvider::stale(
                "the simultaneous fire window is unavailable",
            ));
        }
        let intent = WindowIntent::new(
            record.window().id(),
            record.window().generation(),
            resolved.truth_subject().clone(),
            resolved.truth_targets().to_vec(),
            self.action.clone(),
            resolved.invocation().clone(),
            WindowOrderKey::new(vec![i64::from(source_id.0)])
                .map_err(|_| FireProvider::integrity("the volley order key is invalid"))?,
        )
        .map_err(|_| FireProvider::integrity("the volley intent is invalid"))?;
        let transition = record
            .checked_submit(record.revision(), &parameters.now, intent)
            .map_err(Self::submission_error)?;
        let record = transition.into_record();
        txn.replace(
            &self.projector.windows,
            &self.projector.window,
            record.clone(),
        )
        .map_err(|error| FireProvider::state_error(&error))?;
        let outcome = RulesValue::new(
            self.outcome.clone(),
            serde_json::to_value(FireSubmittedOutcome {
                event: "fire_intent_submitted".to_owned(),
                window: self.projector.window,
                revision: record.revision(),
            })
            .map_err(|_| FireProvider::integrity("the submission outcome could not be encoded"))?,
        )
        .map_err(|_| FireProvider::integrity("the submission outcome exceeds bounds"))?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![resolved.invocation().principal().clone()]))
    }

    fn resolved_revision_keys(
        &self,
        resolved: &ResolvedActionInvocation,
    ) -> Result<Vec<RevisionKey>, ActionError> {
        self.projector.resolved_revision_keys(resolved)
    }

    fn project_receipt(
        &self,
        action: &DefinitionRef,
        outcome: &RulesValue,
        _viewer: &PrincipalId,
    ) -> Result<Option<mush_proto::ProjectedActionResult>, ActionError> {
        if action != &self.action || outcome.schema() != &self.outcome {
            return Err(FireProvider::integrity(
                "fire submission receipt carries the wrong schema",
            ));
        }
        let decoded: FireSubmittedOutcome = serde_json::from_value(outcome.data().clone())
            .map_err(|_| FireProvider::integrity("fire submission outcome is malformed"))?;
        if decoded.event != "fire_intent_submitted" || decoded.revision == 0 {
            return Err(FireProvider::integrity(
                "fire submission outcome violates its contract",
            ));
        }
        mush_proto::ProjectedActionResult::new(
            "submit_fire",
            "Fire intent accepted.",
            json!({ "event": "fire_intent_submitted" }),
        )
        .map(Some)
        .map_err(|_| FireProvider::integrity("fire submission receipt is invalid"))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct VolleyShotOutcome {
    source: ObjId,
    target: ObjId,
    section: SectionId,
    damage: i64,
    to_hit: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct VolleyResolvedOutcome {
    event: String,
    window: ActionWindowId,
    shots: Vec<VolleyShotOutcome>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolveVolleyParameters {
    expected_revision: u64,
    now: ClockStamp,
}

#[derive(Debug)]
struct VolleyResolver {
    units: StateFamilyHandle<ObjId, TacticalUnit>,
    windows: StateFamilyHandle<ActionWindowId, ActionWindowRecord>,
    window: ActionWindowId,
    fire: DefinitionRef,
    action: DefinitionRef,
    outcome: DefinitionRef,
}

impl VolleyResolver {
    fn parameters(invocation: &ActionInvocation) -> Result<ResolveVolleyParameters, ActionError> {
        let parameters: ResolveVolleyParameters =
            serde_json::from_value(invocation.parameters().clone())
                .map_err(|_| FireProvider::invalid("volley resolution parameters are malformed"))?;
        if parameters.expected_revision == 0 || parameters.now.clock != tactical_gate_clock() {
            return Err(FireProvider::invalid(
                "volley resolution requires an exact revision and domain clock",
            ));
        }
        Ok(parameters)
    }

    fn window_error() -> ActionError {
        FireProvider::stale("the simultaneous fire window is no longer resolvable")
    }
}

impl ActionProvider for VolleyResolver {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        if invocation.subject() != &PerceivedRef::SelfActor || !invocation.targets().is_empty() {
            return Err(FireProvider::invalid(
                "volley resolution is an internal actor-self action",
            ));
        }
        Self::parameters(invocation)?;
        Ok(())
    }

    fn prepare_invocation(
        &self,
        _invocation: &ActionInvocation,
        _txn: &mut ActionTxn<'_>,
    ) -> Result<(), ActionError> {
        Err(ActionError::new(
            ActionErrorCode::Unavailable,
            false,
            "volley resolution requires the authoritative action coordinator",
        )
        .expect("static volley migration error is bounded"))
    }

    fn prepare_authoritative_with_context(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        context: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.validate_invocation(invocation)?;
        let parameters = Self::parameters(invocation)?;
        let record = txn
            .get(&self.windows, &self.window)
            .map_err(|error| FireProvider::state_error(&error))?
            .cloned()
            .ok_or_else(Self::window_error)?;
        if record.revision() != parameters.expected_revision
            || record.phase() != ActionWindowPhase::Closed
        {
            return Err(Self::window_error());
        }
        let intents: Vec<_> = record
            .resolution_order()
            .map_err(|_| Self::window_error())?
            .into_iter()
            .cloned()
            .collect();
        if intents.is_empty() {
            return Err(FireProvider::rejected(
                "an empty simultaneous window has no fire to resolve",
            ));
        }

        let weapon_id = ComponentId::new(WEAPON_COMPONENT_ID).expect("static component id");
        let ammo_addr = CellAddr::Component {
            component: weapon_id.clone(),
            cell: CellId::new(AMMO_CELL_ID).expect("static cell id"),
        };
        let heat_addr = CellAddr::Unit {
            cell: CellId::new(HEAT_CELL_ID).expect("static cell id"),
        };
        let ready = ConditionId::new(READY_CONDITION_ID).expect("static condition id");
        let mut decoded = Vec::with_capacity(intents.len());
        let mut unit_ids = BTreeSet::new();
        for intent in &intents {
            if intent.action() != &self.fire {
                return Err(Self::window_error());
            }
            let SubjectKey::Object { id: source } = intent.truth_subject() else {
                return Err(Self::window_error());
            };
            let [SubjectKey::Object { id: target }] = intent.truth_targets() else {
                return Err(Self::window_error());
            };
            if source == target {
                return Err(Self::window_error());
            }
            let fire_parameters = FireProvider::decode_parameters(intent.invocation())?;
            decoded.push((*source, *target, fire_parameters));
            unit_ids.insert(*source);
            unit_ids.insert(*target);
        }

        let mut originals = BTreeMap::new();
        for id in unit_ids {
            let unit = txn
                .get(&self.units, &id)
                .map_err(|error| FireProvider::state_error(&error))?
                .cloned()
                .ok_or_else(Self::window_error)?;
            if unit.object() != id {
                return Err(FireProvider::integrity(
                    "a volley unit disagrees with its family key",
                ));
            }
            originals.insert(id, unit);
        }
        for (source, target, fire_parameters) in &decoded {
            let source_unit = originals.get(source).ok_or_else(Self::window_error)?;
            let target_unit = originals.get(target).ok_or_else(Self::window_error)?;
            if source_unit.revision() != fire_parameters.source_revision
                || fire_parameters
                    .target_revision
                    .is_some_and(|revision| target_unit.revision() != revision)
            {
                return Err(Self::window_error());
            }
            let weapon = source_unit
                .component(&weapon_id)
                .ok_or_else(Self::window_error)?;
            if weapon.condition != ready
                || source_unit
                    .cell(&ammo_addr)
                    .is_none_or(|cell| cell.current <= cell.min)
                || source_unit
                    .cell(&heat_addr)
                    .is_none_or(|cell| cell.current >= cell.max)
            {
                return Err(Self::window_error());
            }
        }

        let input_units: Vec<_> = originals.values().collect();
        let inputs_hash = CanonicalHash::for_canonical_value(
            "toytactics.volley.inputs.v1",
            1,
            &json!({ "window": record, "units": input_units }),
            256 * 1024,
        )
        .map_err(|_| FireProvider::integrity("the volley inputs are not canonical"))?;
        let mut rng = context
            .start_resolution(
                &CanonicalId::new("toytactics.volley").expect("static resolution domain"),
                inputs_hash,
            )
            .map_err(|_| FireProvider::integrity("the volley resolution RNG is unavailable"))?;
        let mut working = originals.clone();
        let mut shots = Vec::with_capacity(decoded.len());
        for (index, (source, target, _)) in decoded.iter().enumerate() {
            let to_hit = rng
                .draw_bounded_u64(
                    &CanonicalId::new(format!("to_hit_{index}"))
                        .expect("bounded volley index is canonical"),
                    12,
                )
                .map_err(|_| FireProvider::integrity("the volley to-hit draw was refused"))?;
            let location = rng
                .draw_bounded_u64(
                    &CanonicalId::new(format!("location_{index}"))
                        .expect("bounded volley index is canonical"),
                    SECTION_IDS.len() as u64,
                )
                .map_err(|_| FireProvider::integrity("the volley location draw was refused"))?;
            let damage = rng
                .draw_bounded_u64(
                    &CanonicalId::new(format!("damage_{index}"))
                        .expect("bounded volley index is canonical"),
                    4,
                )
                .map_err(|_| FireProvider::integrity("the volley damage draw was refused"))?
                + 1;
            let section = SectionId::new(SECTION_IDS[location as usize])
                .expect("static section id is canonical");
            let target_structure = CellAddr::Section {
                section: section.clone(),
                cell: CellId::new(STRUCTURE_CELL_ID).expect("static cell id"),
            };

            let source_unit = working
                .get(source)
                .cloned()
                .ok_or_else(Self::window_error)?;
            let source_plan = UnitMutationPlan::new(
                vec![
                    UnitMutationOp::AdjustCell {
                        addr: ammo_addr.clone(),
                        delta: -1,
                    },
                    UnitMutationOp::AdjustCell {
                        addr: heat_addr.clone(),
                        delta: 1,
                    },
                ],
                Some(source_unit.revision()),
                None,
            )
            .map_err(|_| FireProvider::integrity("the volley source plan was refused"))?;
            let (source_post, _) = apply(&source_unit, &source_plan, parameters.now.at)
                .map_err(|_| Self::window_error())?;
            working.insert(*source, source_post);

            let target_unit = working
                .get(target)
                .cloned()
                .ok_or_else(Self::window_error)?;
            let structure = target_unit
                .cell(&target_structure)
                .ok_or_else(Self::window_error)?;
            if structure.current <= structure.min {
                return Err(Self::window_error());
            }
            let applied_damage = i64::try_from(damage)
                .map_err(|_| FireProvider::integrity("volley damage exceeds the scalar domain"))?
                .min(structure.current - structure.min);
            let target_plan = UnitMutationPlan::new(
                vec![UnitMutationOp::AdjustCell {
                    addr: target_structure,
                    delta: -applied_damage,
                }],
                Some(target_unit.revision()),
                None,
            )
            .map_err(|_| FireProvider::integrity("the volley target plan was refused"))?;
            let (target_post, _) = apply(&target_unit, &target_plan, parameters.now.at)
                .map_err(|_| Self::window_error())?;
            working.insert(*target, target_post);
            shots.push(VolleyShotOutcome {
                source: *source,
                target: *target,
                section,
                damage: applied_damage,
                to_hit,
            });
        }

        for (id, post) in &working {
            if originals.get(id) != Some(post) {
                txn.replace(&self.units, id, post.clone())
                    .map_err(|error| FireProvider::state_error(&error))?;
            }
        }
        let consumed = record
            .checked_consume(
                record.revision(),
                invocation.request(),
                context.commit_token(),
                &parameters.now,
            )
            .map_err(|_| Self::window_error())?
            .into_record();
        txn.replace(&self.windows, &self.window, consumed)
            .map_err(|error| FireProvider::state_error(&error))?;

        let outcome = RulesValue::new(
            self.outcome.clone(),
            serde_json::to_value(VolleyResolvedOutcome {
                event: "volley_resolved".to_owned(),
                window: self.window,
                shots: shots.clone(),
            })
            .map_err(|_| FireProvider::integrity("the volley outcome could not be encoded"))?,
        )
        .map_err(|_| FireProvider::integrity("the volley outcome exceeds canonical bounds"))?;
        let mut resolution_definitions = vec![self.action.clone(), self.fire.clone()];
        for unit in originals.values() {
            resolution_definitions.extend(unit.definition_references());
        }
        resolution_definitions.sort();
        resolution_definitions.dedup();
        let resolution = rng
            .into_prepared(
                resolution_definitions,
                RulesValue::new(
                    self.action.clone(),
                    json!({ "window_revision": record.revision(), "shot_count": shots.len() }),
                )
                .map_err(|_| FireProvider::integrity("the volley modifiers are invalid"))?,
                outcome.clone(),
            )
            .map_err(|_| FireProvider::integrity("the volley transcript is invalid"))?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_resolution(resolution)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

#[derive(Debug, Default)]
pub struct TacticalGateModule {
    captured: Arc<Mutex<Option<(TacticalGateState, TacticalGateDefinitions, UnitSpec)>>>,
    placements: Arc<Mutex<Option<StateFamilyHandle<ObjId, TacticalPlacement>>>>,
}

impl TacticalGateModule {
    fn with_capture(
        captured: Arc<Mutex<Option<(TacticalGateState, TacticalGateDefinitions, UnitSpec)>>>,
        placements: Arc<Mutex<Option<StateFamilyHandle<ObjId, TacticalPlacement>>>>,
    ) -> Self {
        Self {
            captured,
            placements,
        }
    }
}

impl GameModule for TacticalGateModule {
    fn name(&self) -> &'static str {
        TACTICAL_GATE_OWNER
    }

    fn register(&mut self, registrar: &mut Registrar<'_>) -> Result<(), ModError> {
        let fixture = definition_fixture().map_err(ModError::msg)?;
        for (kind, expected) in expected_by_kind(&fixture) {
            registrar.definition_validator(kind, FixtureDefinitionValidator { expected })?;
        }
        let mut registered = BTreeSet::new();
        for descriptor in fixture.descriptors.clone() {
            let reference = registrar.definition(descriptor)?;
            registered.insert(reference);
        }
        // Comparing the complete known references catches accidental descriptor
        // order/content drift before the provider receives any capability.
        for expected in [
            &fixture.definitions.unit,
            &fixture.definitions.weapon,
            &fixture.definitions.fire,
            &fixture.definitions.fired,
            &fixture.definitions.placement_bridge,
            &fixture.definitions.resolve_volley,
            &fixture.definitions.volley_resolved,
            &fixture.definitions.submit_fire,
            &fixture.definitions.fire_submitted,
            &fixture.definitions.submit_placement_bridge,
        ] {
            if !registered.contains(expected) {
                return Err(ModError::msg("Gate-2 definition identity drift"));
            }
        }

        let unit_spec = fixture.unit_spec.clone();
        let validator_spec = unit_spec.clone();
        let units = registrar.state_family(
            StateFamilyDescriptor::new(
                StateFamilyId::new(TACTICAL_UNIT_FAMILY)
                    .map_err(|error| ModError::msg(error.to_string()))?,
                CanonicalId::new("obj_id.decimal.v1")
                    .map_err(|error| ModError::msg(error.to_string()))?,
                CanonicalId::new("toytactics.unit.v1")
                    .map_err(|error| ModError::msg(error.to_string()))?,
                1,
                CanonicalId::new("toytactics.unit.decode_json.v1")
                    .map_err(|error| ModError::msg(error.to_string()))?,
                CanonicalId::new("toytactics.unit.validate.v1")
                    .map_err(|error| ModError::msg(error.to_string()))?,
                CanonicalId::new("toytactics.unit.json.v1")
                    .map_err(|error| ModError::msg(error.to_string()))?,
            )
            .map_err(|error| ModError::msg(error.to_string()))?,
            |key: &ObjId| Ok(key.0.to_string()),
            |value: &Value| {
                serde_json::from_value(value.clone()).map_err(|error| error.to_string())
            },
            move |value: &TacticalUnit| {
                validator_spec
                    .validate_instance(value)
                    .map_err(|error| error.to_string())
            },
            |value: &TacticalUnit| serde_json::to_value(value).map_err(|error| error.to_string()),
        )?;
        let placements = self
            .placements
            .lock()
            .expect("placement capture poisoned")
            .clone()
            .ok_or_else(|| ModError::msg("placement owner must register before tactics"))?;
        registrar.interaction_bridge(BridgeDescriptor::new(
            fixture.definitions.placement_bridge.clone(),
            OwnerId::new(TACTICAL_GATE_OWNER).expect("static tactics owner"),
            OwnerId::new(TACTICAL_PLACEMENT_OWNER).expect("static placement owner"),
            fixture.definitions.fire.clone(),
        ))?;
        registrar.interaction_bridge(BridgeDescriptor::new(
            fixture.definitions.submit_placement_bridge.clone(),
            OwnerId::new(TACTICAL_GATE_OWNER).expect("static tactics owner"),
            OwnerId::new(TACTICAL_PLACEMENT_OWNER).expect("static placement owner"),
            fixture.definitions.submit_fire.clone(),
        ))?;
        let contact_codec = ContactStateCodec;
        let contacts = registrar.state_family(
            contact_state_family_descriptor(
                StateFamilyId::new("contacts").map_err(|error| ModError::msg(error.to_string()))?,
            )
            .map_err(|error| ModError::msg(error.to_string()))?,
            contact_codec,
            contact_codec,
            contact_codec,
            contact_codec,
        )?;
        registrar.authority_guard_validator(
            AuthorityGuardKind::Contact,
            ContactGuardValidator::new(contacts.clone()),
        )?;
        let lease_codec = ControlLeaseStateCodec;
        let control_leases = registrar.state_family(
            control_lease_state_family_descriptor(
                StateFamilyId::new(CONTROL_LEASE_FAMILY)
                    .map_err(|error| ModError::msg(error.to_string()))?,
            )
            .map_err(|error| ModError::msg(error.to_string()))?,
            lease_codec,
            lease_codec,
            lease_codec,
            lease_codec,
        )?;
        registrar.authority_guard_validator(
            AuthorityGuardKind::ControlLease,
            ControlLeaseGuardValidator::new(control_leases.clone()),
        )?;
        let window_codec = ActionWindowStateCodec;
        let windows = registrar.state_family(
            action_window_state_family_descriptor(
                StateFamilyId::new(ACTION_WINDOW_FAMILY)
                    .map_err(|error| ModError::msg(error.to_string()))?,
            )
            .map_err(|error| ModError::msg(error.to_string()))?,
            window_codec,
            window_codec,
            window_codec,
            window_codec,
        )?;
        registrar.authority_guard_validator(
            AuthorityGuardKind::Window,
            ActionWindowGuardValidator::new(windows.clone()),
        )?;
        registrar.domain_clock_provider(
            |_context: &mush_core::ClockProviderContext<'_>, domain: &CanonicalId, generation| {
                if domain.as_str() != "arena" {
                    return Err(ClockProviderFailure::Unavailable);
                }
                if generation == 1 {
                    Ok(7)
                } else {
                    Err(ClockProviderFailure::StaleGeneration {
                        requested: generation,
                        current: 1,
                    })
                }
            },
        )?;
        registrar.action_provider(
            fixture.definitions.fire.clone(),
            FireProvider {
                units: units.clone(),
                contacts: contacts.clone(),
                #[cfg(test)]
                control_leases: control_leases.clone(),
                windows: windows.clone(),
                placements: placements.clone(),
                window: tactical_gate_window_id(),
                action: fixture.definitions.fire.clone(),
                outcome: fixture.definitions.fired.clone(),
            },
        )?;
        registrar.action_provider(
            fixture.definitions.submit_fire.clone(),
            VolleySubmitProvider {
                projector: FireProvider {
                    units: units.clone(),
                    contacts: contacts.clone(),
                    #[cfg(test)]
                    control_leases: control_leases.clone(),
                    windows: windows.clone(),
                    placements: placements.clone(),
                    window: tactical_gate_volley_window_id(),
                    action: fixture.definitions.submit_fire.clone(),
                    outcome: fixture.definitions.fire_submitted.clone(),
                },
                action: fixture.definitions.submit_fire.clone(),
                outcome: fixture.definitions.fire_submitted.clone(),
            },
        )?;
        registrar.action_provider(
            fixture.definitions.resolve_volley.clone(),
            VolleyResolver {
                units: units.clone(),
                windows: windows.clone(),
                window: tactical_gate_volley_window_id(),
                fire: fixture.definitions.submit_fire.clone(),
                action: fixture.definitions.resolve_volley.clone(),
                outcome: fixture.definitions.volley_resolved.clone(),
            },
        )?;
        *self.captured.lock().expect("Gate-2 capture poisoned") = Some((
            TacticalGateState {
                units,
                contacts,
                control_leases,
                windows,
                placements,
            },
            fixture.definitions,
            unit_spec,
        ));
        Ok(())
    }
}

/// Compose the isolated Gate-2 module and install deterministic resolution keys.
#[must_use]
pub fn tactical_gate_world() -> (Engine, TacticalGateState, TacticalGateDefinitions, UnitSpec) {
    tactical_gate_world_with_modules(Vec::new())
}

/// Compose the tactical gate together with additional independent modules.
/// This is used by later vertical gates to exercise the same provider through
/// scheduling, delayed decisions, and restart coordination.
#[must_use]
pub fn tactical_gate_world_with_modules(
    mut additional: Vec<Box<dyn GameModule>>,
) -> (Engine, TacticalGateState, TacticalGateDefinitions, UnitSpec) {
    let captured = Arc::new(Mutex::new(None));
    let placements = Arc::new(Mutex::new(None));
    let placement_module = TacticalPlacementModule {
        captured: Arc::clone(&placements),
    };
    let module = TacticalGateModule::with_capture(Arc::clone(&captured), placements);
    let mut modules: Vec<Box<dyn GameModule>> = vec![Box::new(placement_module), Box::new(module)];
    modules.append(&mut additional);
    let mut engine =
        Engine::with_modules(World::new_minimal(), mush_core::Registry::new(), modules)
            .expect("Gate-2 module composes");
    engine.set_resolution_keys(
        ResolutionKeyRing::new(
            ResolutionSecret::new(1, [0x62; 32]).expect("Gate-2 resolution key"),
            [],
        )
        .expect("Gate-2 resolution ring"),
    );
    engine.set_offer_keys(
        OfferKeyRing::new(
            OfferSecret::new(1, [0x63; 32]).expect("Gate-3 offer key"),
            [],
        )
        .expect("Gate-3 offer key ring"),
    );
    let (state, definitions, unit_spec) = captured
        .lock()
        .expect("Gate-2 capture poisoned")
        .clone()
        .expect("Gate-2 module captured its state family");
    (engine, state, definitions, unit_spec)
}

/// Fixture-only typed bootstrap; gameplay changes must use the fire provider.
pub fn install_tactical_gate_unit(
    engine: &mut Engine,
    state: &TacticalGateState,
    unit: TacticalUnit,
) {
    let id = unit.object();
    let prepared = {
        let mut txn = engine
            .action_transaction(&state.units)
            .expect("Gate-2 bootstrap transaction");
        txn.insert(&state.units, &id, unit)
            .expect("valid Gate-2 unit bootstrap");
        txn.prepare().expect("Gate-2 unit bootstrap prepares")
    };
    let (_, installation) = prepared.into_parts();
    engine
        .install_prepared_action(installation)
        .expect("the fixture installs");
}

/// Fixture-only placement bootstrap for a unit deployed into the tactical arena.
pub fn install_tactical_gate_placement(
    engine: &mut Engine,
    state: &TacticalGateState,
    object: ObjId,
    x: i32,
    y: i32,
) -> TacticalPlacement {
    let placement = TacticalPlacement::new(object, x, y);
    let prepared = {
        let mut txn = engine
            .action_transaction(&state.placements)
            .expect("Gate-4 placement bootstrap transaction");
        txn.insert(&state.placements, &object, placement.clone())
            .expect("valid Gate-4 placement bootstrap");
        txn.prepare().expect("Gate-4 placement bootstrap prepares")
    };
    let (_, installation) = prepared.into_parts();
    engine
        .install_prepared_action(installation)
        .expect("the fixture installs");
    placement
}

/// Fixture-only movement used to prove retained placement guards become stale.
pub fn move_tactical_gate_placement(
    engine: &mut Engine,
    state: &TacticalGateState,
    object: ObjId,
    x: i32,
    y: i32,
) -> TacticalPlacement {
    let current = engine
        .action_state()
        .get(&state.placements, &object)
        .expect("Gate-4 placement read")
        .expect("Gate-4 placement exists");
    let placement = current.moved(x, y);
    let prepared = {
        let mut txn = engine
            .action_transaction(&state.placements)
            .expect("Gate-4 placement movement transaction");
        txn.replace(&state.placements, &object, placement.clone())
            .expect("valid Gate-4 placement movement");
        txn.prepare().expect("Gate-4 placement movement prepares")
    };
    let (_, installation) = prepared.into_parts();
    engine
        .install_prepared_action(installation)
        .expect("the fixture installs");
    placement
}

/// Fixture-only source mutation used by Gate 4 to invalidate retained
/// component and resource assumptions before delayed completion.
pub fn disable_tactical_gate_weapon(
    engine: &mut Engine,
    state: &TacticalGateState,
    object: ObjId,
) -> TacticalUnit {
    let current = engine
        .action_state()
        .get(&state.units, &object)
        .expect("Gate-4 unit read")
        .expect("Gate-4 unit exists");
    let plan = UnitMutationPlan::new(
        vec![UnitMutationOp::SetComponentCondition {
            component: ComponentId::new(WEAPON_COMPONENT_ID).expect("static component id"),
            condition: ConditionId::new("disabled").expect("static condition id"),
        }],
        Some(current.revision()),
        None,
    )
    .expect("Gate-4 component mutation plan");
    let (changed, _) = apply(current, &plan, 7).expect("Gate-4 component mutation applies");
    let prepared = {
        let mut txn = engine
            .action_transaction(&state.units)
            .expect("Gate-4 component transaction");
        txn.replace(&state.units, &object, changed.clone())
            .expect("Gate-4 component post-image");
        txn.prepare().expect("Gate-4 component mutation prepares")
    };
    let (_, installation) = prepared.into_parts();
    engine
        .install_prepared_action(installation)
        .expect("the fixture installs");
    changed
}

/// Fixture-only resource mutation used by Gate 4 to make a retained fire
/// offer unaffordable before delayed completion.
pub fn exhaust_tactical_gate_ammo(
    engine: &mut Engine,
    state: &TacticalGateState,
    object: ObjId,
) -> TacticalUnit {
    let current = engine
        .action_state()
        .get(&state.units, &object)
        .expect("Gate-4 unit read")
        .expect("Gate-4 unit exists");
    let plan = UnitMutationPlan::new(
        vec![UnitMutationOp::SetCell {
            addr: CellAddr::Component {
                component: ComponentId::new(WEAPON_COMPONENT_ID).expect("static component id"),
                cell: CellId::new(AMMO_CELL_ID).expect("static ammo id"),
            },
            value: 0,
        }],
        Some(current.revision()),
        None,
    )
    .expect("Gate-4 resource mutation plan");
    let (changed, _) = apply(current, &plan, 7).expect("Gate-4 resource mutation applies");
    let prepared = {
        let mut txn = engine
            .action_transaction(&state.units)
            .expect("Gate-4 resource transaction");
        txn.replace(&state.units, &object, changed.clone())
            .expect("Gate-4 resource post-image");
        txn.prepare().expect("Gate-4 resource mutation prepares")
    };
    let (_, installation) = prepared.into_parts();
    engine
        .install_prepared_action(installation)
        .expect("the fixture installs");
    changed
}

/// Fixture-only private contact bootstrap.
pub fn install_tactical_gate_contact(
    engine: &mut Engine,
    state: &TacticalGateState,
    id: ContactId,
    target: ObjId,
    viewer: PrincipalId,
) -> ContactState {
    let contact = ContactState::new(ContactStateDraft {
        truth_subject: SubjectKey::Object { id: target },
        audience: vec![viewer],
        observed_at: tactical_gate_tick(6),
    })
    .expect("valid Gate-3 contact");
    let prepared = {
        let mut txn = engine
            .action_transaction(&state.contacts)
            .expect("Gate-3 contact bootstrap transaction");
        txn.insert(&state.contacts, &id, contact.clone())
            .expect("valid Gate-3 contact bootstrap");
        txn.prepare().expect("Gate-3 contact bootstrap prepares")
    };
    let (_, installation) = prepared.into_parts();
    engine
        .install_prepared_action(installation)
        .expect("the fixture installs");
    contact
}

/// Fixture-only active weapons-lease bootstrap.
pub fn install_tactical_gate_weapons_lease(
    engine: &mut Engine,
    state: &TacticalGateState,
    subject: ObjId,
    holder: PrincipalId,
) -> ControlLeaseState {
    let key = ControlLeaseKey::new(
        subject,
        ChannelId::new("weapons").expect("static channel id"),
    );
    let request = RequestUid::from_bytes([0x64; 16]);
    let lease = ControlLeaseState::new(key.clone())
        .expect("valid vacant Gate-3 control lease")
        .checked_request_handoff(
            0,
            ControlHandoffDraft {
                request,
                requested_by: holder.clone(),
                next_holder: Some(holder),
                requested_at: tactical_gate_tick(1),
                boundary: tactical_gate_tick(2),
                reason: CanonicalId::new("fixture_assignment").expect("static reason"),
            },
        )
        .expect("valid Gate-3 control request")
        .checked_apply_handoff(1, request, tactical_gate_tick(2))
        .expect("Gate-3 lease reaches its safe boundary");
    let prepared = {
        let mut txn = engine
            .action_transaction(&state.control_leases)
            .expect("Gate-3 lease bootstrap transaction");
        txn.insert(&state.control_leases, &key, lease.clone())
            .expect("valid Gate-3 lease bootstrap");
        txn.prepare().expect("Gate-3 lease bootstrap prepares")
    };
    let (_, installation) = prepared.into_parts();
    engine
        .install_prepared_action(installation)
        .expect("the fixture installs");
    lease
}

/// Publish the transient station half of one weapons authority bundle.
pub fn publish_tactical_gate_weapons_station(
    engine: &mut Engine,
    actor: ObjId,
    subject: ObjId,
    epoch: u64,
) {
    engine.world.publish_command_subject(ChannelGrant {
        actor,
        subject,
        station: "weapons_console".into(),
        relation: "gunner".into(),
        owner: TACTICAL_GATE_OWNER.into(),
        priority: 50,
        channels: BTreeSet::from([ChannelId::new("weapons").expect("static channel id")]),
        epoch,
    });
}

/// Fixture-only open tactical action-window bootstrap.
pub fn install_tactical_gate_fire_window(
    engine: &mut Engine,
    state: &TacticalGateState,
) -> ActionWindowRecord {
    let window = ActionWindow::new(ActionWindowDraft {
        id: tactical_gate_window_id(),
        clock: tactical_gate_clock(),
        opens: tactical_gate_tick(0),
        closes: tactical_gate_tick(10),
        generation: 1,
        mode: WindowMode::Immediate,
        capacity: 1,
        restart: WindowRestartPolicy::Resume,
        timeout: WindowTimeoutPolicy::Close,
        max_catch_up: 1,
        resolution_secret_version: None,
    })
    .expect("valid Gate-3 fire window");
    let record = ActionWindowRecord::open(window);
    let id = record.window().id();
    let prepared = {
        let mut txn = engine
            .action_transaction(&state.windows)
            .expect("Gate-3 window bootstrap transaction");
        txn.insert(&state.windows, &id, record.clone())
            .expect("valid Gate-3 window bootstrap");
        txn.prepare().expect("Gate-3 window bootstrap prepares")
    };
    let (_, installation) = prepared.into_parts();
    engine
        .install_prepared_action(installation)
        .expect("the fixture installs");
    record
}

/// Fixture-only open simultaneous-fire collection window bootstrap.
pub fn install_tactical_gate_volley_window(
    engine: &mut Engine,
    state: &TacticalGateState,
    capacity: u32,
) -> ActionWindowRecord {
    let window = ActionWindow::new(ActionWindowDraft {
        id: tactical_gate_volley_window_id(),
        clock: tactical_gate_clock(),
        opens: tactical_gate_tick(0),
        closes: tactical_gate_tick(10),
        generation: 1,
        mode: WindowMode::CollectSimultaneous,
        capacity,
        restart: WindowRestartPolicy::Resume,
        timeout: WindowTimeoutPolicy::Close,
        max_catch_up: 1,
        resolution_secret_version: Some(1),
    })
    .expect("valid simultaneous fire window");
    let record = ActionWindowRecord::open(window);
    let id = record.window().id();
    let prepared = {
        let mut txn = engine
            .action_transaction(&state.windows)
            .expect("volley window bootstrap transaction");
        txn.insert(&state.windows, &id, record.clone())
            .expect("valid volley window bootstrap");
        txn.prepare().expect("volley window bootstrap prepares")
    };
    let (_, installation) = prepared.into_parts();
    engine
        .install_prepared_action(installation)
        .expect("the fixture installs");
    record
}

/// Fixture-only durable collection/close path for simultaneous fire intents.
pub fn install_tactical_gate_closed_volley(
    engine: &mut Engine,
    state: &TacticalGateState,
    definitions: &TacticalGateDefinitions,
    submissions: &[(ObjId, ObjId, i64, RequestUid)],
) -> ActionWindowRecord {
    let window = ActionWindow::new(ActionWindowDraft {
        id: tactical_gate_volley_window_id(),
        clock: tactical_gate_clock(),
        opens: tactical_gate_tick(0),
        closes: tactical_gate_tick(10),
        generation: 1,
        mode: WindowMode::CollectSimultaneous,
        capacity: u32::try_from(submissions.len().max(1)).expect("fixture volley capacity"),
        restart: WindowRestartPolicy::Resume,
        timeout: WindowTimeoutPolicy::Close,
        max_catch_up: 1,
        resolution_secret_version: Some(1),
    })
    .expect("valid simultaneous fire window");
    let mut record = ActionWindowRecord::open(window.clone());
    for (source, target, order, request) in submissions {
        let invocation = tactical_gate_fire_invocation(*source, *target, 0, 0, *request);
        let intent = WindowIntent::new(
            window.id(),
            window.generation(),
            SubjectKey::Object { id: *source },
            vec![SubjectKey::Object { id: *target }],
            definitions.submit_fire.clone(),
            invocation,
            WindowOrderKey::new(vec![*order]).expect("fixture order key"),
        )
        .expect("valid simultaneous fire intent");
        record = record
            .checked_submit(record.revision(), &tactical_gate_tick(7), intent)
            .expect("simultaneous intent is admitted")
            .into_record();
    }
    record = record
        .checked_close_due(
            record.revision(),
            RequestUid::from_bytes([0xa0; 16]),
            &tactical_gate_tick(10),
        )
        .expect("simultaneous fire window closes")
        .into_record();
    let id = record.window().id();
    let prepared = {
        let mut txn = engine
            .action_transaction(&state.windows)
            .expect("volley window bootstrap transaction");
        txn.insert(&state.windows, &id, record.clone())
            .expect("valid closed volley bootstrap");
        txn.prepare().expect("closed volley bootstrap prepares")
    };
    let (_, installation) = prepared.into_parts();
    engine
        .install_prepared_action(installation)
        .expect("the fixture installs");
    record
}

/// Exact domain clock routed through the fixture's composed provider.
#[must_use]
pub fn tactical_gate_clock() -> ClockRef {
    ClockRef::DomainTick {
        owner: OwnerId::new(TACTICAL_GATE_OWNER).expect("static owner"),
        id: CanonicalId::new("arena").expect("static domain id"),
        generation: 1,
    }
}

/// Stable fire-window identity for the fixture owner.
#[must_use]
pub fn tactical_gate_window_id() -> ActionWindowId {
    ActionWindowId::derive(
        &OwnerId::new(TACTICAL_GATE_OWNER).expect("static owner"),
        &CanonicalId::new("fire_window").expect("static window id"),
    )
    .expect("static Gate-3 window identity")
}

/// Stable simultaneous-fire window identity for the fixture owner.
#[must_use]
pub fn tactical_gate_volley_window_id() -> ActionWindowId {
    ActionWindowId::derive(
        &OwnerId::new(TACTICAL_GATE_OWNER).expect("static owner"),
        &CanonicalId::new("volley_window").expect("static window id"),
    )
    .expect("static simultaneous window identity")
}

/// Canonical domain clock used by the fixture.
#[must_use]
pub fn tactical_gate_tick(at: i64) -> ClockStamp {
    ClockStamp {
        clock: tactical_gate_clock(),
        at,
    }
}

/// Construct one direct Gate-2 invocation. Later Gate-3 work replaces the
/// known target with an opaque contact while retaining this provider contract.
#[must_use]
pub fn tactical_gate_fire_invocation(
    source: ObjId,
    target: ObjId,
    source_revision: u64,
    target_revision: u64,
    request: RequestUid,
) -> ActionInvocation {
    let definitions = definition_fixture()
        .expect("static Gate-2 definitions")
        .definitions;
    let principal = PrincipalId::Object { id: source };
    ActionInvocation::new(ActionInvocationDraft {
        request,
        principal: principal.clone(),
        actor: source,
        offered_action: ActionOfferId(
            CanonicalHash::for_canonical_value(
                "toytactics.offer.v1",
                1,
                &json!({ "action": definitions.fire }),
                4 * 1024,
            )
            .expect("Gate-2 offer identity is canonical"),
        ),
        subject: PerceivedRef::SelfActor,
        targets: vec![PerceivedRef::KnownObject {
            id: target,
            revision: target_revision,
        }],
        parameters: json!({
            "source_revision": source_revision,
            "target_revision": target_revision,
            "now": tactical_gate_tick(7),
        }),
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal,
                owner: OwnerId::new(TACTICAL_GATE_OWNER).expect("static owner"),
                namespace: CanonicalId::new(FIRE_ACTION_ID).expect("static action namespace"),
            },
            1,
            IdempotencyKey::Request(request),
        ),
    })
    .expect("Gate-2 invocation is canonical")
}

/// Construct the internal action that consumes a closed simultaneous window.
#[must_use]
pub fn tactical_gate_resolve_volley_invocation(
    definitions: &TacticalGateDefinitions,
    expected_revision: u64,
    request: RequestUid,
) -> ActionInvocation {
    let principal = PrincipalId::Object { id: mush_core::GOD };
    ActionInvocation::new(ActionInvocationDraft {
        request,
        principal: principal.clone(),
        actor: mush_core::GOD,
        offered_action: ActionOfferId(
            CanonicalHash::for_canonical_value(
                "toytactics.internal_offer.v1",
                1,
                &json!({ "action": definitions.resolve_volley }),
                4 * 1024,
            )
            .expect("volley resolver identity is canonical"),
        ),
        subject: PerceivedRef::SelfActor,
        targets: Vec::new(),
        parameters: json!({
            "expected_revision": expected_revision,
            "now": tactical_gate_tick(10),
        }),
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal,
                owner: OwnerId::new(TACTICAL_GATE_OWNER).expect("static owner"),
                namespace: CanonicalId::new(RESOLVE_VOLLEY_ACTION_ID)
                    .expect("static action namespace"),
            },
            1,
            IdempotencyKey::Request(request),
        ),
    })
    .expect("volley resolution invocation is canonical")
}

/// Fixed commit token for provider-level Gate-2 tests.
#[must_use]
pub const fn tactical_gate_token() -> CommitToken {
    CommitToken::from_bytes([0x26; 16])
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::{
        ActionProviderPrepareError, AuthorityGuard, LegalActionResolutionError,
        OfferExecutionContext, OfferResolveError, OfferSubmission,
    };

    fn unit(spec: &UnitSpec, id: u32) -> TacticalUnit {
        TacticalUnit::from_spec(spec, ObjId(id)).expect("valid Gate-2 unit")
    }

    fn spawn_unit_player(
        engine: &mut Engine,
        state: &TacticalGateState,
        spec: &UnitSpec,
        name: &str,
    ) -> ObjId {
        let id = engine
            .world
            .create(
                mush_core::ObjKind::Player,
                name,
                mush_core::GOD,
                Some(mush_core::LIMBO),
            )
            .expect("Gate-3 fixture actor");
        install_tactical_gate_unit(engine, state, unit(spec, id.0));
        install_tactical_gate_placement(engine, state, id, 0, 0);
        id
    }

    fn grant_weapons(
        engine: &mut Engine,
        state: &TacticalGateState,
        actor: ObjId,
        subject: ObjId,
    ) -> ControlLeaseState {
        let holder = PrincipalId::Object { id: actor };
        let lease = install_tactical_gate_weapons_lease(engine, state, subject, holder);
        publish_tactical_gate_weapons_station(engine, actor, subject, lease.epoch());
        lease
    }

    #[test]
    fn fire_prepares_source_target_and_resolution_before_install() {
        let (mut engine, state, definitions, spec) = tactical_gate_world();
        let source_id = ObjId(901);
        let target_id = ObjId(902);
        install_tactical_gate_unit(&mut engine, &state, unit(&spec, source_id.0));
        install_tactical_gate_unit(&mut engine, &state, unit(&spec, target_id.0));
        let invocation = tactical_gate_fire_invocation(
            source_id,
            target_id,
            0,
            0,
            RequestUid::from_bytes([0x31; 16]),
        );

        let source_before = engine
            .action_state()
            .canonical(&state.units, &source_id)
            .unwrap()
            .unwrap()
            .as_slice()
            .to_vec();
        let target_before = engine
            .action_state()
            .canonical(&state.units, &target_id)
            .unwrap()
            .unwrap()
            .as_slice()
            .to_vec();
        let (prepared, output) = engine
            .prepare_registered_authoritative_action(
                &definitions.fire,
                &invocation,
                tactical_gate_token(),
            )
            .expect("Gate-2 fire prepares");
        assert_eq!(prepared.operation_count(), 2);
        let (outcome, resolution, _, _, _, _) = output.into_parts();
        assert!(resolution.is_some());
        assert_eq!(
            engine
                .action_state()
                .canonical(&state.units, &source_id)
                .unwrap()
                .unwrap()
                .as_slice(),
            source_before
        );
        assert_eq!(
            engine
                .action_state()
                .canonical(&state.units, &target_id)
                .unwrap()
                .unwrap()
                .as_slice(),
            target_before
        );

        let receipt = engine
            .registry
            .action_providers()
            .unwrap()
            .receipt_projection_table()
            .project(
                &definitions.fire,
                &outcome,
                &PrincipalId::Object { id: source_id },
            )
            .unwrap()
            .unwrap();
        let receipt_json = serde_json::to_value(receipt).unwrap();
        for forbidden in [
            "source",
            "target",
            "source_revision",
            "target_revision",
            "section",
            "damage",
            "to_hit",
            "901",
            "902",
        ] {
            assert!(
                !receipt_json.to_string().contains(forbidden),
                "projected fire receipt leaked {forbidden}"
            );
        }

        let (_, installation) = prepared.into_parts();
        engine
            .install_prepared_action(installation)
            .expect("the fixture installs");
        let source = engine
            .action_state()
            .get(&state.units, &source_id)
            .unwrap()
            .unwrap();
        let target = engine
            .action_state()
            .get(&state.units, &target_id)
            .unwrap()
            .unwrap();
        assert_eq!(source.revision(), 1);
        assert_eq!(target.revision(), 1);
        assert_eq!(
            source
                .cell(&CellAddr::Component {
                    component: ComponentId::new(WEAPON_COMPONENT_ID).unwrap(),
                    cell: CellId::new(AMMO_CELL_ID).unwrap(),
                })
                .unwrap()
                .current,
            1
        );
        assert_eq!(
            source
                .cell(&CellAddr::Unit {
                    cell: CellId::new(HEAT_CELL_ID).unwrap(),
                })
                .unwrap()
                .current,
            1
        );
        assert!(target.cells().any(|(address, cell)| {
            matches!(address, CellAddr::Section { .. }) && cell.current < cell.max
        }));
    }

    #[test]
    fn stale_revision_or_disabled_component_leaves_both_units_unchanged() {
        for disabled in [false, true] {
            let (mut engine, state, definitions, spec) = tactical_gate_world();
            let source_id = ObjId(911);
            let target_id = ObjId(912);
            let mut source = unit(&spec, source_id.0);
            if disabled {
                let plan = UnitMutationPlan::new(
                    vec![UnitMutationOp::SetComponentCondition {
                        component: ComponentId::new(WEAPON_COMPONENT_ID).unwrap(),
                        condition: ConditionId::new(DISABLED_CONDITION_ID).unwrap(),
                    }],
                    Some(0),
                    None,
                )
                .unwrap();
                source = apply(&source, &plan, 1).unwrap().0;
            }
            let source_revision = source.revision();
            install_tactical_gate_unit(&mut engine, &state, source);
            install_tactical_gate_unit(&mut engine, &state, unit(&spec, target_id.0));
            let source_before = engine
                .action_state()
                .canonical(&state.units, &source_id)
                .unwrap()
                .unwrap()
                .as_slice()
                .to_vec();
            let target_before = engine
                .action_state()
                .canonical(&state.units, &target_id)
                .unwrap()
                .unwrap()
                .as_slice()
                .to_vec();
            let invocation = tactical_gate_fire_invocation(
                source_id,
                target_id,
                if disabled {
                    source_revision
                } else {
                    source_revision + 1
                },
                0,
                RequestUid::from_bytes([0x41 + u8::from(disabled); 16]),
            );
            assert!(matches!(
                engine.prepare_registered_authoritative_action(
                    &definitions.fire,
                    &invocation,
                    tactical_gate_token(),
                ),
                Err(ActionProviderPrepareError::Provider { source, .. })
                    if source.code() == ActionErrorCode::Stale
            ));
            assert_eq!(
                engine
                    .action_state()
                    .canonical(&state.units, &source_id)
                    .unwrap()
                    .unwrap()
                    .as_slice(),
                source_before
            );
            assert_eq!(
                engine
                    .action_state()
                    .canonical(&state.units, &target_id)
                    .unwrap()
                    .unwrap()
                    .as_slice(),
                target_before
            );
            assert!(!engine.action_state().has_prepared());
        }
    }

    #[test]
    fn unavailable_target_cannot_stage_the_source_ammo_and_heat_spend() {
        let (mut engine, state, definitions, spec) = tactical_gate_world();
        let source_id = ObjId(921);
        let target_id = ObjId(922);
        let source = unit(&spec, source_id.0);
        let target = unit(&spec, target_id.0);
        let target_plan = UnitMutationPlan::new(
            SECTION_IDS
                .iter()
                .map(|section| UnitMutationOp::SetCell {
                    addr: CellAddr::Section {
                        section: SectionId::new(section).unwrap(),
                        cell: CellId::new(STRUCTURE_CELL_ID).unwrap(),
                    },
                    value: 0,
                })
                .collect(),
            Some(0),
            None,
        )
        .unwrap();
        let target = apply(&target, &target_plan, 1).unwrap().0;
        install_tactical_gate_unit(&mut engine, &state, source);
        install_tactical_gate_unit(&mut engine, &state, target);
        let source_before = engine
            .action_state()
            .canonical(&state.units, &source_id)
            .unwrap()
            .unwrap()
            .as_slice()
            .to_vec();
        let target_before = engine
            .action_state()
            .canonical(&state.units, &target_id)
            .unwrap()
            .unwrap()
            .as_slice()
            .to_vec();
        let invocation = tactical_gate_fire_invocation(
            source_id,
            target_id,
            0,
            1,
            RequestUid::from_bytes([0x51; 16]),
        );
        assert!(matches!(
            engine.prepare_registered_authoritative_action(
                &definitions.fire,
                &invocation,
                tactical_gate_token(),
            ),
            Err(ActionProviderPrepareError::Provider { source, .. })
                if source.code() == ActionErrorCode::Stale
        ));
        assert_eq!(
            engine
                .action_state()
                .canonical(&state.units, &source_id)
                .unwrap()
                .unwrap()
                .as_slice(),
            source_before
        );
        assert_eq!(
            engine
                .action_state()
                .canonical(&state.units, &target_id)
                .unwrap()
                .unwrap()
                .as_slice(),
            target_before
        );
        assert!(!engine.action_state().has_prepared());
    }

    #[test]
    fn identical_fire_inputs_reproduce_writes_outcome_and_rng_evidence() {
        fn prepare_once() -> (
            Vec<mush_core::ReplayWrite>,
            RulesValue,
            mush_core::PreparedResolution,
        ) {
            let (mut engine, state, definitions, spec) = tactical_gate_world();
            let source_id = ObjId(931);
            let target_id = ObjId(932);
            install_tactical_gate_unit(&mut engine, &state, unit(&spec, source_id.0));
            install_tactical_gate_unit(&mut engine, &state, unit(&spec, target_id.0));
            let invocation = tactical_gate_fire_invocation(
                source_id,
                target_id,
                0,
                0,
                RequestUid::from_bytes([0x61; 16]),
            );
            let (prepared, output) = engine
                .prepare_registered_authoritative_action(
                    &definitions.fire,
                    &invocation,
                    tactical_gate_token(),
                )
                .unwrap();
            let writes = prepared.authoritative_replay_writes();
            let (outcome, resolution, _, _, _, _) = output.into_parts();
            engine.abort_prepared_transaction(prepared);
            (writes, outcome, resolution.unwrap())
        }

        assert_eq!(prepare_once(), prepare_once());
    }

    #[test]
    fn closed_simultaneous_fire_resolves_and_consumes_in_one_prepared_bundle() {
        let (mut engine, state, definitions, spec) = tactical_gate_world();
        let first = ObjId(921);
        let second = ObjId(922);
        let target = ObjId(923);
        for id in [first, second, target] {
            install_tactical_gate_unit(&mut engine, &state, unit(&spec, id.0));
        }
        let closed = install_tactical_gate_closed_volley(
            &mut engine,
            &state,
            &definitions,
            &[
                (first, target, 20, RequestUid::from_bytes([0xb1; 16])),
                (second, target, 10, RequestUid::from_bytes([0xb2; 16])),
            ],
        );
        assert_eq!(closed.phase(), ActionWindowPhase::Closed);
        assert_eq!(closed.intents().len(), 2);
        assert!(closed.intents().iter().all(|intent| {
            intent.action() == &definitions.submit_fire
                && intent.truth_targets() == [SubjectKey::Object { id: target }]
        }));
        let window_id = tactical_gate_volley_window_id();
        let unit_before: BTreeMap<_, _> = [first, second, target]
            .into_iter()
            .map(|id| {
                (
                    id,
                    engine
                        .action_state()
                        .canonical(&state.units, &id)
                        .unwrap()
                        .unwrap()
                        .as_slice()
                        .to_vec(),
                )
            })
            .collect();
        let window_before = engine
            .action_state()
            .canonical(&state.windows, &window_id)
            .unwrap()
            .unwrap()
            .as_slice()
            .to_vec();
        let request = RequestUid::from_bytes([0xb3; 16]);
        let invocation =
            tactical_gate_resolve_volley_invocation(&definitions, closed.revision(), request);
        let token = CommitToken::from_bytes([0xb4; 16]);
        let (prepared, output) = engine
            .prepare_registered_authoritative_action(
                &definitions.resolve_volley,
                &invocation,
                token,
            )
            .expect("closed simultaneous fire resolves");
        assert_eq!(prepared.operation_count(), 4);
        for (id, before) in &unit_before {
            assert_eq!(
                engine
                    .action_state()
                    .canonical(&state.units, id)
                    .unwrap()
                    .unwrap()
                    .as_slice(),
                before
            );
        }
        assert_eq!(
            engine
                .action_state()
                .canonical(&state.windows, &window_id)
                .unwrap()
                .unwrap()
                .as_slice(),
            window_before
        );
        let (outcome, resolution, _, _, _, _) = output.into_parts();
        let decoded: VolleyResolvedOutcome =
            serde_json::from_value(outcome.data().clone()).unwrap();
        assert_eq!(decoded.shots.len(), 2);
        assert_eq!(decoded.shots[0].source, second);
        assert_eq!(decoded.shots[1].source, first);
        assert!(resolution.is_some());

        let (_, installation) = prepared.into_parts();
        engine
            .install_prepared_action(installation)
            .expect("the fixture installs");
        assert_eq!(
            engine
                .action_state()
                .get(&state.units, &first)
                .unwrap()
                .unwrap()
                .revision(),
            1
        );
        assert_eq!(
            engine
                .action_state()
                .get(&state.units, &second)
                .unwrap()
                .unwrap()
                .revision(),
            1
        );
        assert_eq!(
            engine
                .action_state()
                .get(&state.units, &target)
                .unwrap()
                .unwrap()
                .revision(),
            2
        );
        assert_eq!(
            engine
                .action_state()
                .get(&state.windows, &window_id)
                .unwrap()
                .unwrap()
                .phase(),
            ActionWindowPhase::Consumed { commit: token }
        );
    }

    #[test]
    fn one_stale_simultaneous_intent_leaves_the_entire_closed_set_unconsumed() {
        let (mut engine, state, definitions, spec) = tactical_gate_world();
        let first = ObjId(931);
        let stale_source = ObjId(932);
        let target = ObjId(933);
        for id in [first, stale_source, target] {
            install_tactical_gate_unit(&mut engine, &state, unit(&spec, id.0));
        }
        let closed = install_tactical_gate_closed_volley(
            &mut engine,
            &state,
            &definitions,
            &[
                (first, target, 1, RequestUid::from_bytes([0xc1; 16])),
                (stale_source, target, 2, RequestUid::from_bytes([0xc2; 16])),
            ],
        );
        let stale_pre = engine
            .action_state()
            .get(&state.units, &stale_source)
            .unwrap()
            .unwrap();
        let (stale_post, _) = apply(
            stale_pre,
            &UnitMutationPlan::new(
                vec![UnitMutationOp::AdjustCell {
                    addr: CellAddr::Unit {
                        cell: CellId::new(HEAT_CELL_ID).unwrap(),
                    },
                    delta: 1,
                }],
                Some(stale_pre.revision()),
                None,
            )
            .unwrap(),
            8,
        )
        .unwrap();
        let prepared = {
            let mut txn = engine.action_transaction(&state.units).unwrap();
            txn.replace(&state.units, &stale_source, stale_post)
                .unwrap();
            txn.prepare().unwrap()
        };
        let (_, installation) = prepared.into_parts();
        engine
            .install_prepared_action(installation)
            .expect("the fixture installs");

        let unit_before: BTreeMap<_, _> = [first, stale_source, target]
            .into_iter()
            .map(|id| {
                (
                    id,
                    engine
                        .action_state()
                        .canonical(&state.units, &id)
                        .unwrap()
                        .unwrap()
                        .as_slice()
                        .to_vec(),
                )
            })
            .collect();
        let window_id = tactical_gate_volley_window_id();
        let window_before = engine
            .action_state()
            .canonical(&state.windows, &window_id)
            .unwrap()
            .unwrap()
            .as_slice()
            .to_vec();
        let invocation = tactical_gate_resolve_volley_invocation(
            &definitions,
            closed.revision(),
            RequestUid::from_bytes([0xc3; 16]),
        );
        assert!(
            engine
                .prepare_registered_authoritative_action(
                    &definitions.resolve_volley,
                    &invocation,
                    CommitToken::from_bytes([0xc4; 16]),
                )
                .is_err()
        );
        assert!(!engine.action_state().has_prepared());
        for (id, before) in &unit_before {
            assert_eq!(
                engine
                    .action_state()
                    .canonical(&state.units, id)
                    .unwrap()
                    .unwrap()
                    .as_slice(),
                before
            );
        }
        assert_eq!(
            engine
                .action_state()
                .canonical(&state.windows, &window_id)
                .unwrap()
                .unwrap()
                .as_slice(),
            window_before
        );
        assert_eq!(
            engine
                .action_state()
                .get(&state.windows, &window_id)
                .unwrap()
                .unwrap()
                .phase(),
            ActionWindowPhase::Closed
        );
    }

    #[test]
    fn live_simultaneous_submissions_replace_slots_then_resolve_the_frozen_set() {
        let (mut engine, state, definitions, spec) = tactical_gate_world();
        let first = spawn_unit_player(&mut engine, &state, &spec, "First Volley Shooter");
        let second = spawn_unit_player(&mut engine, &state, &spec, "Second Volley Shooter");
        let target = ObjId(934_567_890);
        install_tactical_gate_unit(&mut engine, &state, unit(&spec, target.0));
        install_tactical_gate_placement(&mut engine, &state, target, 1, 0);
        grant_weapons(&mut engine, &state, first, first);
        grant_weapons(&mut engine, &state, second, second);
        let first_contact = ContactId::from_bytes([0xd1; 16]);
        let second_contact = ContactId::from_bytes([0xd2; 16]);
        install_tactical_gate_contact(
            &mut engine,
            &state,
            first_contact,
            target,
            PrincipalId::Object { id: first },
        );
        install_tactical_gate_contact(
            &mut engine,
            &state,
            second_contact,
            target,
            PrincipalId::Object { id: second },
        );
        install_tactical_gate_volley_window(&mut engine, &state, 2);

        let unit_before: BTreeMap<_, _> = [first, second, target]
            .into_iter()
            .map(|id| {
                (
                    id,
                    engine
                        .action_state()
                        .canonical(&state.units, &id)
                        .unwrap()
                        .unwrap()
                        .as_slice()
                        .to_vec(),
                )
            })
            .collect();
        let submissions = [
            (
                first,
                first_contact,
                RequestUid::from_bytes([0xd3; 16]),
                CommitToken::from_bytes([0xd4; 16]),
                1,
            ),
            // A different actor occupies the final capacity slot.
            (
                second,
                second_contact,
                RequestUid::from_bytes([0xd5; 16]),
                CommitToken::from_bytes([0xd6; 16]),
                2,
            ),
            // A member may replace its existing slot even when capacity is
            // full; replacement cannot crowd out the other accepted actor.
            (
                first,
                first_contact,
                RequestUid::from_bytes([0xd7; 16]),
                CommitToken::from_bytes([0xd8; 16]),
                2,
            ),
        ];
        for (actor, contact, request, token, expected_intents) in submissions {
            let principal = PrincipalId::Object { id: actor };
            let query = LegalActionQuery::new(
                principal.clone(),
                actor,
                SubjectKey::Object { id: actor },
                PerceivedRef::SelfActor,
                1,
                Some(tactical_gate_tick(7)),
            )
            .unwrap();
            let offers = engine.project_legal_actions(&query).unwrap();
            assert_eq!(offers.projected().len(), 1);
            let projected = offers.projected().pop().unwrap();
            assert_eq!(projected.presentation().verb, SUBMIT_FIRE_ACTION_ID);
            let ProjectedTargetSchema::Choices { choices, .. } = projected.targets() else {
                panic!("simultaneous fire must retain opaque target choices");
            };
            let selected = choices
                .iter()
                .find(|choice| matches!(choice, PerceivedRef::Contact { id, .. } if id == &contact))
                .expect("the actor's private contact is projected")
                .clone();
            let resolved = engine
                .resolve_legal_action(
                    &offers,
                    OfferExecutionContext {
                        request,
                        principal: principal.clone(),
                        actor,
                        idempotency: ScopedIdempotencyKey::new(
                            IdempotencyScope {
                                principal: principal.clone(),
                                owner: OwnerId::new(TACTICAL_GATE_OWNER).unwrap(),
                                namespace: CanonicalId::new(SUBMIT_FIRE_ACTION_ID).unwrap(),
                            },
                            1,
                            IdempotencyKey::Request(request),
                        ),
                        viewer_scope: query.viewer_scope(),
                        now: None,
                    },
                    OfferSubmission {
                        offer: projected.offer(),
                        subject: PerceivedRef::SelfActor,
                        targets: vec![selected],
                        parameters: json!({}),
                    },
                )
                .expect("simultaneous fire offer resolves privately");
            assert_eq!(resolved.action(), &definitions.submit_fire);
            assert!(
                resolved
                    .guards()
                    .authority()
                    .contains(&AuthorityGuard::Definition(
                        definitions.submit_placement_bridge.clone()
                    ))
            );
            for guarded in [actor, target] {
                assert!(
                    resolved
                        .guards()
                        .revisions()
                        .iter()
                        .any(|(key, generation)| {
                            key.owner.as_str() == TACTICAL_PLACEMENT_OWNER
                                && key.subject == SubjectKey::Object { id: guarded }
                                && *generation == 1
                        })
                );
            }

            let window_id = tactical_gate_volley_window_id();
            let window_before = engine
                .action_state()
                .canonical(&state.windows, &window_id)
                .unwrap()
                .unwrap()
                .as_slice()
                .to_vec();
            let (prepared, output) = engine
                .prepare_registered_resolved_authoritative_action(
                    &definitions.submit_fire,
                    resolved.resolved_invocation(),
                    token,
                )
                .expect("simultaneous fire submission prepares");
            assert_eq!(prepared.operation_count(), 1);
            let (outcome, resolution, _, _, _, _) = output.into_parts();
            assert!(resolution.is_none());
            for (id, before) in &unit_before {
                assert_eq!(
                    engine
                        .action_state()
                        .canonical(&state.units, id)
                        .unwrap()
                        .unwrap()
                        .as_slice(),
                    before
                );
            }
            assert_eq!(
                engine
                    .action_state()
                    .canonical(&state.windows, &window_id)
                    .unwrap()
                    .unwrap()
                    .as_slice(),
                window_before
            );
            let receipt = engine
                .registry
                .action_providers()
                .unwrap()
                .receipt_projection_table()
                .project(&definitions.submit_fire, &outcome, &principal)
                .unwrap()
                .unwrap();
            let receipt_json = serde_json::to_string(&receipt).unwrap();
            for forbidden in [
                actor.0.to_string(),
                target.0.to_string(),
                contact.to_hex(),
                tactical_gate_volley_window_id().0.to_hex(),
            ] {
                assert!(
                    !receipt_json.contains(&forbidden),
                    "submission receipt leaked private identifier {forbidden}"
                );
            }
            let (_, installation) = prepared.into_parts();
            engine
                .install_prepared_action(installation)
                .expect("the fixture installs");
            let window = engine
                .action_state()
                .get(&state.windows, &window_id)
                .unwrap()
                .unwrap();
            assert_eq!(window.intents().len(), expected_intents);
            assert!(
                window
                    .intents()
                    .iter()
                    .all(|intent| intent.action() == &definitions.submit_fire)
            );
            if actor == first {
                let intent = window
                    .intents()
                    .iter()
                    .find(|intent| intent.invocation().actor() == first)
                    .unwrap();
                assert_eq!(intent.invocation().request(), request);
            }
        }

        let window_id = tactical_gate_volley_window_id();
        let open = engine
            .action_state()
            .get(&state.windows, &window_id)
            .unwrap()
            .unwrap()
            .clone();
        let closed = open
            .checked_close_due(
                open.revision(),
                RequestUid::from_bytes([0xd9; 16]),
                &tactical_gate_tick(10),
            )
            .unwrap()
            .into_record();
        let prepared = {
            let mut txn = engine.action_transaction(&state.windows).unwrap();
            txn.replace(&state.windows, &window_id, closed.clone())
                .unwrap();
            txn.prepare().unwrap()
        };
        let (_, installation) = prepared.into_parts();
        engine
            .install_prepared_action(installation)
            .expect("the fixture installs");

        let resolve = tactical_gate_resolve_volley_invocation(
            &definitions,
            closed.revision(),
            RequestUid::from_bytes([0xda; 16]),
        );
        let resolution_token = CommitToken::from_bytes([0xdb; 16]);
        let (prepared, output) = engine
            .prepare_registered_authoritative_action(
                &definitions.resolve_volley,
                &resolve,
                resolution_token,
            )
            .expect("the frozen live submission set resolves");
        assert_eq!(prepared.operation_count(), 4);
        assert!(output.into_parts().1.is_some());
        let (_, installation) = prepared.into_parts();
        engine
            .install_prepared_action(installation)
            .expect("the fixture installs");
        assert_eq!(
            engine
                .action_state()
                .get(&state.units, &first)
                .unwrap()
                .unwrap()
                .revision(),
            1
        );
        assert_eq!(
            engine
                .action_state()
                .get(&state.units, &second)
                .unwrap()
                .unwrap()
                .revision(),
            1
        );
        assert_eq!(
            engine
                .action_state()
                .get(&state.units, &target)
                .unwrap()
                .unwrap()
                .revision(),
            2
        );
        assert_eq!(
            engine
                .action_state()
                .get(&state.windows, &window_id)
                .unwrap()
                .unwrap()
                .phase(),
            ActionWindowPhase::Consumed {
                commit: resolution_token
            }
        );
    }

    #[test]
    fn operated_fire_requires_one_exact_station_and_active_weapons_lease() {
        let (mut engine, state, definitions, spec) = tactical_gate_world();
        let actor = engine
            .world
            .create(
                mush_core::ObjKind::Player,
                "Vehicle Gunner",
                mush_core::GOD,
                Some(mush_core::LIMBO),
            )
            .unwrap();
        let source = engine
            .world
            .create(
                mush_core::ObjKind::Thing,
                "Independent Vehicle",
                actor,
                Some(mush_core::LIMBO),
            )
            .unwrap();
        let target = ObjId(876_543_210);
        install_tactical_gate_unit(&mut engine, &state, unit(&spec, source.0));
        install_tactical_gate_unit(&mut engine, &state, unit(&spec, target.0));
        install_tactical_gate_placement(&mut engine, &state, source, 0, 0);
        install_tactical_gate_placement(&mut engine, &state, target, 1, 0);
        let principal = PrincipalId::Object { id: actor };
        let contact_id = ContactId::from_bytes([0x83; 16]);
        install_tactical_gate_contact(&mut engine, &state, contact_id, target, principal.clone());
        install_tactical_gate_fire_window(&mut engine, &state);
        let query = LegalActionQuery::new(
            principal.clone(),
            actor,
            SubjectKey::Object { id: source },
            PerceivedRef::KnownObject {
                id: source,
                revision: 1,
            },
            1,
            Some(tactical_gate_tick(7)),
        )
        .unwrap();

        // Neither half is authority by itself.
        assert!(engine.project_legal_actions(&query).unwrap().is_empty());
        publish_tactical_gate_weapons_station(&mut engine, actor, source, 1);
        assert!(engine.project_legal_actions(&query).unwrap().is_empty());

        let lease =
            install_tactical_gate_weapons_lease(&mut engine, &state, source, principal.clone());
        assert_eq!(lease.epoch(), 1);
        let offers = engine.project_legal_actions(&query).unwrap();
        let projected = offers
            .projected()
            .pop()
            .expect("complete authority offers fire");
        assert_eq!(
            projected.required_channels(),
            &[ChannelId::new("weapons").unwrap()]
        );
        let ProjectedTargetSchema::Choices { choices, .. } = projected.targets() else {
            panic!("operated fire must retain opaque contact choices");
        };
        let request = RequestUid::from_bytes([0x84; 16]);
        let resolved = engine
            .resolve_legal_action(
                &offers,
                OfferExecutionContext {
                    request,
                    principal: principal.clone(),
                    actor,
                    idempotency: ScopedIdempotencyKey::new(
                        IdempotencyScope {
                            principal: principal.clone(),
                            owner: OwnerId::new(TACTICAL_GATE_OWNER).unwrap(),
                            namespace: CanonicalId::new(FIRE_ACTION_ID).unwrap(),
                        },
                        1,
                        IdempotencyKey::Request(request),
                    ),
                    viewer_scope: query.viewer_scope(),
                    now: None,
                },
                OfferSubmission {
                    offer: projected.offer(),
                    subject: PerceivedRef::KnownObject {
                        id: source,
                        revision: 1,
                    },
                    targets: vec![choices[0].clone()],
                    parameters: json!({}),
                },
            )
            .expect("complete control authority resolves");
        assert_eq!(
            resolved.resolved_invocation().truth_subject(),
            &SubjectKey::Object { id: source }
        );
        assert!(resolved.guards().authority().iter().any(|guard| {
            matches!(
                guard,
                AuthorityGuard::ChannelGrant { subject, channel, .. }
                    if *subject == source && channel.as_str() == "weapons"
            )
        }));
        assert!(resolved.guards().authority().iter().any(|guard| {
            matches!(
                guard,
                AuthorityGuard::ControlLease { subject, holder, channel, epoch, .. }
                    if *subject == source
                        && holder == &principal
                        && channel.as_str() == "weapons"
                        && *epoch == 1
            )
        }));
        assert!(resolved.guards().authority().iter().any(|guard| {
            matches!(
                guard,
                AuthorityGuard::Window { owner, id, generation }
                    if owner.as_str() == TACTICAL_GATE_OWNER
                        && *id == tactical_gate_window_id()
                        && *generation == 1
            )
        }));

        let (prepared, output) = engine
            .prepare_registered_resolved_authoritative_action(
                &definitions.fire,
                resolved.resolved_invocation(),
                tactical_gate_token(),
            )
            .expect("the pilot operates the independent vehicle");
        assert_eq!(output.into_parts().0.data()["source"], source.0);
        engine.abort_prepared_transaction(prepared);
        let projector = FireProvider {
            units: state.units.clone(),
            contacts: state.contacts.clone(),
            control_leases: state.control_leases.clone(),
            windows: state.windows.clone(),
            placements: state.placements.clone(),
            window: tactical_gate_window_id(),
            action: definitions.fire.clone(),
            outcome: definitions.fired.clone(),
        };
        assert!(
            projector
                .project_tactical_snapshot(&engine, &principal, source)
                .is_ok(),
            "the active lease authorizes the operated-unit projection"
        );

        // A completed safe-boundary revocation invalidates only the lease
        // witness; the transient station projection remains present but is no
        // longer sufficient to admit this already-projected action.
        let revoke_request = RequestUid::from_bytes([0x85; 16]);
        let revoked = lease
            .checked_request_handoff(
                lease.revision(),
                ControlHandoffDraft {
                    request: revoke_request,
                    requested_by: principal.clone(),
                    next_holder: None,
                    requested_at: tactical_gate_tick(8),
                    boundary: tactical_gate_tick(9),
                    reason: CanonicalId::new("fixture_revocation").unwrap(),
                },
            )
            .unwrap()
            .checked_apply_handoff(lease.revision() + 1, revoke_request, tactical_gate_tick(9))
            .unwrap();
        let lease_key = ControlLeaseKey::new(source, ChannelId::new("weapons").unwrap());
        let prepared = {
            let mut txn = engine.action_transaction(&state.control_leases).unwrap();
            txn.replace(&state.control_leases, &lease_key, revoked)
                .unwrap();
            txn.prepare().unwrap()
        };
        let (_, installation) = prepared.into_parts();
        engine
            .install_prepared_action(installation)
            .expect("the fixture installs");
        let check = resolved
            .guards()
            .revalidate(
                &engine.world,
                engine.action_state(),
                engine.registry.revision_validators().unwrap(),
                engine.registry.authority_guard_validators().unwrap(),
                engine.registry.definitions().unwrap(),
            )
            .unwrap();
        assert_eq!(check.stale_authority().len(), 1);
        assert!(matches!(
            check.stale_authority()[0],
            AuthorityGuard::ControlLease { .. }
        ));
        let error = projector
            .project_tactical_snapshot(&engine, &principal, source)
            .unwrap_err();
        assert_eq!(error.code(), ActionErrorCode::Stale);
    }

    #[test]
    fn closing_the_action_window_invalidates_a_retained_fire_offer() {
        let (mut engine, state, _definitions, spec) = tactical_gate_world();
        let source = spawn_unit_player(&mut engine, &state, &spec, "Window Shooter");
        let target = ObjId(765_432_109);
        install_tactical_gate_unit(&mut engine, &state, unit(&spec, target.0));
        install_tactical_gate_placement(&mut engine, &state, target, 1, 0);
        grant_weapons(&mut engine, &state, source, source);
        let open = install_tactical_gate_fire_window(&mut engine, &state);
        let principal = PrincipalId::Object { id: source };
        install_tactical_gate_contact(
            &mut engine,
            &state,
            ContactId::from_bytes([0x86; 16]),
            target,
            principal.clone(),
        );
        let query = LegalActionQuery::new(
            principal.clone(),
            source,
            SubjectKey::Object { id: source },
            PerceivedRef::SelfActor,
            1,
            None,
        )
        .unwrap();
        let offers = engine.project_legal_actions(&query).unwrap();
        let projected = offers.projected().pop().unwrap();
        let ProjectedTargetSchema::Choices { choices, .. } = projected.targets() else {
            panic!("open fire window must project a contact");
        };
        let request = RequestUid::from_bytes([0x87; 16]);
        let resolved = engine
            .resolve_legal_action(
                &offers,
                OfferExecutionContext {
                    request,
                    principal: principal.clone(),
                    actor: source,
                    idempotency: ScopedIdempotencyKey::new(
                        IdempotencyScope {
                            principal,
                            owner: OwnerId::new(TACTICAL_GATE_OWNER).unwrap(),
                            namespace: CanonicalId::new(FIRE_ACTION_ID).unwrap(),
                        },
                        1,
                        IdempotencyKey::Request(request),
                    ),
                    viewer_scope: query.viewer_scope(),
                    // Engine resolution must read the exact expiry clock; a
                    // caller-provided observation is not authoritative.
                    now: Some(tactical_gate_tick(-1)),
                },
                OfferSubmission {
                    offer: projected.offer(),
                    subject: PerceivedRef::SelfActor,
                    targets: vec![choices[0].clone()],
                    parameters: json!({}),
                },
            )
            .expect("engine-owned clock admits the unexpired offer");

        let closed = open
            .checked_close_due(
                open.revision(),
                RequestUid::from_bytes([0x88; 16]),
                &tactical_gate_tick(10),
            )
            .unwrap()
            .into_record();
        let id = tactical_gate_window_id();
        let prepared = {
            let mut txn = engine.action_transaction(&state.windows).unwrap();
            txn.replace(&state.windows, &id, closed).unwrap();
            txn.prepare().unwrap()
        };
        let (_, installation) = prepared.into_parts();
        engine
            .install_prepared_action(installation)
            .expect("the fixture installs");

        let check = resolved
            .guards()
            .revalidate(
                &engine.world,
                engine.action_state(),
                engine.registry.revision_validators().unwrap(),
                engine.registry.authority_guard_validators().unwrap(),
                engine.registry.definitions().unwrap(),
            )
            .unwrap();
        assert_eq!(check.stale_authority().len(), 1);
        assert!(matches!(
            check.stale_authority()[0],
            AuthorityGuard::Window { .. }
        ));
        assert!(engine.project_legal_actions(&query).unwrap().is_empty());
        assert!(!engine.action_state().has_prepared());
    }

    #[test]
    fn opaque_contact_target_resolves_privately_and_adds_a_final_guard() {
        let (mut engine, state, definitions, spec) = tactical_gate_world();
        let source_id = spawn_unit_player(&mut engine, &state, &spec, "Contact Shooter");
        let hidden_target_id = ObjId(987_654_321);
        install_tactical_gate_unit(&mut engine, &state, unit(&spec, hidden_target_id.0));
        install_tactical_gate_placement(&mut engine, &state, hidden_target_id, 1, 0);
        grant_weapons(&mut engine, &state, source_id, source_id);
        let principal = PrincipalId::Object { id: source_id };
        let contact_id = ContactId::from_bytes([0x71; 16]);
        install_tactical_gate_contact(
            &mut engine,
            &state,
            contact_id,
            hidden_target_id,
            principal.clone(),
        );
        install_tactical_gate_fire_window(&mut engine, &state);
        let query = LegalActionQuery::new(
            principal.clone(),
            source_id,
            SubjectKey::Object { id: source_id },
            PerceivedRef::SelfActor,
            1,
            Some(tactical_gate_tick(7)),
        )
        .unwrap();
        let offers = engine.project_legal_actions(&query).unwrap();
        let projected = offers.projected().pop().expect("contact enables fire");
        let ProjectedTargetSchema::Choices { choices, .. } = projected.targets() else {
            panic!("fire must project an opaque target choice");
        };
        let [contact_ref @ PerceivedRef::Contact { .. }] = choices.as_slice() else {
            panic!("fire must expose exactly one opaque contact");
        };
        let projected_json = serde_json::to_string(&projected).unwrap();
        assert!(!projected_json.contains(&hidden_target_id.0.to_string()));
        assert!(!projected_json.contains("truth_subject"));

        let request = RequestUid::from_bytes([0x72; 16]);
        let resolved = engine
            .resolve_legal_action(
                &offers,
                OfferExecutionContext {
                    request,
                    principal: principal.clone(),
                    actor: source_id,
                    idempotency: ScopedIdempotencyKey::new(
                        IdempotencyScope {
                            principal: principal.clone(),
                            owner: OwnerId::new(TACTICAL_GATE_OWNER).unwrap(),
                            namespace: CanonicalId::new(FIRE_ACTION_ID).unwrap(),
                        },
                        1,
                        IdempotencyKey::Request(request),
                    ),
                    viewer_scope: query.viewer_scope(),
                    now: None,
                },
                OfferSubmission {
                    offer: projected.offer(),
                    subject: PerceivedRef::SelfActor,
                    targets: vec![contact_ref.clone()],
                    parameters: json!({}),
                },
            )
            .expect("selected contact resolves privately");
        assert_eq!(
            resolved.resolved_invocation().truth_targets(),
            &[SubjectKey::Object {
                id: hidden_target_id,
            }]
        );
        assert!(resolved.guards().authority().iter().any(|guard| {
            matches!(
                guard,
                AuthorityGuard::Contact {
                    provider,
                    principal: guarded_principal,
                    id,
                    generation: 1,
                } if provider.as_str() == TACTICAL_GATE_OWNER
                    && guarded_principal == &principal
                    && id == &contact_id
            )
        }));
        let (prepared, output) = engine
            .prepare_registered_resolved_authoritative_action(
                &definitions.fire,
                resolved.resolved_invocation(),
                tactical_gate_token(),
            )
            .expect("the provider receives private target truth");
        let outcome = output.into_parts().0;
        assert_eq!(outcome.data()["target"], hidden_target_id.0);
        let (_, installation) = prepared.into_parts();
        engine
            .install_prepared_action(installation)
            .expect("the fixture installs");
        assert_eq!(
            engine
                .action_state()
                .get(&state.units, &hidden_target_id)
                .unwrap()
                .unwrap()
                .revision(),
            1
        );
    }

    #[test]
    fn resolved_fire_guards_only_the_source_and_selected_target_placements() {
        let (mut engine, state, definitions, spec) = tactical_gate_world();
        let source = spawn_unit_player(&mut engine, &state, &spec, "Selective Shooter");
        let selected_target = ObjId(941);
        let unselected_target = ObjId(942);
        for (target, x) in [(selected_target, 1), (unselected_target, 2)] {
            install_tactical_gate_unit(&mut engine, &state, unit(&spec, target.0));
            install_tactical_gate_placement(&mut engine, &state, target, x, 0);
        }
        grant_weapons(&mut engine, &state, source, source);
        let principal = PrincipalId::Object { id: source };
        let selected_contact = ContactId::from_bytes([0x91; 16]);
        install_tactical_gate_contact(
            &mut engine,
            &state,
            selected_contact,
            selected_target,
            principal.clone(),
        );
        install_tactical_gate_contact(
            &mut engine,
            &state,
            ContactId::from_bytes([0x92; 16]),
            unselected_target,
            principal.clone(),
        );
        install_tactical_gate_fire_window(&mut engine, &state);
        let query = LegalActionQuery::new(
            principal.clone(),
            source,
            SubjectKey::Object { id: source },
            PerceivedRef::SelfActor,
            1,
            Some(tactical_gate_tick(7)),
        )
        .unwrap();
        let offers = engine.project_legal_actions(&query).unwrap();
        let projected = offers.projected().pop().expect("both contacts are legal");
        let ProjectedTargetSchema::Choices { choices, .. } = projected.targets() else {
            panic!("fire must project opaque contacts");
        };
        let selected = choices
            .iter()
            .find(|choice| {
                matches!(choice, PerceivedRef::Contact { id, .. } if id == &selected_contact)
            })
            .expect("selected contact is projected")
            .clone();
        let request = RequestUid::from_bytes([0x93; 16]);
        let resolved = engine
            .resolve_legal_action(
                &offers,
                OfferExecutionContext {
                    request,
                    principal: principal.clone(),
                    actor: source,
                    idempotency: ScopedIdempotencyKey::new(
                        IdempotencyScope {
                            principal,
                            owner: OwnerId::new(TACTICAL_GATE_OWNER).unwrap(),
                            namespace: CanonicalId::new(FIRE_ACTION_ID).unwrap(),
                        },
                        1,
                        IdempotencyKey::Request(request),
                    ),
                    viewer_scope: query.viewer_scope(),
                    now: None,
                },
                OfferSubmission {
                    offer: projected.offer(),
                    subject: PerceivedRef::SelfActor,
                    targets: vec![selected],
                    parameters: json!({}),
                },
            )
            .expect("selected contact resolves");

        let guarded_subjects: Vec<_> = resolved
            .guards()
            .revisions()
            .iter()
            .map(|(key, generation)| (key.subject.clone(), *generation))
            .collect();
        assert_eq!(
            guarded_subjects,
            vec![
                (SubjectKey::Object { id: source }, 1),
                (
                    SubjectKey::Object {
                        id: selected_target,
                    },
                    1,
                ),
            ]
        );
        assert!(
            resolved
                .guards()
                .authority()
                .contains(&AuthorityGuard::Definition(
                    definitions.placement_bridge.clone()
                ))
        );

        move_tactical_gate_placement(&mut engine, &state, unselected_target, 3, 0);
        let check = resolved
            .guards()
            .revalidate(
                &engine.world,
                engine.action_state(),
                engine.registry.revision_validators().unwrap(),
                engine.registry.authority_guard_validators().unwrap(),
                engine.registry.definitions().unwrap(),
            )
            .unwrap();
        assert!(check.is_current());

        move_tactical_gate_placement(&mut engine, &state, selected_target, 4, 0);
        let check = resolved
            .guards()
            .revalidate(
                &engine.world,
                engine.action_state(),
                engine.registry.revision_validators().unwrap(),
                engine.registry.authority_guard_validators().unwrap(),
                engine.registry.definitions().unwrap(),
            )
            .unwrap();
        assert_eq!(check.stale_revisions().len(), 1);
        assert_eq!(
            check.stale_revisions()[0].key.subject,
            SubjectKey::Object {
                id: selected_target,
            }
        );
        assert_eq!(check.stale_revisions()[0].expected, 1);
        assert_eq!(check.stale_revisions()[0].actual, Some(2));
        assert!(!engine.action_state().has_prepared());
    }

    #[test]
    fn delayed_fire_capture_retains_source_and_per_choice_placement_generations() {
        let (mut engine, state, definitions, spec) = tactical_gate_world();
        let source = spawn_unit_player(&mut engine, &state, &spec, "Delayed Shooter");
        let first_target = ObjId(943);
        let second_target = ObjId(944);
        for (target, x) in [(first_target, 1), (second_target, 2)] {
            install_tactical_gate_unit(&mut engine, &state, unit(&spec, target.0));
            install_tactical_gate_placement(&mut engine, &state, target, x, 0);
        }
        grant_weapons(&mut engine, &state, source, source);
        let principal = PrincipalId::Object { id: source };
        for (id, target) in [
            (ContactId::from_bytes([0x94; 16]), first_target),
            (ContactId::from_bytes([0x95; 16]), second_target),
        ] {
            install_tactical_gate_contact(&mut engine, &state, id, target, principal.clone());
        }
        install_tactical_gate_fire_window(&mut engine, &state);
        let query = LegalActionQuery::new(
            principal,
            source,
            SubjectKey::Object { id: source },
            PerceivedRef::SelfActor,
            1,
            Some(tactical_gate_tick(7)),
        )
        .unwrap();
        let projection = engine
            .project_decision_offers(&query)
            .expect("kernel projects delayed fire");
        assert_eq!(projection.len(), 1);
        let (mut actions, mut offered) = projection.into_parts();
        let projected = actions.pop().expect("fire is projected");
        let witness = offered.pop().expect("fire witnesses are projected");

        assert_eq!(witness.offer, projected.offer());
        assert_eq!(witness.common_revisions.len(), 1);
        assert_eq!(
            witness.common_revisions[0].key.subject,
            SubjectKey::Object { id: source }
        );
        assert_eq!(witness.common_revisions[0].generation, 1);
        assert_eq!(witness.target_revisions.len(), 2);
        let guarded_targets: BTreeSet<_> = witness
            .target_revisions
            .iter()
            .map(|target| {
                assert_eq!(target.revisions.len(), 1);
                assert_eq!(target.revisions[0].generation, 1);
                target.revisions[0].key.subject.clone()
            })
            .collect();
        assert_eq!(
            guarded_targets,
            BTreeSet::from([
                SubjectKey::Object { id: first_target },
                SubjectKey::Object { id: second_target },
            ])
        );
        assert_eq!(
            witness.bridge_definitions,
            vec![definitions.placement_bridge]
        );
    }

    #[test]
    fn contact_loss_after_projection_collapses_to_generic_unavailable() {
        let (mut engine, state, _definitions, spec) = tactical_gate_world();
        let source_id = spawn_unit_player(&mut engine, &state, &spec, "Contact Loser");
        let target_id = ObjId(952);
        install_tactical_gate_unit(&mut engine, &state, unit(&spec, target_id.0));
        install_tactical_gate_placement(&mut engine, &state, target_id, 1, 0);
        grant_weapons(&mut engine, &state, source_id, source_id);
        let principal = PrincipalId::Object { id: source_id };
        let contact_id = ContactId::from_bytes([0x73; 16]);
        let contact = install_tactical_gate_contact(
            &mut engine,
            &state,
            contact_id,
            target_id,
            principal.clone(),
        );
        install_tactical_gate_fire_window(&mut engine, &state);
        let query = LegalActionQuery::new(
            principal.clone(),
            source_id,
            SubjectKey::Object { id: source_id },
            PerceivedRef::SelfActor,
            1,
            Some(tactical_gate_tick(7)),
        )
        .unwrap();
        let offers = engine.project_legal_actions(&query).unwrap();
        let projected = offers.projected().pop().unwrap();
        let ProjectedTargetSchema::Choices { choices, .. } = projected.targets() else {
            panic!("fire must project contacts");
        };
        let contact_ref = choices[0].clone();
        let lost = contact
            .checked_mark_lost(
                contact.revision(),
                tactical_gate_tick(7),
                CanonicalId::new("occluded").unwrap(),
            )
            .unwrap();
        let prepared = {
            let mut txn = engine.action_transaction(&state.contacts).unwrap();
            txn.replace(&state.contacts, &contact_id, lost).unwrap();
            txn.prepare().unwrap()
        };
        let (_, installation) = prepared.into_parts();
        engine
            .install_prepared_action(installation)
            .expect("the fixture installs");

        let request = RequestUid::from_bytes([0x74; 16]);
        let error = engine
            .resolve_legal_action(
                &offers,
                OfferExecutionContext {
                    request,
                    principal: principal.clone(),
                    actor: source_id,
                    idempotency: ScopedIdempotencyKey::new(
                        IdempotencyScope {
                            principal,
                            owner: OwnerId::new(TACTICAL_GATE_OWNER).unwrap(),
                            namespace: CanonicalId::new(FIRE_ACTION_ID).unwrap(),
                        },
                        1,
                        IdempotencyKey::Request(request),
                    ),
                    viewer_scope: query.viewer_scope(),
                    now: None,
                },
                OfferSubmission {
                    offer: projected.offer(),
                    subject: PerceivedRef::SelfActor,
                    targets: vec![contact_ref],
                    parameters: json!({}),
                },
            )
            .unwrap_err();
        assert!(matches!(
            error,
            LegalActionResolutionError::Resolve(OfferResolveError::Unavailable)
        ));
    }

    #[test]
    fn tactical_keyframe_line_receipt_and_llm_surfaces_share_one_redacted_dto() {
        const HIDDEN_NAME: &str = "CLASSIFIED NIGHTJAR";

        let (mut engine, state, definitions, spec) = tactical_gate_world();
        let source = spawn_unit_player(&mut engine, &state, &spec, "Projection Pilot");
        let target = engine
            .world
            .create(
                mush_core::ObjKind::Thing,
                HIDDEN_NAME,
                mush_core::GOD,
                Some(mush_core::LIMBO),
            )
            .unwrap();
        let target_unit = unit(&spec, target.0);
        let (target_unit, _) = apply(
            &target_unit,
            &UnitMutationPlan::new(
                vec![
                    UnitMutationOp::SetComponentCondition {
                        component: ComponentId::new(WEAPON_COMPONENT_ID).unwrap(),
                        condition: ConditionId::new(DISABLED_CONDITION_ID).unwrap(),
                    },
                    UnitMutationOp::SetCell {
                        addr: CellAddr::Section {
                            section: SectionId::new("center").unwrap(),
                            cell: CellId::new(STRUCTURE_CELL_ID).unwrap(),
                        },
                        value: 11,
                    },
                ],
                Some(0),
                None,
            )
            .unwrap(),
            6,
        )
        .unwrap();
        install_tactical_gate_unit(&mut engine, &state, target_unit);
        install_tactical_gate_placement(&mut engine, &state, target, 1, 0);
        grant_weapons(&mut engine, &state, source, source);
        let principal = PrincipalId::Object { id: source };
        let contact_id = ContactId::from_bytes([0xe1; 16]);
        let contact = install_tactical_gate_contact(
            &mut engine,
            &state,
            contact_id,
            target,
            principal.clone(),
        );
        install_tactical_gate_fire_window(&mut engine, &state);

        let projector = FireProvider {
            units: state.units.clone(),
            contacts: state.contacts.clone(),
            control_leases: state.control_leases.clone(),
            windows: state.windows.clone(),
            placements: state.placements.clone(),
            window: tactical_gate_window_id(),
            action: definitions.fire.clone(),
            outcome: definitions.fired.clone(),
        };
        let snapshot = projector
            .project_tactical_snapshot(&engine, &principal, source)
            .expect("the viewer-specific tactical snapshot projects");
        assert_eq!(snapshot.own_unit.subject, PerceivedRef::SelfActor);
        assert_eq!(snapshot.contacts.len(), 1);
        assert_eq!(snapshot.contacts[0].condition_band, "damaged");
        assert!(matches!(
            snapshot.contacts[0].contact,
            PerceivedRef::Contact { id, generation: 1, .. } if id == contact_id
        ));
        let snapshot_json = serde_json::to_value(&snapshot).unwrap();
        assert!(serde_json::from_value::<TacticalUnit>(snapshot_json).is_err());
        let keyframe = tactical_snapshot_keyframe(&snapshot, 19, 41).unwrap();
        let panel_json = serde_json::to_value(&keyframe).unwrap();
        let line = render_tactical_snapshot_line(&snapshot);

        let query = LegalActionQuery::new(
            principal.clone(),
            source,
            SubjectKey::Object { id: source },
            PerceivedRef::SelfActor,
            1,
            Some(tactical_gate_tick(7)),
        )
        .unwrap();
        let offers = engine.project_legal_actions(&query).unwrap();
        let projected_actions = offers.projected();
        let projected = projected_actions[0].clone();
        let ProjectedTargetSchema::Choices { choices, .. } = projected.targets() else {
            panic!("the tactical action must expose only an opaque contact choice");
        };
        let request = RequestUid::from_bytes([0xe2; 16]);
        let resolved = engine
            .resolve_legal_action(
                &offers,
                OfferExecutionContext {
                    request,
                    principal: principal.clone(),
                    actor: source,
                    idempotency: ScopedIdempotencyKey::new(
                        IdempotencyScope {
                            principal: principal.clone(),
                            owner: OwnerId::new(TACTICAL_GATE_OWNER).unwrap(),
                            namespace: CanonicalId::new(FIRE_ACTION_ID).unwrap(),
                        },
                        1,
                        IdempotencyKey::Request(request),
                    ),
                    viewer_scope: query.viewer_scope(),
                    now: None,
                },
                OfferSubmission {
                    offer: projected.offer(),
                    subject: PerceivedRef::SelfActor,
                    targets: vec![choices[0].clone()],
                    parameters: json!({}),
                },
            )
            .unwrap();
        let (prepared, output) = engine
            .prepare_registered_resolved_authoritative_action(
                &definitions.fire,
                resolved.resolved_invocation(),
                CommitToken::from_bytes([0xe3; 16]),
            )
            .unwrap();
        let private_outcome = output.into_parts().0;
        let receipt = engine
            .registry
            .action_providers()
            .unwrap()
            .receipt_projection_table()
            .project(&definitions.fire, &private_outcome, &principal)
            .unwrap()
            .unwrap();
        engine.abort_prepared_transaction(prepared);

        let llm_json = json!({
            "tactical_state": snapshot,
            "legal_actions": projected_actions,
            "last_result": receipt,
        });
        let receipt_json = llm_json["last_result"].clone();
        let forbidden_fields = [
            "truth_subject",
            "truth_targets",
            "object",
            "schema",
            "capabilities",
            "asset",
            "target",
            "target_revision",
            "source_revision",
            "damage",
            "to_hit",
        ];
        let forbidden_fragments = [HIDDEN_NAME, DISABLED_CONDITION_ID];
        let forbidden_values = [json!(target.0), json!(11)];
        for (surface, value) in [
            ("tactical keyframe", &panel_json),
            ("tactical TUI", &panel_json),
            ("action receipt", &receipt_json),
            ("LLM tactical context", &llm_json),
        ] {
            crate::assert_projection_json_clean_with_values(
                surface,
                value,
                &forbidden_fields,
                &forbidden_fragments,
                &forbidden_values,
            );
        }
        crate::assert_projection_text_clean(
            "line tactical rendering",
            &line,
            &[
                HIDDEN_NAME,
                DISABLED_CONDITION_ID,
                &format!("#{}", target.0),
                "11",
            ],
        );

        let lost = contact
            .checked_mark_lost(
                contact.revision(),
                tactical_gate_tick(7),
                CanonicalId::new("projection_refresh").unwrap(),
            )
            .unwrap();
        let prepared = {
            let mut txn = engine.action_transaction(&state.contacts).unwrap();
            txn.replace(&state.contacts, &contact_id, lost).unwrap();
            txn.prepare().unwrap()
        };
        let (_, installation) = prepared.into_parts();
        engine
            .install_prepared_action(installation)
            .expect("the fixture installs");
        let refreshed = projector
            .project_tactical_snapshot(&engine, &principal, source)
            .unwrap();
        assert!(
            refreshed.contacts.is_empty(),
            "a refreshed keyframe must omit contacts no longer knowable"
        );
        let refreshed_json =
            serde_json::to_value(tactical_snapshot_keyframe(&refreshed, 20, 0).unwrap()).unwrap();
        crate::assert_projection_json_clean_with_values(
            "contact-loss keyframe",
            &refreshed_json,
            &forbidden_fields,
            &forbidden_fragments,
            &forbidden_values,
        );
    }

    #[test]
    fn two_viewers_receive_distinct_contacts_for_the_same_truth_target() {
        let (mut engine, state, definitions, spec) = tactical_gate_world();
        let first = spawn_unit_player(&mut engine, &state, &spec, "First Viewer");
        let second = spawn_unit_player(&mut engine, &state, &spec, "Second Viewer");
        let hidden_target = ObjId(963);
        install_tactical_gate_unit(&mut engine, &state, unit(&spec, hidden_target.0));
        install_tactical_gate_placement(&mut engine, &state, hidden_target, 1, 0);
        grant_weapons(&mut engine, &state, first, first);
        grant_weapons(&mut engine, &state, second, second);
        let first_contact = ContactId::from_bytes([0x81; 16]);
        let second_contact = ContactId::from_bytes([0x82; 16]);
        install_tactical_gate_contact(
            &mut engine,
            &state,
            first_contact,
            hidden_target,
            PrincipalId::Object { id: first },
        );
        install_tactical_gate_contact(
            &mut engine,
            &state,
            second_contact,
            hidden_target,
            PrincipalId::Object { id: second },
        );
        install_tactical_gate_fire_window(&mut engine, &state);

        let projected_contact = |engine: &Engine, viewer: ObjId| {
            let query = LegalActionQuery::new(
                PrincipalId::Object { id: viewer },
                viewer,
                SubjectKey::Object { id: viewer },
                PerceivedRef::SelfActor,
                1,
                Some(tactical_gate_tick(7)),
            )
            .unwrap();
            let projected = engine
                .project_legal_actions(&query)
                .unwrap()
                .projected()
                .pop()
                .unwrap();
            let ProjectedTargetSchema::Choices { choices, .. } = projected.targets() else {
                panic!("fire must project contacts");
            };
            assert!(
                !serde_json::to_string(&projected)
                    .unwrap()
                    .contains(&hidden_target.0.to_string())
            );
            let PerceivedRef::Contact { id, .. } = choices[0] else {
                panic!("fire target must remain opaque");
            };
            id
        };

        assert_eq!(projected_contact(&engine, first), first_contact);
        assert_eq!(projected_contact(&engine, second), second_contact);
        assert_ne!(first_contact, second_contact);

        let projector = FireProvider {
            units: state.units.clone(),
            contacts: state.contacts.clone(),
            control_leases: state.control_leases.clone(),
            windows: state.windows.clone(),
            placements: state.placements.clone(),
            window: tactical_gate_window_id(),
            action: definitions.fire,
            outcome: definitions.fired,
        };
        for (viewer, expected, forbidden) in [
            (first, first_contact, second_contact),
            (second, second_contact, first_contact),
        ] {
            let snapshot = projector
                .project_tactical_snapshot(&engine, &PrincipalId::Object { id: viewer }, viewer)
                .unwrap();
            let [ProjectedContactUnit { contact, .. }] = snapshot.contacts.as_slice() else {
                panic!("each keyframe must contain exactly that viewer's contact");
            };
            assert!(matches!(
                contact,
                PerceivedRef::Contact { id, .. } if *id == expected
            ));
            assert!(
                !serde_json::to_string(&snapshot)
                    .unwrap()
                    .contains(&forbidden.to_hex())
            );
        }
    }
}

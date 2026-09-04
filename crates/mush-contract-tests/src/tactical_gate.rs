//! Gate-2 fixture: one component-provided attack prepares two tactical-unit
//! post-images and one deterministic resolution before either unit can install.
//!
//! This deliberately lives in the contract-test crate. Production
//! `mush-tactics` still owns its legacy unit-document authority, so proving the
//! K3/K6 transaction shape here does not create a second live authority for
//! production `TacticalUnit` records.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionInvocationDraft, ActionOfferId,
    ActionProvider, ActionStateError, ActionTxn, AuthoritativeActionOutput, CanonicalHash,
    CanonicalId, ClockRef, ClockStamp, CommitToken, DefinitionDescriptor, DefinitionId,
    DefinitionKind, DefinitionRef, DefinitionRegistryBuilder, DefinitionValidationContext,
    DefinitionValidator, Engine, GameModule, IdempotencyKey, IdempotencyScope, ModError, ObjId,
    OwnerId, PerceivedRef, PrepareCtx, PrincipalId, Registrar, RequestUid, ResolutionKeyRing,
    ResolutionSecret, RulesValue, ScopedIdempotencyKey, StateFamilyDescriptor, StateFamilyHandle,
    StateFamilyId, World,
};
use mush_tactics::{
    CellAddr, CellId, CellSpec, ComponentId, ComponentSpec, ConditionId, InstallPoint, SectionId,
    SectionSpec, TacticalUnit, UnitMutationOp, UnitMutationPlan, UnitSpec, UnitSpecBody, apply,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Owner used by the isolated Gate-2 module.
pub const TACTICAL_GATE_OWNER: &str = "toytactics";
/// Component-provided action exercised by Gate 2.
pub const FIRE_ACTION_ID: &str = "fire";
/// Private durable outcome for the fire action.
pub const FIRED_OUTCOME_ID: &str = "fired";
/// Engine-owned family holding the fixture's unit records.
pub const TACTICAL_UNIT_FAMILY: &str = "units";

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
}

#[derive(Clone, Debug)]
pub struct TacticalGateState {
    pub units: StateFamilyHandle<ObjId, TacticalUnit>,
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
        .register(owner, fire_descriptor.clone())
        .map_err(|error| error.to_string())?;
    descriptors.push(fire_descriptor);

    Ok(DefinitionFixture {
        descriptors,
        definitions: TacticalGateDefinitions {
            unit,
            weapon,
            fire,
            fired,
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

#[derive(Debug)]
struct FireProvider {
    units: StateFamilyHandle<ObjId, TacticalUnit>,
    action: DefinitionRef,
    outcome: DefinitionRef,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FireParameters {
    source_revision: u64,
    target_revision: u64,
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

    fn decode(invocation: &ActionInvocation) -> Result<(ObjId, FireParameters), ActionError> {
        if invocation.subject() != &PerceivedRef::SelfActor || invocation.targets().len() != 1 {
            return Err(Self::invalid(
                "fire requires the acting unit as subject and exactly one known target",
            ));
        }
        let PerceivedRef::KnownObject { id: target, .. } = invocation.targets()[0] else {
            return Err(Self::invalid("Gate-2 fire requires a known-object target"));
        };
        if target == invocation.actor() {
            return Err(Self::invalid("a Gate-2 unit cannot fire on itself"));
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
        Ok((target, parameters))
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

    fn prepare_fire(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        context: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let (target_id, parameters) = Self::decode(invocation)?;
        let source_id = invocation.actor();
        let source = self.read_unit(txn, source_id)?;
        let target = self.read_unit(txn, target_id)?;
        if source.revision() != parameters.source_revision
            || target.revision() != parameters.target_revision
        {
            return Err(Self::stale("a Gate-2 unit revision changed"));
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
            .ok_or_else(|| Self::integrity("the selected target section has no structure cell"))?;
        if structure.current <= structure.min {
            return Err(Self::rejected(
                "the selected target section cannot take more damage",
            ));
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

impl ActionProvider for FireProvider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        Self::decode(invocation).map(|_| ())
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
        self.prepare_fire(invocation, txn, context)
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

#[derive(Debug, Default)]
pub struct TacticalGateModule {
    captured: Arc<Mutex<Option<(TacticalGateState, TacticalGateDefinitions, UnitSpec)>>>,
}

impl TacticalGateModule {
    fn with_capture(
        captured: Arc<Mutex<Option<(TacticalGateState, TacticalGateDefinitions, UnitSpec)>>>,
    ) -> Self {
        Self { captured }
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
        let mut registered = BTreeMap::new();
        for descriptor in fixture.descriptors.clone() {
            let reference = registrar.definition(descriptor)?;
            registered.insert(reference.key().kind.clone(), reference);
        }
        // Comparing the complete known references catches accidental descriptor
        // order/content drift before the provider receives any capability.
        for expected in [
            &fixture.definitions.unit,
            &fixture.definitions.weapon,
            &fixture.definitions.fire,
            &fixture.definitions.fired,
        ] {
            if !registered.values().any(|actual| actual == expected) {
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
        registrar.action_provider(
            fixture.definitions.fire.clone(),
            FireProvider {
                units: units.clone(),
                action: fixture.definitions.fire.clone(),
                outcome: fixture.definitions.fired.clone(),
            },
        )?;
        *self.captured.lock().expect("Gate-2 capture poisoned") =
            Some((TacticalGateState { units }, fixture.definitions, unit_spec));
        Ok(())
    }
}

/// Compose the isolated Gate-2 module and install deterministic resolution keys.
#[must_use]
pub fn tactical_gate_world() -> (Engine, TacticalGateState, TacticalGateDefinitions, UnitSpec) {
    let captured = Arc::new(Mutex::new(None));
    let module = TacticalGateModule::with_capture(Arc::clone(&captured));
    let mut engine = Engine::with_modules(
        World::new_minimal(),
        mush_core::Registry::new(),
        vec![Box::new(module)],
    )
    .expect("Gate-2 module composes");
    engine.set_resolution_keys(
        ResolutionKeyRing::new(
            ResolutionSecret::new(1, [0x62; 32]).expect("Gate-2 resolution key"),
            [],
        )
        .expect("Gate-2 resolution ring"),
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
    engine.install_prepared_action(installation);
}

/// Canonical domain clock used by the fixture.
#[must_use]
pub fn tactical_gate_tick(at: i64) -> ClockStamp {
    ClockStamp {
        clock: ClockRef::DomainTick {
            owner: OwnerId::new(TACTICAL_GATE_OWNER).expect("static owner"),
            id: CanonicalId::new("arena").expect("static domain id"),
            generation: 1,
        },
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

/// Fixed commit token for provider-level Gate-2 tests.
#[must_use]
pub const fn tactical_gate_token() -> CommitToken {
    CommitToken::from_bytes([0x26; 16])
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::ActionProviderPrepareError;

    fn unit(spec: &UnitSpec, id: u32) -> TacticalUnit {
        TacticalUnit::from_spec(spec, ObjId(id)).expect("valid Gate-2 unit")
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
        engine.install_prepared_action(installation);
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
    fn target_refusal_cannot_stage_the_source_ammo_and_heat_spend() {
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
                if source.code() == ActionErrorCode::ProviderRejected
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
}

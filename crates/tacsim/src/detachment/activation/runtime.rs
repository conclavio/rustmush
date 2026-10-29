//! Commit a selected roster and its reservations through the common coordinator.
use super::{ActiveDetachment, Reservation, detachment_id};
use crate::detachment::crew::{CrewContract, CrewDuty, CrewHandles, personnel};
use crate::detachment::planning::{DraftRef, PlanningHandles, projection};
use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, ActionTxn,
    AuthoritativeActionOutput, AuthorityWitnessRequest, BridgeDescriptor, CanonicalHash,
    CanonicalId, ChannelId, Ctx, DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef,
    Engine, Flags, IdempotencyKeyText, LegalActionQuery, ModError, ObjId, ObjKind, OwnerId,
    PerceivedRef, PrepareCtx, PrincipalId, ProjectedParameter, ProjectedParameterKind,
    ProviderOfferDraft, Registrar, ResolvedActionInvocation, RevisionKey, RevisionKind,
    RevisionSet, RevisionStamp, RevisionValidator, RevisionValidatorContext, RulesValue,
    StateFamilyHandle, SubjectKey, World,
};
use mush_org::unit_authority::{UnitCapability, operation as unit_operation};
use mush_tactics::detachment::planning::{
    ConsentState, Draft, PlanId, PlannedController, PlanningBook,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const ACTION: &str = "tacsim_detachment_activation";
pub const OUTCOME: &str = "tacsim_detachment_activation_result";
const RECORDS: &str = "tacsim.detachment_active";
const RESERVATIONS: &str = "tacsim.detachment_reservations";
const REVISION: &str = "detachment_activation_input";
const MAX_INPUT: usize = 16 * 1024;
const MAX_PENDING: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Operation {
    Activate {
        id: PlanId,
        expected_revision: u64,
        before_activation: u64,
        duty: CrewDuty,
    },
    Deactivate {
        id: PlanId,
        expected_activation: u64,
    },
}
impl Operation {
    fn id(&self) -> PlanId {
        match self {
            Self::Activate { id, .. } | Self::Deactivate { id, .. } => *id,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Intent {
    at: i64,
    operation: Operation,
}
impl Intent {
    fn current(&self, now: i64) -> bool {
        now.checked_sub(self.at)
            .is_some_and(|age| (0..=300).contains(&age))
    }
}
#[derive(Clone, Default)]
struct Pending(BTreeMap<ObjId, Intent>);
fn pending(world: &World, actor: ObjId) -> Option<&Intent> {
    world.module_state::<Pending>()?.0.get(&actor)
}

#[derive(Clone, Debug)]
pub struct ActivationHandles {
    pub records: StateFamilyHandle<DraftRef, ActiveDetachment>,
    pub reservations: StateFamilyHandle<ObjId, Reservation>,
    pub action: DefinitionRef,
    pub(super) plans: StateFamilyHandle<ObjId, PlanningBook>,
    pub(super) contracts: StateFamilyHandle<ObjId, CrewContract>,
    pub(super) enabled: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
// External tagging keeps Draft's numeric consent-map keys on JSON's key
// decoder. Internally tagged buffering would deserialize those keys as text.
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum Selection {
    Activate { draft: Draft },
    Deactivate { detachment: u64, revision: u64 },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    intent: Intent,
    selection: Selection,
}
#[derive(Clone, Debug, Serialize)]
struct Observation {
    npc_terms: BTreeMap<ObjId, CanonicalHash>,
    players: Vec<projection::ReadyStationProof>,
    entities: BTreeSet<ObjId>,
}
struct Snapshot {
    input: Input,
    observation: Observation,
    before: Option<ActiveDetachment>,
}

fn refused() -> ActionError {
    crate::port_tick::port_error(
        ActionErrorCode::ProviderRejected,
        "Detachment preparation changed or is unavailable. Review the current plan and unit permissions.",
    )
}
fn bad(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
fn descriptor(kind: &str, id: &str, value: Value) -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new(kind).map_err(bad)?,
        DefinitionId::new(id).map_err(bad)?,
        1,
        value,
    )
    .map_err(bad)
}
pub fn outcome_descriptor() -> Result<DefinitionDescriptor, ModError> {
    descriptor(
        "outcome",
        OUTCOME,
        json!({"format":1,"fields":["organizer","draft","detachment","revision","active"]}),
    )
}
pub fn action_descriptor(
    outcomes: &BTreeMap<String, DefinitionRef>,
) -> Result<DefinitionDescriptor, ModError> {
    let outcome = outcomes
        .get(OUTCOME)
        .cloned()
        .ok_or_else(|| bad("activation outcome missing"))?;
    descriptor(
        "action",
        ACTION,
        json!({"format":1,"subject":"self_actor","targets":0,"families":[RECORDS,RESERVATIONS]}),
    )
    .map(|d| d.as_current().with_references(vec![outcome]))
}
pub fn register(
    reg: &mut Registrar<'_>,
    actions: &BTreeMap<String, DefinitionRef>,
    outcomes: &BTreeMap<String, DefinitionRef>,
    planning: &PlanningHandles,
    contracts: &CrewHandles,
    enabled: bool,
) -> Result<(), ModError> {
    let records = reg.state_family(
        crate::port::families::family_descriptor(
            RECORDS,
            "tacsim.detachment_active.key.v1",
            RECORDS,
        )?
        .with_limits(32, 64 * 1024)
        .map_err(bad)?,
        |key: &DraftRef| {
            detachment_id(*key)?;
            Ok(format!("{}-{}", key.organizer.0, key.id.0))
        },
        |v: &Value| serde_json::from_value(v.clone()).map_err(|e| e.to_string()),
        ActiveDetachment::validate,
        |v: &ActiveDetachment| serde_json::to_value(v).map_err(|e| e.to_string()),
    )?;
    let reservations = reg.state_family(
        crate::port::families::family_descriptor(
            RESERVATIONS,
            "tacsim.detachment_reservations.key.v1",
            RESERVATIONS,
        )?
        .with_limits(16, 2048)
        .map_err(bad)?,
        |key: &ObjId| Ok(key.0.to_string()),
        |v: &Value| serde_json::from_value(v.clone()).map_err(|e| e.to_string()),
        Reservation::validate,
        |v: &Reservation| serde_json::to_value(v).map_err(|e| e.to_string()),
    )?;
    let action = actions
        .get(ACTION)
        .cloned()
        .ok_or_else(|| bad("activation action missing"))?;
    let outcome = outcomes
        .get(OUTCOME)
        .cloned()
        .ok_or_else(|| bad("activation outcome missing"))?;
    if enabled {
        let bridge = descriptor(
            "bridge",
            "tacsim_activation_unit_rights",
            json!({"format":1,"purpose":"current_unit_use_for_activation"}),
        )?;
        reg.definition_validator(
            DefinitionKind::new("bridge").map_err(bad)?,
            crate::definitions::exact_validator(vec![bridge.clone()]),
        )?;
        let definition = reg.definition(bridge)?;
        reg.interaction_bridge(BridgeDescriptor::new(
            definition,
            OwnerId::new(crate::NS).map_err(bad)?,
            OwnerId::new("org").map_err(bad)?,
            action.clone(),
        ))?;
    }
    let handles = ActivationHandles {
        records,
        reservations,
        action: action.clone(),
        plans: planning.families.books.clone(),
        contracts: contracts.contracts.clone(),
        enabled,
    };
    reg.action_provider(
        action.clone(),
        Provider {
            handles: handles.clone(),
            outcome,
        },
    )?;
    reg.revision_validator(
        RevisionKind::new(CanonicalId::new(REVISION).map_err(bad)?),
        Validator {
            handles: handles.clone(),
        },
    )?;
    reg.state(Pending::default());
    reg.state(handles);
    Ok(())
}

fn validate_record(
    reference: DraftRef,
    record: Option<ActiveDetachment>,
) -> Result<Option<ActiveDetachment>, String> {
    if let Some(record) = &record {
        record.validate()?;
        if record.reference() != reference {
            return Err("Detachment record identity mismatch.".into());
        }
    }
    Ok(record)
}
fn source(book: Option<&PlanningBook>, actor: ObjId, id: PlanId) -> Result<Draft, String> {
    let book = book.ok_or("No prepared draft.")?;
    book.validate().map_err(|e| e.to_string())?;
    if book.owner != actor {
        return Err("Wrong draft owner.".into());
    }
    book.drafts
        .get(&id)
        .cloned()
        .ok_or_else(|| "No prepared draft.".into())
}
fn observe(
    world: &World,
    draft: &Draft,
    duty: CrewDuty,
    mut load: impl FnMut(ObjId) -> Result<Option<CrewContract>, String>,
) -> Result<Observation, String> {
    draft.validate().map_err(|e| e.to_string())?;
    if draft.plan.slots.is_empty()
        || draft.plan.financial_terms.is_some()
        || !projection::physically_aboard(world, draft.organizer, draft.plan.carrier)
    {
        return Err("Preparation unavailable.".into());
    }
    let mut result = Observation {
        npc_terms: BTreeMap::new(),
        players: Vec::new(),
        entities: BTreeSet::new(),
    };
    for row in personnel::resolve(world, draft, &mut load)? {
        let terms = row.terms.map_err(|_| "NPC preparation unavailable.")?;
        if !terms.duties.contains(&duty) {
            return Err("NPC duty is outside the employment agreement.".into());
        }
        result.npc_terms.insert(row.character, terms.digest()?);
    }
    for slot in &draft.plan.slots {
        let unit = slot.unit.ok_or("Unit selection is incomplete.")?;
        if slot.mind.is_some()
            || mush_tactics::module::unit_of(world, unit).is_none()
            || !projection::physically_aboard(world, unit, draft.plan.carrier)
        {
            return Err("Unit preparation unavailable.".into());
        }
        let character = match slot.controller {
            PlannedController::Npc { character } => character,
            PlannedController::Player { character } => {
                let consent = draft
                    .consents
                    .get(&character)
                    .ok_or("Player is not ready.")?;
                if consent.state != ConsentState::Ready
                    || consent.revision != draft.revision
                    || consent.plan_hash != draft.plan_hash
                {
                    return Err("Player is not ready for this revision.".into());
                }
                result.players.push(
                    projection::ready_station(world, character, draft)
                        .ok_or("Player's current station changed.")?,
                );
                character
            }
            _ => {
                return Err(
                    "Only player and hired-NPC assignments can activate in this cut.".into(),
                );
            }
        };
        result.entities.insert(unit);
        result.entities.insert(character);
    }
    if result.entities.len() != draft.plan.slots.len() * 2 {
        return Err("Assignments overlap.".into());
    }
    Ok(result)
}

fn snapshot(
    world: &World,
    actor: ObjId,
    intent: &Intent,
    before: Option<ActiveDetachment>,
    draft: Option<Draft>,
    mut contract: impl FnMut(ObjId) -> Result<Option<CrewContract>, String>,
    mut reservation: impl FnMut(ObjId) -> Result<Option<Reservation>, String>,
) -> Result<Snapshot, String> {
    if !intent.current(world.now())
        || pending(world, actor) != Some(intent)
        || world
            .get(actor)
            .is_none_or(|o| o.kind != ObjKind::Player || o.flags.contains(Flags::GOING))
        || world
            .module_state::<crate::TacsimStore>()
            .is_none_or(|s| !s.enabled())
    {
        return Err("Preparation input expired.".into());
    }
    let reference = DraftRef {
        organizer: actor,
        id: intent.operation.id(),
    };
    let before = validate_record(reference, before)?;
    let (selection, observation) = match intent.operation {
        Operation::Activate {
            id,
            expected_revision,
            before_activation,
            duty,
        } => {
            if before.as_ref().is_some_and(ActiveDetachment::active)
                || before.as_ref().map_or(0, |r| r.revision) != before_activation
            {
                return Err(
                    "Deactivate the previous roster before activating another revision.".into(),
                );
            }
            let draft = draft.ok_or("Draft missing.")?;
            if draft.organizer != actor || draft.id != id || draft.revision != expected_revision {
                return Err("Draft changed.".into());
            }
            let observation = observe(world, &draft, duty, &mut contract)?;
            for entity in &observation.entities {
                if reservation(*entity)?.is_some() {
                    return Err("A unit or person is already reserved.".into());
                }
            }
            (Selection::Activate { draft }, observation)
        }
        Operation::Deactivate {
            expected_activation,
            ..
        } => {
            let record = before
                .as_ref()
                .filter(|r| r.active() && r.revision == expected_activation)
                .ok_or("Active roster changed.")?;
            let entities = record.entities();
            for entity in &entities {
                if reservation(*entity)?.as_ref() != Some(&Reservation::for_entity(record, *entity))
                {
                    return Err("Reservation changed.".into());
                }
            }
            (
                Selection::Deactivate {
                    detachment: record.detachment,
                    revision: record.revision,
                },
                Observation {
                    npc_terms: BTreeMap::new(),
                    players: Vec::new(),
                    entities,
                },
            )
        }
    };
    Ok(Snapshot {
        input: Input {
            intent: intent.clone(),
            selection,
        },
        observation,
        before,
    })
}

fn fingerprint(actor: ObjId, intent: &Intent) -> Result<CanonicalHash, ActionError> {
    CanonicalHash::for_canonical_value(
        "tacsim.activation_input.v1",
        1,
        &json!({"actor":actor,"intent":intent}),
        2048,
    )
    .map_err(|_| refused())
}
fn key(actor: ObjId) -> Result<RevisionKey, ActionError> {
    Ok(RevisionKey {
        owner: OwnerId::new(crate::NS).map_err(|_| refused())?,
        kind: RevisionKind::new(CanonicalId::new(REVISION).map_err(|_| refused())?),
        subject: SubjectKey::Object { id: actor },
        facet: None,
    })
}
fn generation(world: &World, snapshot: Result<Snapshot, String>) -> Result<u64, ActionError> {
    let Ok(snapshot) = snapshot else { return Ok(0) };
    let hash = CanonicalHash::for_canonical_value(
        "tacsim.activation_preimage.v1",
        1,
        &json!({
            "input": snapshot.input,
            "observation": snapshot.observation,
            "clock": world.world_ic_now().clock,
        }),
        128 * 1024,
    )
    .map_err(|_| refused())?;
    let bytes: [u8; 8] = hash
        .as_bytes()
        .get(..8)
        .and_then(|b| b.try_into().ok())
        .ok_or_else(refused)?;
    Ok(u64::from_be_bytes(bytes).max(1))
}
struct Validator {
    handles: ActivationHandles,
}
impl RevisionValidator for Validator {
    fn current_generation(
        &self,
        ctx: &RevisionValidatorContext<'_>,
        coordinate: &RevisionKey,
    ) -> Result<Option<u64>, ActionError> {
        let SubjectKey::Object { id: actor } = coordinate.subject else {
            return Ok(None);
        };
        if coordinate != &key(actor)? {
            return Ok(None);
        }
        let Some(intent) = pending(ctx.world(), actor) else {
            return Ok(None);
        };
        let reference = DraftRef {
            organizer: actor,
            id: intent.operation.id(),
        };
        let before = ctx
            .action_state(&self.handles.records, &reference)
            .map_err(|_| refused())?
            .cloned();
        let draft = match intent.operation {
            Operation::Activate { id, .. } => source(
                ctx.action_state(&self.handles.plans, &actor)
                    .map_err(|_| refused())?,
                actor,
                id,
            )
            .ok(),
            _ => None,
        };
        let state = snapshot(
            ctx.world(),
            actor,
            intent,
            before,
            draft,
            |npc| {
                ctx.action_state(&self.handles.contracts, &npc)
                    .map(|v| v.cloned())
                    .map_err(|e| e.to_string())
            },
            |entity| {
                ctx.action_state(&self.handles.reservations, &entity)
                    .map(|v| v.cloned())
                    .map_err(|e| e.to_string())
            },
        );
        generation(ctx.world(), state).map(Some)
    }
}

struct Provider {
    handles: ActivationHandles,
    outcome: DefinitionRef,
}
fn chosen(i: &ActionInvocation) -> Result<Input, ActionError> {
    if i.principal() != &(PrincipalId::Object { id: i.actor() })
        || i.subject() != &PerceivedRef::SelfActor
        || !i.targets().is_empty()
        || i.parameters().as_object().is_none_or(|p| p.len() != 2)
    {
        return Err(refused());
    }
    let value = i.parameters().get("input").ok_or_else(refused)?;
    mush_core::canonical_json_bytes_with_limit(value, MAX_INPUT).map_err(|_| refused())?;
    let input: Input = serde_json::from_value(value.clone()).map_err(|_| refused())?;
    if i.parameters().get("request").and_then(Value::as_str)
        != Some(fingerprint(i.actor(), &input.intent)?.to_hex().as_str())
    {
        return Err(refused());
    }
    Ok(input)
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, i: &ActionInvocation) -> Result<(), ActionError> {
        chosen(i).map(|_| ())
    }
    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &LegalActionQuery,
        engine: &Engine,
    ) -> Result<Option<ProviderOfferDraft>, ActionError> {
        if !self.handles.enabled
            || action != &self.handles.action
            || query.projected_subject() != &PerceivedRef::SelfActor
            || query.truth_subject() != &(SubjectKey::Object { id: query.actor() })
        {
            return Ok(None);
        }
        let actor = query.actor();
        let Some(intent) = pending(&engine.world, actor) else {
            return Ok(None);
        };
        let reference = DraftRef {
            organizer: actor,
            id: intent.operation.id(),
        };
        let before = engine
            .action_state()
            .get(&self.handles.records, &reference)
            .map_err(|_| refused())?
            .cloned();
        let draft = match intent.operation {
            Operation::Activate { id, .. } => source(
                engine
                    .action_state()
                    .get(&self.handles.plans, &actor)
                    .map_err(|_| refused())?,
                actor,
                id,
            )
            .ok(),
            _ => None,
        };
        let state = snapshot(
            &engine.world,
            actor,
            intent,
            before,
            draft,
            |npc| {
                engine
                    .action_state()
                    .get(&self.handles.contracts, &npc)
                    .map(|v| v.cloned())
                    .map_err(|e| e.to_string())
            },
            |entity| {
                engine
                    .action_state()
                    .get(&self.handles.reservations, &entity)
                    .map(|v| v.cloned())
                    .map_err(|e| e.to_string())
            },
        );
        let Ok(state) = state else { return Ok(None) };
        let fixed = serde_json::to_value(&state.input).map_err(|_| refused())?;
        let revisions = RevisionSet::from_stamps([RevisionStamp {
            key: key(actor)?,
            generation: generation(&engine.world, Ok(state))?,
        }])
        .map_err(|_| refused())?;
        crate::surface::act::offer_draft_with_revisions(
            action,
            engine,
            json!({"input":fixed}),
            vec![ProjectedParameter {
                id: CanonicalId::new("request").map_err(|_| refused())?,
                required: true,
                kind: ProjectedParameterKind::Text { maximum_bytes: 64 },
            }],
            (
                "detachment-activation",
                "Prepare or release a detachment",
                "Commit the selected roster and its exclusive reservations.",
            ),
            revisions,
        )
        .map(Some)
    }
    fn resolved_revision_keys(
        &self,
        resolved: &ResolvedActionInvocation,
    ) -> Result<Vec<RevisionKey>, ActionError> {
        let input = chosen(resolved.invocation())?;
        let Selection::Activate { draft } = input.selection else {
            return Ok(Vec::new());
        };
        let mut keys = BTreeSet::new();
        for slot in &draft.plan.slots {
            let unit = slot.unit.ok_or_else(refused)?;
            keys.insert(unit_operation::revision_key(
                resolved.invocation().actor(),
                unit,
                UnitCapability::Deploy,
            )?);
            keys.insert(unit_operation::revision_key(
                slot.controller.character().ok_or_else(refused)?,
                unit,
                UnitCapability::OperateCrew,
            )?);
        }
        Ok(keys.into_iter().collect())
    }
    fn resolved_authority_witnesses(
        &self,
        resolved: &ResolvedActionInvocation,
        _engine: &Engine,
    ) -> Result<Vec<AuthorityWitnessRequest>, ActionError> {
        let input = chosen(resolved.invocation())?;
        let Selection::Activate { draft } = input.selection else {
            return Ok(Vec::new());
        };
        draft
            .plan
            .slots
            .iter()
            .filter_map(|s| match s.controller {
                PlannedController::Player { character } => Some((s.unit, character)),
                _ => None,
            })
            .map(|(unit, character)| {
                Ok(AuthorityWitnessRequest::ControlLease {
                    provider: OwnerId::new("sim").map_err(|_| refused())?,
                    subject: unit.ok_or_else(refused)?,
                    channel: ChannelId::new("helm").ok_or_else(refused)?,
                    holder: PrincipalId::Object { id: character },
                })
            })
            .collect()
    }
    // Raw invocations retain the default refusal. Only Core's regenerated,
    // resolved offer can supply the current Org and Sim guards above.
    fn prepare_resolved_authoritative_with_context(
        &self,
        resolved: &ResolvedActionInvocation,
        txn: &mut ActionTxn<'_>,
        context: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        if !self.handles.enabled {
            return Err(refused());
        }
        let i = resolved.invocation();
        let input = chosen(i)?;
        let reference = DraftRef {
            organizer: i.actor(),
            id: input.intent.operation.id(),
        };
        let before = txn
            .get(&self.handles.records, &reference)
            .map_err(|_| refused())?
            .cloned();
        let draft = match input.intent.operation {
            Operation::Activate { id, .. } => source(
                txn.get(&self.handles.plans, &i.actor())
                    .map_err(|_| refused())?,
                i.actor(),
                id,
            )
            .ok(),
            _ => None,
        };
        let world = context.world().map_err(|_| refused())?;
        // Read the selected rows once through this actual transaction. The
        // bounded local maps let the two pure checks share those preimages
        // without overlapping mutable borrows of ActionTxn.
        let mut contracts = BTreeMap::new();
        if let Some(draft) = &draft {
            for slot in &draft.plan.slots {
                if let PlannedController::Npc { character } = slot.controller {
                    contracts.insert(
                        character,
                        txn.get(&self.handles.contracts, &character)
                            .map_err(|_| refused())?
                            .cloned(),
                    );
                }
            }
        }
        let entities: BTreeSet<_> = draft
            .as_ref()
            .map(|d| &d.plan)
            .or_else(|| before.as_ref().map(|r| &r.plan))
            .into_iter()
            .flat_map(|p| &p.slots)
            .flat_map(|s| s.unit.into_iter().chain(s.controller.character()))
            .collect();
        let mut reservations = BTreeMap::new();
        for entity in entities {
            reservations.insert(
                entity,
                txn.get(&self.handles.reservations, &entity)
                    .map_err(|_| refused())?
                    .cloned(),
            );
        }
        let state = snapshot(
            world,
            i.actor(),
            &input.intent,
            before,
            draft,
            |npc| Ok(contracts.get(&npc).cloned().flatten()),
            |entity| Ok(reservations.get(&entity).cloned().flatten()),
        )
        .map_err(|_| refused())?;
        if state.input != input {
            return Err(refused());
        }
        let record = match (&input.intent.operation, &input.selection) {
            (Operation::Activate { duty, .. }, Selection::Activate { draft }) => ActiveDetachment {
                schema: 1,
                organizer: i.actor(),
                draft: draft.id,
                detachment: detachment_id(reference).map_err(|_| refused())?,
                revision: state
                    .before
                    .as_ref()
                    .map_or(0, |r| r.revision)
                    .checked_add(1)
                    .ok_or_else(refused)?,
                source_revision: draft.revision,
                plan_hash: draft.plan_hash.clone(),
                plan: draft.plan.clone(),
                duty: *duty,
                npc_terms: state.observation.npc_terms,
                activated_at: world.world_ic_now(),
                closed_at: None,
            },
            (Operation::Deactivate { .. }, Selection::Deactivate { .. }) => {
                let mut record = state.before.clone().ok_or_else(refused)?;
                record.closed_at = Some(world.world_ic_now());
                record
            }
            _ => return Err(refused()),
        };
        record.validate().map_err(|_| refused())?;
        for entity in record.entities() {
            if record.active() {
                txn.insert(
                    &self.handles.reservations,
                    &entity,
                    Reservation::for_entity(&record, entity),
                )
            } else {
                txn.remove(&self.handles.reservations, &entity)
            }
            .map_err(|_| refused())?;
        }
        if state.before.is_some() {
            txn.replace(&self.handles.records, &reference, record.clone())
        } else {
            txn.insert(&self.handles.records, &reference, record.clone())
        }
        .map_err(|_| refused())?;
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({
                "organizer": record.organizer,
                "draft": record.draft,
                "detachment": record.detachment,
                "revision": record.revision,
                "active": record.active(),
            }),
        )
        .map_err(|_| refused())?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![i.principal().clone()]))
    }
}

pub(super) fn read(ctx: &Ctx<'_>, reference: DraftRef) -> Result<Option<ActiveDetachment>, String> {
    let handles = ctx
        .w
        .module_state::<ActivationHandles>()
        .ok_or("Detachment activation unavailable.")?;
    let record = validate_record(
        reference,
        ctx.action_state(&handles.records, &reference)
            .map_err(|e| e.to_string())?
            .cloned(),
    )?;
    let participant = record.as_ref().is_some_and(|record| {
        record.plan.slots.iter().any(|slot| {
            matches!(slot.controller, PlannedController::Player { character } if character == ctx.actor)
        })
    });
    if reference.organizer != ctx.actor && !participant {
        return Err("No visible active detachment.".into());
    }
    Ok(record)
}
pub(super) fn submit(ctx: &mut Ctx<'_>, operation: Operation) -> Result<(), String> {
    if ctx.authoritative_pending(ctx.actor)
        || ctx.authoritative_commit_pending()
        || ctx.authoritative_queue_full()
    {
        return Err("An authoritative request is already pending.".into());
    }
    let handles = ctx
        .w
        .module_state::<ActivationHandles>()
        .cloned()
        .filter(|h| h.enabled)
        .ok_or("Detachment activation unavailable.")?;
    let intent = Intent {
        at: ctx.w.now(),
        operation,
    };
    detachment_id(DraftRef {
        organizer: ctx.actor,
        id: intent.operation.id(),
    })?;
    let queue = ctx
        .w
        .module_state_mut::<Pending>()
        .ok_or("Detachment activation unavailable.")?;
    queue.0.retain(|_, input| input.current(intent.at));
    if queue.0.len() >= MAX_PENDING && !queue.0.contains_key(&ctx.actor) {
        return Err("Detachment preparation is busy.".into());
    }
    queue.0.insert(ctx.actor, intent.clone());
    let digest = fingerprint(ctx.actor, &intent)
        .map_err(|_| "Invalid detachment input.")?
        .to_hex();
    let idempotency =
        IdempotencyKeyText::parse_canonical(format!("detachment-activation/{digest}"))
            .map_err(|_| "Invalid detachment request.")?;
    ctx.submit_authoritative(
        handles.action,
        PerceivedRef::SelfActor,
        Vec::new(),
        json!({"request":digest}),
        idempotency,
    );
    Ok(())
}

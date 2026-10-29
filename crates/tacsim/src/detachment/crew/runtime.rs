//! One typed contract per NPC, committed through the ordinary coordinator.
use super::{CrewContract, HireOffer, MAX_CONTRACT_BYTES, offers};
use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, ActionTxn,
    AuthoritativeActionOutput, CanonicalHash, CanonicalId, ClockStamp, DefinitionDescriptor,
    DefinitionId, DefinitionKind, DefinitionRef, Engine, LegalActionQuery, ModError, ObjId,
    OwnerId, PerceivedRef, PrepareCtx, PrincipalId, ProjectedParameter, ProjectedParameterKind,
    ProviderOfferDraft, Registrar, RevisionKey, RevisionKind, RevisionSet, RevisionStamp,
    RevisionValidator, RevisionValidatorContext, RulesValue, StateFamilyHandle, SubjectKey, World,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const ACTION: &str = "tacsim_crew_hire";
pub const OUTCOME: &str = "tacsim_crew_hire_result";
pub const FAMILY: &str = "tacsim.crew_contracts";
const REVISION: &str = "crew_hiring_presence";
#[derive(Clone, Debug)]
pub struct CrewHandles {
    pub contracts: StateFamilyHandle<ObjId, CrewContract>,
    pub action: DefinitionRef,
}
fn bad(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
fn refused() -> ActionError {
    crate::port_tick::port_error(
        ActionErrorCode::ProviderRejected,
        "NPC hire or terms unavailable",
    )
}
fn descriptor(kind: &str, id: &str, payload: Value) -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new(kind).map_err(bad)?,
        DefinitionId::new(id).map_err(bad)?,
        1,
        payload,
    )
    .map_err(bad)
}
pub fn outcome_descriptor() -> Result<DefinitionDescriptor, ModError> {
    descriptor(
        "outcome",
        OUTCOME,
        json!({"format":1,"fields":["npc","employer","terms_hash"]}),
    )
}
pub fn action_descriptor(
    outcomes: &BTreeMap<String, DefinitionRef>,
) -> Result<DefinitionDescriptor, ModError> {
    let outcome = outcomes
        .get(OUTCOME)
        .cloned()
        .ok_or_else(|| bad("crew outcome missing"))?;
    descriptor(
        "action",
        ACTION,
        json!({"format":1,"subject":"self_actor","targets":0,"family":FAMILY}),
    )
    .map(|d| d.as_current().with_references(vec![outcome]))
}
pub fn register(
    reg: &mut Registrar<'_>,
    actions: &BTreeMap<String, DefinitionRef>,
    outcomes: &BTreeMap<String, DefinitionRef>,
) -> Result<CrewHandles, ModError> {
    let family = crate::port::families::family_descriptor(
        FAMILY,
        "tacsim.crew_contracts.key.v1",
        "tacsim.crew_contracts",
    )?
    .with_limits(16, MAX_CONTRACT_BYTES)
    .map_err(bad)?;
    let contracts = reg.state_family(
        family,
        |npc: &ObjId| Ok(npc.0.to_string()),
        |v: &Value| serde_json::from_value(v.clone()).map_err(|e| e.to_string()),
        CrewContract::validate,
        |v: &CrewContract| serde_json::to_value(v).map_err(|e| e.to_string()),
    )?;
    let action = actions
        .get(ACTION)
        .cloned()
        .ok_or_else(|| bad("crew action missing"))?;
    let outcome = outcomes
        .get(OUTCOME)
        .cloned()
        .ok_or_else(|| bad("crew outcome missing"))?;
    reg.action_provider(
        action.clone(),
        Provider {
            action: action.clone(),
            outcome,
            contracts: contracts.clone(),
        },
    )?;
    reg.revision_validator(
        RevisionKind::new(CanonicalId::new(REVISION).map_err(bad)?),
        Validator {
            contracts: contracts.clone(),
        },
    )?;
    let handles = CrewHandles { contracts, action };
    reg.state(handles.clone());
    Ok(handles)
}
pub(super) fn check_contract(npc: ObjId, contract: Option<&CrewContract>) -> Result<bool, String> {
    if let Some(contract) = contract {
        contract.validate()?;
        if contract.npc != npc {
            return Err("NPC contract identity mismatch.".into());
        }
    }
    Ok(contract.is_some())
}
fn key(actor: ObjId) -> Result<RevisionKey, ActionError> {
    Ok(RevisionKey {
        owner: OwnerId::new(crate::NS).map_err(|_| refused())?,
        kind: RevisionKind::new(CanonicalId::new(REVISION).map_err(|_| refused())?),
        subject: SubjectKey::Object { id: actor },
        facet: None,
    })
}
fn generation(
    world: &World,
    actor: ObjId,
    offered: Result<Vec<HireOffer>, String>,
) -> Result<u64, ActionError> {
    let hash = CanonicalHash::for_canonical_value(
        "tacsim.crew_presence.v1",
        1,
        &json!({"actor":actor,"room":world.get(actor).and_then(|o| o.location),
            "clock":world.world_ic_now().clock,"offers":offered.ok()}),
        128 * 1024,
    )
    .map_err(|_| refused())?;
    let bytes: [u8; 8] = hash
        .as_bytes()
        .get(..8)
        .and_then(|b| b.try_into().ok())
        .ok_or_else(refused)?;
    Ok(u64::from_be_bytes(bytes))
}
struct Validator {
    contracts: StateFamilyHandle<ObjId, CrewContract>,
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
        let offered = offers(ctx.world(), actor, |npc| {
            check_contract(
                npc,
                ctx.action_state(&self.contracts, &npc)
                    .map_err(|e| e.to_string())?,
            )
        });
        generation(ctx.world(), actor, offered).map(Some)
    }
}
struct Provider {
    action: DefinitionRef,
    outcome: DefinitionRef,
    contracts: StateFamilyHandle<ObjId, CrewContract>,
}
fn chosen(invocation: &ActionInvocation) -> Result<(HireOffer, ClockStamp), ActionError> {
    if invocation.subject() != &PerceivedRef::SelfActor
        || !invocation.targets().is_empty()
        || invocation.principal()
            != &(PrincipalId::Object {
                id: invocation.actor(),
            })
    {
        return Err(refused());
    }
    let p = invocation
        .parameters()
        .as_object()
        .filter(|p| p.len() == 4)
        .ok_or_else(refused)?;
    let npc = p.get("npc").and_then(Value::as_str).ok_or_else(refused)?;
    let all = p
        .get("offers")
        .and_then(Value::as_object)
        .filter(|o| o.len() <= 32)
        .ok_or_else(refused)?;
    let offer: HireOffer = serde_json::from_value(all.get(npc).cloned().ok_or_else(refused)?)
        .map_err(|_| refused())?;
    if offer.npc.0.to_string() != npc
        || p.get("terms").and_then(Value::as_str) != Some(offer.terms_hash.to_hex().as_str())
    {
        return Err(refused());
    }
    let stamp =
        serde_json::from_value(p.get("at").cloned().ok_or_else(refused)?).map_err(|_| refused())?;
    Ok((offer, stamp))
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        chosen(invocation).map(|_| ())
    }
    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &LegalActionQuery,
        engine: &Engine,
    ) -> Result<Option<ProviderOfferDraft>, ActionError> {
        if action != &self.action
            || query.projected_subject() != &PerceivedRef::SelfActor
            || query.truth_subject() != &(SubjectKey::Object { id: query.actor() })
        {
            return Ok(None);
        }
        let offered = offers(&engine.world, query.actor(), |npc| {
            check_contract(
                npc,
                engine
                    .action_state()
                    .get(&self.contracts, &npc)
                    .map_err(|e| e.to_string())?,
            )
        });
        let Ok(rows) = &offered else { return Ok(None) };
        if rows.is_empty() {
            return Ok(None);
        }
        let fixed: BTreeMap<_, _> = rows
            .iter()
            .map(|offer| (offer.npc.0.to_string(), offer))
            .collect();
        let choices = fixed.keys().cloned().collect();
        let revisions = RevisionSet::from_stamps([RevisionStamp {
            key: key(query.actor())?,
            generation: generation(&engine.world, query.actor(), offered.clone())?,
        }])
        .map_err(|_| refused())?;
        crate::surface::act::offer_draft_with_revisions(
            action,
            engine,
            json!({"offers":fixed,"at":engine.world.world_ic_now()}),
            vec![
                ProjectedParameter {
                    id: CanonicalId::new("npc").map_err(|_| refused())?,
                    required: true,
                    kind: ProjectedParameterKind::Choice { choices },
                },
                ProjectedParameter {
                    id: CanonicalId::new("terms").map_err(|_| refused())?,
                    required: true,
                    kind: ProjectedParameterKind::Text { maximum_bytes: 64 },
                },
            ],
            (
                "crew-hire",
                "Hire an NPC crew member",
                "Accept the NPC's displayed port hiring terms.",
            ),
            revisions,
        )
        .map(Some)
    }
    fn prepare_authoritative_with_context(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let (offer, stamp) = chosen(invocation)?;
        let world = ctx.world().map_err(|_| refused())?;
        let current = offers(world, invocation.actor(), |npc| {
            check_contract(
                npc,
                txn.get(&self.contracts, &npc).map_err(|e| e.to_string())?,
            )
        })
        .map_err(|_| refused())?;
        if !current.contains(&offer)
            || stamp.clock != world.world_ic_now().clock
            || stamp.at > world.world_ic_now().at
        {
            return Err(refused());
        }
        let contract = CrewContract {
            schema: 1,
            npc: offer.npc,
            employer: invocation.actor(),
            hired_at: stamp,
            terms: offer.terms,
            terms_hash: offer.terms_hash,
        };
        contract.validate().map_err(|_| refused())?;
        txn.insert(&self.contracts, &contract.npc, contract.clone())
            .map_err(|_| refused())?;
        let outcome = RulesValue::new(self.outcome.clone(), json!({"npc":contract.npc,"employer":contract.employer,"terms_hash":contract.terms_hash})).map_err(|_| refused())?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

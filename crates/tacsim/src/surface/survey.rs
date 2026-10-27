//! Funded survey commissions: bounded work receipts, independent of chart retention.
//!
//! A command authorizes one exact attempt in RAM; the authoritative acceptance reserves
//! its board slot and gateway-account slot before Occator funds it. Only successful new
//! soundings on a funded attempt add coverage, in the chart's own transaction.

use std::collections::BTreeMap;

use mush_core::mechanics::OwnerId;
use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, ActionTxn,
    AuthoritativeActionOutput, CanonicalHash, CanonicalId, Ctx, DefinitionRef, Engine,
    IdempotencyKeyText, ModError, ObjId, ProjectedParameterKind, ProviderOfferDraft, Registrar,
    RevisionKey, RevisionKind, RevisionSet, RevisionStamp, RevisionValidator,
    RevisionValidatorContext, RulesValue, StateFamilyHandle, SubjectKey, World,
};
use mush_org::PrincipalId;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::act::{offer_draft_with_revisions, parameter, self_query};
use super::address::{pocket_in_block, pockets_in_reach};
use super::excursion::{Act, ActParams};
use crate::port_tick::port_error;

pub const SURVEY_FAMILY: &str = "tacsim.survey";
pub const SURVEY_SCHEMA: u16 = 1;
pub const SURVEY_ACTION: &str = "tacsim_survey_accept";
pub const SURVEY_OUTCOME: &str = "tacsim_survey_accept_result";
pub const SURVEY_CANCEL_ACTION: &str = "tacsim_survey_cancel";
pub const SURVEY_CANCEL_OUTCOME: &str = "tacsim_survey_cancel_result";
pub const MAX_SURVEY_RECORD_BYTES: usize = 4096;
const REVISION: &str = "survey_world";
const MAX_ORDERS: usize = 128;

/// A key from the finite authored board; no player can allocate another offer.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SurveyKey(String);

impl SurveyKey {
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        crate::port::survey::offer(text).map(|_| Self(text.to_owned()))
    }

    #[must_use]
    pub fn key_text(&self) -> String {
        self.0.clone()
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The successful sounding that completed the last missing pocket.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurveyCompletion {
    pub tv: ObjId,
    pub ordinal: u64,
    pub at_ic: i64,
    pub at_utc: i64,
}

/// The contractor's committed request to return the reserved reward to Occator.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurveyCancellation {
    pub at_ic: i64,
    pub at_utc: i64,
}

/// One world's current attempt on one authored offer. Completed rows are permanent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurveyRecord {
    pub schema: u16,
    pub offer: SurveyKey,
    pub revision: u64,
    pub account: mush_core::AccountId,
    pub pilot: ObjId,
    pub sponsor: PrincipalId,
    pub sponsor_account: u64,
    pub payee: u64,
    pub contract: u64,
    pub site: CanonicalId,
    /// A=0 through P=15.
    pub block: u8,
    pub reward_minor: i64,
    pub accepted_ic: i64,
    pub deadline_ic: i64,
    /// Pocket-number order, with each byte's low bit first.
    pub coverage: [u8; 32],
    pub completed: Option<SurveyCompletion>,
    /// Contractor cancellation is retained until the contract refunds its sponsor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancelled: Option<SurveyCancellation>,
}

impl SurveyRecord {
    #[must_use]
    pub fn covered(&self) -> u16 {
        self.coverage
            .iter()
            .map(|byte| byte.count_ones() as u16)
            .sum()
    }

    /// Validate bounded shape and the frozen authored terms, never chart contents.
    pub fn check(&self) -> Result<(), String> {
        let offer = crate::port::survey::offer(self.offer.as_str())
            .ok_or_else(|| "the survey offer is not on the board".to_owned())?;
        if self.schema != SURVEY_SCHEMA
            || self.revision != offer.revision
            || self.site.as_str() != offer.site
            || self.block != offer.block
            || self.reward_minor != offer.reward_minor
            || self.reward_minor <= 0
            || self.accepted_ic.checked_add(offer.term_ic_s) != Some(self.deadline_ic)
            || self.deadline_ic <= self.accepted_ic
            || self.pilot.0 == 0
            || !matches!(self.sponsor, PrincipalId::Organization(_))
        {
            return Err("the survey attempt's terms are invalid".to_owned());
        }
        if self.completed.is_some() != (self.covered() == 256) {
            return Err("survey completion must cover all 256 pockets".to_owned());
        }
        if self
            .cancelled
            .as_ref()
            .is_some_and(|at| at.at_ic < self.accepted_ic)
            || (self.cancelled.is_some() && self.completed.is_some())
        {
            return Err("a survey cancellation must precede completion".to_owned());
        }
        if self.completed.as_ref().is_some_and(|done| {
            done.tv.0 == 0
                || done.ordinal == 0
                || done.at_ic < self.accepted_ic
                || done.at_ic > self.deadline_ic
        }) {
            return Err("the survey completion is outside its attempt".to_owned());
        }
        Ok(())
    }
}

#[derive(Clone, Default)]
struct SurveyOrders(BTreeMap<ObjId, SurveyRecord>);

/// Bind an authenticated command's exact accepted, still-unfunded contract to its action.
pub(crate) fn put_order(ctx: &mut Ctx<'_>, record: SurveyRecord) -> Result<(), String> {
    record.check()?;
    if record.pilot != ctx.actor
        || record.covered() != 0
        || record.cancelled.is_some()
        || crate::port::licence::gateway_account(ctx.w, ctx.actor) != Some(record.account)
        || !order_valid(ctx.w, &record)
    {
        return Err("the survey contract does not match your acceptance".to_owned());
    }
    let stale: Vec<_> = ctx
        .w
        .module_state::<SurveyOrders>()
        .into_iter()
        .flat_map(|orders| orders.0.iter())
        .filter(|(_, row)| row.deadline_ic < ctx.w.world_ic_now().at || !order_valid(ctx.w, row))
        .map(|(actor, _)| *actor)
        .collect();
    let orders = ctx
        .w
        .module_state_mut::<SurveyOrders>()
        .ok_or_else(|| "survey acceptance is unavailable".to_owned())?;
    for actor in stale {
        orders.0.remove(&actor);
    }
    if orders.0.len() >= MAX_ORDERS && !orders.0.contains_key(&ctx.actor) {
        return Err("survey acceptance is busy; retry shortly".to_owned());
    }
    orders.0.insert(ctx.actor, record);
    Ok(())
}

/// Drop only the order whose durable acceptance the command's pull tick observed.
pub(crate) fn forget_order(w: &mut World, pilot: ObjId, contract: u64) {
    if let Some(orders) = w.module_state_mut::<SurveyOrders>()
        && orders
            .0
            .get(&pilot)
            .is_some_and(|row| row.contract == contract)
    {
        orders.0.remove(&pilot);
    }
}

#[must_use]
pub fn accept_key(offer: &str, contract: u64) -> Option<IdempotencyKeyText> {
    SurveyKey::parse(offer)?;
    IdempotencyKeyText::parse_canonical(format!("tacsim.survey/{offer}/{contract}/accept")).ok()
}

fn error(message: &'static str) -> ActionError {
    port_error(ActionErrorCode::Integrity, message)
}

/// A filed order cannot revive a cancelled contract or reserve an already-funded deal.
fn order_valid(w: &World, row: &SurveyRecord) -> bool {
    crate::port::survey::contract_valid(w, row)
        && w.module_state::<mush_economy::EconomyStore>()
            .and_then(|store| {
                store
                    .snapshot()
                    .contract(mush_economy::ContractId(row.contract))
            })
            .is_some_and(|contract| {
                contract.state == mush_economy::ContractState::Offered
                    && contract.pending.is_none()
                    && contract.escrow.is_empty()
            })
}

fn keys() -> impl Iterator<Item = SurveyKey> {
    crate::port::survey::offers()
        .iter()
        .filter_map(|offer| SurveyKey::parse(offer.id))
}

fn check_row(key: &SurveyKey, row: &SurveyRecord) -> Result<(), ActionError> {
    if &row.offer != key || row.check().is_err() {
        return Err(error("the survey record does not match its board slot"));
    }
    Ok(())
}

fn engine_rows(
    engine: &Engine,
    family: &StateFamilyHandle<SurveyKey, SurveyRecord>,
) -> Result<Vec<Option<SurveyRecord>>, ActionError> {
    keys()
        .map(|key| {
            let row = engine
                .action_state()
                .get(family, &key)
                .map_err(|_| error("the survey board cannot be read"))?
                .cloned();
            if let Some(row) = &row {
                check_row(&key, row)?;
            }
            Ok(row)
        })
        .collect()
}

fn generation(
    w: &World,
    actor: ObjId,
    mode: &str,
    rows: &[Option<SurveyRecord>],
) -> Result<u64, ActionError> {
    let order = (mode == "accept")
        .then(|| {
            w.module_state::<SurveyOrders>()
                .and_then(|orders| orders.0.get(&actor))
        })
        .flatten();
    let states: Vec<_> = rows
        .iter()
        .flatten()
        .filter(|row| mode == "accept" || row.pilot == actor)
        .map(|row| {
            json!({
                "row": row,
                "active": crate::port::survey::contract_active(w, row),
                "replaceable": crate::port::survey::contract_replaceable(w, row),
            })
        })
        .collect();
    let value = json!({
        "enabled": w.module_state::<crate::store::TacsimStore>().is_some_and(crate::store::TacsimStore::enabled),
        "account": crate::port::licence::gateway_account(w, actor),
        "order": order,
        "order_valid": order.is_some_and(|row| order_valid(w, row)),
        "states": states,
    });
    let hash = CanonicalHash::for_canonical_value("tacsim.survey_world.v1", 1, &value, 32768)
        .map_err(|_| error("the survey revision cannot be encoded"))?;
    let text = hash.to_hex();
    let prefix = text
        .get(..16)
        .ok_or_else(|| error("the survey revision is malformed"))?;
    u64::from_str_radix(prefix, 16).map_err(|_| error("the survey revision is malformed"))
}

fn revision(actor: ObjId, mode: &str, generation: u64) -> Result<RevisionSet, ActionError> {
    let id = |text| CanonicalId::new(text).map_err(|_| error("the survey revision is invalid"));
    RevisionSet::from_stamps([RevisionStamp {
        key: RevisionKey {
            owner: OwnerId::new(crate::NS).map_err(|_| error("the survey owner is invalid"))?,
            kind: RevisionKind::new(id(REVISION)?),
            subject: SubjectKey::Object { id: actor },
            facet: Some(id(mode)?),
        },
        generation,
    }])
    .map_err(|_| error("the survey revision is invalid"))
}

struct SurveyValidator(StateFamilyHandle<SurveyKey, SurveyRecord>);

impl RevisionValidator for SurveyValidator {
    fn current_generation(
        &self,
        ctx: &RevisionValidatorContext<'_>,
        key: &RevisionKey,
    ) -> Result<Option<u64>, ActionError> {
        let SubjectKey::Object { id: actor } = key.subject else {
            return Ok(None);
        };
        let Some(mode) = key
            .facet
            .as_ref()
            .map(CanonicalId::as_str)
            .filter(|mode| matches!(*mode, "accept" | "reading" | "cancel"))
        else {
            return Ok(None);
        };
        let rows: Result<Vec<_>, _> = keys()
            .map(|key| {
                let row = ctx
                    .action_state(&self.0, &key)
                    .map_err(|_| error("the survey board cannot be read"))?
                    .cloned();
                if let Some(row) = &row {
                    check_row(&key, row)?;
                }
                Ok(row)
            })
            .collect();
        generation(ctx.world(), actor, mode, &rows?).map(Some)
    }
}

pub(crate) fn register(
    reg: &mut Registrar<'_>,
    family: StateFamilyHandle<SurveyKey, SurveyRecord>,
) -> Result<(), ModError> {
    reg.state(SurveyOrders::default());
    reg.revision_validator(
        RevisionKind::new(CanonicalId::new(REVISION).map_err(|e| ModError::msg(e.to_string()))?),
        SurveyValidator(family),
    )
}

pub(crate) struct SurveyProvider {
    pub action: DefinitionRef,
    pub outcome: DefinitionRef,
    pub surveys: StateFamilyHandle<SurveyKey, SurveyRecord>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Acceptance {
    record: SurveyRecord,
    previous: Vec<Option<SurveyRecord>>,
    replaceable: Vec<SurveyKey>,
}

impl ActionProvider for SurveyProvider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        if invocation.subject() != &mush_core::PerceivedRef::SelfActor
            || !invocation.targets().is_empty()
            || invocation
                .parameters()
                .get("offer")
                .and_then(Value::as_str)
                .and_then(SurveyKey::parse)
                .is_none()
            || invocation
                .parameters()
                .get("contract")
                .and_then(Value::as_u64)
                .is_none()
        {
            return Err(port_error(
                ActionErrorCode::InvalidRequest,
                "a survey acceptance names its offer and contract",
            ));
        }
        Ok(())
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &Engine,
    ) -> Result<Option<ProviderOfferDraft>, ActionError> {
        if !self_query(action, &self.action, query) {
            return Ok(None);
        }
        let w = &engine.world;
        if !w
            .module_state::<crate::store::TacsimStore>()
            .is_some_and(crate::store::TacsimStore::enabled)
        {
            return Ok(None);
        }
        let actor = query.actor();
        let Some(record) = w
            .module_state::<SurveyOrders>()
            .and_then(|orders| orders.0.get(&actor))
            .cloned()
        else {
            return Ok(None);
        };
        if record.pilot != actor
            || record.check().is_err()
            || crate::port::licence::gateway_account(w, actor) != Some(record.account)
            || !order_valid(w, &record)
            || w.world_ic_now().at > record.deadline_ic
        {
            return Ok(None);
        }
        let previous = engine_rows(engine, &self.surveys)?;
        let replaceable = previous
            .iter()
            .flatten()
            .filter(|row| crate::port::survey::contract_replaceable(w, row))
            .map(|row| row.offer.clone())
            .collect();
        let stamp = generation(w, actor, "accept", &previous)?;
        let accepted = Acceptance {
            record,
            previous,
            replaceable,
        };
        let fixed = json!({"accepted": accepted});
        let parameters = vec![
            parameter(
                "offer",
                true,
                ProjectedParameterKind::Choice {
                    choices: vec![accepted.record.offer.key_text()],
                },
            )?,
            parameter(
                "contract",
                true,
                ProjectedParameterKind::Integer {
                    minimum: 0,
                    maximum: i64::MAX,
                },
            )?,
        ];
        offer_draft_with_revisions(
            action,
            engine,
            fixed,
            parameters,
            (
                "tacsim-survey-accept",
                "Accept a survey commission",
                "Reserve an exact funded-work offer.",
            ),
            revision(actor, "accept", stamp)?,
        )
        .map(Some)
    }

    fn prepare_authoritative(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.validate_invocation(invocation)?;
        let accepted: Acceptance = invocation
            .parameters()
            .get("accepted")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .ok_or_else(|| error("the survey acceptance has no command order"))?;
        let record = accepted.record;
        record
            .check()
            .map_err(|_| error("the survey acceptance is invalid"))?;
        if record.pilot != invocation.actor()
            || record.covered() != 0
            || record.cancelled.is_some()
            || invocation.parameters().get("offer").and_then(Value::as_str)
                != Some(record.offer.as_str())
            || invocation
                .parameters()
                .get("contract")
                .and_then(Value::as_u64)
                != Some(record.contract)
            || accepted.previous.len() != crate::port::survey::offers().len()
        {
            return Err(port_error(
                ActionErrorCode::Unauthorized,
                "the survey order is another acceptance",
            ));
        }
        let mut existed = false;
        for (key, before) in keys().zip(&accepted.previous) {
            let current = txn
                .get(&self.surveys, &key)
                .map_err(|_| error("the survey board cannot be read"))?
                .cloned();
            if &current != before {
                return Err(port_error(
                    ActionErrorCode::Conflict,
                    "the survey board changed; retry",
                ));
            }
            if let Some(current) = current {
                check_row(&key, &current)?;
                let reusable = current.completed.is_none() && accepted.replaceable.contains(&key);
                if key == record.offer {
                    existed = true;
                    if !reusable {
                        return Err(port_error(
                            ActionErrorCode::Conflict,
                            "that survey offer is already reserved",
                        ));
                    }
                }
                if current.account == record.account && current.completed.is_none() && !reusable {
                    return Err(port_error(
                        ActionErrorCode::Conflict,
                        "your account already has a survey commission",
                    ));
                }
            }
        }
        let offer = record.offer.clone();
        let contract = record.contract;
        if existed {
            txn.replace(&self.surveys, &offer, record)
        } else {
            txn.insert(&self.surveys, &offer, record)
        }
        .map_err(|_| error("the survey acceptance did not stage"))?;
        let output = RulesValue::new(
            self.outcome.clone(),
            json!({
                "actor": invocation.actor().0, "offer": offer.key_text(), "contract": contract,
            }),
        )
        .map_err(|_| error("the survey outcome is too large"))?;
        Ok(AuthoritativeActionOutput::new(output)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

/// The contractor may return funded escrow through a retained cancellation fact.
pub(crate) struct CancelProvider {
    pub action: DefinitionRef,
    pub outcome: DefinitionRef,
    pub surveys: StateFamilyHandle<SurveyKey, SurveyRecord>,
}

fn cancel_reference(invocation: &ActionInvocation) -> Result<(SurveyKey, u64), ActionError> {
    if invocation.subject() != &mush_core::PerceivedRef::SelfActor
        || !invocation.targets().is_empty()
    {
        return Err(port_error(
            ActionErrorCode::InvalidRequest,
            "survey cancellation is self-scoped",
        ));
    }
    let offer = invocation
        .parameters()
        .get("offer")
        .and_then(Value::as_str)
        .and_then(SurveyKey::parse)
        .ok_or_else(|| error("survey cancellation names its offer"))?;
    let contract = invocation
        .parameters()
        .get("contract")
        .and_then(Value::as_u64)
        .ok_or_else(|| error("survey cancellation names its contract"))?;
    Ok((offer, contract))
}

/// Separate from acceptance so neither operation can replay as the other.
#[must_use]
pub fn cancel_key(offer: &str, contract: u64) -> Option<IdempotencyKeyText> {
    SurveyKey::parse(offer)?;
    IdempotencyKeyText::parse_canonical(format!("tacsim.survey/{offer}/{contract}/cancel")).ok()
}

impl ActionProvider for CancelProvider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        cancel_reference(invocation).map(|_| ())
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &Engine,
    ) -> Result<Option<ProviderOfferDraft>, ActionError> {
        if !self_query(action, &self.action, query)
            || !engine
                .world
                .module_state::<crate::TacsimStore>()
                .is_some_and(crate::TacsimStore::enabled)
        {
            return Ok(None);
        }
        let actor = query.actor();
        let rows = engine_rows(engine, &self.surveys)?;
        let candidates: Vec<_> = rows
            .iter()
            .flatten()
            .filter(|row| {
                row.pilot == actor
                    && row.completed.is_none()
                    && row.cancelled.is_none()
                    && crate::port::survey::contract_active(&engine.world, row)
            })
            .cloned()
            .collect();
        if candidates.is_empty() {
            return Ok(None);
        }
        let stamp = generation(&engine.world, actor, "cancel", &rows)?;
        let fixed = json!({"cancellations": candidates,
            "now": engine.world.world_ic_now().at, "wall": engine.world.now()});
        let parameters = vec![
            parameter(
                "offer",
                true,
                ProjectedParameterKind::Choice {
                    choices: candidates.iter().map(|row| row.offer.key_text()).collect(),
                },
            )?,
            parameter(
                "contract",
                true,
                ProjectedParameterKind::Integer {
                    minimum: 0,
                    maximum: i64::MAX,
                },
            )?,
        ];
        offer_draft_with_revisions(
            action,
            engine,
            fixed,
            parameters,
            (
                "tacsim-survey-cancel",
                "Cancel a survey commission",
                "Return an unfinished commission's reward to Occator.",
            ),
            revision(actor, "cancel", stamp)?,
        )
        .map(Some)
    }

    fn prepare_authoritative(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let (offer, contract) = cancel_reference(invocation)?;
        let candidates: Vec<SurveyRecord> = invocation
            .parameters()
            .get("cancellations")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .ok_or_else(|| error("survey cancellation fixes its funded attempts"))?;
        if candidates.len() > crate::port::survey::offers().len() {
            return Err(error("survey cancellation is out of bounds"));
        }
        let mut row = candidates
            .into_iter()
            .find(|row| row.offer == offer && row.contract == contract)
            .ok_or_else(|| {
                port_error(
                    ActionErrorCode::Unauthorized,
                    "that survey is not yours to cancel",
                )
            })?;
        check_row(&offer, &row)?;
        if row.pilot != invocation.actor() || row.completed.is_some() || row.cancelled.is_some() {
            return Err(port_error(
                ActionErrorCode::Unauthorized,
                "that survey cannot be cancelled",
            ));
        }
        let actual = txn
            .get(&self.surveys, &offer)
            .map_err(|_| error("the survey cannot be read"))?;
        if actual != Some(&row) {
            return Err(port_error(
                ActionErrorCode::Conflict,
                "that survey has changed; retry",
            ));
        }
        let at_ic = invocation
            .parameters()
            .get("now")
            .and_then(Value::as_i64)
            .ok_or_else(|| error("survey cancellation fixes its IC instant"))?;
        let at_utc = invocation
            .parameters()
            .get("wall")
            .and_then(Value::as_i64)
            .ok_or_else(|| error("survey cancellation fixes its wall instant"))?;
        row.cancelled = Some(SurveyCancellation { at_ic, at_utc });
        row.check()
            .map_err(|_| error("the survey cancellation is invalid"))?;
        txn.replace(&self.surveys, &offer, row)
            .map_err(|_| error("the survey cancellation did not stage"))?;
        let output = RulesValue::new(
            self.outcome.clone(),
            json!({
                "actor": invocation.actor().0, "offer": offer.key_text(), "contract": contract,
            }),
        )
        .map_err(|_| error("the survey cancellation outcome is too large"))?;
        Ok(AuthoritativeActionOutput::new(output)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

/// A server-authored snapshot of the funded attempts this reading can advance.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadingOffer {
    eligible: Vec<(SurveyKey, u64)>,
    unavailable: bool,
}

fn unavailable_reading() -> (ReadingOffer, RevisionSet) {
    (
        ReadingOffer {
            eligible: Vec::new(),
            unavailable: true,
        },
        RevisionSet::default(),
    )
}

pub(crate) fn reading_offer(
    engine: &Engine,
    family: &StateFamilyHandle<SurveyKey, SurveyRecord>,
    actor: ObjId,
) -> Result<(ReadingOffer, RevisionSet), ActionError> {
    // The ancillary board must never prevent a rig from recovering, unloading or
    // finishing a mining cycle. A sounding alone needs this proof to finish.
    let Ok(rows) = engine_rows(engine, family) else {
        return Ok(unavailable_reading());
    };
    if !rows.iter().flatten().any(|row| row.pilot == actor) {
        // A newly accepted commission cannot qualify an already-started sounding.
        // No unrelated board changes need invalidate this operator's finishes.
        return Ok((
            ReadingOffer {
                eligible: Vec::new(),
                unavailable: false,
            },
            RevisionSet::default(),
        ));
    }
    let eligible = rows
        .iter()
        .flatten()
        .filter(|row| {
            row.pilot == actor
                && row.completed.is_none()
                && row.cancelled.is_none()
                && crate::port::survey::contract_active(&engine.world, row)
        })
        .map(|row| (row.offer.clone(), row.contract))
        .collect();
    let Ok(stamp) = generation(&engine.world, actor, "reading", &rows) else {
        return Ok(unavailable_reading());
    };
    Ok((
        ReadingOffer {
            eligible,
            unavailable: false,
        },
        revision(actor, "reading", stamp)?,
    ))
}

/// Called only after the exact sounding's chart write succeeded, in the same bundle.
pub(crate) fn log_reading(
    txn: &mut ActionTxn<'_>,
    family: &StateFamilyHandle<SurveyKey, SurveyRecord>,
    offered: &ReadingOffer,
    act: &Act,
    tv: ObjId,
    at_utc: i64,
) -> Result<(), ActionError> {
    let ActParams::Sound { cell, radius, .. } = &act.params else {
        return Ok(());
    };
    if offered.unavailable {
        return Err(error(
            "the survey board is unavailable; the sounding will retry",
        ));
    }
    if offered.eligible.len() > crate::port::survey::offers().len() {
        return Err(error("the sounding's survey offers are out of bounds"));
    }
    for (key, contract) in &offered.eligible {
        let Some(mut row) = txn
            .get(family, key)
            .map_err(|_| error("the survey record cannot be read"))?
            .cloned()
        else {
            continue;
        };
        check_row(key, &row)?;
        if row.contract != *contract
            || row.completed.is_some()
            || row.cancelled.is_some()
            || row.pilot != act.operator
            || row.site != act.site
            // IC seconds do not order two commands inside the same instant. Refuse that
            // boundary rather than count work that may have begun before acceptance.
            || act.started_ic <= row.accepted_ic
            || act.due_ic > row.deadline_ic
        {
            continue;
        }
        let before = row.coverage;
        let letter = char::from(b'A'.saturating_add(row.block));
        for (px, py) in pockets_in_reach((cell[0], cell[1]), i64::from(*radius)) {
            let Some((block, index)) = pocket_in_block(px, py) else {
                continue;
            };
            if block == letter {
                let byte = row
                    .coverage
                    .get_mut(index / 8)
                    .ok_or_else(|| error("the sounded pocket is outside its block"))?;
                *byte |= 1_u8 << (index % 8);
            }
        }
        if row.coverage == before {
            continue;
        }
        if row.covered() == 256 {
            row.completed = Some(SurveyCompletion {
                tv,
                ordinal: u64::from(act.ordinal),
                at_ic: act.due_ic,
                at_utc,
            });
        }
        row.check()
            .map_err(|_| error("the survey progress is invalid"))?;
        txn.replace(family, key, row)
            .map_err(|_| error("the survey progress did not stage"))?;
    }
    Ok(())
}

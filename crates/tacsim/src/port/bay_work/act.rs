//! **The bay's inventory acts** (doc 36 cut GC.8b; Q-36-36, Q-36-37, Q-36-39): the one
//! authoritative action, `tacsim_bay_work`, that moves matter for the bay.
//!
//! - **Draw**: a finished order's munitions (Q-36-36) and `industrial_parts` (Q-36-31's
//!   `parts_t`) are debited from the carrier's hold, all or nothing, and a part fitted from a
//!   lot consumes that lot, refused if it is hot.
//! - **Mint**: a stripped part becomes a lot in the carrier's hold carrying its provenance
//!   (Q-36-37); a scrapped hull becomes `structural_alloys` and a lot per salvageable part
//!   (Q-36-39).
//! - **Hot**: a stripped part's provenance is flagged hot when the claim it came under is
//!   vacated (the seam GC.5/GC.6's contest fills; a staff fixture until then).
//!
//! **Two writes, and why they are two** (the mill's D1). The unit's change rides `mush_tactics`'
//! one plan path and the bay record the normal lane, while an inventory pool or lot moves only
//! inside an authoritative bundle, whose prepare has no world. So the command or the port tick
//! writes the record's owed act ([`OwedAct`]) beside the unit change, and the port tick submits
//! it as the actor who owes it until the hull's ledger ([`BayLedger`], family `tacsim.bay_work`)
//! shows its ordinal landed. The ledger is the replay guard: a prepare refuses any ordinal but
//! the ledger's next, so a resubmission after a restart lands once. A refused act is retried
//! [`MAX_ACT_TRIES`] times, then given up: a draw cancels its order (the fee released, nothing
//! drawn), a mint is paged for staff.
//!
//! **Provenance** ([`PartProvenance`], family `tacsim.bay_parts`, keyed by the lot) is written
//! in the mint's own bundle and read by the bay before it fits a lot: the parent hull, its
//! `Missing` slot, the part's condition, the claim reference and its legality.

use std::collections::BTreeMap;

use mush_core::{
    ActionErrorCode, ActionInvocation, ActionProvider, ActionTxn, AuthoritativeActionOutput,
    CanonicalHash, CanonicalId, DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef,
    ModError, ObjId, PerceivedRef, ProjectedParameter, ProjectedParameterKind,
    ProjectedParameterSchema, ProjectedTargetSchema, ProviderOfferDraft, Registrar, RevisionSet,
    RulesValue, StateFamilyHandle, SubjectKey, World,
};
use mush_economy::VaultId;
use mush_inventory::record::EndpointRef;
use mush_inventory::{Grade, InventoryFamilies, LotId};
use mush_tactics::field_ops::{
    ComponentCondition, ContrabandReason, ProvenanceLegalityStatus, SalvageClaimId,
};
use mush_tactics::unit::ComponentId;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::port_tick::port_error;
use crate::store::TacsimStore;

/// The action's id.
pub const BAY_ACT_ACTION: &str = "tacsim_bay_work";
/// Its outcome.
pub const BAY_ACT_OUTCOME: &str = "tacsim_bay_work_result";
/// The ledger family: one [`BayLedger`] per hull.
pub const BAY_WORK_FAMILY: &str = "tacsim.bay_work";
/// The provenance family: one [`PartProvenance`] per stripped part's lot.
pub const BAY_PARTS_FAMILY: &str = "tacsim.bay_parts";
/// A refused act is submitted this many times before it is given up.
pub const MAX_ACT_TRIES: u8 = 3;
/// How many landed acts a ledger keeps, its audit tail.
pub const MAX_RECENT_ACTS: usize = 16;
/// The most goods or lots one act moves.
pub const MAX_ACT_ROWS: usize = 32;
/// The most rows one actor's offer carries.
const MAX_OFFER_ROWS: usize = 16;
/// The lot identity's hash domain.
const LOT_DOMAIN: &str = "tacsim.bay_lot.v1";

// ---- the records --------------------------------------------------------------------------

/// Units of one good.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoodUnits {
    /// The good.
    pub good: CanonicalId,
    /// Units of it (tonnes for bulk, the good's own unit for munitions).
    pub units: u64,
}

/// **A stripped part's provenance** (Q-36-37, Q-F6): written in the bundle that mints its lot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartProvenance {
    /// The lot: the key.
    pub lot: LotId,
    /// The hull it came off.
    pub parent_hull: ObjId,
    /// The parent hull's design.
    pub parent_schema: DefinitionRef,
    /// The slot it left, now `Missing` on the parent.
    pub slot: ComponentId,
    /// The part's definition: it fits only a slot of this definition.
    pub definition: DefinitionRef,
    /// Its condition when it came off (intact, damaged or jammed).
    pub condition: ComponentCondition,
    /// The salvage claim it came off under, when a salvor stripped it.
    pub claim: Option<u64>,
    /// Clean, or hot (Q-F6).
    pub legality: ProvenanceLegalityStatus,
    /// Who took it off.
    pub stripped_by: ObjId,
    /// When, IC.
    pub stripped_at_ic: i64,
    /// Whether it came off in a scrapping, not a strip.
    pub scrapped: bool,
    /// The hull it was fitted into, once fitted (its lot is then consumed).
    pub fitted_into: Option<ObjId>,
}

impl PartProvenance {
    /// Whether lawful workshops refuse it.
    #[must_use]
    pub const fn is_hot(&self) -> bool {
        matches!(
            self.legality,
            ProvenanceLegalityStatus::ContrabandHotPart { .. }
        )
    }

    /// Shape.
    ///
    /// # Errors
    ///
    /// A sentence naming what is wrong.
    pub fn check(&self) -> Result<(), String> {
        if self.parent_hull.0 == 0 || self.stripped_by.0 == 0 {
            return Err("a provenance names a hull and a hand".to_owned());
        }
        if !self.condition.is_salvageable() {
            return Err("a provenance's part is salvageable".to_owned());
        }
        Ok(())
    }
}

/// One lot a mint makes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LotMint {
    /// Its identity ([`lot_id`]).
    pub lot: LotId,
    /// What it is.
    pub commodity: CanonicalId,
    /// How much.
    pub quantity: u64,
    /// Its grade.
    pub grade: u8,
    /// A part's provenance, for a part.
    pub part: Option<PartProvenance>,
}

/// **What an act does.**
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum OwedKind {
    /// Debit an order's goods from the hold, and consume a fitted part's lot.
    Draw {
        /// The order's sequence.
        order: u32,
        /// The goods.
        goods: Vec<GoodUnits>,
        /// The lot a fitted part comes from.
        fit: Option<LotId>,
    },
    /// Mint lots into the hold.
    Mint {
        /// Why: `strip` or `scrap`.
        why: String,
        /// The lots.
        lots: Vec<LotMint>,
    },
    /// Flag a stripped part hot: the claim it came off under was vacated.
    Hot {
        /// The part's lot.
        lot: LotId,
        /// The vacated claim.
        claim: u64,
        /// The rightful owner who alone may clear it.
        rightful_owner: ObjId,
    },
}

impl OwedKind {
    /// The op word.
    #[must_use]
    pub const fn word(&self) -> &'static str {
        match self {
            Self::Draw { .. } => "draw",
            Self::Mint { .. } => "mint",
            Self::Hot { .. } => "hot",
        }
    }
}

/// **An inventory act a hull's record owes** (the two writes, above).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwedAct {
    /// Its ordinal on the hull's ledger.
    pub act: u32,
    /// Who submits it.
    pub by: ObjId,
    /// The hold it draws from or mints into (`VaultId.0`); `None` for a flag.
    pub vault: Option<u64>,
    /// What it does.
    pub kind: OwedKind,
    /// Submissions so far.
    pub tries: u8,
}

impl OwedAct {
    /// Bounds.
    ///
    /// # Errors
    ///
    /// A sentence naming what is wrong.
    pub fn check(&self) -> Result<(), String> {
        let rows = match &self.kind {
            OwedKind::Draw { goods, .. } => goods.len(),
            OwedKind::Mint { lots, .. } => lots.len(),
            OwedKind::Hot { .. } => 0,
        };
        if self.act == 0 || rows > MAX_ACT_ROWS {
            return Err("an owed act has an ordinal and at most 32 rows".to_owned());
        }
        if !matches!(self.kind, OwedKind::Hot { .. }) && self.vault.is_none() {
            return Err("a draw or a mint names its hold".to_owned());
        }
        Ok(())
    }
}

/// One landed act, as the ledger's tail keeps it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LandedAct {
    /// Its ordinal.
    pub act: u32,
    /// `draw`, `mint` or `hot`.
    pub op: String,
    /// Who.
    pub by: ObjId,
    /// When, IC.
    pub at_ic: i64,
}

/// **One hull's ledger** (`tacsim.bay_work`): the last landed ordinal and the audit tail.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BayLedger {
    /// The hull: the key.
    pub hull: ObjId,
    /// The last landed act's ordinal.
    pub acts: u32,
    /// The last [`MAX_RECENT_ACTS`] acts.
    pub recent: Vec<LandedAct>,
}

impl BayLedger {
    /// Bounds.
    ///
    /// # Errors
    ///
    /// A sentence naming what is wrong.
    pub fn check(&self) -> Result<(), String> {
        if self.recent.len() > MAX_RECENT_ACTS || self.acts == 0 {
            return Err("a ledger holds at most 16 recent acts and one landed".to_owned());
        }
        Ok(())
    }
}

/// **A minted lot's identity**: the hull, the act and the row, hashed — derived, never
/// allocated, so a replay names the same lot.
///
/// # Errors
///
/// The hash's own refusal, as text.
pub fn lot_id(hull: ObjId, act: u32, row: usize) -> Result<LotId, String> {
    let hash = CanonicalHash::for_canonical_value(
        LOT_DOMAIN,
        1,
        &json!({"hull": hull.0, "act": act, "row": row}),
        256,
    )
    .map_err(|e| e.to_string())?;
    let mut head = [0u8; 8];
    head.copy_from_slice(&hash.as_bytes()[..8]);
    Ok(LotId(u64::from_be_bytes(head)))
}

// ---- registration -------------------------------------------------------------------------

/// The capabilities the bay reads its families through, and the action when it registered.
#[derive(Clone, Debug)]
pub struct BayActHandles {
    /// The registered action; `None` on a roster without the inventory cell.
    pub action: Option<DefinitionRef>,
    /// `tacsim.bay_work`.
    pub ledgers: StateFamilyHandle<ObjId, BayLedger>,
    /// `tacsim.bay_parts`.
    pub parts: StateFamilyHandle<LotId, PartProvenance>,
    /// The inventory's families, when delegated.
    pub inventory: Option<InventoryFamilies>,
}

fn mod_error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}

fn descriptor(kind: &str, id: &str, payload: Value) -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new(kind).map_err(mod_error)?,
        DefinitionId::new(id).map_err(mod_error)?,
        1,
        payload,
    )
    .map_err(mod_error)
}

/// The outcome's descriptor (`crate::port_tick`'s one outcome validator).
///
/// # Errors
///
/// A descriptor that does not build.
pub fn outcome_descriptor() -> Result<DefinitionDescriptor, ModError> {
    descriptor(
        "outcome",
        BAY_ACT_OUTCOME,
        json!({"format":1,"fields":["actor","hull","act","op"]}),
    )
}

/// The action's descriptor (`crate::port_tick`'s one action validator).
///
/// # Errors
///
/// The outcome unregistered, or a descriptor that does not build.
pub fn action_descriptor(
    outcomes: &BTreeMap<String, DefinitionRef>,
) -> Result<DefinitionDescriptor, ModError> {
    let outcome = outcomes
        .get(BAY_ACT_OUTCOME)
        .cloned()
        .ok_or_else(|| ModError::msg("the bay act's outcome did not register"))?;
    descriptor(
        "action",
        BAY_ACT_ACTION,
        json!({"format":1,"subject":"self_actor","targets":0,"family":BAY_WORK_FAMILY}),
    )
    .map(|d| d.with_references(vec![outcome]))
}

/// **Register the bay's families, its provider and its handles** — the families
/// unconditionally, the provider only with the inventory cell (the mill's rule).
///
/// # Errors
///
/// Any registration the registrar refuses.
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    actions: &BTreeMap<String, DefinitionRef>,
    outcomes: &BTreeMap<String, DefinitionRef>,
    inventory: Option<&InventoryFamilies>,
) -> Result<(), ModError> {
    use crate::port::families::family_descriptor;
    let ledgers = reg.state_family::<ObjId, BayLedger, _, _, _, _>(
        family_descriptor(BAY_WORK_FAMILY, "tacsim.bay_work_key.v1", "tacsim.bay_work")?,
        |key: &ObjId| Ok(key.0.to_string()),
        |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
        |value: &BayLedger| value.check(),
        |value: &BayLedger| serde_json::to_value(value).map_err(|e| e.to_string()),
    )?;
    let parts = reg.state_family::<LotId, PartProvenance, _, _, _, _>(
        family_descriptor(
            BAY_PARTS_FAMILY,
            "tacsim.bay_parts_key.v1",
            "tacsim.bay_parts",
        )?,
        |key: &LotId| Ok(format!("{:016x}", key.0)),
        |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
        |value: &PartProvenance| value.check(),
        |value: &PartProvenance| serde_json::to_value(value).map_err(|e| e.to_string()),
    )?;
    let action = match (actions.get(BAY_ACT_ACTION), inventory) {
        (Some(action), Some(inventory)) => {
            let outcome = outcomes
                .get(BAY_ACT_OUTCOME)
                .cloned()
                .ok_or_else(|| ModError::msg("the bay act's outcome did not register"))?;
            reg.action_provider(
                action.clone(),
                BayActProvider {
                    action: action.clone(),
                    outcome,
                    ledgers: ledgers.clone(),
                    parts: parts.clone(),
                    inventory: inventory.clone(),
                },
            )?;
            Some(action.clone())
        }
        _ => None,
    };
    reg.state(BayActHandles {
        action,
        ledgers,
        parts,
        inventory: inventory.cloned(),
    });
    Ok(())
}

// ---- reads --------------------------------------------------------------------------------

/// The last landed ordinal on `hull`'s ledger: 0 for none, `None` when it cannot be read.
pub(crate) fn landed(ctx: &mush_core::Ctx<'_>, hull: ObjId) -> Option<u32> {
    let handles = ctx.w.module_state::<BayActHandles>()?;
    match ctx.action_state(&handles.ledgers, &hull) {
        Ok(None) => Some(0),
        Ok(Some(ledger)) if ledger.hull == hull => Some(ledger.acts),
        _ => None,
    }
}

/// **A lot's provenance**, when the bay minted it; `Err` when the family cannot be read.
///
/// # Errors
///
/// The sentence for an unreadable family.
pub fn provenance(ctx: &mush_core::Ctx<'_>, lot: LotId) -> Result<Option<PartProvenance>, String> {
    let handles = ctx
        .w
        .module_state::<BayActHandles>()
        .ok_or("The bay's records are not loaded.")?;
    match ctx.action_state(&handles.parts, &lot) {
        Ok(found) => Ok(found.filter(|p| p.lot == lot).cloned()),
        Err(_) => Err("The bay's part records cannot be read.".to_owned()),
    }
}

/// **A lot, where it stands**, read through the delegated inventory family.
///
/// # Errors
///
/// The sentence for an unwired or unreadable family.
pub fn lot(ctx: &mush_core::Ctx<'_>, lot: LotId) -> Result<Option<mush_inventory::Lot>, String> {
    let inventory = ctx
        .w
        .module_state::<BayActHandles>()
        .and_then(|h| h.inventory.clone())
        .ok_or("The inventory is not wired to the bay.")?;
    ctx.action_state(&inventory.lots, &lot)
        .map(|l| l.cloned())
        .map_err(|_| "The lot cannot be read.".to_owned())
}

/// **Submit `owed` as its actor** (the tick's swap: `ctx.actor` is set for the one call and
/// restored). The idempotency key names the hull and the ordinal.
pub(crate) fn submit(ctx: &mut mush_core::Ctx<'_>, hull: ObjId, owed: &OwedAct) -> bool {
    let Some(action) = ctx
        .w
        .module_state::<BayActHandles>()
        .and_then(|h| h.action.clone())
    else {
        return false;
    };
    let Ok(key) = mush_core::IdempotencyKeyText::parse_canonical(format!(
        "tacsim.bay.act/{}/{}/{}",
        hull.0, owed.act, owed.tries
    )) else {
        return false;
    };
    let saved = ctx.actor;
    ctx.actor = owed.by;
    ctx.submit_authoritative(
        action,
        PerceivedRef::SelfActor,
        vec![],
        json!({"hull": hull.0, "act": owed.act}),
        key,
    );
    ctx.actor = saved;
    true
}

// ---- the provider -------------------------------------------------------------------------

/// **`tacsim_bay_work`**: one owed act — a draw, a mint or a flag — and the hull's ledger
/// advanced exactly once per ordinal, in one bundle.
pub(crate) struct BayActProvider {
    pub action: DefinitionRef,
    pub outcome: DefinitionRef,
    pub ledgers: StateFamilyHandle<ObjId, BayLedger>,
    pub parts: StateFamilyHandle<LotId, PartProvenance>,
    pub inventory: InventoryFamilies,
}

fn invalid() -> mush_core::ActionError {
    port_error(
        ActionErrorCode::InvalidRequest,
        "the bay act is self-scoped, takes no targets and names one hull and its act",
    )
}

fn parameters(
    invocation: &ActionInvocation,
) -> Result<(&Map<String, Value>, u32, u32), mush_core::ActionError> {
    if invocation.subject() != &PerceivedRef::SelfActor || !invocation.targets().is_empty() {
        return Err(invalid());
    }
    let parameters = invocation.parameters().as_object().ok_or_else(invalid)?;
    let hull = parameters
        .get("hull")
        .and_then(Value::as_u64)
        .and_then(|h| u32::try_from(h).ok())
        .ok_or_else(invalid)?;
    let act = parameters
        .get("act")
        .and_then(Value::as_u64)
        .and_then(|a| u32::try_from(a).ok())
        .filter(|a| *a > 0)
        .ok_or_else(invalid)?;
    Ok((parameters, hull, act))
}

/// **The acts `actor` owes**, as the offer's fixed rows: every hull whose record owes an act
/// by `actor`, keyed by the hull. Derived from authoritative state at projection, never from
/// the submitter.
fn owed_rows(w: &World, actor: ObjId) -> Map<String, Value> {
    let Some(store) = w.module_state::<TacsimStore>() else {
        return Map::new();
    };
    let mut rows = Map::new();
    for hull in store.bay_owing_hulls() {
        if rows.len() >= MAX_OFFER_ROWS {
            break;
        }
        let Some(crate::port::bay_work::BayHullState::Present(row)) = store.bay_hull(hull) else {
            continue;
        };
        let Some(owed) = row.owed.as_ref().filter(|o| o.by == actor) else {
            continue;
        };
        if let Ok(value) = serde_json::to_value(owed) {
            rows.insert(hull.0.to_string(), value);
        }
    }
    rows
}

impl ActionProvider for BayActProvider {
    fn validate_invocation(
        &self,
        invocation: &ActionInvocation,
    ) -> Result<(), mush_core::ActionError> {
        parameters(invocation).map(|_| ())
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<ProviderOfferDraft>, mush_core::ActionError> {
        let SubjectKey::Object { id: subject } = query.truth_subject() else {
            return Ok(None);
        };
        if action != &self.action
            || query.actor() != *subject
            || query.projected_subject() != &PerceivedRef::SelfActor
        {
            return Ok(None);
        }
        let rows = owed_rows(&engine.world, query.actor());
        if rows.is_empty() {
            return Ok(None);
        }
        let integrity = |message: &'static str| port_error(ActionErrorCode::Integrity, message);
        let definitions = engine
            .registry
            .definitions()
            .ok_or_else(|| integrity("the bay act's definition registry is unavailable"))?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())
            .map_err(|_| integrity("the bay act's guard registry is unavailable"))?;
        guards
            .add_definition(action.clone(), definitions)
            .map_err(|_| integrity("the bay act's definition guard is unavailable"))?;
        let hull = ProjectedParameter {
            id: CanonicalId::new("hull")
                .map_err(|_| integrity("the bay act's parameter id is invalid"))?,
            required: true,
            kind: ProjectedParameterKind::Integer {
                minimum: 1,
                maximum: i64::from(u32::MAX),
            },
        };
        let act = ProjectedParameter {
            id: CanonicalId::new("act")
                .map_err(|_| integrity("the bay act's parameter id is invalid"))?,
            required: true,
            kind: ProjectedParameterKind::Integer {
                minimum: 1,
                maximum: i64::from(u32::MAX),
            },
        };
        Ok(Some(ProviderOfferDraft {
            // The prepare has no world, so the IC instant rides the offer beside the rows.
            fixed_parameters: json!({
                "rows": Value::Object(rows),
                "now": engine.world.world_ic_now().at,
            }),
            parameters: ProjectedParameterSchema::new(vec![hull, act])
                .map_err(|_| integrity("the bay act's parameter schema is invalid"))?,
            targets: ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: mush_core::ActionPresentation::new(
                "tacsim-bay-work",
                "Bay stores",
                "The bay draws an order's goods from a carrier's hold, or lays a stripped or \
                 scrapped hull's lots into it.",
            )
            .map_err(|_| integrity("the bay act's presentation is invalid"))?,
            guards: guards
                .finish()
                .map_err(|_| integrity("the bay act's offer guards are invalid"))?,
        }))
    }

    #[allow(clippy::too_many_lines)]
    fn prepare_authoritative(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, mush_core::ActionError> {
        // 1. The parameters, and the server-authenticated row the offer carried.
        let (parameters, hull, act) = parameters(invocation)?;
        let hull = ObjId(hull);
        let now = parameters
            .get("now")
            .and_then(Value::as_i64)
            .ok_or_else(invalid)?;
        let owed: OwedAct = parameters
            .get("rows")
            .and_then(Value::as_object)
            .and_then(|rows| rows.get(&hull.0.to_string()))
            .cloned()
            .and_then(|row| serde_json::from_value(row).ok())
            .ok_or_else(invalid)?;
        owed.check().map_err(|_| invalid())?;
        // 2. The row is this actor's and names this act.
        let actor = invocation.actor();
        if owed.by != actor || owed.act != act {
            return Err(port_error(
                ActionErrorCode::Unauthorized,
                "that bay act is not yours to submit",
            ));
        }
        // 3. The replay guard: only the ledger's next ordinal.
        let prior = txn
            .get(&self.ledgers, &hull)
            .map_err(|_| {
                port_error(
                    ActionErrorCode::Integrity,
                    "the bay ledgers are unavailable",
                )
            })?
            .cloned();
        if prior.as_ref().is_some_and(|l| l.hull != hull) {
            return Err(port_error(
                ActionErrorCode::Integrity,
                "a bay ledger is stored under another hull's key",
            ));
        }
        let next = prior.as_ref().map_or(Some(1), |l| l.acts.checked_add(1));
        if next != Some(act) {
            return Err(port_error(
                ActionErrorCode::Conflict,
                "that bay act has already landed",
            ));
        }
        // 4. The act.
        let refused =
            |message: &'static str| port_error(ActionErrorCode::ProviderRejected, message);
        let endpoint = owed.vault.map(|v| EndpointRef(VaultId(v)));
        match &owed.kind {
            OwedKind::Draw { goods, fit, .. } => {
                let endpoint = endpoint.ok_or_else(invalid)?;
                for row in goods {
                    let available = self
                        .inventory
                        .available(txn, endpoint, &row.good)
                        .map_err(|_| refused("the hold cannot be read"))?;
                    if available < row.units {
                        return Err(refused("the hold does not carry the goods the work needs"));
                    }
                }
                for row in goods.iter().filter(|r| r.units > 0) {
                    self.inventory
                        .debit(txn, endpoint, &row.good, row.units)
                        .map_err(|_| refused("the hold's goods could not be drawn"))?;
                }
                if let Some(lot) = fit {
                    let record = txn
                        .get(&self.parts, lot)
                        .map_err(|_| refused("the part's provenance cannot be read"))?
                        .cloned()
                        .ok_or_else(|| refused("the bay fits only parts it can trace"))?;
                    if record.is_hot() || record.fitted_into.is_some() {
                        return Err(refused("that part is hot, or already fitted"));
                    }
                    let held = txn
                        .get(&self.inventory.lots, lot)
                        .map_err(|_| refused("the part's lot cannot be read"))?
                        .cloned()
                        .filter(|l| l.endpoint() == endpoint)
                        .ok_or_else(|| refused("the part's lot is not in the hold"))?;
                    self.inventory
                        .consume_product(txn, &held)
                        .map_err(|_| refused("the part's lot could not be taken"))?;
                    let fitted = PartProvenance {
                        fitted_into: Some(hull),
                        ..record
                    };
                    txn.replace(&self.parts, lot, fitted)
                        .map_err(|_| refused("the part's provenance did not stage"))?;
                }
            }
            OwedKind::Mint { why, lots } => {
                let endpoint = endpoint.ok_or_else(invalid)?;
                let mut rows = Vec::with_capacity(lots.len());
                for row in lots {
                    let grade = Grade::new(row.grade).map_err(|_| invalid())?;
                    rows.push((row.commodity.clone(), row.quantity, grade, row.lot));
                }
                // One census write for every lot: one bundle writes a record once.
                self.inventory
                    .mint_origins(
                        txn,
                        endpoint,
                        &rows,
                        &mush_economy::ExternalRef {
                            provider: "tacsim".into(),
                            kind: format!("bay_{why}").into(),
                            id: format!("{}/{act}", hull.0).into(),
                            revision: Some(u64::from(act)),
                        },
                    )
                    .map_err(|_| refused("the lots could not be laid in the hold"))?;
                for row in lots {
                    if let Some(part) = &row.part {
                        if part.lot != row.lot || part.parent_hull != hull {
                            return Err(invalid());
                        }
                        txn.insert(&self.parts, &row.lot, part.clone())
                            .map_err(|_| refused("the part's provenance did not stage"))?;
                    }
                }
            }
            OwedKind::Hot {
                lot,
                claim,
                rightful_owner,
            } => {
                let record = txn
                    .get(&self.parts, lot)
                    .map_err(|_| refused("the part's provenance cannot be read"))?
                    .cloned()
                    .filter(|p| p.parent_hull == hull)
                    .ok_or_else(|| refused("no such part came off that hull"))?;
                let claim_id = SalvageClaimId::new(*claim).map_err(|_| invalid())?;
                let flagged = PartProvenance {
                    legality: ProvenanceLegalityStatus::ContrabandHotPart {
                        rightful_owner: *rightful_owner,
                        reason: ContrabandReason::VacatedSalvageClaim { claim_id },
                    },
                    ..record
                };
                txn.replace(&self.parts, lot, flagged)
                    .map_err(|_| refused("the part's provenance did not stage"))?;
            }
        }
        // 5. The ledger.
        let mut recent = prior.as_ref().map(|l| l.recent.clone()).unwrap_or_default();
        recent.push(LandedAct {
            act,
            op: owed.kind.word().to_owned(),
            by: actor,
            at_ic: now,
        });
        let excess = recent.len().saturating_sub(MAX_RECENT_ACTS);
        recent.drain(..excess);
        let ledger = BayLedger {
            hull,
            acts: act,
            recent,
        };
        let staged = if prior.is_some() {
            txn.replace(&self.ledgers, &hull, ledger)
        } else {
            txn.insert(&self.ledgers, &hull, ledger)
        };
        staged
            .map_err(|_| port_error(ActionErrorCode::Integrity, "the bay ledger did not stage"))?;
        // 6. The outcome, witnessed by the principal.
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({"actor": actor.0, "hull": hull.0, "act": act, "op": owed.kind.word()}),
        )
        .map_err(|_| {
            port_error(
                ActionErrorCode::BoundsExceeded,
                "the bay act's outcome exceeds canonical bounds",
            )
        })?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gc8b_a_lot_id_is_derived_and_distinct_per_row() {
        let a = lot_id(ObjId(7), 1, 0).expect("an id");
        assert_eq!(a, lot_id(ObjId(7), 1, 0).expect("the same id"));
        assert_ne!(a, lot_id(ObjId(7), 1, 1).expect("another row"));
        assert_ne!(a, lot_id(ObjId(7), 2, 0).expect("another act"));
        assert_ne!(a, lot_id(ObjId(8), 1, 0).expect("another hull"));
    }
}

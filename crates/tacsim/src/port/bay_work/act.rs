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
use mush_market::{CapacityPlan, CapacityProvider, CapacityReadError, CapacityRefusal};
use mush_tactics::field_ops::{
    ComponentCondition, ContrabandReason, ProvenanceLegalityStatus, SalvageClaimId,
};
use mush_tactics::unit::ComponentId;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::port::market::PortCapacity;
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

/// **Where a stripped part came from** (Q-36-37): the parent hull and the hand.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stripped {
    /// The hull it came off.
    pub parent_hull: ObjId,
    /// The parent hull's design.
    pub parent_schema: DefinitionRef,
    /// The slot it left, now `Missing` on the parent.
    pub slot: ComponentId,
    /// The salvage claim it came off under, when a salvor stripped it.
    pub claim: Option<u64>,
    /// Who took it off.
    pub stripped_by: ObjId,
    /// When, IC.
    pub stripped_at_ic: i64,
    /// Whether it came off in a scrapping, not a strip.
    pub scrapped: bool,
}

/// **How a part came to exist** (doc 39 cut IN.2, Q-39-13): every origin is traced, and only a
/// stripped part can be hot (a made or seeded part has no claim to be hot under).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum PartOrigin {
    /// Taken off a hull by a strip, a scrap or a refit.
    Stripped(Stripped),
    /// Made by a work order at a facility (the part commission).
    Manufactured {
        /// The work order.
        order: u64,
        /// The facility's object.
        facility: ObjId,
    },
    /// Minted from a vendor's stock row (the stock clerk's receipt).
    Seeded {
        /// The stock issue's key text ([`crate::port::families::StockIssueId`]).
        receipt: String,
    },
}

/// **A part's provenance** (Q-36-37, Q-F6; doc 39 cut IN.2): written in the bundle that mints
/// its lot, keyed by the lot, and kept for the lot's whole life — a sale moves the lot itself,
/// so the row follows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartProvenance {
    /// The lot: the key.
    pub lot: LotId,
    /// **The hardware row it is** (doc 39 cut IN.1, Q-39-3): a weapon, sensor or plant row; it
    /// fits every slot that names this row, whatever the unit.
    pub definition: DefinitionRef,
    /// Its condition (intact, damaged or jammed), whatever the origin.
    pub condition: ComponentCondition,
    /// Clean, or hot (Q-F6); only a stripped part is ever hot.
    pub legality: ProvenanceLegalityStatus,
    /// The hull it was fitted into, once fitted (its lot is then consumed).
    pub fitted_into: Option<ObjId>,
    /// How it came to exist.
    pub origin: PartOrigin,
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

    /// The strip it came from, when it was stripped.
    #[must_use]
    pub const fn stripped(&self) -> Option<&Stripped> {
        match &self.origin {
            PartOrigin::Stripped(s) => Some(s),
            PartOrigin::Manufactured { .. } | PartOrigin::Seeded { .. } => None,
        }
    }

    /// The hull it came off, when it was stripped.
    #[must_use]
    pub fn parent_hull(&self) -> Option<ObjId> {
        self.stripped().map(|s| s.parent_hull)
    }

    /// The origin's word, as the requester reads it.
    #[must_use]
    pub const fn origin_word(&self) -> &'static str {
        match &self.origin {
            PartOrigin::Stripped(s) if s.scrapped => "scrapped",
            PartOrigin::Stripped(_) => "stripped",
            PartOrigin::Manufactured { .. } => "manufactured",
            PartOrigin::Seeded { .. } => "seeded",
        }
    }

    /// Shape.
    ///
    /// # Errors
    ///
    /// A sentence naming what is wrong.
    pub fn check(&self) -> Result<(), String> {
        match &self.origin {
            PartOrigin::Stripped(s) if s.parent_hull.0 == 0 || s.stripped_by.0 == 0 => {
                return Err("a stripped part's provenance names a hull and a hand".to_owned());
            }
            PartOrigin::Manufactured { order, facility } if *order == 0 || facility.0 == 0 => {
                return Err("a made part's provenance names its order and facility".to_owned());
            }
            PartOrigin::Seeded { receipt } if receipt.is_empty() || receipt.len() > 128 => {
                return Err("a seeded part's provenance names its receipt".to_owned());
            }
            _ => {}
        }
        if self.is_hot() && self.stripped().is_none() {
            return Err("only a stripped part can be hot".to_owned());
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
        /// **Where that lot lies** (doc 39 cut IN.6): the storefront vault (`VaultId.0`) of a
        /// part from the workshop's stores; `None` for a part in the hold the goods come from.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fit_at: Option<u64>,
    },
    /// Mint lots into the hold, or (doc 39 cut IN.8) a workshop scrap's or a bale return's into
    /// the operator's stores.
    Mint {
        /// Why: `strip`, `scrap` or `return`.
        why: String,
        /// The lots.
        lots: Vec<LotMint>,
        /// **What lands with it** (doc 39 cut IN.8): the holder's credit and the shop bale, for
        /// a workshop scrap or a return; `None` for a strip.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        settle: Option<Box<super::scrap::Settlement>>,
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
    /// **Hold an order's part** (doc 39 cut IN.2, Q-39-27's "reserved"): from the order to the
    /// draw the lot is the order's, so no sale, move or split can take it ([`bay_holder`]).
    Hold {
        /// The order's sequence.
        order: u32,
        /// The part's lot.
        lot: LotId,
    },
    /// Release the part a cancelled order held, where it lies.
    Release {
        /// The order's sequence.
        order: u32,
        /// The part's lot.
        lot: LotId,
    },
    /// **The part commission** (doc 39 cut IN.2, §4.11): an untraced industry lot of a part is
    /// consumed by its lineage and the one-unit lots minted, each with its provenance row, in
    /// one bundle — so a part is fittable exactly when its row exists.
    Commission {
        /// The industry lot.
        source: LotId,
        /// The lineage it must carry.
        lineage: CommissionSource,
        /// The one-unit lots, a unit of the source each.
        lots: Vec<LotMint>,
    },
}

/// **The industry lineage a commission consumes** (doc 39 §4.11's two industry births).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum CommissionSource {
    /// A work order's delivery (`industry`/`workorder.output`, `wo/<order>`): the lots carry
    /// `Manufactured` rows naming the order.
    WorkOrder {
        /// The work order.
        order: u64,
    },
    /// A bay refit's removed part (`industry`/`bay_refit`, the order): the lot carries a
    /// `Stripped` row.
    BayRefit {
        /// The refit order.
        order: u64,
    },
}

impl CommissionSource {
    /// Whether `lineage` is this source's industry lineage.
    #[must_use]
    pub fn matches(&self, lineage: &mush_inventory::Lineage) -> bool {
        let (kind, id) = match self {
            Self::WorkOrder { order } => ("workorder.output", format!("wo/{order:016x}")),
            Self::BayRefit { order } => ("bay_refit", order.to_string()),
        };
        matches!(lineage, mush_inventory::Lineage::Origin { source }
            if source.provider == "industry"
                && source.kind == kind
                && source.id == id.as_str()
                && source.revision == Some(1))
    }

    /// Whether `origin` is the row this source gives a part.
    #[must_use]
    pub fn gives(&self, origin: &PartOrigin) -> bool {
        match (self, origin) {
            (Self::WorkOrder { order }, PartOrigin::Manufactured { order: made, .. }) => {
                made == order
            }
            (Self::BayRefit { .. }, PartOrigin::Stripped(_)) => true,
            _ => false,
        }
    }
}

/// **The holder text a bay order holds its part under** (doc 39 cut IN.2).
#[must_use]
pub fn bay_holder(hull: ObjId, order: u32) -> String {
    format!("tacsim.bay/{}/{order}", hull.0)
}

impl OwedKind {
    /// The op word.
    #[must_use]
    pub const fn word(&self) -> &'static str {
        match self {
            Self::Draw { .. } => "draw",
            Self::Mint { .. } => "mint",
            Self::Hot { .. } => "hot",
            Self::Hold { .. } => "hold",
            Self::Release { .. } => "release",
            Self::Commission { .. } => "commission",
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
            OwedKind::Mint { lots, .. } | OwedKind::Commission { lots, .. } => lots.len(),
            OwedKind::Hot { .. } | OwedKind::Hold { .. } | OwedKind::Release { .. } => 0,
        };
        if self.act == 0 || rows > MAX_ACT_ROWS {
            return Err("an owed act has an ordinal and at most 32 rows".to_owned());
        }
        if !matches!(self.kind, OwedKind::Hot { .. }) && self.vault.is_none() {
            return Err("a draw, a mint, a hold or a commission names its hold".to_owned());
        }
        if let OwedKind::Commission { lineage, lots, .. } = &self.kind
            && (lots.is_empty()
                || lots.iter().any(|l| {
                    l.quantity != 1 || l.part.as_ref().is_none_or(|p| !lineage.gives(&p.origin))
                }))
        {
            return Err("a commission mints one-unit parts with their rows".to_owned());
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
) -> Result<StateFamilyHandle<LotId, PartProvenance>, ModError> {
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
        parts: parts.clone(),
        inventory: inventory.cloned(),
    });
    Ok(parts)
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

/// **The hold on a lot**, when a listing or a bay order holds it (doc 39 cut IN.2).
///
/// # Errors
///
/// The sentence for an unwired or unreadable family.
pub fn lot_hold(
    ctx: &mush_core::Ctx<'_>,
    lot: LotId,
) -> Result<Option<mush_inventory::LotHold>, String> {
    let inventory = ctx
        .w
        .module_state::<BayActHandles>()
        .and_then(|h| h.inventory.clone())
        .ok_or("The inventory is not wired to the bay.")?;
    ctx.action_state(&inventory.lot_holds, &lot)
        .map(|h| h.cloned())
        .map_err(|_| "The lot's hold cannot be read.".to_owned())
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
/// by `actor`, keyed by the hull, and beside them (doc 36a P2-4) the capacity plan of the hold
/// each mint lays into, keyed the same, for the holds [`PortCapacity`] governs. Derived from
/// authoritative state at projection, never from the submitter.
fn owed_rows(w: &World, actor: ObjId) -> (Map<String, Value>, Map<String, Value>) {
    let Some(store) = w.module_state::<TacsimStore>() else {
        return (Map::new(), Map::new());
    };
    let mut rows = Map::new();
    let mut plans = Map::new();
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
            if let (OwedKind::Mint { .. }, Some(vault)) = (&owed.kind, owed.vault)
                && let Some(plan) = PortCapacity.plan(w, EndpointRef(VaultId(vault)))
            {
                plans.insert(hull.0.to_string(), plan.0);
            }
            rows.insert(hull.0.to_string(), value);
        }
    }
    (rows, plans)
}

impl BayActProvider {
    /// **Lay a mint's lots into the hold**: the hold's room checked by the plan the offer
    /// carried (doc 36a P2-4), then every lot minted under one `bay_<why>` origin.
    #[allow(clippy::too_many_arguments)]
    fn lay(
        &self,
        txn: &mut ActionTxn<'_>,
        parameters: &Map<String, Value>,
        endpoint: EndpointRef,
        hull: ObjId,
        act: u32,
        why: &str,
        lots: &[LotMint],
    ) -> Result<(), mush_core::ActionError> {
        let refused =
            |message: &'static str| port_error(ActionErrorCode::ProviderRejected, message);
        // Doc 36a P2-4: what the lots lay in the hold must fit it, by the plan the offer
        // carried for this hull; a hold the offer carried no plan for is ungoverned. The pools
        // are read here, in the bundle that writes them.
        if let Some(plan) = parameters
            .get("plans")
            .and_then(Value::as_object)
            .and_then(|plans| plans.get(&hull.0.to_string()))
        {
            let plan = CapacityPlan(plan.clone());
            // Doc 39 cut IN.1: a part weighs its catalogue mass, and the parts already aboard
            // are read from the lot tally (a part never pools).
            let mut bulk: BTreeMap<CanonicalId, u64> = BTreeMap::new();
            for row in lots {
                let slot = bulk.entry(row.commodity.clone()).or_insert(0);
                *slot = slot.saturating_add(row.quantity);
            }
            for (commodity, quantity) in &bulk {
                let mut read = |good: &CanonicalId| {
                    let pooled = self
                        .inventory
                        .pool(txn, endpoint, good)
                        .map(|pool| pool.map_or(0, |pool| pool.quantity))
                        .map_err(|_| CapacityReadError)?;
                    if !crate::port_tick::is_part(good) {
                        return Ok(pooled);
                    }
                    self.inventory
                        .lot_quantity(txn, endpoint, good)
                        .map(|held| pooled.saturating_add(held))
                        .map_err(|_| CapacityReadError)
                };
                PortCapacity
                    .check(&plan, commodity, *quantity, &mut read)
                    .map_err(|refusal| match refusal {
                        CapacityRefusal::Refused { .. } => {
                            refused("the hold has no room for the lots")
                        }
                        CapacityRefusal::Unreadable => refused("the hold's room cannot be read"),
                    })?;
            }
        }
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
        Ok(())
    }
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
        let (rows, plans) = owed_rows(&engine.world, query.actor());
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
            // The prepare has no world, so the IC instant rides the offer beside the rows, and
            // so does each mint's hold plan (doc 36a P2-4).
            fixed_parameters: json!({
                "rows": Value::Object(rows),
                "plans": Value::Object(plans),
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
            OwedKind::Draw {
                goods,
                fit,
                fit_at,
                order,
            } => {
                let endpoint = endpoint.ok_or_else(invalid)?;
                let fit_endpoint = fit_at.map_or(endpoint, |v| EndpointRef(VaultId(v)));
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
                        .filter(|l| l.endpoint() == fit_endpoint)
                        .ok_or_else(|| {
                            refused("the part's lot is not in the hold or the stores")
                        })?;
                    // Doc 39 cut IN.2: the order held the lot at its take; the draw ends the
                    // hold as it consumes the lot.
                    self.inventory
                        .consume_held(txn, &held, &bay_holder(hull, *order))
                        .map_err(|_| refused("the part's lot is not held for this order"))?;
                    let fitted = PartProvenance {
                        fitted_into: Some(hull),
                        ..record
                    };
                    txn.replace(&self.parts, lot, fitted)
                        .map_err(|_| refused("the part's provenance did not stage"))?;
                }
            }
            OwedKind::Mint { why, lots, .. } => {
                let endpoint = endpoint.ok_or_else(invalid)?;
                self.lay(txn, parameters, endpoint, hull, act, why, lots)?;
                for row in lots {
                    if let Some(part) = &row.part {
                        if part.lot != row.lot || part.parent_hull() != Some(hull) {
                            return Err(invalid());
                        }
                        txn.insert(&self.parts, &row.lot, part.clone())
                            .map_err(|_| refused("the part's provenance did not stage"))?;
                    }
                }
            }
            OwedKind::Hold { order, lot } => {
                let endpoint = endpoint.ok_or_else(invalid)?;
                let record = txn
                    .get(&self.parts, lot)
                    .map_err(|_| refused("the part's provenance cannot be read"))?
                    .cloned()
                    .ok_or_else(|| refused("the bay holds only parts it can trace"))?;
                if record.is_hot() || record.fitted_into.is_some() {
                    return Err(refused("that part is hot, or already fitted"));
                }
                self.inventory
                    .hold_lot(txn, endpoint, *lot, &bay_holder(hull, *order))
                    .map_err(|_| refused("the part is not in the hold, or is held already"))?;
            }
            OwedKind::Release { order, lot } => {
                // A release after the hold ended (the lot consumed, or never held) moves
                // nothing; the ledger still advances, so the act lands once.
                let holder = bay_holder(hull, *order);
                let held = self
                    .inventory
                    .lot_hold(txn, *lot)
                    .map_err(|_| refused("the part's hold cannot be read"))?
                    .is_some_and(|h| h.holder() == holder);
                if held {
                    self.inventory
                        .release_lot(txn, *lot, &holder)
                        .map_err(|_| refused("the part's hold could not be released"))?;
                }
            }
            OwedKind::Commission {
                source,
                lineage,
                lots,
            } => {
                let endpoint = endpoint.ok_or_else(invalid)?;
                let from = txn
                    .get(&self.inventory.lots, source)
                    .map_err(|_| refused("the industry lot cannot be read"))?
                    .cloned()
                    .filter(|l| l.endpoint() == endpoint && lineage.matches(l.lineage()))
                    .ok_or_else(|| refused("no such industry lot is there to commission"))?;
                // Conservation: a unit of the source a lot, the same good, nothing more.
                let units = u64::try_from(lots.len()).map_err(|_| invalid())?;
                if from.quantity() != units || lots.iter().any(|l| &l.commodity != from.commodity())
                {
                    return Err(invalid());
                }
                let mut rows = Vec::with_capacity(lots.len());
                for row in lots {
                    rows.push((row.lot, Grade::new(row.grade).map_err(|_| invalid())?));
                }
                self.inventory
                    .commission_lots(txn, &from, &rows)
                    .map_err(|_| refused("the industry lot could not be commissioned"))?;
                for row in lots {
                    let part = row.part.as_ref().ok_or_else(invalid)?;
                    if part.lot != row.lot || !lineage.gives(&part.origin) {
                        return Err(invalid());
                    }
                    txn.insert(&self.parts, &row.lot, part.clone())
                        .map_err(|_| refused("the part's provenance did not stage"))?;
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
                    .filter(|p| p.parent_hull() == Some(hull))
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

    fn row() -> DefinitionRef {
        mush_core::DefinitionRegistryBuilder::new()
            .register(
                mush_core::mechanics::OwnerId::new("tacsim").expect("an owner"),
                DefinitionDescriptor::new(
                    DefinitionKind::new("sensor").expect("a kind"),
                    DefinitionId::new("tacsim.sensor.visual_mk1").expect("an id"),
                    1,
                    json!({"fixture": true}),
                )
                .expect("a descriptor"),
            )
            .expect("registers")
    }

    fn part(origin: PartOrigin) -> PartProvenance {
        PartProvenance {
            lot: LotId(7),
            definition: row(),
            condition: ComponentCondition::Intact,
            legality: ProvenanceLegalityStatus::CleanTitle,
            fitted_into: None,
            origin,
        }
    }

    /// **Doc 39 cut IN.2 (Q-39-13)**: every origin is traced; only a stripped part can be hot;
    /// each origin names what made it.
    #[test]
    fn in2_every_origin_is_traced_and_only_a_stripped_part_is_hot() {
        let seeded = part(PartOrigin::Seeded {
            receipt: "ab".repeat(32),
        });
        seeded.check().expect("a seeded part");
        assert_eq!(seeded.origin_word(), "seeded");
        assert_eq!(seeded.parent_hull(), None);
        let made = part(PartOrigin::Manufactured {
            order: 3,
            facility: ObjId(9),
        });
        made.check().expect("a made part");
        let hot = PartProvenance {
            legality: ProvenanceLegalityStatus::ContrabandHotPart {
                rightful_owner: ObjId(4),
                reason: ContrabandReason::VacatedSalvageClaim {
                    claim_id: SalvageClaimId::new(1).expect("a claim"),
                },
            },
            ..made.clone()
        };
        assert!(hot.check().is_err(), "a made part is never hot");
        assert!(
            part(PartOrigin::Manufactured {
                order: 0,
                facility: ObjId(9)
            })
            .check()
            .is_err()
        );
        assert!(
            part(PartOrigin::Seeded {
                receipt: String::new()
            })
            .check()
            .is_err()
        );
        // The row's wire names its origin.
        let wire = serde_json::to_value(&made).expect("encodes");
        assert_eq!(wire["origin"]["manufactured"]["order"], 3);
        let back: PartProvenance = serde_json::from_value(wire).expect("decodes");
        assert_eq!(back, made);
    }

    /// **The part commission's shape** (doc 39 cut IN.2, §4.11): it consumes only its source's
    /// industry lineage and mints one-unit parts whose rows that source gives.
    #[test]
    fn in2_a_commission_names_its_lineage_and_mints_one_unit_parts() {
        let order = CommissionSource::WorkOrder { order: 0x2a };
        let delivered = mush_inventory::Lineage::Origin {
            source: mush_economy::ExternalRef {
                provider: "industry".into(),
                kind: "workorder.output".into(),
                id: "wo/000000000000002a".into(),
                revision: Some(1),
            },
        };
        assert!(order.matches(&delivered));
        assert!(!CommissionSource::WorkOrder { order: 0x2b }.matches(&delivered));
        assert!(!CommissionSource::BayRefit { order: 0x2a }.matches(&delivered));
        let made = PartOrigin::Manufactured {
            order: 0x2a,
            facility: ObjId(9),
        };
        assert!(order.gives(&made));
        assert!(!CommissionSource::BayRefit { order: 0x2a }.gives(&made));
        let mint = |quantity, origin: PartOrigin| LotMint {
            lot: LotId(1),
            commodity: CanonicalId::new("tacsim.part.sensor.visual_mk1").expect("an id"),
            quantity,
            grade: 9,
            part: Some(part(origin)),
        };
        let owed = |lots| OwedAct {
            act: 1,
            by: ObjId(2),
            vault: Some(5),
            kind: OwedKind::Commission {
                source: LotId(3),
                lineage: order,
                lots,
            },
            tries: 0,
        };
        owed(vec![mint(1, made.clone())])
            .check()
            .expect("a commission");
        assert!(
            owed(vec![mint(2, made.clone())]).check().is_err(),
            "one unit a lot"
        );
        assert!(owed(Vec::new()).check().is_err(), "at least one part");
        let seeded = PartOrigin::Seeded {
            receipt: "ab".repeat(32),
        };
        assert!(
            owed(vec![mint(1, seeded)]).check().is_err(),
            "the source's row"
        );
    }

    #[test]
    fn gc8b_a_lot_id_is_derived_and_distinct_per_row() {
        let a = lot_id(ObjId(7), 1, 0).expect("an id");
        assert_eq!(a, lot_id(ObjId(7), 1, 0).expect("the same id"));
        assert_ne!(a, lot_id(ObjId(7), 1, 1).expect("another row"));
        assert_ne!(a, lot_id(ObjId(7), 2, 0).expect("another act"));
        assert_ne!(a, lot_id(ObjId(8), 1, 0).expect("another hull"));
    }
}

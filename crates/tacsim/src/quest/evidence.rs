//! Owner receipts for optional lessons. Conversation and display projections are
//! never evidence. Survey receipts share the successful act's transaction; paid
//! trade receipts are copied before the market's bounded history can be pruned.
use super::model::{CheckEvidence, Checkpoint, Objective, QuestKey, QuestRecord, Stamp};
use crate::port::{coop, families::PortHandles};
use crate::surface::excursion::{Act, ActKind};
use mush_core::{
    ActionError, ActionErrorCode, ActionTxn, CanonicalId, Ctx, ModError, ModuleBootData, ModuleCtx,
    ModuleWrite, ObjId, Registrar, StateFamilyHandle, World,
};
use mush_economy::{ContractId, ContractState, EconomicEvent, EconomyStore, TransactionId};
use mush_market::{MarketStore, PaidKind, Quote};
use mush_org::PrincipalId;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use smol_str::SmolStr;
use std::collections::{BTreeMap, BTreeSet};

const SURVEY_FAMILY: &str = "tacsim.quest_survey_evidence";
const TRADE_PREFIX: &str = "tacsim:quest_trade:";
const MAX_WATCHES: usize = 128;
const MAX_PURCHASE_HOLDS: usize = 8;
const MAX_TRADE_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurveyReceipt {
    pub actor: ObjId,
    pub tv: ObjId,
    pub ordinal: u32,
    pub site: CanonicalId,
    pub mode: String,
    pub started_ic: i64,
    pub due_ic: i64,
    pub at_utc: i64,
}
impl SurveyReceipt {
    fn check(&self) -> Result<(), String> {
        if self.actor.0 == 0
            || self.tv.0 == 0
            || self.ordinal == 0
            || !matches!(self.mode.as_str(), "scan" | "sound")
            || self.site.as_str().len() > 128
            || self.due_ic <= self.started_ic
        {
            return Err("invalid completed survey receipt".into());
        }
        Ok(())
    }
}

/// Separate slots preserve a completed scan when a later sounding completes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurveyEvidenceBook {
    pub schema: u16,
    pub actor: ObjId,
    pub scan: Option<SurveyReceipt>,
    pub sound: Option<SurveyReceipt>,
}
impl SurveyEvidenceBook {
    pub fn check(&self) -> Result<(), String> {
        if self.schema != 1 || self.actor.0 == 0 {
            return Err("invalid survey evidence book".into());
        }
        for (mode, receipt) in [("scan", &self.scan), ("sound", &self.sound)] {
            if let Some(receipt) = receipt {
                receipt.check()?;
                if receipt.actor != self.actor || receipt.mode != mode {
                    return Err("survey evidence identity mismatch".into());
                }
            }
        }
        Ok(())
    }
    fn get(&self, mode: &str) -> Option<&SurveyReceipt> {
        match mode {
            "scan" => self.scan.as_ref(),
            "sound" => self.sound.as_ref(),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TradeReceipt {
    pub actor: ObjId,
    pub quote: u64,
    pub contract: u64,
    pub transaction: u64,
    pub market: String,
    pub commodity: String,
    pub quantity: u64,
    /// The same real hold receives the purchase and supplies the sale.
    pub hold: u64,
    pub purchase: bool,
    pub accepted_at: i64,
    pub settled_at: i64,
}
impl TradeReceipt {
    fn check(&self) -> Result<(), String> {
        if self.actor.0 == 0
            || self.quantity == 0
            || self.market.len() > 128
            || CanonicalId::new(&self.market).is_err()
            || self.commodity.len() > 128
            || CanonicalId::new(&self.commodity).is_err()
            || self.accepted_at > self.settled_at
        {
            return Err("invalid paid trade receipt".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ObjectiveProof {
    CoopEnrolment {
        actor: ObjId,
        account: mush_core::AccountId,
        hull: ObjId,
        lease: u64,
        stake: u64,
    },
    Trade {
        purchase: TradeReceipt,
        sale: TradeReceipt,
    },
    Passenger {
        actor: ObjId,
        npc: ObjId,
        role: String,
        origin: String,
        destination: String,
    },
    Contact {
        actor: ObjId,
        npc: ObjId,
        name: String,
    },
    Survey {
        receipt: SurveyReceipt,
    },
}
impl ObjectiveProof {
    pub fn check(&self, objective: &Objective) -> Result<(), String> {
        let valid = match (self, objective) {
            (Self::CoopEnrolment { actor, hull, .. }, Objective::CoopEnrolment) => {
                actor.0 != 0 && hull.0 != 0
            }
            (
                Self::Trade { purchase, sale },
                Objective::Trade {
                    origin,
                    destination,
                    commodity,
                    quantity,
                },
            ) => {
                purchase.check()?;
                sale.check()?;
                purchase.purchase
                    && !sale.purchase
                    && purchase.actor == sale.actor
                    && purchase.hold == sale.hold
                    && purchase.quote != sale.quote
                    && purchase.contract != sale.contract
                    && purchase.transaction != sale.transaction
                    && purchase.market == *origin
                    && sale.market == *destination
                    && purchase.commodity == *commodity
                    && sale.commodity == *commodity
                    && purchase.quantity >= *quantity
                    && sale.quantity >= *quantity
                    && purchase.settled_at <= sale.accepted_at
            }
            (
                Self::Passenger {
                    actor,
                    npc,
                    role,
                    origin,
                    destination,
                },
                Objective::Passenger { npc: expected },
            ) => {
                actor.0 != 0
                    && npc == expected
                    && !role.is_empty()
                    && role.len() <= 160
                    && origin != destination
                    && origin.len() <= 128
                    && destination.len() <= 128
                    && CanonicalId::new(origin).is_ok()
                    && CanonicalId::new(destination).is_ok()
            }
            (Self::Survey { receipt }, Objective::Survey { mode }) => {
                receipt.check()?;
                &receipt.mode == mode
            }
            (Self::Contact { actor, npc, name }, Objective::Contact { npc: expected }) => {
                actor.0 != 0
                    && npc.0 != 0
                    && actor != npc
                    && npc == expected
                    && !name.trim().is_empty()
                    && name.len() <= 128
                    && !name.contains('\0')
            }
            _ => false,
        };
        valid
            .then_some(())
            .ok_or_else(|| "objective evidence does not match this lesson".into())
    }
    pub fn check_for(
        &self,
        objective: &Objective,
        actor: ObjId,
        accepted: &Stamp,
        checked: &Stamp,
    ) -> Result<(), String> {
        self.check(objective)?;
        let valid = checked.at_utc >= accepted.at_utc
            && checked.at_ic >= accepted.at_ic
            && match self {
                Self::CoopEnrolment { actor: who, .. }
                | Self::Passenger { actor: who, .. }
                | Self::Contact { actor: who, .. } => *who == actor,
                Self::Trade { purchase, sale } => {
                    purchase.actor == actor
                        && purchase.accepted_at >= accepted.at_utc
                        && sale.settled_at <= checked.at_utc
                }
                Self::Survey { receipt } => {
                    receipt.actor == actor
                        && receipt.started_ic > accepted.at_ic
                        && receipt.due_ic <= checked.at_ic
                        && receipt.at_utc >= accepted.at_utc
                        && receipt.at_utc <= checked.at_utc
                }
            };
        valid
            .then_some(())
            .ok_or_else(|| "objective evidence belongs to another actor or time".into())
    }
}

pub fn register_survey_family(
    reg: &mut Registrar<'_>,
) -> Result<StateFamilyHandle<ObjId, SurveyEvidenceBook>, ModError> {
    reg.state(TradeEvidenceStore::default());
    reg.state_family(
        crate::port::families::family_descriptor(
            SURVEY_FAMILY,
            "tacsim.quest_survey_actor.v1",
            SURVEY_FAMILY,
        )?
        .with_limits(32, 4096)
        .map_err(|e| ModError::msg(e.to_string()))?,
        |actor: &ObjId| {
            if actor.0 != 0 {
                Ok(actor.0.to_string())
            } else {
                Err("invalid survey actor".into())
            }
        },
        |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
        SurveyEvidenceBook::check,
        |book: &SurveyEvidenceBook| serde_json::to_value(book).map_err(|e| e.to_string()),
    )
}

pub(crate) fn log_survey(
    txn: &mut ActionTxn<'_>,
    family: &StateFamilyHandle<ObjId, SurveyEvidenceBook>,
    act: &Act,
    tv: ObjId,
    at_utc: i64,
) -> Result<(), ActionError> {
    let mode = match act.kind {
        ActKind::Scan => "scan",
        ActKind::Sound => "sound",
        _ => return Ok(()),
    };
    let error = || {
        crate::port_tick::port_error(
            ActionErrorCode::Integrity,
            "quest survey evidence is unreadable",
        )
    };
    let existing = txn
        .get(family, &act.operator)
        .map_err(|_| error())?
        .cloned();
    let mut book = existing.clone().unwrap_or(SurveyEvidenceBook {
        schema: 1,
        actor: act.operator,
        scan: None,
        sound: None,
    });
    book.check().map_err(|_| error())?;
    if book.actor != act.operator {
        return Err(error());
    }
    let receipt = SurveyReceipt {
        actor: act.operator,
        tv,
        ordinal: act.ordinal,
        site: act.site.clone(),
        mode: mode.into(),
        started_ic: act.started_ic,
        due_ic: act.due_ic,
        at_utc,
    };
    let slot = if mode == "scan" {
        &mut book.scan
    } else {
        &mut book.sound
    };
    // A late old finish cannot replace a more recently started, completed act.
    if slot.as_ref().is_none_or(|old| {
        (old.started_ic, old.due_ic, old.at_utc)
            <= (receipt.started_ic, receipt.due_ic, receipt.at_utc)
    }) {
        *slot = Some(receipt);
    }
    book.check().map_err(|_| error())?;
    if existing.is_some() {
        txn.replace(family, &act.operator, book)
    } else {
        txn.insert(family, &act.operator, book)
    }
    .map_err(|_| error())?;
    Ok(())
}

/// Used by the quest provider's real typed read at offer, guard and prepare.
pub fn proof_current(
    book: Option<&SurveyEvidenceBook>,
    actor: ObjId,
    proof: &ObjectiveProof,
) -> bool {
    match proof {
        ObjectiveProof::Survey { receipt } => book.is_some_and(|book| {
            book.check().is_ok() && book.actor == actor && book.get(&receipt.mode) == Some(receipt)
        }),
        _ => true,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TradeWatch {
    key: QuestKey,
    checkpoint: String,
    accepted: Stamp,
    objective: Objective,
    /// Unmatched purchases are candidates, never acknowledged quest progress.
    /// Keep a bounded latest purchase per hold so replacing a ship does not
    /// strand the lesson or erase another still-usable hold's evidence.
    #[serde(default)]
    candidates: Vec<TradeReceipt>,
    purchase: Option<TradeReceipt>,
    sale: Option<TradeReceipt>,
}
impl TradeWatch {
    fn matches_purchase(&self, receipt: &TradeReceipt) -> bool {
        let Objective::Trade {
            origin,
            commodity,
            quantity,
            ..
        } = &self.objective
        else {
            return false;
        };
        receipt.purchase
            && receipt.market == *origin
            && receipt.commodity == *commodity
            && receipt.quantity >= *quantity
            && receipt.accepted_at >= self.accepted.at_utc
    }
    fn remember_purchase(&mut self, receipt: TradeReceipt) {
        if self.sale.is_some() || !self.matches_purchase(&receipt) {
            return;
        }
        let order = |r: &TradeReceipt| (r.accepted_at, r.settled_at, r.quote);
        if let Some(old) = self
            .candidates
            .iter_mut()
            .find(|old| old.hold == receipt.hold)
        {
            if order(old) < order(&receipt) {
                *old = receipt;
            }
        } else {
            if self.candidates.len() == MAX_PURCHASE_HOLDS
                && let Some((oldest, _)) = self
                    .candidates
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, r)| order(r))
            {
                if self
                    .candidates
                    .get(oldest)
                    .is_some_and(|old| order(&receipt) <= order(old))
                {
                    return;
                }
                self.candidates.remove(oldest);
            }
            self.candidates.push(receipt);
        }
        self.candidates.sort_by_key(|r| r.hold);
    }
    fn select_pair(&mut self, sale: &TradeReceipt, paid: impl Fn(&TradeReceipt) -> bool) {
        if self.sale.is_some() || sale.purchase {
            return;
        }
        let Some(purchase) = self
            .candidates
            .iter()
            .find(|purchase| {
                purchase.hold == sale.hold
                    && paid(purchase)
                    && ObjectiveProof::Trade {
                        purchase: (*purchase).clone(),
                        sale: sale.clone(),
                    }
                    .check(&self.objective)
                    .is_ok()
            })
            .cloned()
        else {
            return;
        };
        self.purchase = Some(purchase);
        self.sale = Some(sale.clone());
        self.candidates.clear();
    }
    fn migrate_unmatched(&mut self) {
        if self.sale.is_none()
            && let Some(purchase) = self.purchase.take()
        {
            self.remember_purchase(purchase);
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TradeBook {
    schema: u16,
    actor: ObjId,
    watches: Vec<TradeWatch>,
}
impl TradeBook {
    fn check(&self) -> Result<(), String> {
        if self.schema != 1
            || self.actor.0 == 0
            || self.watches.len() > MAX_WATCHES
            || serde_json::to_vec(self).map_err(|e| e.to_string())?.len() > MAX_TRADE_BYTES
        {
            return Err("invalid quest trade evidence book".into());
        }
        let mut seen = BTreeSet::new();
        for watch in &self.watches {
            watch.key.check()?;
            watch.objective.check()?;
            if !matches!(watch.objective, Objective::Trade { .. })
                || watch.candidates.len() > MAX_PURCHASE_HOLDS
                || watch.checkpoint.len() > 64
                || CanonicalId::new(&watch.checkpoint).is_err()
                || !seen.insert((watch.key.clone(), watch.checkpoint.clone()))
            {
                return Err("invalid quest trade watch".into());
            }
            let mut holds = BTreeSet::new();
            for candidate in &watch.candidates {
                candidate.check()?;
                if candidate.actor != self.actor
                    || !watch.matches_purchase(candidate)
                    || !holds.insert(candidate.hold)
                    || watch.sale.is_some()
                {
                    return Err("invalid unmatched quest purchase".into());
                }
            }
            if let Some(purchase) = &watch.purchase {
                purchase.check()?;
                if purchase.actor != self.actor
                    || !purchase.purchase
                    || purchase.accepted_at < watch.accepted.at_utc
                {
                    return Err("invalid quest purchase".into());
                }
            }
            if let Some(sale) = &watch.sale {
                let purchase = watch.purchase.clone().ok_or("sale without purchase")?;
                ObjectiveProof::Trade {
                    purchase,
                    sale: sale.clone(),
                }
                .check(&watch.objective)?;
            }
        }
        Ok(())
    }
}

/// Owner documents, not a UI projection. Unreadable keys remain claimed.
#[derive(Clone, Default)]
pub(crate) struct TradeEvidenceStore {
    books: BTreeMap<ObjId, TradeBook>,
    unreadable: BTreeSet<ObjId>,
}

pub(crate) fn boot(ctx: &mut ModuleCtx<'_>, data: &ModuleBootData) -> Result<(), ModError> {
    let mut loaded = TradeEvidenceStore::default();
    for (key, value) in &data.kv {
        let Some(id) = key.strip_prefix(TRADE_PREFIX) else {
            continue;
        };
        let Ok(raw) = id.parse::<u32>() else { continue };
        let actor = ObjId(raw);
        if raw == 0 || raw.to_string() != id {
            continue;
        }
        let book = serde_json::from_value::<TradeBook>(value.clone())
            .ok()
            .filter(|book| book.actor == actor && book.check().is_ok());
        if let Some(book) = book {
            loaded.books.insert(actor, book);
        } else {
            loaded.unreadable.insert(actor);
            tracing::warn!(
                actor = actor.0,
                "quest trade evidence is unreadable; progress fails closed"
            );
        }
    }
    *ctx.w
        .module_state_mut::<TradeEvidenceStore>()
        .ok_or_else(|| ModError::msg("quest trade evidence store missing"))? = loaded;
    Ok(())
}
fn put_book(ctx: &mut Ctx<'_>, book: TradeBook) -> Result<(), String> {
    book.check()?;
    let value = serde_json::to_value(&book).map_err(|e| e.to_string())?;
    let actor = book.actor;
    let store = ctx
        .w
        .module_state_mut::<TradeEvidenceStore>()
        .ok_or("quest evidence unavailable")?;
    if store.unreadable.contains(&actor) {
        return Err("quest evidence needs staff repair".into());
    }
    store.books.insert(actor, book);
    crate::store::push_op(
        ctx.out,
        ModuleWrite::JsonSet {
            key: SmolStr::new(format!("{TRADE_PREFIX}{}", actor.0)),
            value,
        },
    );
    Ok(())
}

/// A provisional command watch cannot grant credit: capture and the commit guard
/// must match the actual accepted typed quest and its exact frozen terms.
pub(crate) fn watch(ctx: &mut Ctx<'_>, row: &QuestRecord) -> Result<(), String> {
    row.check()?;
    let store = ctx
        .w
        .module_state::<TradeEvidenceStore>()
        .ok_or("quest evidence unavailable")?;
    if store.unreadable.contains(&row.actor) {
        return Err("quest evidence needs staff repair".into());
    }
    let old = store.books.get(&row.actor).cloned();
    let mut book = old.clone().unwrap_or(TradeBook {
        schema: 1,
        actor: row.actor,
        watches: Vec::new(),
    });
    for checkpoint in &row.definition.checkpoints {
        if !matches!(checkpoint.objective, Objective::Trade { .. }) {
            continue;
        }
        let watch = TradeWatch {
            key: row.key.clone(),
            checkpoint: checkpoint.id.clone(),
            accepted: row.accepted.clone(),
            objective: checkpoint.objective.clone(),
            candidates: Vec::new(),
            purchase: None,
            sale: None,
        };
        if let Some(existing) = book
            .watches
            .iter_mut()
            .find(|w| w.key == row.key && w.checkpoint == checkpoint.id)
        {
            if existing.accepted != watch.accepted || existing.objective != watch.objective {
                // An uncommitted prior request cannot reserve another acceptance's evidence.
                *existing = watch;
            }
        } else {
            if book.watches.len() >= MAX_WATCHES {
                return Err("quest trade evidence is full".into());
            }
            book.watches.push(watch);
        }
    }
    if !book.watches.is_empty() && old.as_ref() != Some(&book) {
        put_book(ctx, book)?;
    }
    Ok(())
}

fn paid_transaction(w: &World, contract: u64, transaction: u64, at: i64) -> bool {
    let Some(economy) = w.module_state::<EconomyStore>() else {
        return false;
    };
    let transaction = TransactionId(transaction);
    let key = mush_economy::settle_key(ContractId(contract));
    let receipt = economy
        .snapshot()
        .idempotency(&key)
        .or_else(|| economy.cold_key(&key).and_then(|cold| cold.record.as_ref()));
    !economy.is_frozen()
        && !economy.snapshot().is_reversed(transaction)
        && receipt.is_some_and(|receipt| {
            receipt.idempotency == key
                && receipt.transaction == transaction
                && receipt.posted_at == at
        })
        && economy
            .snapshot()
            .contract(ContractId(contract))
            // Economy may prune an old terminal contract. Its exact settlement
            // key still resolves through the journal-backed cold receipt store.
            .is_none_or(|c| {
                c.pending.is_none() && c.state == ContractState::Settled { transaction }
            })
}
fn from_quote(w: &World, q: &Quote) -> Option<TradeReceipt> {
    let lot = q.lot.as_ref()?;
    lot.check().ok()?;
    let settled = q.settled?;
    let contract = q.contract?;
    if !q.activated
        || q.self_dealt
        || q.quantity == 0
        || q.price.minor() <= 0
        || !paid_transaction(w, contract.0, settled.transaction.0, settled.at)
    {
        return None;
    }
    let (actor, hold, purchase) = match (&q.buyer, &q.seller, lot.kind) {
        (PrincipalId::Object(actor), PrincipalId::Organization(_), PaidKind::Draw)
            if q.drawn_at.is_some() =>
        {
            (*actor, lot.destination.0.0.0, true)
        }
        (PrincipalId::Organization(_), PrincipalId::Object(actor), PaidKind::Deliver)
            if lot.marker.is_some() =>
        {
            (*actor, lot.source.0.0.0, false)
        }
        _ => return None,
    };
    let receipt = TradeReceipt {
        actor,
        quote: q.id.0,
        contract: contract.0,
        transaction: settled.transaction.0,
        market: q.market.as_str().into(),
        commodity: lot.commodity.0.as_str().into(),
        quantity: q.quantity,
        hold,
        purchase,
        accepted_at: lot.accepted_at,
        settled_at: settled.at,
    };
    receipt.check().ok()?;
    Some(receipt)
}
fn record_quote(ctx: &mut Ctx<'_>, q: &Quote) {
    let Some(receipt) = from_quote(ctx.w, q) else {
        return;
    };
    let Some(mut book) = ctx
        .w
        .module_state::<TradeEvidenceStore>()
        .and_then(|s| s.books.get(&receipt.actor))
        .cloned()
    else {
        return;
    };
    let before = book.clone();
    for watch in &mut book.watches {
        watch.migrate_unmatched();
        let Objective::Trade {
            destination,
            commodity,
            quantity,
            ..
        } = &watch.objective
        else {
            continue;
        };
        if watch.sale.is_some()
            || receipt.accepted_at < watch.accepted.at_utc
            || receipt.quantity < *quantity
            || receipt.commodity != *commodity
        {
            continue;
        }
        if receipt.purchase {
            watch.remember_purchase(receipt.clone());
        } else if receipt.market == *destination {
            watch.select_pair(&receipt, |purchase| {
                paid_transaction(
                    ctx.w,
                    purchase.contract,
                    purchase.transaction,
                    purchase.settled_at,
                )
            });
        }
    }
    if book != before
        && let Err(error) = put_book(ctx, book)
    {
        tracing::warn!(%error, "quest trade receipt could not be retained");
    }
}
pub(crate) fn on_economic_event(ctx: &mut Ctx<'_>, event: &EconomicEvent) {
    let EconomicEvent::ContractChanged { id, .. } = event else {
        return;
    };
    let quote = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|market| {
            market
                .quote_of_contract(*id)
                .and_then(|id| market.quote(id))
        })
        .cloned();
    if let Some(quote) = quote {
        record_quote(ctx, &quote);
    }
}

/// Repair missed notifications from retained owner rows, and prefetch old Co-op
/// stake keys through the economy's cold-receipt path. No history is invented.
pub(crate) fn reconcile(ctx: &mut Ctx<'_>) {
    let books: Vec<_> = ctx
        .w
        .module_state::<TradeEvidenceStore>()
        .into_iter()
        .flat_map(|store| store.books.values())
        .cloned()
        .collect();
    for mut book in books {
        let before = book.clone();
        for watch in &mut book.watches {
            for receipt in [&watch.purchase, &watch.sale]
                .into_iter()
                .flatten()
                .chain(watch.candidates.iter())
            {
                let key = mush_economy::settle_key(ContractId(receipt.contract));
                let _ = mush_economy::resolve_idempotency(ctx, &key);
            }
            // Unknown cold keys are not failures. Only an actual owner reversal
            // releases the first pair, allowing the pilot to try the lesson again.
            let reversed = |receipt: &TradeReceipt| {
                ctx.w.module_state::<EconomyStore>().is_some_and(|economy| {
                    economy
                        .snapshot()
                        .is_reversed(TransactionId(receipt.transaction))
                })
            };
            if watch.purchase.as_ref().is_some_and(reversed) {
                watch.purchase = None;
                watch.sale = None;
            } else if watch.sale.as_ref().is_some_and(reversed) {
                watch.sale = None;
            }
            watch.candidates.retain(|receipt| !reversed(receipt));
            watch.migrate_unmatched();
        }
        if before != book
            && let Err(error) = put_book(ctx, book)
        {
            tracing::warn!(%error, "quest reversed trade evidence could not be repaired");
        }
    }
    let quotes: Vec<_> = ctx
        .w
        .module_state::<MarketStore>()
        .into_iter()
        .flat_map(|market| market.quotes())
        .filter(|q| q.settled.is_some())
        .cloned()
        .collect();
    for quote in quotes {
        record_quote(ctx, &quote);
    }
    let stakes: Vec<_> = ctx
        .w
        .module_state::<crate::store::TacsimStore>()
        .into_iter()
        .flat_map(|s| s.enrolments())
        .filter(|e| e.status == coop::EnrolmentStatus::Active && ctx.w.is_connected(e.pilot))
        .map(|e| coop::stake_key(ContractId(e.lease)))
        .collect();
    for key in stakes {
        let _ = mush_economy::resolve_idempotency(ctx, &key);
    }
}

fn coop_proof(w: &World, actor: ObjId) -> Option<ObjectiveProof> {
    let account = crate::port::licence::gateway_account(w, actor)?;
    let e = w
        .module_state::<crate::store::TacsimStore>()?
        .enrolments()
        .find(|e| {
            e.account == account && e.pilot == actor && e.status == coop::EnrolmentStatus::Active
        })?;
    e.check().ok()?;
    let charter = coop::live_charter(w, actor)?;
    if charter.hull != e.hull || charter.contract.0 != e.lease {
        return None;
    }
    let economy = w.module_state::<EconomyStore>()?;
    let key = coop::stake_key(ContractId(e.lease));
    let stake = economy
        .snapshot()
        .idempotency(&key)
        .or_else(|| economy.cold_key(&key).and_then(|c| c.record.as_ref()))?;
    if economy.is_frozen()
        || economy.snapshot().is_reversed(stake.transaction)
        || stake.idempotency != key
    {
        return None;
    }
    Some(ObjectiveProof::CoopEnrolment {
        actor,
        account,
        hull: e.hull,
        lease: e.lease,
        stake: stake.transaction.0,
    })
}
fn passenger_proof(
    w: &World,
    actor: ObjId,
    npc: ObjId,
    checkpoint: &Checkpoint,
) -> Option<ObjectiveProof> {
    let role = crate::port::passenger::traveller(w, npc)?;
    if !super::close_world(w, npc, actor)
        || w.get(npc)?.location != Some(checkpoint.room)
        || !crate::port::passenger::at_station(w, npc, &role.origin)
    {
        return None;
    }
    Some(ObjectiveProof::Passenger {
        actor,
        npc,
        role: role.key,
        origin: role.origin,
        destination: role.destination,
    })
}
/// Public presence only: private persona, memories and conversation are not
/// proof of an introduction, and the quest never summons or moves its contact.
pub(crate) fn contact_available(w: &World, npc: ObjId, room: ObjId) -> bool {
    w.get(room).is_some_and(|place| {
        place.kind == mush_core::ObjKind::Room && !place.flags.contains(mush_core::Flags::GOING)
    }) && w.get(npc).is_some_and(|object| {
        object.kind == mush_core::ObjKind::Npc
            && object.location == Some(room)
            && !object
                .flags
                .intersects(mush_core::Flags::GOING | mush_core::Flags::DARK)
            && !mush_sim::deployed::is_custodied(w, npc)
            && object
                .attrs
                .get(mush_npc::PROFILE_ATTR)
                .and_then(|attr| attr.as_json())
                .and_then(|value| {
                    serde_json::from_value::<mush_npc::AgentProfile>(value.clone()).ok()
                })
                .is_some_and(|profile| profile.enabled && profile.validate().is_ok())
    })
}
fn contact_proof(
    w: &World,
    actor: ObjId,
    npc: ObjId,
    checkpoint: &Checkpoint,
) -> Option<ObjectiveProof> {
    if !contact_available(w, npc, checkpoint.room) || !super::close_world(w, npc, actor) {
        return None;
    }
    let proof = ObjectiveProof::Contact {
        actor,
        npc,
        name: w.get(npc)?.name.to_string(),
    };
    proof.check(&checkpoint.objective).ok()?;
    Some(proof)
}
fn trade_proof(w: &World, row: &QuestRecord, checkpoint: &Checkpoint) -> Option<ObjectiveProof> {
    let store = w.module_state::<TradeEvidenceStore>()?;
    if store.unreadable.contains(&row.actor) {
        return None;
    }
    let book = store.books.get(&row.actor)?;
    book.check().ok()?;
    let watch = book.watches.iter().find(|watch| {
        watch.key == row.key
            && watch.checkpoint == checkpoint.id
            && watch.accepted == row.accepted
            && watch.objective == checkpoint.objective
    })?;
    let purchase = watch.purchase.clone()?;
    let sale = watch.sale.clone()?;
    if !paid_transaction(
        w,
        purchase.contract,
        purchase.transaction,
        purchase.settled_at,
    ) || !paid_transaction(w, sale.contract, sale.transaction, sale.settled_at)
    {
        return None;
    }
    Some(ObjectiveProof::Trade { purchase, sale })
}

pub(crate) fn capture(
    ctx: &Ctx<'_>,
    row: &QuestRecord,
    checkpoint: &Checkpoint,
) -> Result<CheckEvidence, String> {
    if ctx.actor != row.actor {
        return Err("this is another character's lesson".into());
    }
    let proof = match &checkpoint.objective {
        Objective::Observation => None,
        Objective::CoopEnrolment => Some(
            coop_proof(ctx.w, row.actor).ok_or("Your own paid Co-op charter must be in force.")?,
        ),
        Objective::Passenger { npc } => {
            if !ctx.can_perceive(row.actor, *npc) || !ctx.can_perceive(*npc, row.actor) {
                return Err("Meet the traveller in person first.".into());
            }
            Some(
                passenger_proof(ctx.w, row.actor, *npc, checkpoint)
                    .ok_or("Meet the traveller at the named bar first.")?,
            )
        }
        Objective::Contact { npc } => {
            if !ctx.can_perceive(row.actor, *npc) || !ctx.can_perceive(*npc, row.actor) {
                return Err("Meet the named contact in person first.".into());
            }
            Some(
                contact_proof(ctx.w, row.actor, *npc, checkpoint)
                    .ok_or("Meet the named contact at the specified place first.")?,
            )
        }
        Objective::Trade { .. } => Some(trade_proof(ctx.w, row, checkpoint).ok_or(
            "Complete the paid purchase and later delivery sale after accepting this lesson.",
        )?),
        Objective::Survey { mode } => {
            let handles = ctx
                .w
                .module_state::<PortHandles>()
                .ok_or("survey evidence unavailable")?;
            let book = ctx
                .action_state(&handles.families.quest_survey_evidence, &row.actor)
                .map_err(|_| "survey evidence unreadable")?
                .ok_or("Complete the requested survey after accepting this lesson.")?;
            book.check()?;
            if book.actor != row.actor {
                return Err("survey evidence actor mismatch".into());
            }
            Some(ObjectiveProof::Survey {
                receipt: book
                    .get(mode)
                    .cloned()
                    .ok_or("Complete the requested survey after accepting this lesson.")?,
            })
        }
    };
    let at = Stamp::now(ctx.w);
    if let Some(proof) = &proof {
        proof.check_for(&checkpoint.objective, row.actor, &row.accepted, &at)?;
    }
    let detail_text = match &proof {
        None => {
            if ctx.w.get(row.actor).and_then(|p| p.location) != Some(checkpoint.room) {
                return Err("Visit the named place first.".into());
            }
            super::detail(ctx.w, checkpoint).ok_or("The public detail is unavailable.")?
        }
        Some(ObjectiveProof::CoopEnrolment { .. }) => {
            "Your own Co-op enrolment has a paid stake and a live charter.".into()
        }
        Some(ObjectiveProof::Passenger {
            origin,
            destination,
            ..
        }) => format!(
            "You met the traveller in person. Their public route is {origin} to {destination}."
        ),
        Some(ObjectiveProof::Trade { purchase, sale }) => format!(
            "Paid purchase of {} {} at {}, followed by a paid delivery sale of {} at {} from the same hold.",
            purchase.quantity, purchase.commodity, purchase.market, sale.quantity, sale.market
        ),
        Some(ObjectiveProof::Survey { receipt }) => format!(
            "Your rig completed a {} at {} after this lesson was accepted.",
            receipt.mode, receipt.site
        ),
        Some(ObjectiveProof::Contact { name, .. }) => {
            format!("You acknowledged meeting {name} in person at the named place.")
        }
    };
    Ok(CheckEvidence {
        checkpoint: checkpoint.id.clone(),
        room: checkpoint.room,
        detail_hash: super::model::detail_hash(&detail_text)?,
        detail_text,
        at,
        proof,
    })
}

pub(crate) fn guard(w: &World, row: &QuestRecord, evidence: &CheckEvidence) -> bool {
    let Some(checkpoint) = row
        .definition
        .checkpoints
        .iter()
        .find(|c| c.id == evidence.checkpoint)
    else {
        return false;
    };
    if evidence.check().is_err() || evidence.room != checkpoint.room {
        return false;
    }
    let Some(proof) = &evidence.proof else {
        return checkpoint.objective == Objective::Observation
            && w.get(row.actor).and_then(|p| p.location) == Some(checkpoint.room)
            && super::detail(w, checkpoint).as_deref() == Some(evidence.detail_text.as_str());
    };
    if proof
        .check_for(
            &checkpoint.objective,
            row.actor,
            &row.accepted,
            &evidence.at,
        )
        .is_err()
    {
        return false;
    }
    match proof {
        ObjectiveProof::CoopEnrolment { .. } => coop_proof(w, row.actor).as_ref() == Some(proof),
        ObjectiveProof::Passenger { npc, .. } => {
            passenger_proof(w, row.actor, *npc, checkpoint).as_ref() == Some(proof)
        }
        ObjectiveProof::Contact { npc, .. } => {
            contact_proof(w, row.actor, *npc, checkpoint).as_ref() == Some(proof)
        }
        ObjectiveProof::Trade { .. } => trade_proof(w, row, checkpoint).as_ref() == Some(proof),
        // The provider additionally rereads the exact typed survey slot at all
        // three action boundaries; World alone cannot manufacture that read.
        ObjectiveProof::Survey { .. } => true,
    }
}

/// A later reversal cannot turn acknowledged paid work into an unpaid reward.
/// Survey receipts, once acknowledged, remain historical evidence even when the
/// operator performs another survey and advances the latest-receipt slots.
pub(crate) fn completion_ready(w: &World, row: &QuestRecord) -> bool {
    row.checks.iter().all(|evidence| match &evidence.proof {
        Some(ObjectiveProof::Trade { purchase, sale }) => {
            paid_transaction(
                w,
                purchase.contract,
                purchase.transaction,
                purchase.settled_at,
            ) && paid_transaction(w, sale.contract, sale.transaction, sale.settled_at)
        }
        _ => true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contact_fixture() -> (World, QuestRecord, CheckEvidence, ObjId) {
        use super::super::model::{Prerequisite, QuestDef};
        use mush_core::{Attr, GOD, ObjKind};
        let mut w = World::new_minimal();
        let room = w.create(ObjKind::Room, "Desk", GOD, None).unwrap();
        let elsewhere = w.create(ObjKind::Room, "Elsewhere", GOD, None).unwrap();
        let actor = w
            .create(ObjKind::Player, "Captain", GOD, Some(room))
            .unwrap();
        let npc = w.create(ObjKind::Npc, "Contact", GOD, Some(room)).unwrap();
        w.set_connected(actor, true);
        w.set_attr(
            npc,
            mush_npc::PROFILE_ATTR,
            Attr::json(serde_json::json!(mush_npc::AgentProfile::default()), GOD),
        )
        .unwrap();
        let checkpoint = Checkpoint {
            id: "contact".into(),
            title: "Meet the contact".into(),
            room_pack: "test.station".into(),
            room_entry: "room.desk".into(),
            room,
            detail: String::new(),
            objective: Objective::Contact { npc },
            instructions: "Meet the contact at their desk.".into(),
        };
        let proof = contact_proof(&w, actor, npc, &checkpoint).unwrap();
        let row = QuestRecord {
            schema: 1,
            key: QuestKey {
                source: "test/quest.contact".into(),
                booking: None,
                actor: Some(actor),
            },
            revision: 1,
            actor,
            npc,
            passage: None,
            prerequisite_receipt: None,
            definition: QuestDef {
                schema: 1,
                key: "test/quest.contact".into(),
                title: "A local introduction".into(),
                introduction: "Meet the contact at their desk.".into(),
                destination: "ceres".into(),
                prerequisite: Prerequisite::Open,
                choice: None,
                checkpoints: vec![checkpoint],
                completion: "The captain met the contact in person.".into(),
            },
            accepted: Stamp {
                at_utc: 10,
                at_ic: 10,
            },
            checks: Vec::new(),
            completed: None,
            closed: None,
        };
        let detail_text =
            "You acknowledged meeting Contact in person at the named place.".to_owned();
        let evidence = CheckEvidence {
            checkpoint: "contact".into(),
            room,
            detail_hash: super::super::model::detail_hash(&detail_text).unwrap(),
            detail_text,
            at: Stamp {
                at_utc: 11,
                at_ic: 11,
            },
            proof: Some(proof),
        };
        row.check().unwrap();
        (w, row, evidence, elsewhere)
    }

    #[test]
    fn contact_proof_requires_exact_actor_target_and_fresh_acknowledgement() {
        let (w, row, evidence, _) = contact_fixture();
        let proof = evidence.proof.as_ref().unwrap();
        let objective = &row.definition.checkpoints[0].objective;
        assert!(
            proof
                .check_for(objective, row.actor, &row.accepted, &evidence.at)
                .is_ok()
        );
        assert!(guard(&w, &row, &evidence));
        assert!(proof.check(&Objective::Contact { npc: row.actor }).is_err());
        assert!(proof.check(&Objective::Passenger { npc: row.npc }).is_err());
        assert!(Objective::Contact { npc: ObjId(0) }.check().is_err());
        assert!(
            proof
                .check_for(objective, row.npc, &row.accepted, &evidence.at)
                .is_err()
        );
        assert!(
            proof
                .check_for(
                    objective,
                    row.actor,
                    &row.accepted,
                    &Stamp {
                        at_utc: 9,
                        at_ic: 11
                    }
                )
                .is_err()
        );
        for name in [
            "".to_owned(),
            " ".to_owned(),
            "x".repeat(129),
            "bad\0name".to_owned(),
        ] {
            assert!(
                ObjectiveProof::Contact {
                    actor: row.actor,
                    npc: row.npc,
                    name
                }
                .check(objective)
                .is_err()
            );
        }
    }

    #[test]
    fn contact_guard_rejects_departed_hidden_disabled_and_changed_contacts() {
        use mush_core::{Attr, Flags, GOD, ObjKind};
        for change in 0..9 {
            let (mut w, row, evidence, elsewhere) = contact_fixture();
            match change {
                0 => w.move_to(row.npc, elsewhere).unwrap(),
                1 => w.get_mut(row.npc).unwrap().flags.insert(Flags::DARK),
                2 => w.get_mut(row.npc).unwrap().flags.insert(Flags::GOING),
                3 => w.get_mut(row.npc).unwrap().kind = ObjKind::Thing,
                4 => {
                    w.remove_attr(row.npc, mush_npc::PROFILE_ATTR).unwrap();
                }
                5 => {
                    w.set_attr(
                        row.npc,
                        mush_npc::PROFILE_ATTR,
                        Attr::json(serde_json::json!({"enabled": false}), GOD),
                    )
                    .unwrap();
                }
                6 => w.set_name(row.npc, "Changed identity").unwrap(),
                7 => w.set_connected(row.actor, false),
                8 => w.move_to(row.actor, elsewhere).unwrap(),
                _ => unreachable!(),
            }
            assert!(!guard(&w, &row, &evidence), "change {change}");
        }
        let (w, row, _, _) = contact_fixture();
        assert!(
            contact_proof(
                &w,
                row.actor,
                ObjId(u32::MAX),
                &row.definition.checkpoints[0]
            )
            .is_none()
        );
    }

    fn survey(mode: &str) -> SurveyReceipt {
        SurveyReceipt {
            actor: ObjId(7),
            tv: ObjId(8),
            ordinal: 1,
            site: CanonicalId::new("ahuna").unwrap(),
            mode: mode.into(),
            started_ic: 101,
            due_ic: 110,
            at_utc: 20,
        }
    }
    fn trade(purchase: bool) -> TradeReceipt {
        TradeReceipt {
            actor: ObjId(7),
            quote: u64::from(!purchase),
            contract: u64::from(!purchase),
            transaction: u64::from(!purchase),
            market: if purchase { "stickney" } else { "ceres" }.into(),
            commodity: "industrial_parts".into(),
            quantity: 1,
            hold: 0,
            purchase,
            accepted_at: if purchase { 10 } else { 20 },
            settled_at: if purchase { 15 } else { 25 },
        }
    }
    fn objective() -> Objective {
        Objective::Trade {
            origin: "stickney".into(),
            destination: "ceres".into(),
            commodity: "industrial_parts".into(),
            quantity: 1,
        }
    }
    fn trade_watch() -> TradeWatch {
        TradeWatch {
            key: QuestKey {
                source: "test/quest.trade".into(),
                booking: None,
                actor: Some(ObjId(7)),
            },
            checkpoint: "trade".into(),
            accepted: Stamp {
                at_utc: 10,
                at_ic: 10,
            },
            objective: objective(),
            candidates: Vec::new(),
            purchase: None,
            sale: None,
        }
    }

    #[test]
    fn unmatched_purchases_keep_old_and_replacement_hulls_until_a_pair_is_paid() {
        let old_hull = trade(true);
        let replacement_hull = TradeReceipt {
            hold: 5,
            quote: 2,
            contract: 2,
            transaction: 2,
            accepted_at: 16,
            settled_at: 18,
            ..trade(true)
        };
        let mut watch = trade_watch();
        watch.remember_purchase(old_hull.clone());
        watch.remember_purchase(replacement_hull.clone());
        assert_eq!(watch.candidates.len(), 2);
        assert!(watch.purchase.is_none());

        let mut using_original = watch.clone();
        using_original.select_pair(&trade(false), |_| true);
        assert_eq!(using_original.purchase, Some(old_hull));
        assert!(using_original.candidates.is_empty());

        let sale = TradeReceipt {
            hold: 5,
            quote: 3,
            contract: 3,
            transaction: 3,
            ..trade(false)
        };
        watch.select_pair(&sale, |_| false);
        assert!(
            watch.sale.is_none(),
            "an unpaid or reversed candidate cannot complete the pair"
        );
        watch.select_pair(&sale, |_| true);
        assert_eq!(watch.purchase, Some(replacement_hull));
        assert_eq!(watch.sale, Some(sale));
        let frozen = watch.clone();
        watch.remember_purchase(TradeReceipt {
            hold: 99,
            quote: 99,
            ..trade(true)
        });
        watch.select_pair(&trade(false), |_| true);
        assert_eq!(watch, frozen, "the first completed pair is permanent");
    }

    #[test]
    fn unmatched_candidates_are_bounded_and_keep_only_latest_purchase_per_hold() {
        let mut watch = trade_watch();
        for id in 0..10 {
            watch.remember_purchase(TradeReceipt {
                hold: id,
                quote: id,
                contract: id,
                transaction: id,
                accepted_at: 20 + id as i64,
                settled_at: 30 + id as i64,
                ..trade(true)
            });
        }
        assert_eq!(watch.candidates.len(), MAX_PURCHASE_HOLDS);
        assert!(watch.candidates.iter().all(|r| r.hold >= 2));
        watch.remember_purchase(trade(true));
        assert!(
            watch.candidates.iter().all(|r| r.hold >= 2),
            "replayed old quotes cannot evict newer holds"
        );
        let newer = TradeReceipt {
            hold: 2,
            quote: 20,
            contract: 20,
            transaction: 20,
            accepted_at: 40,
            settled_at: 50,
            ..trade(true)
        };
        watch.remember_purchase(newer.clone());
        assert_eq!(watch.candidates.len(), MAX_PURCHASE_HOLDS);
        assert_eq!(watch.candidates.iter().find(|r| r.hold == 2), Some(&newer));
        let book = TradeBook {
            schema: 1,
            actor: ObjId(7),
            watches: vec![watch],
        };
        assert!(book.check().is_ok());
    }

    #[test]
    fn survey_receipts_bind_actor_fresh_start_and_success_time() {
        let objective = Objective::Survey {
            mode: "scan".into(),
        };
        let accepted = Stamp {
            at_utc: 10,
            at_ic: 100,
        };
        let checked = Stamp {
            at_utc: 20,
            at_ic: 110,
        };
        let mut receipt = survey("scan");
        let proof = ObjectiveProof::Survey {
            receipt: receipt.clone(),
        };
        assert!(
            proof
                .check_for(&objective, ObjId(7), &accepted, &checked)
                .is_ok()
        );
        assert!(
            proof
                .check_for(&objective, ObjId(9), &accepted, &checked)
                .is_err()
        );
        receipt.started_ic = accepted.at_ic;
        assert!(
            ObjectiveProof::Survey { receipt }
                .check_for(&objective, ObjId(7), &accepted, &checked)
                .is_err()
        );
        assert!(
            proof
                .check_for(
                    &objective,
                    ObjId(7),
                    &accepted,
                    &Stamp {
                        at_utc: 19,
                        at_ic: 109
                    }
                )
                .is_err()
        );
    }

    #[test]
    fn survey_book_keeps_both_modes_and_rejects_changed_or_foreign_receipt() {
        let scan = survey("scan");
        let sound = survey("sound");
        let book = SurveyEvidenceBook {
            schema: 1,
            actor: ObjId(7),
            scan: Some(scan.clone()),
            sound: Some(sound.clone()),
        };
        assert!(book.check().is_ok());
        assert!(proof_current(
            Some(&book),
            ObjId(7),
            &ObjectiveProof::Survey {
                receipt: scan.clone()
            }
        ));
        assert!(proof_current(
            Some(&book),
            ObjId(7),
            &ObjectiveProof::Survey { receipt: sound }
        ));
        assert!(!proof_current(
            Some(&book),
            ObjId(9),
            &ObjectiveProof::Survey {
                receipt: scan.clone()
            }
        ));
        let mut changed = scan;
        changed.ordinal += 1;
        assert!(!proof_current(
            Some(&book),
            ObjId(7),
            &ObjectiveProof::Survey { receipt: changed }
        ));
        assert!(!proof_current(
            None,
            ObjId(7),
            &ObjectiveProof::Survey {
                receipt: survey("scan")
            }
        ));
    }

    #[test]
    fn trade_pair_requires_same_actor_hold_route_and_forward_paid_order() {
        let purchase = trade(true);
        let sale = trade(false);
        let proof = ObjectiveProof::Trade {
            purchase: purchase.clone(),
            sale: sale.clone(),
        };
        // Contract, transaction and vault zero are legitimate owner identities.
        assert!(
            proof
                .check_for(
                    &objective(),
                    ObjId(7),
                    &Stamp {
                        at_utc: 10,
                        at_ic: 10
                    },
                    &Stamp {
                        at_utc: 25,
                        at_ic: 25
                    }
                )
                .is_ok()
        );
        for wrong in [
            TradeReceipt {
                actor: ObjId(9),
                ..sale.clone()
            },
            TradeReceipt {
                hold: 4,
                ..sale.clone()
            },
            TradeReceipt {
                market: "stickney".into(),
                ..sale.clone()
            },
            TradeReceipt {
                commodity: "water".into(),
                ..sale.clone()
            },
            TradeReceipt {
                accepted_at: 14,
                ..sale.clone()
            },
        ] {
            assert!(
                ObjectiveProof::Trade {
                    purchase: purchase.clone(),
                    sale: wrong
                }
                .check(&objective())
                .is_err()
            );
        }
        assert!(
            proof
                .check_for(
                    &objective(),
                    ObjId(7),
                    &Stamp {
                        at_utc: 11,
                        at_ic: 10
                    },
                    &Stamp {
                        at_utc: 25,
                        at_ic: 25
                    }
                )
                .is_err()
        );
    }
}

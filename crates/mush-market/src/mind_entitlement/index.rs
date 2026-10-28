//! 17g C2f (the C2b brief §1; the C2e plan §3.2): the per-buyer entitlement
//! index, `market_mind_entitlement_actors`.
//!
//! One row per buyer with an unfunded quote or an open (funded) purchase,
//! keyed by the buyer's `ObjId`. Like v2's `ActorOffers` it **authorizes
//! nothing**: it only lets C2g's `+minds/buy` find the actor's own quote and
//! purchase with a bounded read; every provider still rereads the quote or
//! purchase row and rechecks its buyer. Each write is in the same transaction
//! as the step it follows:
//! - **Quote** (both protocols): the buyer's row, pruned of each indexed quote
//!   whose own row has expired at the world clock (`Fund` refuses a quote at
//!   or after `expires_at_utc`), plus the new quote; a ninth live quote
//!   refuses the Quote with its existing refusal.
//! - **Fund**: the funded quote leaves the row and the operation joins it; a
//!   ninth open purchase refuses the Fund (nothing commits, no money moves).
//! - **Grant and Refund**: the operation leaves the row **of its own buyer**
//!   (`quote.buyer`; GOD may refund, so never the actor's row), and the row is
//!   deleted when both of its sets are empty. mush-core's closed owner-index
//!   table admits exactly this mutation in the terminal
//!   (`terminal_capacity::owner_index`, R2); the Quote and Fund writes are
//!   ordinary typed writes, covered by the boot audit.
//!
//! The boot audit (`check`) is clock-free: every indexed quote exists, names
//! that buyer and was not funded; every indexed operation exists, is funded
//! and names that buyer; every funded operation is indexed under its buyer.
//! It never checks that an indexed quote is unexpired: an abandoned quote
//! stays indexed until the buyer's next Quote prunes it.
use super::codec::Row;
use super::{EntitlementPurchaseV1, EntitlementQuoteV1};
use mush_core::{ActionTxn, CommitToken, ModuleCtx, ObjId, StateFamilyHandle};
use mush_roll::commerce::purchase::PurchasePhase;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The C2b bound on one buyer's live, unfunded quotes.
pub(crate) const MAX_BUYER_QUOTES: usize = 8;
/// The C2b bound on one buyer's open (funded) purchases.
pub(crate) const MAX_BUYER_PURCHASES: usize = 8;

/// The three families the index joins: the retained quotes and operations
/// (its heads) and the index itself. `Families::index` lends them; the
/// lifecycle rig registers them alone.
pub(super) struct Index<'f> {
    pub(super) quotes: &'f StateFamilyHandle<CommitToken, EntitlementQuoteV1>,
    pub(super) operations: &'f StateFamilyHandle<CommitToken, EntitlementPurchaseV1>,
    pub(super) actors: &'f StateFamilyHandle<ObjId, EntitlementActorIndex>,
}

/// One buyer's unfunded quotes and open purchases.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EntitlementActorIndex {
    pub(super) format: u16,
    pub(super) actor: ObjId,
    pub(super) quotes: BTreeSet<CommitToken>,
    pub(super) operations: BTreeSet<CommitToken>,
}
impl EntitlementActorIndex {
    fn empty(actor: ObjId) -> Self {
        Self {
            format: 1,
            actor,
            quotes: BTreeSet::new(),
            operations: BTreeSet::new(),
        }
    }
    fn is_empty(&self) -> bool {
        self.quotes.is_empty() && self.operations.is_empty()
    }
    /// 17g C2g: the buyer this row indexes.
    pub(crate) const fn actor(&self) -> ObjId {
        self.actor
    }
    /// 17g C2g: the buyer's indexed (unfunded) quotes, live or expired.
    pub(crate) const fn quotes(&self) -> &BTreeSet<CommitToken> {
        &self.quotes
    }
    /// 17g C2g: the buyer's open (funded) purchases.
    pub(crate) const fn operations(&self) -> &BTreeSet<CommitToken> {
        &self.operations
    }
}
impl Row for EntitlementActorIndex {
    const FAMILY: &'static str = "market_mind_entitlement_actors";
    const MAX_BYTES: usize = 4096;
    fn key(&self) -> String {
        self.actor.0.to_string()
    }
    fn validate(&self) -> Result<(), String> {
        if self.format != 1
            || self.actor.0 == 0
            || self.quotes.len() > MAX_BUYER_QUOTES
            || self.operations.len() > MAX_BUYER_PURCHASES
            || self.is_empty()
            || !self.quotes.is_disjoint(&self.operations)
        {
            return Err("invalid entitlement buyer index".into());
        }
        Ok(())
    }
}

fn row(
    txn: &mut ActionTxn<'_>,
    f: &Index<'_>,
    buyer: ObjId,
) -> Result<(bool, EntitlementActorIndex), String> {
    let row = txn
        .get(f.actors, &buyer)
        .map_err(|e| e.to_string())?
        .cloned();
    let existed = row.is_some();
    let row = row.unwrap_or_else(|| EntitlementActorIndex::empty(buyer));
    if existed {
        row.validate()?;
    }
    if row.actor != buyer || row.format != 1 {
        return Err("crossed entitlement buyer index".into());
    }
    Ok((existed, row))
}

fn put(
    txn: &mut ActionTxn<'_>,
    f: &Index<'_>,
    existed: bool,
    row: EntitlementActorIndex,
) -> Result<(), String> {
    let buyer = row.actor;
    if row.is_empty() {
        if existed {
            txn.remove(f.actors, &buyer).map_err(|e| e.to_string())?;
        }
        return if txn
            .get(f.actors, &buyer)
            .map_err(|e| e.to_string())?
            .is_none()
        {
            Ok(())
        } else {
            Err("entitlement buyer index post-image differs".into())
        };
    }
    row.validate()?;
    if existed {
        txn.replace(f.actors, &buyer, row.clone())
    } else {
        txn.insert(f.actors, &buyer, row.clone())
    }
    .map_err(|e| e.to_string())?;
    if txn.get(f.actors, &buyer).map_err(|e| e.to_string())? != Some(&row) {
        return Err("entitlement buyer index post-image differs".into());
    }
    Ok(())
}

/// The Quote's write, after the quote row is inserted: the buyer's retained
/// live quotes plus `quote`. An indexed quote whose row expired at `now` is
/// pruned; a ninth live quote refuses.
pub(super) fn quoted(
    txn: &mut ActionTxn<'_>,
    f: &Index<'_>,
    buyer: ObjId,
    quote: CommitToken,
    now: i64,
) -> Result<(), String> {
    let (existed, mut index) = row(txn, f, buyer)?;
    let mut live = BTreeSet::new();
    for token in &index.quotes {
        let indexed = txn
            .get(f.quotes, token)
            .map_err(|e| e.to_string())?
            .ok_or("indexed entitlement quote missing")?;
        // M8c B3: the row is keyed by the acting character (module doc).
        if indexed.publication != *token || indexed.buyer.acting_character() != buyer {
            return Err("indexed entitlement quote differs".into());
        }
        if now < indexed.expires_at_utc {
            live.insert(*token);
        }
    }
    if live.len() >= MAX_BUYER_QUOTES {
        return Err("entitlement buyer quote capacity unavailable".into());
    }
    if !live.insert(quote) || index.operations.contains(&quote) {
        return Err("entitlement quote already indexed".into());
    }
    index.quotes = live;
    put(txn, f, existed, index)
}

/// The Fund's write: the funded quote leaves the buyer's row and the new
/// operation joins it; a ninth open purchase refuses.
pub(super) fn funded(
    txn: &mut ActionTxn<'_>,
    f: &Index<'_>,
    buyer: ObjId,
    quote: CommitToken,
    operation: CommitToken,
) -> Result<(), String> {
    let (existed, mut index) = row(txn, f, buyer)?;
    if !existed || !index.quotes.remove(&quote) {
        return Err("funded quote is not indexed for its buyer".into());
    }
    if index.operations.len() >= MAX_BUYER_PURCHASES {
        return Err("entitlement buyer purchase capacity unavailable".into());
    }
    if !index.operations.insert(operation) {
        return Err("entitlement operation already indexed".into());
    }
    put(txn, f, existed, index)
}

/// The Grant's and Refund's write: the operation leaves its own buyer's row
/// (never the actor's: GOD may refund); the row goes when it is empty.
pub(super) fn released(
    txn: &mut ActionTxn<'_>,
    f: &Index<'_>,
    buyer: ObjId,
    operation: CommitToken,
) -> Result<(), String> {
    let (existed, mut index) = row(txn, f, buyer)?;
    if !existed || !index.operations.remove(&operation) {
        return Err("terminal operation is not indexed for its buyer".into());
    }
    put(txn, f, existed, index)
}

/// The clock-free boot audit of the index (the entitlement boot audit's last
/// step, and the lifecycle rig's whole boot): every quote's buyer, every
/// operation's buyer and openness, and every index row, scanned in bounded
/// pages, then `check`.
pub(super) fn audit(ctx: &ModuleCtx<'_>, f: &Index<'_>) -> Result<(), String> {
    let mut quotes = BTreeMap::new();
    scan(ctx, f.quotes, |key, q| {
        if key != q.key() {
            return Err("entitlement quote key differs".into());
        }
        quotes.insert(q.publication, q.buyer.acting_character());
        Ok(())
    })?;
    let mut funded_quotes = BTreeSet::new();
    let mut operations = BTreeMap::new();
    scan(ctx, f.operations, |key, p| {
        if key != p.key() {
            return Err("entitlement operation key differs".into());
        }
        funded_quotes.insert(p.quote.publication);
        operations.insert(
            p.operation,
            (
                p.quote.buyer.acting_character(),
                p.phase == PurchasePhase::Funded {},
            ),
        );
        Ok(())
    })?;
    let mut rows = Vec::new();
    scan(ctx, f.actors, |key, row| {
        rows.push((key.to_owned(), row.clone()));
        Ok(())
    })?;
    check(rows, &quotes, &funded_quotes, &operations)
}

fn scan<K: Send + Sync + 'static, R: Row + Clone + Send + Sync + 'static>(
    ctx: &ModuleCtx<'_>,
    family: &StateFamilyHandle<K, R>,
    mut inspect: impl FnMut(&str, &R) -> Result<(), String>,
) -> Result<(), String> {
    let mut after: Option<String> = None;
    let mut count = 0_u64;
    loop {
        let page = ctx
            .scan_action_state(family, after.as_deref(), 128)
            .map_err(|e| e.to_string())?;
        for entry in page.entries() {
            count = count.checked_add(1).ok_or("entitlement index count")?;
            if count > super::families::MAX_RETAINED {
                return Err("retained entitlement bound exceeded".into());
            }
            entry.value().validate()?;
            inspect(entry.encoded_key(), entry.value())?;
        }
        let Some(next) = page.next_after() else {
            return Ok(());
        };
        if page.entries().is_empty() || after.as_deref().is_some_and(|old| old >= next.as_str()) {
            return Err("nonprogressing entitlement page".into());
        }
        after = Some(next.to_string());
    }
}

/// The clock-free boot check: `rows` are the index rows with their stored
/// keys, `quotes` every retained quote's buyer, `funded_quotes` the quotes an
/// operation consumed, `operations` every operation's buyer and whether it is
/// still funded (open).
pub(super) fn check(
    rows: impl IntoIterator<Item = (String, EntitlementActorIndex)>,
    quotes: &BTreeMap<CommitToken, ObjId>,
    funded_quotes: &BTreeSet<CommitToken>,
    operations: &BTreeMap<CommitToken, (ObjId, bool)>,
) -> Result<(), String> {
    let mut indexed = BTreeSet::new();
    for (key, row) in rows {
        row.validate()?;
        if row.key() != key {
            return Err("entitlement buyer index key differs".into());
        }
        for quote in &row.quotes {
            if quotes.get(quote) != Some(&row.actor) || funded_quotes.contains(quote) {
                return Err("indexed entitlement quote is missing, foreign or funded".into());
            }
        }
        for operation in &row.operations {
            if operations.get(operation) != Some(&(row.actor, true)) || !indexed.insert(*operation)
            {
                return Err("indexed entitlement operation is missing, foreign or closed".into());
            }
        }
    }
    if operations
        .iter()
        .any(|(operation, (_, open))| *open && !indexed.contains(operation))
    {
        return Err("open entitlement operation is not indexed".into());
    }
    Ok(())
}

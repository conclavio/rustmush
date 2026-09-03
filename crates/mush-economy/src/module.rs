//! The `EconomyModule` (E2b) — the cut that makes the pure E2a engine LIVE: the
//! double-entry journal on the E0 `Ledger`, the full-bundle atomic `WriteBatch`, the
//! strict durable money ack-gate, and directional boot self-heal.
//!
//! ## Contracts (from the E2 design review)
//! - **The journal is the sole authority.** Balances, the idempotency index, the
//!   txid index, and the reversed set are ALL re-derived from the journal at boot
//!   ([`crate::boot`]); a projection that merely lags self-heals, and only real
//!   drift / a lost tail / an unreplayable journal freezes (R-B1/R-B2).
//! - **One post = one atomic bundle** ([`crate::post`]): the journal append + every
//!   touched account projection + the allocator `meta`, all on one `WriteBatch`,
//!   gated on the durable ack; the "posted" confirmation is emitted only on a durable
//!   `Ok(Ok)` via the strict gate (invariant 9 / R-R1).
//! - **No balance-mutation API outside validated posting** (§18-E2 exit criterion) —
//!   [`crate::post`]/[`crate::reverse`] are the only writers, and both go through the
//!   pure E2a `validate`.
//! - **Conservation tripwire** (R-R5): with a zero start and all mint/burn through a
//!   system source/sink, the sum of every account's balance per instrument is
//!   invariantly zero; a non-zero sum at boot is corruption → whole-economy freeze.

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_core::module::{
    GameModule, ModError, ModuleBootData, ModuleCtx, Registrar, WorldEvent, WorldEventKind,
};
use mush_core::{Ctx, Ledger};

use crate::boot;
use crate::ids::{AccountId, TransactionId};
use crate::record::{Account, EconomicTransaction};
use crate::snapshot::EconomySnapshot;

/// The module id and persistence namespace.
pub const NS: &str = "economy";
/// The single journal log id (the world's double-entry ledger).
pub const JOURNAL_LOG: &str = "journal";

/// The in-RAM economy slot — the single source of truth at runtime. The
/// authoritative [`EconomySnapshot`] (instruments + account projections +
/// idempotency + reversed), the append-only journal [`Ledger`] (open tail in RAM),
/// the txid index (for cold reversal — journal-derived at boot), the monotonic
/// allocators, and the whole-economy freeze latch.
#[derive(Clone, Debug, Default)]
pub struct EconomyStore {
    pub(crate) snapshot: EconomySnapshot,
    /// The double-entry journal (append-only; open tail in RAM, cold chunks paged
    /// from `data.kv` at boot).
    pub journal: Ledger,
    /// Every transaction by id — journal-derived at boot, so a cold reversal can read
    /// the original without a store round-trip.
    pub(crate) transactions: BTreeMap<TransactionId, EconomicTransaction>,
    pub(crate) next_account: u64,
    pub(crate) next_transaction: u64,
    /// The whole-economy freeze latch (boot drift / conservation break / a dropped
    /// durable write). While set, [`crate::post`] refuses everything.
    pub frozen: bool,
}

impl EconomyStore {
    /// Assemble a store from boot-derived parts (used by [`crate::boot`]).
    pub(crate) fn from_parts(
        snapshot: EconomySnapshot,
        journal: Ledger,
        transactions: BTreeMap<TransactionId, EconomicTransaction>,
        next_account: u64,
        next_transaction: u64,
        frozen: bool,
    ) -> EconomyStore {
        EconomyStore { snapshot, journal, transactions, next_account, next_transaction, frozen }
    }

    /// The authoritative snapshot (read seam).
    #[must_use]
    pub fn snapshot(&self) -> &EconomySnapshot {
        &self.snapshot
    }
    /// Whether the economy is frozen.
    #[must_use]
    pub fn is_frozen(&self) -> bool {
        self.frozen
    }
    /// A transaction by id (for reversal + staff audit).
    #[must_use]
    pub fn transaction(&self, id: TransactionId) -> Option<&EconomicTransaction> {
        self.transactions.get(&id)
    }
    /// The transactions in id order (staff audit).
    pub fn transactions(&self) -> impl Iterator<Item = &EconomicTransaction> {
        self.transactions.values()
    }

    /// Mint the next account id.
    pub fn alloc_account(&mut self) -> AccountId {
        let id = AccountId(self.next_account);
        self.next_account += 1;
        id
    }
    /// Mint the next transaction id.
    pub(crate) fn alloc_transaction(&mut self) -> TransactionId {
        let id = TransactionId(self.next_transaction);
        self.next_transaction += 1;
        id
    }
    /// The next transaction id WITHOUT allocating it (a unique idempotency-key seed
    /// for interactive commands).
    #[must_use]
    pub fn peek_next_transaction(&self) -> u64 {
        self.next_transaction
    }
    /// The allocator resume document.
    pub(crate) fn meta_doc(&self) -> serde_json::Value {
        serde_json::json!({
            "next_account_id": self.next_account,
            "next_transaction_id": self.next_transaction,
        })
    }

    /// Record a posted transaction into the RAM indexes (called by `apply`-adjacent
    /// module code after the pure apply advances the snapshot).
    pub(crate) fn index_transaction(&mut self, tx: EconomicTransaction) {
        self.transactions.insert(tx.id, tx);
    }
}

/// The economy [`GameModule`]. Hand one to `Engine::with_modules` (after the org
/// module — economy authorizes through it).
#[derive(Debug, Default)]
pub struct EconomyModule;

impl EconomyModule {
    /// A fresh module.
    #[must_use]
    pub fn new() -> EconomyModule {
        EconomyModule
    }
}

impl GameModule for EconomyModule {
    fn name(&self) -> &'static str {
        NS
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.state(EconomyStore::default());
        reg.subscribe(WorldEventKind::ObjDestroyed, Arc::new(on_obj_destroyed));
        crate::commands::register(reg)?;
        Ok(())
    }

    fn on_world_loaded(&mut self, ctx: &mut ModuleCtx<'_>, data: ModuleBootData) -> Result<(), ModError> {
        let store = boot::load_boot(ctx, data)?;
        *ctx.w.module_state_mut::<EconomyStore>().expect("economy store slot") = store;
        Ok(())
    }
}

/// `ObjDestroyed` observer: FREEZE (never delete) every account a destroyed object
/// owned — money is preserved for staff resolution; a frozen account takes no new
/// posting. (ObjIds are never reused, so no impostor can inherit it.) The frozen
/// projections are re-persisted.
fn on_obj_destroyed(ctx: &mut Ctx<'_>, ev: &WorldEvent) {
    let WorldEvent::ObjDestroyed { obj } = ev else { return };
    let obj = *obj;
    let frozen: Vec<Account> = {
        let Some(store) = ctx.w.module_state_mut::<EconomyStore>() else { return };
        let ids: Vec<AccountId> = store
            .snapshot
            .accounts()
            .filter(|a| matches!(a.owner, mush_org::PrincipalId::Object(o) if o == obj))
            .filter(|a| a.status == crate::record::AccountStatus::Active)
            .map(|a| a.id)
            .collect();
        let mut changed = Vec::new();
        for id in ids {
            if let Some(a) = store.snapshot.account_mut(id) {
                a.status = crate::record::AccountStatus::Frozen;
                a.revision += 1;
                changed.push(a.clone());
            }
        }
        changed
    };
    for a in &frozen {
        crate::api::persist_account(ctx.out, a);
    }
}

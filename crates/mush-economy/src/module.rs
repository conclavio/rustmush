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
use mush_core::{Ctx, Ledger, ObjId};
use smol_str::SmolStr;

use crate::boot;
use crate::custody::{ContainerProvider, CustodyProvider, CustodyTable};
use crate::ids::{AccountId, TransactionId, VaultId};
use crate::record::{Account, EconomicTransaction, Vault};
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
    /// The vault-id allocator (E3b1); never reused, resumed from `meta` floored at
    /// one past every journal-derived custody effect.
    pub(crate) next_vault: u64,
    /// The whole-economy freeze latch (boot drift / conservation break / a dropped
    /// durable write). While set, [`crate::post`] refuses everything.
    pub frozen: bool,
    /// The wiring-frozen custody provider table (injected after boot, the
    /// `OrgModule::with_predicates` shape). The built-in `ContainerProvider` is
    /// always last.
    pub(crate) providers: CustodyTable,
}

impl EconomyStore {
    /// Assemble a store from boot-derived parts (used by [`crate::boot`]). The
    /// provider table is empty here — the module injects the frozen one after boot.
    pub(crate) fn from_parts(
        snapshot: EconomySnapshot,
        journal: Ledger,
        transactions: BTreeMap<TransactionId, EconomicTransaction>,
        next_account: u64,
        next_transaction: u64,
        next_vault: u64,
        frozen: bool,
    ) -> EconomyStore {
        EconomyStore {
            snapshot,
            journal,
            transactions,
            next_account,
            next_transaction,
            next_vault,
            frozen,
            providers: CustodyTable::default(),
        }
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
    /// Mint the next vault id (E3b1).
    pub fn alloc_vault(&mut self) -> VaultId {
        let id = VaultId(self.next_vault);
        self.next_vault += 1;
        id
    }
    /// The wiring-frozen custody provider table (an `Arc` clone — cheap).
    #[must_use]
    pub(crate) fn providers(&self) -> CustodyTable {
        self.providers.clone()
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
    /// The allocator resume document — high-water marks boot never lowers.
    pub(crate) fn meta_doc(&self) -> serde_json::Value {
        serde_json::json!({
            "next_account_id": self.next_account,
            "next_transaction_id": self.next_transaction,
            "next_vault_id": self.next_vault,
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
pub struct EconomyModule {
    providers: CustodyTable,
}

impl EconomyModule {
    /// A fresh module with only the built-in container custody provider.
    #[must_use]
    pub fn new() -> EconomyModule {
        EconomyModule::with_custody(Vec::new())
    }

    /// A module with a wiring-frozen custody provider table (E3c hands in
    /// `mush-sim`'s interior provider). The built-in [`ContainerProvider`] is
    /// **always appended last**, so a wired provider wins the routing race for
    /// backings it handles and the container fallback is the last resort.
    #[must_use]
    pub fn with_custody(mut providers: Vec<Arc<dyn CustodyProvider>>) -> EconomyModule {
        providers.push(Arc::new(ContainerProvider));
        EconomyModule { providers: Arc::new(providers) }
    }
}

impl GameModule for EconomyModule {
    fn name(&self) -> &'static str {
        NS
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.state(EconomyStore::default());
        reg.subscribe(WorldEventKind::ObjDestroyed, Arc::new(on_obj_destroyed));
        reg.destroy_gate(Arc::new(EconomicDestroyGate));
        crate::commands::register(reg)?;
        Ok(())
    }

    fn on_world_loaded(&mut self, ctx: &mut ModuleCtx<'_>, data: ModuleBootData) -> Result<(), ModError> {
        let mut store = boot::load_boot(ctx, data, &self.providers)?;
        // Inject the wiring-frozen table into the freshly booted slot.
        store.providers = self.providers.clone();
        *ctx.w.module_state_mut::<EconomyStore>().expect("economy store slot") = store;
        Ok(())
    }
}

// ---- the destroy gate (§4.9) ----------------------------------------------

/// The pre-destroy veto: an object carrying (or containing) live economic weight
/// is not destroyable. Refusing here — at the command layer, before
/// `World::destroy` marks the husk `GOING` and disperses its contents — is the
/// only place that can still see what a vault holds; the `ObjDestroyed` backstop
/// and the boot passes are for destruction that bypasses `@destroy`.
///
/// The message is **neutral for non-staff** (§4.5 existence hiding) and detailed
/// for staff.
#[derive(Debug)]
struct EconomicDestroyGate;

impl mush_core::behavior::DestroyGate for EconomicDestroyGate {
    fn check(
        &self,
        w: &mush_core::World,
        _reg: &mush_core::Registry,
        actor: ObjId,
        target: ObjId,
    ) -> Result<(), String> {
        let Some(store) = w.module_state::<EconomyStore>() else { return Ok(()) };
        let mut reasons: Vec<String> = Vec::new();
        for id in crate::api::subtree(w, target) {
            reasons.extend(economic_holds(w, store, id));
        }
        if reasons.is_empty() {
            return Ok(());
        }
        if crate::api::is_staff(w, actor) {
            Err(format!("Economic records block that destruction: {}", reasons.join("; ")))
        } else {
            Err("You cannot destroy that right now.".to_string())
        }
    }
}

/// Every reason `id` may not be destroyed: its own title, a vault it backs, and
/// the accounts/titles it holds as a principal (§15.3's gates).
fn economic_holds(w: &mush_core::World, store: &EconomyStore, id: ObjId) -> Vec<String> {
    use crate::record::{AccountStatus, AssetLifecycle, CustodyRef, VaultStatus};
    let mut out = Vec::new();
    let snap = store.snapshot();
    // (a) it IS a titled asset.
    if snap.asset(id).is_some_and(|a| a.lifecycle != AssetLifecycle::WrittenOff) {
        out.push(format!("#{} holds a live title record", id.0));
    }
    // (b) it backs a vault.
    for backing in [CustodyRef::Container(id), CustodyRef::SpatialInterior(id)] {
        let Some(v) = snap.vault_by_backing(&backing) else { continue };
        match v.status {
            VaultStatus::Active => out.push(format!("#{} backs active vault #{}", id.0, v.id)),
            VaultStatus::Frozen => {
                let held = crate::custody::resolve(&store.providers, w, &v.backing, None)
                    .ok()
                    .and_then(|p| p.contents(w, &v.backing))
                    .is_none_or(|c| !c.is_empty());
                if held {
                    out.push(format!("#{} backs frozen vault #{}, which still holds things", id.0, v.id));
                }
            }
            VaultStatus::Closed => {}
        }
    }
    // (c) it is a principal with money or titles.
    let principal = mush_org::PrincipalId::Object(id);
    for a in snap.accounts() {
        if a.owner == principal && !a.posted_balance.is_zero() && a.status != AccountStatus::Closed {
            out.push(format!("#{} owns account #{} with a non-zero balance", id.0, a.id));
        }
    }
    let titled: Vec<String> = snap
        .assets_of(&principal)
        .filter(|a| a.lifecycle != AssetLifecycle::WrittenOff)
        .map(|a| format!("#{}", a.object.0))
        .collect();
    if !titled.is_empty() {
        out.push(format!("#{} holds title to {}", id.0, titled.join(", ")));
    }
    out
}

/// `ObjDestroyed` observer: the programmatic-destruction backstop (§4.9).
///
/// FREEZE (never delete) every account a destroyed object owned — money is
/// preserved for staff resolution; a frozen account takes no new posting. (ObjIds
/// are never reused, so no impostor can inherit it.) E3b1 extends that to title:
/// every asset **on** the dead object, **owned by** it, **assigned to** it, or
/// **naming** it as creditor/claimant latches `frozen`, and every vault it backed
/// goes `Frozen`. These are **doc writes, not journal effects** — a reduction of
/// authority, never a title move. The observer runs after `World::destroy` has
/// already dispersed contents, so what a vault *lost* is caught by the boot
/// custody reconciliation instead.
fn on_obj_destroyed(ctx: &mut Ctx<'_>, ev: &WorldEvent) {
    use crate::record::{AccountStatus, CustodyRef, Encumbrance, VaultStatus};
    let WorldEvent::ObjDestroyed { obj } = ev else { return };
    let obj = *obj;
    let principal = mush_org::PrincipalId::Object(obj);
    let (accounts, assets, vaults): (Vec<Account>, Vec<crate::record::EconomicAsset>, Vec<Vault>) = {
        let Some(store) = ctx.w.module_state_mut::<EconomyStore>() else { return };
        let ids: Vec<AccountId> = store
            .snapshot
            .accounts()
            .filter(|a| a.owner == principal)
            .filter(|a| a.status == AccountStatus::Active)
            .map(|a| a.id)
            .collect();
        let mut accounts = Vec::new();
        for id in ids {
            if let Some(a) = store.snapshot.account_mut(id) {
                a.status = AccountStatus::Frozen;
                a.revision += 1;
                accounts.push(a.clone());
            }
        }
        let names = |e: &Encumbrance| match e {
            Encumbrance::Lien { creditor, .. } => *creditor == principal,
            Encumbrance::Disputed { claimant, .. } => *claimant == principal,
        };
        let touched: Vec<ObjId> = store
            .snapshot
            .assets()
            .filter(|a| !a.frozen)
            .filter(|a| {
                a.object == obj
                    || a.title_owner == principal
                    || a.assignment.as_ref().is_some_and(|x| x.assigned_to == principal)
                    || a.encumbrances.iter().any(names)
            })
            .map(|a| a.object)
            .collect();
        let mut assets = Vec::new();
        for id in touched {
            if let Some(a) = store.snapshot.asset_mut(id) {
                // The latch is doc authority: it must NOT disturb a journal-authority
                // field. Bumping `revision` here would make the healthy doc disagree
                // with the journal at its own claimed sequence, and the next boot
                // would read the freeze itself as drift (D-E3-1 / C2).
                a.frozen = true;
                a.frozen_reason = Some(SmolStr::new_static("object-destroyed"));
                assets.push(a.clone());
            }
        }
        let backed: Vec<VaultId> = [CustodyRef::Container(obj), CustodyRef::SpatialInterior(obj)]
            .iter()
            .filter_map(|b| store.snapshot.vault_by_backing(b))
            .filter(|v| v.status == VaultStatus::Active)
            .map(|v| v.id)
            .collect();
        let mut vaults = Vec::new();
        for id in backed {
            if let Some(v) = store.snapshot.vault_mut(id) {
                v.status = VaultStatus::Frozen;
                v.revision += 1;
                vaults.push(v.clone());
            }
        }
        (accounts, assets, vaults)
    };
    for a in &accounts {
        crate::api::persist_account(ctx.out, a);
    }
    for a in &assets {
        crate::api::persist_asset(ctx.out, a);
    }
    for v in &vaults {
        crate::api::persist_vault(ctx.out, v);
    }
}

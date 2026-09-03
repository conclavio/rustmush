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
use crate::config::EconomyConfig;
use crate::custody::{ContainerProvider, CustodyProvider, CustodyTable};
use crate::escalation::Escalation;
use crate::ids::{
    AccountId, ApprovalRequestId, CadenceId, EscalationId, HoldId, ObligationId, TransactionId,
    VaultId,
};
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
    ///
    /// `pub(crate)` since the E4b1 verify fold (B7): `install_commit`'s reachability
    /// argument is *"the journal tail has exactly one mutator, and it is this
    /// function"* — an argument a `pub` field quietly voided, since any crate holding
    /// `&mut EconomyStore` could have appended. Nothing outside this crate ever used
    /// it; [`EconomyStore::journal`] is the read accessor.
    pub(crate) journal: Ledger,
    /// Every transaction by id — journal-derived at boot, so a cold reversal can read
    /// the original without a store round-trip.
    pub(crate) transactions: BTreeMap<TransactionId, EconomicTransaction>,
    pub(crate) next_account: u64,
    pub(crate) next_transaction: u64,
    /// The vault-id allocator (E3b1); never reused, resumed from `meta` floored at
    /// one past every journal-derived custody effect.
    pub(crate) next_vault: u64,
    /// The approval-request-id allocator (E3b2); never reused, resumed from `meta`
    /// floored at one past every journal-derived `consumed_approval`.
    pub(crate) next_approval: u64,
    /// The hold-id allocator (E4a). E4b1 resumes it from `meta` floored at one past
    /// every **journal-derived** hold effect — never from surviving `hold:{id}` docs,
    /// because GC deletes terminal ones and re-minting a garbage-collected id under a
    /// stale `Contract.escrow` reference would attach live escrow to a dead contract.
    /// [`EconomyStore::peek_next_hold`] additionally floors it from the snapshot, so
    /// the pure path can never mint a colliding id even mid-plan.
    pub(crate) next_hold: u64,
    /// The obligation-id allocator (E4b1); never reused, resumed from `meta` floored
    /// past every parsable `obligation:` key — including a corrupt one, whose id was
    /// still minted.
    pub(crate) next_obligation: u64,
    /// The cadence-clock-id allocator (E4b1), floored the same way.
    pub(crate) next_cadence: u64,
    /// The escalation-id allocator (E4b1, §4.6), floored the same way — an
    /// unreadable escalation leaf must never have its id re-minted onto a different
    /// alert, or an acknowledgement would resume the wrong subject.
    pub(crate) next_escalation: u64,
    /// The open + recently-acknowledged level-2/3 escalation records (§4.6), by id.
    /// Leaf docs (`escalation:{id}`): a corrupt one is skipped loudly at boot.
    pub(crate) escalations: BTreeMap<EscalationId, Escalation>,
    /// When each **terminal** hold became terminal (wall clock), journal-derived at
    /// boot and stamped at install. RAM-only on purpose: it is a GC schedule, not
    /// economic truth, so it is re-derived from the journal rather than stored — and
    /// a hold record itself therefore keeps the byte shape E4a journaled.
    pub(crate) hold_terminal_at: BTreeMap<HoldId, i64>,
    /// How many **consecutive** sweep fires have failed to expire each still-`Held`
    /// reservation whose deadline has passed (E4b1 verify fold, S6). RAM-only, like
    /// [`EconomyStore::hold_terminal_at`]: it is a ladder counter, not economic
    /// truth, and a restart is a fair place for the run to begin again.
    pub(crate) hold_expiry_failures: BTreeMap<HoldId, u32>,
    /// How many **consecutive** sweep fires have deferred each obligation's
    /// settlement as transient (S7). RAM-only for the same reason.
    pub(crate) obligation_defers: BTreeMap<ObligationId, u32>,
    /// The wall-clock instant of the last completed sweep fire — the evidence
    /// [`crate::classify_forgiveness`] reads to tell downtime from overload. Durable
    /// in `meta`; a sweep that did not run (a frozen economy) does not stamp it.
    pub(crate) last_sweep_real: i64,
    /// The IC instant of the last completed sweep fire (the time-jump evidence).
    pub(crate) last_sweep_ic: i64,
    /// The wall-clock instant this world's economy last finished booting (E4b1
    /// verify fold, S3). Durable in `meta`, stamped at the end of every boot: a gap
    /// between two sweeps that **contains** this instant provably contains a restart,
    /// which is downtime however short it was — the evidence that stops a 90-second
    /// restart classifying as `Overload` and paging a staffer on the second one.
    pub(crate) last_boot_real: i64,
    /// The accounts **this** recovery latch froze, and only those (E4b1 verify fold,
    /// B4a). Durable in `meta`, accumulated across every boot the latch survives.
    ///
    /// `@econ/unfreeze` clears exactly this list, so an account frozen for its own
    /// reason — a destroyed owner, a balance that would not fit `i64` — stays frozen
    /// and stays visible as a thing needing attention. Without it, recovery was
    /// either "unfreeze every account by hand" or "unfreeze them all and lose the
    /// list of which ones were really broken".
    pub(crate) boot_frozen: Vec<AccountId>,
    /// The whole-economy freeze latch (boot drift / conservation break / a dropped
    /// durable write). While set, [`crate::post`] refuses everything, and so do the
    /// non-recovery control verbs (`@account/open`, `@account/approval`,
    /// `@vault/create`).
    ///
    /// **Durable** (`meta.frozen`, written by `meta_doc`): the boot that
    /// detects drift also heals the drifted projections and persists them, so
    /// nothing in the docs would ever re-derive the freeze — only staff
    /// `@econ/unfreeze` clears it.
    ///
    /// `pub(crate)` since the E4b1 verify fold (B7): a latch any crate could clear by
    /// assignment is not a latch. [`EconomyStore::is_frozen`] reads it and
    /// [`crate::set_economy_frozen`] is the one durable writer.
    pub(crate) frozen: bool,
    /// The wiring-frozen custody provider table (injected after boot, the
    /// `OrgModule::with_predicates` shape). The built-in `ContainerProvider` is
    /// always last.
    pub(crate) providers: CustodyTable,
    /// The ruleset's configurable limits (M1), injected after boot the same way.
    /// Absent configuration is [`EconomyConfig::default`], so a server with no
    /// `[economy]` section behaves exactly as it did before the setting existed.
    pub(crate) config: EconomyConfig,
}

/// Everything [`crate::boot`] derives, handed to [`EconomyStore::from_parts`] as one
/// value rather than a dozen positional arguments (E4b1 doubled the count).
pub(crate) struct BootParts {
    pub(crate) snapshot: EconomySnapshot,
    pub(crate) journal: Ledger,
    pub(crate) transactions: BTreeMap<TransactionId, EconomicTransaction>,
    pub(crate) escalations: BTreeMap<EscalationId, Escalation>,
    pub(crate) hold_terminal_at: BTreeMap<HoldId, i64>,
    pub(crate) next_account: u64,
    pub(crate) next_transaction: u64,
    pub(crate) next_vault: u64,
    pub(crate) next_approval: u64,
    pub(crate) next_hold: u64,
    pub(crate) next_obligation: u64,
    pub(crate) next_cadence: u64,
    pub(crate) next_escalation: u64,
    pub(crate) last_sweep_real: i64,
    pub(crate) last_sweep_ic: i64,
    pub(crate) last_boot_real: i64,
    pub(crate) boot_frozen: Vec<AccountId>,
    pub(crate) frozen: bool,
}

impl EconomyStore {
    /// Assemble a store from boot-derived parts (used by [`crate::boot`]). The
    /// provider table is empty here — the module injects the frozen one after boot.
    pub(crate) fn from_parts(p: BootParts) -> EconomyStore {
        EconomyStore {
            snapshot: p.snapshot,
            journal: p.journal,
            transactions: p.transactions,
            escalations: p.escalations,
            hold_terminal_at: p.hold_terminal_at,
            next_account: p.next_account,
            next_transaction: p.next_transaction,
            next_vault: p.next_vault,
            next_approval: p.next_approval,
            next_hold: p.next_hold,
            next_obligation: p.next_obligation,
            next_cadence: p.next_cadence,
            next_escalation: p.next_escalation,
            hold_expiry_failures: BTreeMap::new(),
            obligation_defers: BTreeMap::new(),
            last_sweep_real: p.last_sweep_real,
            last_sweep_ic: p.last_sweep_ic,
            last_boot_real: p.last_boot_real,
            boot_frozen: p.boot_frozen,
            frozen: p.frozen,
            providers: CustodyTable::default(),
            config: EconomyConfig::default(),
        }
    }

    /// The authoritative snapshot (read seam).
    #[must_use]
    pub fn snapshot(&self) -> &EconomySnapshot {
        &self.snapshot
    }
    /// The double-entry journal (read seam, B7). The tail has exactly one mutator —
    /// `api::install_commit` — and that is the whole reachability argument its two
    /// post-move assertions rest on.
    #[must_use]
    pub fn journal(&self) -> &Ledger {
        &self.journal
    }
    /// Whether the economy is frozen.
    #[must_use]
    pub fn is_frozen(&self) -> bool {
        self.frozen
    }
    /// The ruleset's configurable limits (M1) — read **live**, never captured, so a
    /// lowered mint cap binds the very next settlement.
    #[must_use]
    pub fn config(&self) -> &EconomyConfig {
        &self.config
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

    /// Mint the next account id, or `None` at the ceiling (E4b1 verify fold, B2).
    ///
    /// Every allocator here used to be `self.next += 1`, which **panics in a debug
    /// build and silently wraps in a release one** at `u64::MAX` — and one of them
    /// (`alloc_escalation`) is called from the sweep, where a panic aborts the world
    /// task. An exhausted allocator is not a reason to lose the world: the mint
    /// fails, the caller logs loudly and refuses that one operation, and everything
    /// else keeps running.
    pub fn alloc_account(&mut self) -> Option<AccountId> {
        let id = AccountId(self.next_account);
        self.next_account = self.next_account.checked_add(1)?;
        Some(id)
    }
    /// Mint the next vault id (E3b1), or `None` at the ceiling.
    pub fn alloc_vault(&mut self) -> Option<VaultId> {
        let id = VaultId(self.next_vault);
        self.next_vault = self.next_vault.checked_add(1)?;
        Some(id)
    }
    /// Mint the next approval-request id (E3b2), or `None` at the ceiling.
    pub fn alloc_approval(&mut self) -> Option<ApprovalRequestId> {
        let id = ApprovalRequestId(self.next_approval);
        self.next_approval = self.next_approval.checked_add(1)?;
        Some(id)
    }
    /// The wiring-frozen custody provider table (an `Arc` clone — cheap).
    #[must_use]
    pub(crate) fn providers(&self) -> CustodyTable {
        self.providers.clone()
    }
    /// The next transaction id WITHOUT allocating it (a unique idempotency-key seed
    /// for interactive commands).
    #[must_use]
    pub fn peek_next_transaction(&self) -> u64 {
        self.next_transaction
    }
    /// Mint the next obligation id (E4b1), or `None` at the ceiling.
    pub fn alloc_obligation(&mut self) -> Option<ObligationId> {
        let id = ObligationId(self.next_obligation);
        self.next_obligation = self.next_obligation.checked_add(1)?;
        Some(id)
    }
    /// Mint the next cadence-clock id (E4b1), or `None` at the ceiling.
    pub fn alloc_cadence(&mut self) -> Option<CadenceId> {
        let id = CadenceId(self.next_cadence);
        self.next_cadence = self.next_cadence.checked_add(1)?;
        Some(id)
    }
    /// Mint the next escalation id (E4b1, §4.6), or `None` at the ceiling.
    ///
    /// This is the one allocator the **sweep** calls, so it is the one whose
    /// overflow would have taken the world task down (B2). A `page` that cannot mint
    /// an id still suspends its subject, with a `Suspended { Staff }` reason and a
    /// loud log — the subject stops either way, which is the part that protects
    /// money.
    pub fn alloc_escalation(&mut self) -> Option<EscalationId> {
        let id = EscalationId(self.next_escalation);
        self.next_escalation = self.next_escalation.checked_add(1)?;
        Some(id)
    }
    /// One escalation record.
    #[must_use]
    pub fn escalation(&self, id: EscalationId) -> Option<&Escalation> {
        self.escalations.get(&id)
    }
    /// Every escalation record, in id order.
    pub fn escalations(&self) -> impl Iterator<Item = &Escalation> {
        self.escalations.values()
    }
    /// Install (or replace) an escalation record in RAM. The doc write is
    /// [`crate::api::persist_escalation`]'s; both always happen together.
    pub(crate) fn put_escalation(&mut self, e: Escalation) {
        self.escalations.insert(e.id, e);
    }
    /// When a terminal hold became terminal, if this store knows (GC evidence).
    #[must_use]
    pub(crate) fn hold_terminal_at(&self, id: HoldId) -> Option<i64> {
        self.hold_terminal_at.get(&id).copied()
    }
    /// The last completed sweep's two stamps (wall, IC).
    #[must_use]
    pub fn last_sweep(&self) -> (i64, i64) {
        (self.last_sweep_real, self.last_sweep_ic)
    }
    /// When this economy last finished booting (S3's classifier evidence).
    #[must_use]
    pub fn last_boot(&self) -> i64 {
        self.last_boot_real
    }
    /// Whether the gap since the last recorded sweep **contains a boot** — the S3
    /// evidence handed to [`crate::classify_forgiveness`].
    #[must_use]
    pub fn gap_spans_boot(&self) -> bool {
        self.last_boot_real > self.last_sweep_real
    }
    /// The accounts the recovery latch froze, in id order (B4a) — exactly what
    /// `@econ/unfreeze` restores.
    pub fn boot_frozen(&self) -> impl Iterator<Item = AccountId> + '_ {
        self.boot_frozen.iter().copied()
    }
    /// Forget the boot-freeze list (`@econ/unfreeze` has just restored it).
    pub(crate) fn clear_boot_frozen(&mut self) {
        self.boot_frozen.clear();
    }
    /// How many consecutive sweep fires have failed to expire this reservation (S6).
    #[must_use]
    pub fn expiry_refusals(&self, id: HoldId) -> u32 {
        self.hold_expiry_failures.get(&id).copied().unwrap_or(0)
    }
    /// How many consecutive sweep fires have deferred this obligation (S7).
    #[must_use]
    pub fn deferred_fires(&self, id: ObligationId) -> u32 {
        self.obligation_defers.get(&id).copied().unwrap_or(0)
    }
    /// The next hold id validation may mint. E4b1 resumes `next_hold` from the
    /// **journal's** hold effects at boot (never from surviving docs, which GC
    /// deletes); the snapshot floor stays as the belt, so a stale allocator can never
    /// aim a placement at a live reservation even mid-plan.
    #[must_use]
    pub fn peek_next_hold(&self) -> u64 {
        self.next_hold.max(self.snapshot.next_hold_floor())
    }
    /// The allocator resume document — high-water marks boot never lowers, plus the
    /// **durable** whole-economy freeze latch.
    ///
    /// `frozen` lives here because a latch that only exists in RAM is not a latch:
    /// the boot that detects drift freezes, heals the drifted docs to the journal,
    /// persists them — and the NEXT boot then reads a clean world and comes up
    /// spending. Boot ORs the stored value into what it derives (a read tolerant of
    /// the field's absence, for pre-E3 meta docs), and only staff `@econ/unfreeze`
    /// clears it.
    pub(crate) fn meta_doc(&self) -> serde_json::Value {
        serde_json::json!({
            "next_account_id": self.next_account,
            "next_transaction_id": self.next_transaction,
            "next_vault_id": self.next_vault,
            "next_approval_id": self.next_approval,
            "next_hold_id": self.next_hold,
            "next_obligation_id": self.next_obligation,
            "next_cadence_id": self.next_cadence,
            "next_escalation_id": self.next_escalation,
            "last_sweep_real": self.last_sweep_real,
            "last_sweep_ic": self.last_sweep_ic,
            "last_boot_real": self.last_boot_real,
            "boot_frozen": self.boot_frozen.iter().map(|a| a.0).collect::<Vec<u64>>(),
            "frozen": self.frozen,
        })
    }

    /// Record a posted transaction into the RAM indexes (called by `apply`-adjacent
    /// module code after the pure apply advances the snapshot).
    ///
    /// E4b1 also advances the hold allocator past every reservation this transaction
    /// **placed**, and stamps the GC clock of every one it made terminal. The
    /// allocator floor may never come from the surviving docs (GC deletes terminal
    /// ones), so it is maintained here and at boot from the journal alone.
    pub(crate) fn index_transaction(&mut self, tx: EconomicTransaction) {
        for effect in &tx.hold_effects {
            match effect.kind {
                crate::record::HoldEffectKind::Placed => {
                    self.next_hold = self.next_hold.max(effect.hold.id.0.saturating_add(1));
                }
                crate::record::HoldEffectKind::Captured
                | crate::record::HoldEffectKind::Released
                | crate::record::HoldEffectKind::Expired => {
                    self.hold_terminal_at.insert(effect.hold.id, tx.posted_at);
                }
            }
        }
        self.transactions.insert(tx.id, tx);
    }

    /// Stamp the end of a sweep fire (§4.3) — the classifier's evidence.
    pub(crate) fn stamp_sweep(&mut self, real: i64, ic: i64) {
        self.last_sweep_real = real;
        self.last_sweep_ic = ic;
    }
}

/// The economy [`GameModule`]. Hand one to `Engine::with_modules` (after the org
/// module — economy authorizes through it).
#[derive(Debug, Default)]
pub struct EconomyModule {
    providers: CustodyTable,
    config: EconomyConfig,
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
        EconomyModule { providers: Arc::new(providers), config: EconomyConfig::default() }
    }

    /// Wire the ruleset's configurable limits (M1). Omitting this is
    /// [`EconomyConfig::default`] — the pre-M1 behaviour, with the default mint cap.
    #[must_use]
    pub fn with_config(mut self, config: EconomyConfig) -> EconomyModule {
        self.config = config;
        self
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
        // The ONE economy tick (D-E4-3): holds, obligations and GC on a single
        // bounded `Coarse` sweep. The core scheduler is in-RAM and re-armed from
        // scratch at boot, so there are deliberately no per-record timers — every
        // catch-up is computed from schedule arithmetic, never from a tick count.
        reg.tick(
            crate::sweep::SWEEP_TAG,
            mush_core::module::TickCadence::Coarse(std::time::Duration::from_secs(
                crate::SWEEP_PERIOD_SECS.unsigned_abs(),
            )),
            Arc::new(crate::sweep::sweep),
        )?;
        crate::commands::register(reg)?;
        Ok(())
    }

    fn on_world_loaded(&mut self, ctx: &mut ModuleCtx<'_>, data: ModuleBootData) -> Result<(), ModError> {
        let mut store = boot::load_boot(ctx, data, &self.providers)?;
        // Inject the wiring-frozen table and config into the freshly booted slot.
        store.providers = self.providers.clone();
        store.config = self.config.clone();
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
                // E3's split, applied here too (E4b1 verify fold, B8): `status` is
                // doc authority and `revision` is JOURNAL authority (`apply` bumps it
                // once per touched account per entry, and boot re-derives it as
                // `1 + touches`). Bumping it from a doc-authority write made the
                // healthy doc claim a revision its own `last_journal_seq` never
                // produced — the exact shape `set_account_status` and
                // `set_asset_frozen` already refuse, and the asset arm below is the
                // model. Boot's `max` hid the damage; an `expected_revisions` guard
                // written against the pre-freeze value did not.
                a.status = AccountStatus::Frozen;
                accounts.push(a.clone());
            }
        }
        let names = |e: &Encumbrance| match e {
            Encumbrance::Lien { creditor, .. } => *creditor == principal,
            Encumbrance::Disputed { claimant, .. } => *claimant == principal,
            Encumbrance::Lease { lessor, .. } => *lessor == principal,
            // Collateral names no principal of its own — the contract does, and a
            // destroyed party is E4b2's contract reconciliation, not an asset freeze.
            Encumbrance::Collateral { .. } => false,
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

// ---- the org scope-use binder (§4.8.2) ------------------------------------

/// Answers `mush-org`'s `@org/scope retire` question — *does the economy still bind
/// this compartment?* — through the wiring-frozen [`mush_org::ScopeUseProvider`]
/// table.
///
/// The dependency runs economy → org, so org cannot look for an account itself; it
/// asks, exactly as an org `AccessPolicy` asks faction about standing. Retiring a
/// bound scope would strand the records pointing at it: after E3b2 a retired scope
/// authorizes nobody, so a live org account under it becomes unspendable by anyone
/// but staff.
#[derive(Debug)]
pub struct EconomyScopeUse;

impl mush_org::ScopeUseProvider for EconomyScopeUse {
    fn binder(&self) -> &str {
        "the economy"
    }

    fn scope_in_use(&self, w: &mush_core::World, scope: mush_org::OrgScopeId) -> Option<String> {
        use crate::record::{AccountStatus, AssetLifecycle, VaultStatus};
        let store = w.module_state::<EconomyStore>()?;
        let snap = store.snapshot();
        if let Some(a) =
            snap.accounts().find(|a| a.scope == Some(scope) && a.status != AccountStatus::Closed)
        {
            return Some(format!("account #{} is still open on it", a.id));
        }
        if let Some(x) =
            snap.assets().find(|x| x.scope == Some(scope) && x.lifecycle != AssetLifecycle::WrittenOff)
        {
            return Some(format!("asset #{} is still titled to it", x.object.0));
        }
        if let Some(v) = snap.vaults().find(|v| v.scope == Some(scope) && v.status != VaultStatus::Closed)
        {
            return Some(format!("vault #{} is still bound to it", v.id));
        }
        None
    }
}

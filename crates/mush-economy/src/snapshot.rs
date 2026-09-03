//! [`EconomySnapshot`] — the in-RAM authoritative economic state the pure engine
//! reads (E2a, grown in E3a). It is the E2a analog of `OrgState`: the instrument
//! registry, the account projections, the idempotency index, the set of
//! already-reversed transactions, and (E3a) the asset / vault / approval maps plus
//! their derived indexes. The engine validates against ONE immutable snapshot
//! (invariant 5); [`crate::apply`] returns a NEW snapshot rather than mutating in
//! place.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use mush_core::ObjId;
use mush_org::PrincipalId;

use crate::clock::CadenceClock;
use crate::contract::{ContractState, EconomicContract, TypedCondition};
use crate::ids::{
    AccountId, ApprovalRequestId, CadenceId, ContractId, HoldId, InstrumentId, ObligationId,
    TransactionId, VaultId,
};
use crate::obligation::Obligation;
use crate::record::{
    Account, ApprovalRequest, ApprovalStatus, CustodyRef, EconomicAsset, Hold, IdempotencyKey,
    IdempotencyRecord, InstrumentDef, Vault,
};

/// The authoritative economic state (pre-image for validation). In the live module
/// (E2b/E3b) this is re-derived from the journal at boot; here it is the value the
/// pure engine reads and `apply` advances.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EconomySnapshot {
    instruments: BTreeMap<InstrumentId, InstrumentDef>,
    accounts: BTreeMap<AccountId, Account>,
    idempotency: BTreeMap<IdempotencyKey, IdempotencyRecord>,
    reversed: BTreeSet<TransactionId>,
    // ---- E3a ----
    assets: BTreeMap<ObjId, EconomicAsset>,
    vaults: BTreeMap<VaultId, Vault>,
    approvals: BTreeMap<ApprovalRequestId, ApprovalRequest>,
    /// The ONE live (pending) request per plan hash (A1: duplicate live requests per
    /// hash would let a plan collect approvals twice).
    approvals_by_hash: BTreeMap<[u8; 32], ApprovalRequestId>,
    /// Derived: the assets each principal holds title to. Keyed by a `Hash`-only
    /// `PrincipalId`; the VALUES are ordered sets, so every iteration a caller sees
    /// is deterministic (the map itself is never iterated wholesale, and nothing
    /// derived from it is ever hashed).
    assets_by_owner: HashMap<PrincipalId, BTreeSet<ObjId>>,
    /// Derived: one vault per backing.
    vaults_by_backing: BTreeMap<CustodyRef, VaultId>,
    // ---- E4a ----
    holds: BTreeMap<HoldId, Hold>,
    /// Derived: every hold on an account, in id order — the `held_amount`
    /// re-derivation and the per-account hold cap read it.
    holds_by_account: BTreeMap<AccountId, BTreeSet<HoldId>>,
    cadences: BTreeMap<CadenceId, CadenceClock>,
    obligations: BTreeMap<ObligationId, Obligation>,
    contracts: BTreeMap<ContractId, EconomicContract>,
    /// Derived: the `(object, vault)` pairs a **live** contract's `DepositInto`
    /// condition holds, and which contract holds each. Only that contract's own
    /// settlement or refund may withdraw the object (§3.5).
    contract_locks: BTreeMap<(ObjId, VaultId), ContractId>,
}

impl EconomySnapshot {
    /// An empty snapshot.
    #[must_use]
    pub fn new() -> EconomySnapshot {
        EconomySnapshot::default()
    }

    // ---- reads ------------------------------------------------------------

    /// Look up an instrument.
    #[must_use]
    pub fn instrument(&self, id: &InstrumentId) -> Option<&InstrumentDef> {
        self.instruments.get(id)
    }
    /// Look up an account.
    #[must_use]
    pub fn account(&self, id: AccountId) -> Option<&Account> {
        self.accounts.get(&id)
    }
    /// The idempotency record for a key, if any.
    #[must_use]
    pub fn idempotency(&self, key: &IdempotencyKey) -> Option<&IdempotencyRecord> {
        self.idempotency.get(key)
    }
    /// Whether a transaction has already been reversed.
    #[must_use]
    pub fn is_reversed(&self, tx: TransactionId) -> bool {
        self.reversed.contains(&tx)
    }
    /// Every account (staff inspection / conservation checks).
    pub fn accounts(&self) -> impl Iterator<Item = &Account> {
        self.accounts.values()
    }
    /// Every instrument.
    pub fn instruments(&self) -> impl Iterator<Item = &InstrumentDef> {
        self.instruments.values()
    }

    /// Look up an asset by the object it titles.
    #[must_use]
    pub fn asset(&self, object: ObjId) -> Option<&EconomicAsset> {
        self.assets.get(&object)
    }
    /// Every asset, in object order.
    pub fn assets(&self) -> impl Iterator<Item = &EconomicAsset> {
        self.assets.values()
    }
    /// The assets a principal holds title to, in object order.
    pub fn assets_of(&self, owner: &PrincipalId) -> impl Iterator<Item = &EconomicAsset> {
        self.assets_by_owner
            .get(owner)
            .into_iter()
            .flat_map(|set| set.iter().filter_map(|o| self.assets.get(o)))
    }
    /// Look up a vault.
    #[must_use]
    pub fn vault(&self, id: VaultId) -> Option<&Vault> {
        self.vaults.get(&id)
    }
    /// Every vault, in id order.
    pub fn vaults(&self) -> impl Iterator<Item = &Vault> {
        self.vaults.values()
    }
    /// The single vault bound to a backing, if any (one vault per backing).
    #[must_use]
    pub fn vault_by_backing(&self, backing: &CustodyRef) -> Option<&Vault> {
        self.vaults_by_backing.get(backing).and_then(|id| self.vaults.get(id))
    }
    /// Whether `object` is the backing of some vault (entering one is a `Deposit`,
    /// never a plain withdrawal destination — invariant 15 / A2).
    #[must_use]
    pub fn is_vault_backing(&self, object: ObjId) -> bool {
        self.vaults_by_backing.contains_key(&CustodyRef::Container(object))
            || self.vaults_by_backing.contains_key(&CustodyRef::SpatialInterior(object))
    }
    /// Look up an approval request.
    #[must_use]
    pub fn approval(&self, id: ApprovalRequestId) -> Option<&ApprovalRequest> {
        self.approvals.get(&id)
    }
    /// Every approval request, in id order.
    pub fn approvals(&self) -> impl Iterator<Item = &ApprovalRequest> {
        self.approvals.values()
    }
    /// The ONE live (pending) request bound to a plan hash, if any.
    #[must_use]
    pub fn live_approval_for_hash(&self, hash: &[u8; 32]) -> Option<&ApprovalRequest> {
        self.approvals_by_hash
            .get(hash)
            .and_then(|id| self.approvals.get(id))
            .filter(|r| r.status == ApprovalStatus::Pending)
    }

    // ---- E4a reads --------------------------------------------------------

    /// Look up a hold.
    #[must_use]
    pub fn hold(&self, id: HoldId) -> Option<&Hold> {
        self.holds.get(&id)
    }
    /// Every hold, in id order.
    pub fn holds(&self) -> impl Iterator<Item = &Hold> {
        self.holds.values()
    }
    /// The holds against one account, in id order.
    pub fn holds_of(&self, account: AccountId) -> impl Iterator<Item = &Hold> {
        self.holds_by_account
            .get(&account)
            .into_iter()
            .flat_map(|set| set.iter().filter_map(|h| self.holds.get(h)))
    }
    /// One past the highest hold id in the snapshot — the allocator floor a caller
    /// with no persisted allocator can still mint safely from. E4b1 floors the real
    /// allocator from the JOURNAL's hold effects instead, because GC deletes terminal
    /// hold docs and a stale `Contract.escrow` id must never be re-minted.
    #[must_use]
    pub fn next_hold_floor(&self) -> u64 {
        self.holds.keys().next_back().map_or(1, |h| h.0.saturating_add(1))
    }
    /// How many `Held` holds an account currently carries (invariant 26's cap).
    #[must_use]
    pub fn active_holds_on(&self, account: AccountId) -> usize {
        self.holds_of(account).filter(|h| h.status.is_held()).count()
    }
    /// Look up a cadence clock.
    #[must_use]
    pub fn cadence(&self, id: CadenceId) -> Option<&CadenceClock> {
        self.cadences.get(&id)
    }
    /// Every cadence clock, in id order.
    pub fn cadences(&self) -> impl Iterator<Item = &CadenceClock> {
        self.cadences.values()
    }
    /// Look up an obligation.
    #[must_use]
    pub fn obligation(&self, id: ObligationId) -> Option<&Obligation> {
        self.obligations.get(&id)
    }
    /// Every obligation, in id order.
    pub fn obligations(&self) -> impl Iterator<Item = &Obligation> {
        self.obligations.values()
    }
    /// Look up a contract.
    #[must_use]
    pub fn contract(&self, id: ContractId) -> Option<&EconomicContract> {
        self.contracts.get(&id)
    }
    /// Every contract, in id order.
    pub fn contracts(&self) -> impl Iterator<Item = &EconomicContract> {
        self.contracts.values()
    }
    /// Every `(object, vault)` pair a live contract holds (§3.5). Re-derived at boot
    /// from the `Active`/`ReadyToSettle` contracts' `DepositInto` conditions.
    #[must_use]
    pub fn contract_locks(&self) -> BTreeSet<(ObjId, VaultId)> {
        self.contract_locks.keys().copied().collect()
    }
    /// Which contract holds `object` in `vault`, if any.
    #[must_use]
    pub fn custody_lock(&self, object: ObjId, vault: VaultId) -> Option<ContractId> {
        self.contract_locks.get(&(object, vault)).copied()
    }

    // ---- writes (test construction + `apply`) -----------------------------

    /// Insert/replace an instrument.
    pub fn put_instrument(&mut self, def: InstrumentDef) {
        self.instruments.insert(def.id.clone(), def);
    }
    /// Insert/replace an account.
    pub fn put_account(&mut self, account: Account) {
        self.accounts.insert(account.id, account);
    }
    /// Record an idempotency result.
    pub fn put_idempotency(&mut self, record: IdempotencyRecord) {
        self.idempotency.insert(record.idempotency.clone(), record);
    }
    /// Mark a transaction reversed (at-most-once guard).
    pub fn mark_reversed(&mut self, tx: TransactionId) {
        self.reversed.insert(tx);
    }
    /// Insert/replace an asset, keeping the owner index coherent.
    pub fn put_asset(&mut self, asset: EconomicAsset) {
        let object = asset.object;
        if let Some(prev) = self.assets.get(&object)
            && prev.title_owner != asset.title_owner
            && let Some(set) = self.assets_by_owner.get_mut(&prev.title_owner)
        {
            set.remove(&object);
            if set.is_empty() {
                self.assets_by_owner.remove(&prev.title_owner);
            }
        }
        self.assets_by_owner.entry(asset.title_owner.clone()).or_default().insert(object);
        self.assets.insert(object, asset);
    }
    /// Insert/replace a vault, keeping the backing index coherent (one vault per
    /// backing; a re-bound vault releases its old backing).
    pub fn put_vault(&mut self, vault: Vault) {
        if let Some(prev) = self.vaults.get(&vault.id)
            && prev.backing != vault.backing
        {
            self.vaults_by_backing.remove(&prev.backing);
        }
        self.vaults_by_backing.insert(vault.backing.clone(), vault.id);
        self.vaults.insert(vault.id, vault);
    }
    /// Insert/replace an approval request, keeping the by-hash index coherent.
    ///
    /// The index holds at most ONE live request per hash, **first one wins**: a
    /// second pending request for the same hash never displaces the live one (E3b
    /// refuses to mint it in the first place; this keeps the pure index honest if it
    /// somehow does).
    pub fn put_approval(&mut self, request: ApprovalRequest) {
        let hash = request.plan_hash;
        let id = request.id;
        let pending = request.status == ApprovalStatus::Pending;
        match self.approvals_by_hash.get(&hash).copied() {
            Some(cur) if cur == id => {
                if !pending {
                    self.approvals_by_hash.remove(&hash);
                }
            }
            Some(cur) => {
                // Keep the incumbent unless it is no longer live.
                let incumbent_live =
                    self.approvals.get(&cur).is_some_and(|r| r.status == ApprovalStatus::Pending);
                if !incumbent_live && pending {
                    self.approvals_by_hash.insert(hash, id);
                }
            }
            None => {
                if pending {
                    self.approvals_by_hash.insert(hash, id);
                }
            }
        }
        self.approvals.insert(id, request);
    }
    /// Insert/replace a hold, keeping the per-account index coherent.
    pub fn put_hold(&mut self, hold: Hold) {
        if let Some(prev) = self.holds.get(&hold.id)
            && prev.account != hold.account
            && let Some(set) = self.holds_by_account.get_mut(&prev.account)
        {
            set.remove(&hold.id);
            if set.is_empty() {
                self.holds_by_account.remove(&prev.account);
            }
        }
        self.holds_by_account.entry(hold.account).or_default().insert(hold.id);
        self.holds.insert(hold.id, hold);
    }
    /// Drop a **terminal** hold from the projection (the sweep's GC, §4.3 step 4).
    /// Its whole history stays in the journal forever; only the read-side record and
    /// its `hold:{id}` doc go. The allocator floor is journal-derived precisely so a
    /// collected id can never be re-minted (D-E4-1).
    pub(crate) fn remove_hold(&mut self, id: HoldId) {
        if let Some(prev) = self.holds.remove(&id)
            && let Some(set) = self.holds_by_account.get_mut(&prev.account)
        {
            set.remove(&id);
            if set.is_empty() {
                self.holds_by_account.remove(&prev.account);
            }
        }
    }
    /// Drop a garbage-collected `Draft` contract (the sweep's GC, §4.3 step 4). Its
    /// custody locks go with it — a draft holds none, because a lock exists only
    /// while a contract is `Active`/`ReadyToSettle`. The allocator floor comes from
    /// `meta`'s high-water mark, which every mint persists, so a collected id can
    /// never be re-minted.
    pub(crate) fn remove_contract(&mut self, id: ContractId) {
        self.contract_locks.retain(|_, c| *c != id);
        self.contracts.remove(&id);
    }
    /// Drop a dead approval request (the sweep's GC — the same rule boot applies).
    pub(crate) fn remove_approval(&mut self, id: ApprovalRequestId) {
        if let Some(prev) = self.approvals.remove(&id)
            && self.approvals_by_hash.get(&prev.plan_hash) == Some(&id)
        {
            self.approvals_by_hash.remove(&prev.plan_hash);
        }
    }
    /// Insert/replace a cadence clock.
    pub fn put_cadence(&mut self, cadence: CadenceClock) {
        self.cadences.insert(cadence.id, cadence);
    }
    /// Insert/replace an obligation.
    pub fn put_obligation(&mut self, obligation: Obligation) {
        self.obligations.insert(obligation.id, obligation);
    }
    /// Insert/replace a contract, re-deriving its custody locks.
    ///
    /// A contract holds `(object, vault)` only while it is `Active` or
    /// `ReadyToSettle`; every other state (including `Disputed`, whose escrow is
    /// frozen but whose deposits are not the economy's to strand forever) releases
    /// the lock, so a terminal contract can never trap someone's property.
    pub fn put_contract(&mut self, contract: EconomicContract) {
        self.contract_locks.retain(|_, c| *c != contract.id);
        if matches!(contract.state, ContractState::Active | ContractState::ReadyToSettle) {
            for condition in &contract.consent.conditions {
                if let TypedCondition::DepositInto { object, vault } = condition {
                    self.contract_locks.insert((*object, *vault), contract.id);
                }
            }
        }
        self.contracts.insert(contract.id, contract);
    }
    /// Mutable access to an account (used by `apply` to fold a delta).
    pub(crate) fn account_mut(&mut self, id: AccountId) -> Option<&mut Account> {
        self.accounts.get_mut(&id)
    }
    /// Mutable access to an asset — **doc-authority fields only** (the `frozen`
    /// latch; the recovery backstop and boot liveness pass). Journal-authority
    /// fields move only through [`crate::apply`]; changing `title_owner` here would
    /// desync the owner index.
    pub(crate) fn asset_mut(&mut self, object: ObjId) -> Option<&mut EconomicAsset> {
        self.assets.get_mut(&object)
    }
    /// Mutable access to a vault — **`status`/`revision` only** (staff freeze/close
    /// and the recovery backstop); re-binding a backing must go through
    /// [`EconomySnapshot::put_vault`] so the index stays coherent.
    pub(crate) fn vault_mut(&mut self, id: VaultId) -> Option<&mut Vault> {
        self.vaults.get_mut(&id)
    }
    /// Mark an approval request consumed by a transaction, dropping it from the live
    /// by-hash index (used by `apply`).
    pub(crate) fn consume_approval(&mut self, id: ApprovalRequestId, transaction: TransactionId) {
        if let Some(r) = self.approvals.get_mut(&id) {
            r.status = ApprovalStatus::Consumed { transaction };
            r.revision += 1;
            let hash = r.plan_hash;
            if self.approvals_by_hash.get(&hash) == Some(&id) {
                self.approvals_by_hash.remove(&hash);
            }
        }
    }
}

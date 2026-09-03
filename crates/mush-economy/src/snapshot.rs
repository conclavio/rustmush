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

use crate::ids::{AccountId, ApprovalRequestId, InstrumentId, TransactionId, VaultId};
use crate::record::{
    Account, ApprovalRequest, ApprovalStatus, CustodyRef, EconomicAsset, IdempotencyKey,
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
    /// Mutable access to an account (used by `apply` to fold a delta).
    pub(crate) fn account_mut(&mut self, id: AccountId) -> Option<&mut Account> {
        self.accounts.get_mut(&id)
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

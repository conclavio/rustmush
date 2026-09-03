//! [`EconomySnapshot`] — the in-RAM authoritative economic state the pure engine
//! reads (E2a). It is the E2a analog of `OrgState`: the instrument registry, the
//! account projections, the idempotency index, and the set of already-reversed
//! transactions. The engine validates against ONE immutable snapshot (invariant 5);
//! [`crate::apply`] returns a NEW snapshot rather than mutating in place.

use std::collections::{BTreeMap, BTreeSet};

use crate::ids::{AccountId, InstrumentId, TransactionId};
use crate::record::{Account, IdempotencyKey, IdempotencyRecord, InstrumentDef};

/// The authoritative economic state (pre-image for validation). In the live module
/// (E2b) this is re-derived from the journal at boot; here it is the value the pure
/// engine reads and `apply` advances.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EconomySnapshot {
    instruments: BTreeMap<InstrumentId, InstrumentDef>,
    accounts: BTreeMap<AccountId, Account>,
    idempotency: BTreeMap<IdempotencyKey, IdempotencyRecord>,
    reversed: BTreeSet<TransactionId>,
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
    /// Mutable access to an account (used by `apply` to fold a delta).
    pub(crate) fn account_mut(&mut self, id: AccountId) -> Option<&mut Account> {
        self.accounts.get_mut(&id)
    }
}

//! Active-station focus and the account seam (E7 cut 5; 10b §3.5).
//!
//! Cut 5 B1 lands the **account-distinctness oracle holder**: the
//! composition-installed [`mush_core::AccountIdentity`] is carried in world
//! state so command handlers reach it synchronously inside a world command,
//! without a store scan. It is `None` until wired through
//! `SimulationModule::with_accounts`; **every consumer fails closed on `None`**
//! (the oracle answers `None` and the acquiring command refuses — never a
//! default to "same" or "distinct"). Boot never hydrates a focus map; a
//! reconnecting player re-claims (D-E7-34).
//!
//! The world-owned ephemeral **focus** itself (`FocusState` / `ActiveFocus` /
//! `ReachProof`) is cut 5 B2's and lands in this module.

use std::sync::Arc;

use mush_core::AccountIdentity;

// ---- E7c5: B1 (the account oracle holder) ----

/// The composition-installed account-distinctness oracle, held in world state.
///
/// A newtype over `Option<Arc<dyn AccountIdentity>>` so `SimulationModule`'s
/// `register` can `reg.state(..)` it and command handlers can read it through
/// `World::module_state::<AccountOracle>()`. `None` means no oracle was wired
/// (an old composition, or a test rig); [`AccountOracle::account_of`] and
/// friends then answer `None`, and their callers refuse.
#[derive(Clone, Default)]
pub struct AccountOracle(Option<Arc<dyn AccountIdentity>>);

impl std::fmt::Debug for AccountOracle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountOracle")
            .field("wired", &self.0.is_some())
            .finish()
    }
}

impl AccountOracle {
    /// Wrap a composition-installed oracle.
    #[must_use]
    pub fn new(oracle: Arc<dyn AccountIdentity>) -> Self {
        Self(Some(oracle))
    }

    /// The unwired holder every consumer fails closed against.
    #[must_use]
    pub fn unwired() -> Self {
        Self(None)
    }

    /// Whether an oracle is wired at all.
    #[must_use]
    pub fn is_wired(&self) -> bool {
        self.0.is_some()
    }

    /// The account a character belongs to, or `None` (unknown/unwired — the
    /// caller must refuse).
    #[must_use]
    pub fn account_of(&self, character: mush_core::ObjId) -> Option<mush_core::AccountId> {
        self.0
            .as_ref()
            .and_then(|oracle| oracle.account_of(character))
    }

    /// Whether two characters resolve to the same account; `None` when unknown
    /// or unwired.
    #[must_use]
    pub fn same_account(&self, a: mush_core::ObjId, b: mush_core::ObjId) -> Option<bool> {
        self.0.as_ref().and_then(|oracle| oracle.same_account(a, b))
    }

    /// Whether every character resolves to a distinct account; `None` when any
    /// is unknown or the oracle is unwired.
    #[must_use]
    pub fn all_accounts_distinct(&self, characters: &[mush_core::ObjId]) -> Option<bool> {
        self.0
            .as_ref()
            .and_then(|oracle| oracle.all_accounts_distinct(characters))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn e7c5_b1_unwired_oracle_fails_closed() {
        let oracle = AccountOracle::unwired();
        assert!(!oracle.is_wired());
        assert_eq!(oracle.account_of(mush_core::ObjId(7)), None);
        assert_eq!(
            oracle.same_account(mush_core::ObjId(7), mush_core::ObjId(8)),
            None
        );
        assert_eq!(oracle.all_accounts_distinct(&[mush_core::ObjId(7)]), None);
    }
}

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

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_core::{AccountId, AccountIdentity, ClockStamp, ObjId};

use crate::data::StationId;

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

// ---- E7c5: B2 (the world-owned ephemeral focus) ----

/// Upper bound on billets a single `+stations` / `+focus` pass **visits**
/// (D-LBK-5). This bounds cut 5's own enumeration; it is **not** a load-refusing
/// cap on `HostAssignments.seats` (which has none — §7 gap, contradiction (ix)).
pub const MAX_BILLETS_LISTED: usize = 64;

/// Proof that an account's foreground control of a station is physically or
/// logically reachable — the fifth invariant ("ownership alone never satisfies
/// reach"). Cut 5 ships a **stub predicate that refuses on the unknown**; cut 7
/// (`mush-propagation`) tightens `RemoteLink` against a real delivery envelope.
///
/// **No derived `Ord`/`PartialOrd`** and **no `max`-shaped combinator** may ever
/// rank these variants (E5a lens A: `Access`'s derived `Ord` ranked `Refused`
/// last, so the obvious strict combinator failed open). A probe bans it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReachProof {
    /// A physical console co-located with the operator (C4A). Reach holds only
    /// while the actor is actually in `room`.
    Console { room: ObjId },
    /// A remote link through a terminal over a delivery path. `path` is an
    /// `ObjId` stub in cut 5 (a relay/link endpoint); cut 7 replaces it with a
    /// real `EnvelopeId` and proves live delivery. Reach holds only while both
    /// the terminal and the path object are live.
    RemoteLink { terminal: ObjId, path: ObjId },
}

impl ReachProof {
    /// The **stub** reach predicate (cut 7 tightens it): a console must be
    /// co-located; a remote link's terminal and path must both be live.
    /// Anything unknown **refuses**.
    #[must_use]
    pub fn is_reachable(&self, w: &mush_core::World, actor: ObjId) -> bool {
        let live = |id: ObjId| {
            w.get(id)
                .is_some_and(|o| !o.flags.contains(mush_core::Flags::GOING))
        };
        match self {
            ReachProof::Console { room } => {
                live(*room) && w.get(actor).and_then(|o| o.location) == Some(*room)
            }
            ReachProof::RemoteLink { terminal, path } => live(*terminal) && live(*path),
        }
    }
}

/// One account's foreground control surface (invariant 1, C21A/T12/Q20A). It is
/// **world memory, never a record**: no document, no key, no state family, no
/// K6 lease keyed on the account (D-E7-34). Boot starts empty; a reconnecting
/// player re-claims.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActiveFocus {
    /// The immutable account this focus belongs to.
    pub account: AccountId,
    /// The character currently driving it.
    pub actor: ObjId,
    /// The operated unit.
    pub unit: ObjId,
    /// The billet's station on that unit.
    pub station: StationId,
    /// When it was claimed (IC clock), for the refusal message.
    pub acquired_at: ClockStamp,
    /// The unit's control generation **captured at claim** (contradiction
    /// (viii)): kept for the refusal message and **never compared** — every
    /// comparison is `World::channel_grant_is_current`.
    pub epoch: u64,
    /// The reach proof under which it was claimed.
    pub reach: ReachProof,
    /// **E7c10 B4 (T12).** Whether this is an OOC **proxy seat**: the account
    /// directs an NPC (`actor`) it does not own, under a `campaign.seat` grant.
    /// It changes no authority — the live authority is still the billet, the
    /// narrowed `ChannelGrant` and K6's lease — but it is what makes the
    /// represented NPC clearly identified in every render, what selects the
    /// `seat.activate` handoff reason, and what cut 10's guardrail predicate
    /// asks before refusing an administrative verb (T11's command half).
    pub proxy: bool,
}

/// The world-owned ephemeral focus map (B2). Installed via `reg.state(..)` — an
/// in-RAM slot, **never persisted, never in an `Outbox`, never hydrated at
/// boot**. Its single-writer atomicity (one `&mut` [`FocusState::claim`]
/// relinquishing the prior focus and installing the new one in one borrow) is
/// what makes "at most one foreground focus per account" hold against any
/// interleaving of two connections of one account (invariant 1; answer 10).
#[derive(Clone, Debug, Default)]
pub struct FocusState {
    by_account: BTreeMap<AccountId, ActiveFocus>,
    by_actor: BTreeMap<ObjId, AccountId>,
}

impl FocusState {
    /// The account's current foreground focus, if any.
    #[must_use]
    pub fn focus_of_account(&self, account: AccountId) -> Option<&ActiveFocus> {
        self.by_account.get(&account)
    }

    /// The focus an actor is currently driving, if any.
    #[must_use]
    pub fn focus_of_actor(&self, actor: ObjId) -> Option<&ActiveFocus> {
        self.by_actor
            .get(&actor)
            .and_then(|account| self.by_account.get(account))
    }

    /// How many foreground foci are live (one per account).
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_account.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_account.is_empty()
    }

    /// **Relinquish-then-install in a single `&mut` borrow** (the atomicity
    /// argument). Relinquishes the account's prior focus and any focus the new
    /// actor already drove (defensive — normally the same), installs the new
    /// one, and returns every focus it displaced (the caller refreshes their
    /// witnesses). Two claims for one account cannot interleave into two: the
    /// world's single writer serializes them, and the loser sees the winner's
    /// focus already installed.
    pub fn claim(&mut self, focus: ActiveFocus) -> Vec<ActiveFocus> {
        let mut released = Vec::new();
        if let Some(prior) = self.by_account.remove(&focus.account) {
            self.by_actor.remove(&prior.actor);
            released.push(prior);
        }
        if let Some(account) = self.by_actor.remove(&focus.actor)
            && let Some(prior) = self.by_account.remove(&account)
        {
            self.by_actor.remove(&prior.actor);
            released.push(prior);
        }
        self.by_actor.insert(focus.actor, focus.account);
        self.by_account.insert(focus.account, focus);
        released
    }

    /// Release an account's foreground focus. Idempotent: returns the released
    /// focus, or `None` when the account held none. The inverse of one half of
    /// [`FocusState::claim`].
    pub fn relinquish(&mut self, account: AccountId) -> Option<ActiveFocus> {
        let focus = self.by_account.remove(&account)?;
        self.by_actor.remove(&focus.actor);
        Some(focus)
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

    fn stamp(at: i64) -> ClockStamp {
        ClockStamp {
            clock: mush_core::ClockRef::WorldIc { generation: 1 },
            at,
        }
    }

    fn focus(account: AccountId, actor: u32, unit: u32) -> ActiveFocus {
        ActiveFocus {
            account,
            actor: ObjId(actor),
            unit: ObjId(unit),
            station: StationId::new("pilot").unwrap(),
            acquired_at: stamp(1),
            epoch: 1,
            reach: ReachProof::Console { room: ObjId(1) },
            proxy: false,
        }
    }

    #[test]
    fn e7c5_b2_claim_is_relinquish_then_install_single_writer() {
        let a = AccountId::mint("Nick", 1).unwrap();
        let mut state = FocusState::default();
        // First claim installs; nothing displaced.
        assert!(state.claim(focus(a, 7, 100)).is_empty());
        assert_eq!(state.len(), 1);
        assert_eq!(
            state.focus_of_actor(ObjId(7)).map(|f| f.unit),
            Some(ObjId(100))
        );
        // A second acquire by the SAME account (a different connection's actor
        // even) atomically relinquishes the first: exactly one focus remains,
        // and the displaced focus is returned so its witnesses can be refreshed.
        let released = state.claim(focus(a, 8, 200));
        assert_eq!(released.len(), 1);
        assert_eq!(released[0].actor, ObjId(7));
        assert_eq!(
            state.len(),
            1,
            "two acquires for one account leave exactly one"
        );
        assert_eq!(state.focus_of_account(a).map(|f| f.actor), Some(ObjId(8)));
        assert!(
            state.focus_of_actor(ObjId(7)).is_none(),
            "the loser holds nothing"
        );
    }

    #[test]
    fn e7c5_b2_relinquish_is_idempotent() {
        let a = AccountId::mint("Nick", 1).unwrap();
        let mut state = FocusState::default();
        state.claim(focus(a, 7, 100));
        assert_eq!(state.relinquish(a).map(|f| f.actor), Some(ObjId(7)));
        assert!(state.is_empty());
        // Releasing again is a no-op, not a panic.
        assert!(state.relinquish(a).is_none());
        assert!(state.is_empty());
    }

    #[test]
    fn e7c5_b2_two_accounts_coexist_independently() {
        let a = AccountId::mint("Nick", 1).unwrap();
        let b = AccountId::mint("Zoe", 2).unwrap();
        let mut state = FocusState::default();
        state.claim(focus(a, 7, 100));
        assert!(state.claim(focus(b, 9, 300)).is_empty());
        assert_eq!(state.len(), 2, "distinct accounts each hold a focus");
        assert_eq!(state.focus_of_actor(ObjId(7)).map(|f| f.account), Some(a));
        assert_eq!(state.focus_of_actor(ObjId(9)).map(|f| f.account), Some(b));
    }

    /// A compile-time probe that **no `Ord` exists** over the fail-open-prone
    /// types (E5a lens A: `Access`'s derived `Ord` ranked `Refused` last, so the
    /// obvious strict combinator failed open). `IsOrd::<T>::VALUE` resolves to
    /// the inherent `true` only when `T: Ord`; otherwise the trait's `false`.
    /// Deriving `Ord` on either type flips the const and fails this test.
    #[test]
    fn e7c5_b2_reach_and_focus_are_not_ordered() {
        use std::marker::PhantomData;
        struct IsOrd<T>(PhantomData<T>);
        trait ViaTrait {
            const VALUE: bool = false;
        }
        impl<T> ViaTrait for IsOrd<T> {}
        impl<T: Ord> IsOrd<T> {
            // Only reachable if a type below IS Ord (which must never happen);
            // the `dead_code` allow keeps the probe green under `-D warnings`.
            #[allow(dead_code)]
            const VALUE: bool = true;
        }
        const { assert!(!<IsOrd<ReachProof>>::VALUE, "ReachProof must not be Ord") };
        const { assert!(!<IsOrd<ActiveFocus>>::VALUE, "ActiveFocus must not be Ord") };
    }
}

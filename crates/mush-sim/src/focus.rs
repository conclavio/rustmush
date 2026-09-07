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

// ---- E7c11: B0 (the proxy DRIVING ingress, and T11's guardrail) ----

/// One live OOC **proxy drive**: the account, the character actually typing, the
/// NPC that account directs, and the seat that authorises it.
///
/// Cut 10 shipped the seat's consent, attribution, narrowing, gate and give-back
/// but **not its driving ingress**: [`ActiveFocus`] for a proxy seat carries
/// `actor = npc`, so `FocusState::focus_of_actor(driver)` answered `None` and
/// every station-gated verb the driver typed refused with "you hold no focused
/// station". The seat was undrivable. This is the missing bind.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProxyDrive {
    /// The account whose one foreground focus this drive consumes (T12).
    pub account: AccountId,
    /// The human character typing the command. It never moves and never acts
    /// (T13); it is only the identity the ingress resolves *from*.
    pub driver: ObjId,
    /// The NPC the account directs — the actor every downstream authority check
    /// reads (T16: the human adds judgment, never an absent character's stats).
    pub npc: ObjId,
    /// The unit the seat is on.
    pub unit: ObjId,
    /// The station the seat is on.
    pub station: StationId,
}

/// The live proxy drive `actor` is typing through, if any.
///
/// **Fails closed on every unknown.** An unwired [`AccountOracle`], an unmapped
/// character, an account with no foreground focus, a focus that is not a proxy
/// seat, or a focus whose `actor` *is* the caller (an NPC acting for itself, or
/// a staged submit under the NPC) all answer `None` — which leaves the caller on
/// exactly the pre-cut-11 path.
///
/// It is **idempotent by construction**: `proxy_drive_of(w, npc)` is `None`
/// (an NPC maps to no account), so [`driving_actor`] is a fixed point after one
/// application and no chain of seats can hop from one NPC to another.
#[must_use]
pub fn proxy_drive_of(world: &mush_core::World, actor: ObjId) -> Option<ProxyDrive> {
    let account = world.module_state::<AccountOracle>()?.account_of(actor)?;
    let focus = world
        .module_state::<FocusState>()?
        .focus_of_account(account)?;
    (focus.proxy && focus.actor != actor).then(|| ProxyDrive {
        account,
        driver: actor,
        npc: focus.actor,
        unit: focus.unit,
        station: focus.station.clone(),
    })
}

/// **The driving ingress** (E7 cut 11 B0; T16).
///
/// The in-world actor whose billet, focused station, published `ChannelGrant`
/// and held contacts a station-gated command acts *through*. For an ordinary
/// player this is `actor` itself; for a proxy driver it is the NPC.
///
/// **Authority is not widened.** Nothing here mints, publishes or copies a
/// grant: the NPC's own `ChannelGrant` — narrowed to the seat's channels by
/// `links::desired_links` — remains the only authority, so a driver who takes a
/// `movement`-only seat can steer and cannot fire, and a driver with no live
/// seat resolves to themselves and is refused exactly as before.
#[must_use]
pub fn driving_actor(world: &mush_core::World, actor: ObjId) -> ObjId {
    proxy_drive_of(world, actor).map_or(actor, |drive| drive.npc)
}

/// T11's nine acts a proxy seat **never** confers.
///
/// > "Human-directed NPC control grants tactical discretion, not title,
/// > administration or an unrestricted possession verb. It does not by default
/// > permit asset transfer or abandonment, scuttling, fallback changes,
/// > loadout/crew mutation, organization spending, surrender of unrelated
/// > assets, departure from the authorized operation, or assumption of another
/// > station." — doc 10 §19 T11
///
/// Six of the nine are refused by *inertness*: they are economy, org and staff
/// verbs keyed on `ctx.actor`, which the ingress deliberately never rewrites, so
/// a driver's own authority over them is byte-identical before and after
/// activation. The remaining three ride the station lane the ingress opens, and
/// [`seat_restriction`] refuses them explicitly.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ProxyAct {
    /// Moving title in an asset (E4b2 `@claim/transfer`, a market sale).
    AssetTransfer,
    /// Abandoning an asset (`AssetLifecycle::Abandoned`).
    Abandonment,
    /// Destroying the unit or its cargo deliberately.
    Scuttling,
    /// Re-declaring a station's fallback controller (`@station/fallback`).
    FallbackChange,
    /// Mutating the unit's loadout or crew roster (`@crew/*`).
    LoadoutOrCrew,
    /// Spending the commanding organization's money.
    OrganizationSpending,
    /// Surrendering an asset the operation does not cover.
    UnrelatedSurrender,
    /// Taking the unit out of the authorized operation (`@journey/depart`,
    /// `@journey/divert`).
    OperationDeparture,
    /// Assuming another station (`+focus` onto a different billet).
    StationAssumption,
}

impl ProxyAct {
    /// Every act, in declaration order — the closed vocabulary a test enumerates.
    pub const ALL: [ProxyAct; 9] = [
        ProxyAct::AssetTransfer,
        ProxyAct::Abandonment,
        ProxyAct::Scuttling,
        ProxyAct::FallbackChange,
        ProxyAct::LoadoutOrCrew,
        ProxyAct::OrganizationSpending,
        ProxyAct::UnrelatedSurrender,
        ProxyAct::OperationDeparture,
        ProxyAct::StationAssumption,
    ];

    /// The stable token this act is named by in a refusal and in a test name.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            ProxyAct::AssetTransfer => "asset transfer",
            ProxyAct::Abandonment => "abandonment",
            ProxyAct::Scuttling => "scuttling",
            ProxyAct::FallbackChange => "a fallback change",
            ProxyAct::LoadoutOrCrew => "a loadout or crew change",
            ProxyAct::OrganizationSpending => "organization spending",
            ProxyAct::UnrelatedSurrender => "surrender of an unrelated asset",
            ProxyAct::OperationDeparture => "departure from the operation",
            ProxyAct::StationAssumption => "assumption of another station",
        }
    }

    /// Whether this act is refused by [`seat_restriction`] itself rather than by
    /// the driver's own (unchanged) authority. The three are exactly the acts
    /// that ride the station lane B0's ingress opens.
    #[must_use]
    pub const fn is_guarded(self) -> bool {
        matches!(
            self,
            ProxyAct::FallbackChange | ProxyAct::OperationDeparture | ProxyAct::StationAssumption
        )
    }
}

/// **T11's guardrail** (E7 cut 11 B0, deviation (h)).
///
/// `Err(line)` when `actor` is driving a proxy seat right now: a seat is
/// tactical discretion and nothing else. `Ok(())` for everybody else — an
/// ordinary billet-holder, staff at their own console, an NPC acting for itself
/// — so the predicate narrows the *proxy* and changes nothing for anyone else.
///
/// It lives in `mush-sim` (never `mush-campaign`) because `mush-propagation` and
/// `mush-sim` itself are its callers and `mush-campaign` depends on `mush-sim`:
/// putting it in the campaign crate is the dependency cycle 10b §3.10 deviation
/// (c) recorded.
///
/// # Errors
///
/// The player-facing refusal, naming the act and the exit (`+seat/leave`).
pub fn seat_restriction(
    world: &mush_core::World,
    actor: ObjId,
    act: ProxyAct,
) -> Result<(), String> {
    let Some(drive) = proxy_drive_of(world, actor) else {
        return Ok(());
    };
    Err(format!(
        "A seat grants tactical discretion, not {}: you are directing another crew member on \
         `{}`. +seat/leave first.",
        act.token(),
        drive.station
    ))
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

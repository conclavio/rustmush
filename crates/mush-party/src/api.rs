//! The seam the **ruleset** calls (E6a, 08f §3.2, decision D-E6-5).
//!
//! ## Authority-free by design
//!
//! [`lock`], [`release`] and [`unlock`] check **state**, never **standing**. They do
//! not ask who the actor is, whether they may spend, whether they lead the party, or
//! whether the contract exists — and they must not, because they are called from
//! `ruleset-shadowrun`'s `+run/crew accept` and its settlement path, *after* the
//! economy has already decided that the payer holds `contract.accept` on the scope
//! that funds the run. A second authority check here would be a second opinion on a
//! question this crate cannot see the evidence for, and a party that could refuse the
//! ruleset's `release` would be a party that outlives its own contract.
//!
//! The ruleset is therefore the only caller. There is no command that reaches these
//! three, and no `+party` verb locks or unlocks anything.
//!
//! ## The contract handle is opaque
//!
//! [`ContractRef`] is a `u64` this crate never interprets (D-E6-5): the party is not
//! an economic principal, and the contract never points back at the party. Nothing
//! here resolves it, compares it against an economy, checks it still exists, or lets
//! it decide anything. That is what makes the dependency list — `mush-core`, `serde`,
//! `smol_str` — an enforceable claim rather than a promise.
//!
//! ## The four arrows
//!
//! ```text
//! Forming ──lock(id, by)──────────────────────────────▶ Locked{by}
//! Locked  ──release(id, Settled|Released)─────────────▶ Disbanded{how}
//! Locked  ──unlock(id)  (the contract was AMENDED)────▶ Forming
//! ```
//!
//! A substitution is the third arrow and only the third: while `Locked`, **every**
//! membership verb refuses, so the ruleset must amend the contract first, call
//! [`unlock`], let the players change the roster, and lock again.

use mush_core::{Ctx, ObjId, World};

use crate::error::PartyError;
use crate::ids::{ContractRef, PartyId};
use crate::module::{PartyStore, is_staff, put_party};
use crate::record::{DisbandCause, Party, PartyStatus};

/// The refusal every caller renders for a party the actor may not view.
///
/// Existence hiding (08a decision 13): identical text for a party that does not
/// exist and one the actor simply may not see, so a refusal can never be used to
/// enumerate who is running with whom.
pub const NO_SUCH_PARTY: &str = "No such party.";

/// Freeze a `Forming` party against `by` (the ruleset's `+run/crew accept`).
///
/// Authority-free: the caller has already established that the payer may accept.
/// The party's `expires_at` is left where it is and **never consulted** while locked
/// — the contract's own deadline governs a locked party (the expiry tick skips it).
///
/// **Outstanding invitations are dropped here** (E6a verify fold, D6). A `Locked`
/// party refuses `/accept` *and* `/uninvite`, so an invitation that survives the lock
/// is one nobody can act on and nobody can withdraw — and [`may_view`] admits an
/// invitee, so a stranger the leader once invited would keep watching the crew roster
/// for the life of the run. The count is logged, never announced: a withdrawn
/// invitation to a job that has started is not news.
///
/// # Errors
///
/// - [`PartyError::NoStanding`] — no such party (render [`NO_SUCH_PARTY`]).
/// - [`PartyError::State`] — it is not `Forming`: already `Locked`, or `Disbanded`.
/// - [`PartyError::Empty`] — it has no members. Defensive: the record invariants make
///   an empty live roster unrepresentable, and an empty roster frozen against a
///   contract would be a payout table addressed to nobody.
pub fn lock(ctx: &mut Ctx<'_>, id: PartyId, by: ContractRef) -> Result<(), PartyError> {
    let mut p = snapshot(ctx.w, id)?;
    match p.status {
        PartyStatus::Forming => {}
        PartyStatus::Locked { .. } | PartyStatus::Disbanded { .. } => {
            return Err(PartyError::State);
        }
    }
    if p.members.is_empty() {
        return Err(PartyError::Empty);
    }
    let pruned = p.invited.len();
    p.invited.clear();
    if pruned > 0 {
        tracing::info!(
            party = %id,
            pruned,
            "party: outstanding invitations were dropped at the lock; a locked roster cannot \
             accept them and cannot withdraw them"
        );
    }
    p.status = PartyStatus::Locked { by };
    p.bump();
    put_party(ctx, p);
    Ok(())
}

/// Release a `Locked` party when its contract reached a terminal state.
///
/// `how` is the ruleset's classification: [`DisbandCause::Settled`] when the contract
/// settled, [`DisbandCause::Released`] when it ended without settling (cancelled,
/// refunded, expired). Any other cause is accepted and stored verbatim — this crate
/// does not second-guess a caller that can see the contract when it cannot.
///
/// # Errors
///
/// - [`PartyError::NoStanding`] — no such party (render [`NO_SUCH_PARTY`]).
/// - [`PartyError::State`] — it is not `Locked`.
pub fn release(ctx: &mut Ctx<'_>, id: PartyId, how: DisbandCause) -> Result<(), PartyError> {
    let mut p = snapshot(ctx.w, id)?;
    if !p.is_locked() {
        return Err(PartyError::State);
    }
    p.disband(how);
    put_party(ctx, p);
    Ok(())
}

/// Return a `Locked` party to `Forming` for a substitution — **after** the ruleset
/// has amended the contract (§3.2).
///
/// The idle clock is **refreshed** here, and that is load-bearing: a party locked for
/// longer than [`crate::PARTY_IDLE_TTL`] would otherwise come back to `Forming` with
/// a deadline already in the past and be collected by the very next expiry fire,
/// before anybody could substitute anyone.
///
/// # Errors
///
/// - [`PartyError::NoStanding`] — no such party (render [`NO_SUCH_PARTY`]).
/// - [`PartyError::State`] — it is not `Locked`.
pub fn unlock(ctx: &mut Ctx<'_>, id: PartyId) -> Result<(), PartyError> {
    let mut p = snapshot(ctx.w, id)?;
    if !p.is_locked() {
        return Err(PartyError::State);
    }
    let now = ctx.w.now();
    p.status = PartyStatus::Forming;
    p.touch(now);
    put_party(ctx, p);
    Ok(())
}

/// Every party `who` is an accepted member of, in id order.
///
/// **Live parties only** — `Forming` and `Locked`. A disbanded party is not one you
/// are in, and the ruleset's callers (`+run/apply <id>=<party>`) mean the crew that
/// can still take a run. Bounded by [`crate::MAX_PARTIES`], which the live projection
/// already bounds, so this can never return an unbounded list. Invitations do not
/// count: an invitee has not joined.
#[must_use]
pub fn parties_of(w: &World, who: ObjId) -> Vec<PartyId> {
    w.module_state::<PartyStore>()
        .map(|s| s.live_parties_of(who))
        .unwrap_or_default()
}

/// Whether `viewer` may see a party at all.
///
/// The three viewers of §3.4's existence-hiding rule, and no others: **a member**, **an
/// invitee**, and **staff**. Everyone else — including a player who was in the party
/// yesterday and left — gets [`NO_SUCH_PARTY`] from every surface that names it, the
/// same text a party that never existed gets.
///
/// Deliberately expiry-blind on the invitation (see [`Party::is_invited`]): a lapsed
/// invitation the expiry tick has not collected yet still grants sight, never use.
#[must_use]
pub fn may_view(w: &World, viewer: ObjId, id: PartyId) -> bool {
    let Some(store) = w.module_state::<PartyStore>() else {
        return false;
    };
    let Some(p) = store.party(id) else {
        return false;
    };
    p.is_member(viewer) || p.is_invited(viewer) || is_staff(w, viewer)
}

/// The party record, cloned, or [`PartyError::NoStanding`].
fn snapshot(w: &World, id: PartyId) -> Result<Party, PartyError> {
    w.module_state::<PartyStore>()
        .and_then(|s| s.party(id))
        .cloned()
        .ok_or(PartyError::NoStanding)
}

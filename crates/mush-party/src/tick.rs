//! The idle-expiry tick (E6a, §3.2's third arrow out of `Forming`).
//!
//! ## What one fire does
//!
//! Walks [`PartyStore::live_ids`] in id order and, for at most
//! [`MAX_PARTY_EXPIRIES_PER_FIRE`] parties:
//!
//! 1. **disbands a `Forming` party whose `expires_at` has passed**, as
//!    `Disbanded { how: Expired }`;
//! 2. otherwise **prunes lapsed invitations** off a `Forming` party.
//!
//! ## What one fire never does
//!
//! - **A `Locked` party never expires.** Its contract's own deadline governs it, and
//!   this crate cannot read that deadline — it holds an opaque [`crate::ContractRef`]
//!   and nothing else (D-E6-5). Expiring a locked roster out from under a live
//!   contract is exactly the failure the opacity exists to prevent, so the tick skips
//!   `Locked` entirely: neither its expiry nor its invitations are touched, because a
//!   locked roster is frozen and an invitation is roster state. When the ruleset
//!   calls [`crate::unlock`], the party comes back to `Forming` with a **refreshed**
//!   idle clock and the tick collects any stale invitations on its next fire.
//! - **A pruned invitation does not refresh the idle clock.** Housekeeping is not
//!   activity; only a membership change is ([`crate::Party::touch`]).
//! - **Nothing terminal is revisited.** Disbanding removes the party from the `live`
//!   projection, so the next fire does not see it again.
//!
//! ## Panic-free
//!
//! No `?`, no `unwrap`, no `expect`, no panicking index, every `match` exhaustive and
//! every arithmetic `saturating_*`: a tick handler runs in the world task, so a panic
//! here takes the world down.

use mush_core::{Ctx, SceneId};

use crate::ids::PartyId;
use crate::module::{MAX_PARTY_EXPIRIES_PER_FIRE, PartyStore, put_party};
use crate::record::{DisbandCause, Party, PartyStatus};

/// What one fire did — read by the tests, which is the only way a budget claim can be
/// checked.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct TickStats {
    /// Parties disbanded as `Expired`.
    pub expired: usize,
    /// Parties whose lapsed invitations were pruned.
    pub pruned: usize,
}

/// One fire.
pub(crate) fn tick(ctx: &mut Ctx<'_>, _tag: &str, _scene: Option<SceneId>) {
    let _ = sweep(ctx);
}

/// The sweep, returning what it cost. Split out from [`tick`] so a test can assert
/// the budget rather than infer it.
pub(crate) fn sweep(ctx: &mut Ctx<'_>) -> TickStats {
    let now = ctx.w.now();
    let mut stats = TickStats::default();

    let ids: Vec<PartyId> = match ctx.w.module_state::<PartyStore>() {
        Some(store) => store.live_ids().collect(),
        None => return stats,
    };

    for id in ids {
        if stats.expired.saturating_add(stats.pruned) >= MAX_PARTY_EXPIRIES_PER_FIRE {
            break;
        }
        let Some(work) = plan(ctx, id, now) else {
            continue;
        };
        match work {
            Work::Expire(mut p) => {
                p.disband(DisbandCause::Expired);
                put_party(ctx, p);
                stats.expired = stats.expired.saturating_add(1);
            }
            Work::Prune(mut p) => {
                if p.prune_invitations(now) > 0 {
                    // Housekeeping, not activity: the revision moves, the idle clock
                    // does not.
                    p.bump();
                    put_party(ctx, p);
                    stats.pruned = stats.pruned.saturating_add(1);
                }
            }
        }
    }
    stats
}

/// The one unit of work a party is due for this fire, if any.
enum Work {
    /// `Forming`, and past its idle deadline.
    Expire(Party),
    /// `Forming`, and holding at least one lapsed invitation.
    Prune(Party),
}

/// Decide without cloning anything the fire is not about to write.
fn plan(ctx: &Ctx<'_>, id: PartyId, now: i64) -> Option<Work> {
    let store = ctx.w.module_state::<PartyStore>()?;
    let p = store.party(id)?;
    match p.status {
        PartyStatus::Forming => {
            if p.expires_at <= now {
                Some(Work::Expire(p.clone()))
            } else if p.invited.iter().any(|i| i.is_expired(now)) {
                Some(Work::Prune(p.clone()))
            } else {
                None
            }
        }
        // A locked roster is frozen, and its contract's deadline governs it.
        PartyStatus::Locked { .. } => None,
        // Terminal parties are not in the `live` projection; this arm is the
        // projection's own consistency check, and it does nothing.
        PartyStatus::Disbanded { .. } => None,
    }
}

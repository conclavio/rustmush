//! Immutable, opaque account identity and the synchronous distinctness oracle
//! (E7 cut 5 B1; 10b §3.5, OPEN 4 as the owner amended it; D-E7-34).
//!
//! An account is identified by an opaque [`AccountId`] minted **once**, at
//! `ObjectStore::create_account`, from a domain-tagged hash over the account's
//! `(lowercased name, created)` — immutable thereafter and **stable across a
//! rename** (a forward guarantee: the id lives on the record and is never
//! re-derived from the current name; there is no rename or delete path today).
//!
//! [`AccountIdentity`] is the synchronous oracle the world consults **inside a
//! command**: which account a character belongs to, whether two characters are
//! the same account, and whether a set of characters are all distinct. Every
//! answer is `Option`; **`None` means unknown and every calling policy MUST
//! fail closed** (never default to "same" or "distinct"). The implementation
//! lives in `mush-server` (built from a boot-hydrated character → account-id
//! map) and is installed into `mush-sim` through
//! `SimulationModule::with_accounts`. The id is **never rendered** into a
//! player-facing surface or a journal (R3/R7): [`AccountId`]'s `Display` and
//! `Debug` are redacted, and only the store round-trips it through
//! `Serialize`/`Deserialize` (transparent over the hex hash) inside the account
//! document.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::action::{CanonicalHash, CanonicalHashError};
use crate::obj::ObjId;

/// Canonical hashing domain for the account id (v1).
const ACCOUNT_ID_DOMAIN: &str = "account_id.v1";
/// Bound on the canonical value hashed to mint an account id.
const ACCOUNT_ID_MAX_BYTES: usize = 8 * 1024;

/// An opaque, immutable account identity.
///
/// Minted once from `(lowercased name, created)` (see [`AccountId::mint`]) and
/// stored on the account record; a rename never re-derives it. The value is
/// treated as sensitive for display — `Display`/`Debug` are redacted so it
/// cannot leak into output or a journal — but it is not a secret key, so the
/// store serializes it as the underlying lowercase-hex hash for its own
/// round-trip. There is deliberately **no `From<&str>`**: an id is only ever
/// minted from a full `(name, created)` pair.
#[derive(Clone, Copy, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct AccountId(CanonicalHash);

impl AccountId {
    /// Mint the immutable id from the account's name and creation second.
    ///
    /// Deterministic and idempotent: the same `(name, created)` always yields
    /// the same id (the boot back-fill relies on this), and the name is
    /// lowercased first so the id does not depend on display case. A rename
    /// changes only the display case / storage key, never `created`, so the
    /// stored id is preserved.
    pub fn mint(name: &str, created: i64) -> Result<Self, CanonicalHashError> {
        CanonicalHash::for_canonical_value(
            ACCOUNT_ID_DOMAIN,
            1,
            &json!({ "name": name.to_lowercase(), "created": created }),
            ACCOUNT_ID_MAX_BYTES,
        )
        .map(Self)
    }

    /// The underlying canonical hash — for the store's own persistence
    /// round-trip only, never for display.
    #[must_use]
    pub const fn canonical_hash(self) -> CanonicalHash {
        self.0
    }
}

impl std::fmt::Debug for AccountId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AccountId(<redacted>)")
    }
}

impl std::fmt::Display for AccountId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<account>")
    }
}

/// The synchronous account-distinctness oracle (OPEN 4, D-E7-34).
///
/// The world consults this inside a command to answer *which account* a
/// character belongs to and whether characters are the same or all distinct.
/// **Every answer is `Option`; `None` means unknown and every calling policy
/// MUST fail closed.** `same_account` and `all_accounts_distinct` are derived
/// from [`AccountIdentity::account_of`] so an implementation need only map a
/// character to its id, and so the fail-closed rule cannot be forgotten.
pub trait AccountIdentity: Send + Sync {
    /// The account a character belongs to, or `None` when the character is not
    /// mapped (unknown, unmapped, or its account carries no id yet).
    fn account_of(&self, character: ObjId) -> Option<AccountId>;

    /// **Record a character the world has just made** (16b cut F.4, the K6
    /// point).
    ///
    /// The map is hydrated once at boot, so before this existed a character
    /// created *after* boot was unmapped until the next restart and every
    /// account policy failed closed on it — which is the right behaviour for an
    /// unknown character and the wrong one for a character the world created a
    /// millisecond ago. The one writer is the world task, in the same turn that
    /// creates the Player, so the mapping exists before the reply leaves.
    ///
    /// `&self` because both readers hold the same `Arc<dyn AccountIdentity>`:
    /// an implementation that can learn carries its own interior mutability.
    /// The default cannot learn and says so, which keeps every fixed test
    /// oracle honest rather than silently accepting a write it drops.
    ///
    /// Returns whether the mapping is now in place. A character already claimed
    /// by a **different** account is a data-integrity fault: the implementation
    /// must leave it unmapped (never overwrite) and answer `false`.
    fn link(&self, character: ObjId, account: AccountId) -> bool {
        let _ = (character, account);
        false
    }

    /// **Forget a character** — the other half of [`AccountIdentity::link`], for
    /// a character that is destroyed or unlinked from its account. Returns
    /// whether a mapping was removed.
    fn unlink(&self, character: ObjId) -> bool {
        let _ = character;
        false
    }

    /// Whether two characters resolve to the **same** account. `None` if either
    /// is unknown — the caller must refuse, never assume "different".
    fn same_account(&self, a: ObjId, b: ObjId) -> Option<bool> {
        match (self.account_of(a), self.account_of(b)) {
            (Some(a), Some(b)) => Some(a == b),
            _ => None,
        }
    }

    /// Whether every character in `characters` resolves to a **distinct**
    /// account. `None` if any character is unknown — the caller must refuse,
    /// never assume "distinct". An empty or singleton set is `Some(true)`.
    fn all_accounts_distinct(&self, characters: &[ObjId]) -> Option<bool> {
        let mut seen = BTreeSet::new();
        for &character in characters {
            let id = self.account_of(character)?;
            if !seen.insert(id) {
                return Some(false);
            }
        }
        Some(true)
    }
}

/// **The server-side, audit-only seat attribution seam** (E7 cut 10 B3; Q19A /
/// doc 10 R3, R7; 10b §3.10 OPEN 4).
///
/// A proxy seat's mechanical journal records the **in-world actor** and an
/// **opaque decision-driver token**, and nothing else. The row that links that
/// token to an account lives **outside the world**: it is never a world
/// document, never a journal, contract or canon record, and no ordinary
/// projection, render or export can reverse it. Exactly one staff verb resolves
/// it, and even that verb prints no account id.
///
/// **Corrected by the E7 cut 10 independent verify fold (K6/A2/A5).** As shipped,
/// [`SeatAudit::account_for`] had **no production caller at all** — the staff
/// verb asked [`SeatAudit::knows`], a boolean — so the attribution the cut
/// claims had neither an input nor an output. It has two now, and neither
/// renders an id: `@seat/audit <op>/<id>=<character>` answers SAME / DIFFERENT /
/// UNKNOWN, and `mush_campaign::seatstate::audit_is` answers "is that row *this
/// caller's*", which is what `+seat/activate` and `+seat/leave` must ask.
///
/// The implementation lives at the composition root beside `AccountIdentity`'s
/// (`mush-server`), and reaches the campaign layer through
/// `CampaignModule::with_seat_audit` on the `with_accounts` builder shape.
/// **Unwired, every consumer fails closed**: a grant with no audit row refuses
/// activation and pages rather than activating un-attributed.
pub trait SeatAudit: Send + Sync {
    /// **Mint the opaque driver token and record it against `account`** (E7 cut
    /// 10 independent verify fold, E7(a)/K5).
    ///
    /// `seed` is the seat's own deterministic coordinates; the implementation
    /// folds a **process-scoped nonce** into them, so the returned token is a
    /// value the world cannot recompute from public coordinates and cannot be
    /// forged for a seat the forger does not hold. The token lives in the
    /// *grant*, never in the hashed offer, so `campaign.seat.v1` does not move.
    ///
    /// `None` = the row could not be stored, and the caller must then refuse the
    /// grant rather than proceed un-attributed.
    fn record(&self, seed: &str, account: AccountId) -> Option<String>;

    /// Whether a row exists for `token`. This is the *only* question ordinary
    /// (non-staff) code may ask, and it answers a boolean, never an identity.
    fn knows(&self, token: &str) -> bool;

    /// The account behind `token` — **staff only**, and never rendered. `None`
    /// when no row exists.
    ///
    /// Its production callers are `mush_campaign::seatstate::audit_is` (which
    /// answers "is this row *this caller's*", never an identity) and
    /// `@seat/audit`, which answers SAME / DIFFERENT / UNKNOWN and prints no id.
    fn account_for(&self, token: &str) -> Option<AccountId>;

    /// Forget a row (a terminal grant's attribution is retained only as long as
    /// the grant is live). Returns whether a row was removed.
    fn forget(&self, token: &str) -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// A fixed in-memory oracle for the fail-closed probes.
    struct MapOracle(BTreeMap<ObjId, AccountId>);

    impl AccountIdentity for MapOracle {
        fn account_of(&self, character: ObjId) -> Option<AccountId> {
            self.0.get(&character).copied()
        }
    }

    #[test]
    fn e7c5_b1_mint_is_deterministic_and_case_insensitive_and_rename_stable() {
        let a = AccountId::mint("Nick", 1_000).unwrap();
        let b = AccountId::mint("nick", 1_000).unwrap();
        // Case does not change the id — a rename that changes only display case
        // (there is no rename verb; this pins the forward guarantee) preserves it.
        assert_eq!(a, b);
        // A different creation second is a different account, so a different id.
        assert_ne!(a, AccountId::mint("Nick", 1_001).unwrap());
        // A different name is a different id.
        assert_ne!(a, AccountId::mint("Zoe", 1_000).unwrap());
    }

    #[test]
    fn e7c5_b1_display_and_debug_are_redacted() {
        let id = AccountId::mint("Nick", 1_000).unwrap();
        // Neither surface reveals the hash — the id never leaks into output or
        // a journal (R7). Two distinct ids render identically.
        let other = AccountId::mint("Zoe", 1_000).unwrap();
        assert_eq!(format!("{id}"), "<account>");
        assert_eq!(format!("{id}"), format!("{other}"));
        assert_eq!(format!("{id:?}"), "AccountId(<redacted>)");
        // The redacted forms carry none of the underlying hex.
        let hex = id.canonical_hash().to_hex();
        assert!(!format!("{id}").contains(&hex[..8]));
        assert!(!format!("{id:?}").contains(&hex[..8]));
    }

    #[test]
    fn e7c5_b1_oracle_fails_closed_on_unknown() {
        let nick = AccountId::mint("Nick", 1).unwrap();
        let oracle = MapOracle(BTreeMap::from([
            (ObjId(7), nick),
            (ObjId(8), nick),
            (ObjId(9), AccountId::mint("Zoe", 1).unwrap()),
        ]));
        // Known pairs answer.
        assert_eq!(oracle.same_account(ObjId(7), ObjId(8)), Some(true));
        assert_eq!(oracle.same_account(ObjId(7), ObjId(9)), Some(false));
        // An unknown character is None on every question — never a default.
        assert_eq!(oracle.account_of(ObjId(99)), None);
        assert_eq!(oracle.same_account(ObjId(7), ObjId(99)), None);
        assert_eq!(oracle.same_account(ObjId(99), ObjId(7)), None);
        // Distinctness: known-distinct is true, a shared account is false, and
        // any unknown member refuses.
        assert_eq!(
            oracle.all_accounts_distinct(&[ObjId(7), ObjId(9)]),
            Some(true)
        );
        assert_eq!(
            oracle.all_accounts_distinct(&[ObjId(7), ObjId(8)]),
            Some(false)
        );
        assert_eq!(oracle.all_accounts_distinct(&[ObjId(7), ObjId(99)]), None);
        assert_eq!(oracle.all_accounts_distinct(&[]), Some(true));
        assert_eq!(oracle.all_accounts_distinct(&[ObjId(7)]), Some(true));
    }
}

//! E7 cut 10 B3 — the **server-side, audit-only** seat attribution log
//! (10b §3.10 OPEN 4; doc 10 R3/R7, Q19A/T9).
//!
//! A proxy seat's mechanical journal records the in-world **actor** and an
//! opaque **driver token**, and nothing else. The row that links a token to an
//! account is the one place the two meet, and it lives **here**: process-local,
//! behind a lock, reachable only through `mush_core::SeatAudit`, and installed
//! into `mush-campaign` at composition by `CampaignModule::with_seat_audit` —
//! `mush-server`'s `accounts_identity` shape exactly.
//!
//! **What it deliberately is not.** It is not a world document, not a journal
//! line, not a contract party, not a canon entity and not an `AccountRec` field.
//! No projection, render or export can reverse it; `@seat/audit` answers
//! *whether* a token is attributed and never prints an id.
//!
//! **What that costs, recorded honestly.** The log does **not** survive a
//! restart, because persisting it would put an `account_id -> driver_token`
//! pair on disk in the world's own store, which is the exact coupling R3/R7
//! forbids, and building a separate durable side store is cut 11's archive
//! question rather than this cut's. The consequence is designed for rather than
//! hidden: a grant whose row is missing **refuses activation and pages**, with
//! the runbook telling the driver to re-take the seat. A dormant grant across a
//! reboot is therefore re-accepted, never silently activated un-attributed.

//! **Where the opaque driver token is minted** (E7 cut 10 independent verify
//! fold, decision E7(a)). Cut 10 derived the token from four public-or-near-
//! public coordinates under a fixed domain, so it inverted: a token identified
//! its seat exactly and could be forged for a seat the forger did not hold.
//! The mint moved *here*, where the account row lives: [`SeatAuditLog::record`]
//! folds a **process-scoped nonce** (a per-log salt and a monotonic counter)
//! into the caller's seed, so the world cannot recompute the token. Nothing
//! about consent moves — the token sits in the grant, never in the hashed offer.

use std::collections::BTreeMap;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};

use mush_core::{AccountId, CanonicalHash, SeatAudit};

/// The process-scoped nonce source. It is minted once per process and never
/// rendered; two servers, and two logs in one process, mint disjoint tokens.
static NONCE: AtomicU64 = AtomicU64::new(0);

/// The in-process token → account log.
#[derive(Debug, Default)]
pub struct SeatAuditLog {
    rows: RwLock<BTreeMap<String, AccountId>>,
    /// The per-log half of the nonce, minted at construction from the process
    /// clock and the global counter. Never rendered, never persisted.
    salt: u64,
}

impl SeatAuditLog {
    /// A fresh, empty log.
    #[must_use]
    pub fn new() -> SeatAuditLog {
        let clock = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        SeatAuditLog {
            rows: RwLock::new(BTreeMap::new()),
            salt: clock ^ NONCE.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed),
        }
    }

    /// How many rows it carries (diagnostics only; never rendered in-world).
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.read().map(|rows| rows.len()).unwrap_or(0)
    }

    /// Whether it carries no rows.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A poisoned lock is a refusal, never a panic and never a default: a seat that
/// cannot be attributed is a seat that is not taken.
impl SeatAudit for SeatAuditLog {
    fn record(&self, seed: &str, account: AccountId) -> Option<String> {
        // The nonce is folded into the canonical body, so the token is opaque
        // to every party that holds only the seat's public coordinates.
        let n = NONCE.fetch_add(1, Ordering::Relaxed);
        let body = serde_json::json!({
            "salt": format!("{:016x}", self.salt),
            "n": n,
            "seed": seed,
        });
        let token = CanonicalHash::for_canonical_value("server.seat.driver.v1", 1, &body, 4096)
            .ok()?
            .to_hex();
        let token = token[..token.len().min(32)].to_owned();
        let mut rows = self.rows.write().ok()?;
        rows.insert(token.clone(), account);
        Some(token)
    }

    fn knows(&self, token: &str) -> bool {
        self.rows
            .read()
            .map(|rows| rows.contains_key(token))
            .unwrap_or(false)
    }

    fn account_for(&self, token: &str) -> Option<AccountId> {
        self.rows.read().ok()?.get(token).copied()
    }

    fn forget(&self, token: &str) -> bool {
        self.rows
            .write()
            .map(|mut rows| rows.remove(token).is_some())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn e7c10_b3_the_audit_log_answers_a_boolean_and_forgets_on_demand() {
        let log = SeatAuditLog::new();
        let account = AccountId::mint("Nick", 1).expect("mints");
        assert!(!log.knows("abc"), "an unknown token is not attributed");
        let token = log.record("abc", account).expect("mints a token");
        assert!(log.knows(&token));
        assert_eq!(log.account_for(&token), Some(account));
        assert!(log.forget(&token));
        assert!(
            !log.knows(&token),
            "a forgotten token is unattributed again"
        );
        assert!(!log.forget(&token), "forgetting twice is not an error");
    }

    // ---- E7 cut 10 independent verify fold ----

    /// **K5. The driver token is minted where the account row lives, so it is
    /// not derivable from the seat's public coordinates.** The same seed, in two
    /// logs (two processes), mints two different tokens, and neither is the seed
    /// itself — the property cut 10 claimed ("opaque") and did not have.
    #[test]
    fn vf_k5_a_driver_token_is_not_derivable_from_the_seats_public_coordinates() {
        let account = AccountId::mint("Nick", 1).expect("mints");
        let a = SeatAuditLog::new();
        let b = SeatAuditLog::new();
        let seed = "7.3.50.1200";
        let one = a.record(seed, account).expect("mints");
        let two = b.record(seed, account).expect("mints");
        assert_ne!(one, two, "the same coordinates mint two unrelated tokens");
        assert_ne!(one, seed);
        assert!(!one.contains(seed), "the token never carries its seed");
        // …and twice in ONE process, too: the counter moves under the salt.
        assert_ne!(one, a.record(seed, account).expect("mints"));
    }

    /// **Two seats for one player produce two rows nothing correlates.** The log
    /// is keyed by token, and a token carries no account material at all, so the
    /// only thing that could join them is this log — which no projection reads.
    #[test]
    fn e7c10_b3_two_tokens_for_one_account_are_independent_rows() {
        let log = SeatAuditLog::new();
        let account = AccountId::mint("Nick", 1).expect("mints");
        let a = log.record("aaa", account).expect("mints");
        let b = log.record("bbb", account).expect("mints");
        assert!(log.forget(&a));
        assert!(
            log.knows(&b),
            "revoking one seat leaves the other attributed"
        );
    }
}

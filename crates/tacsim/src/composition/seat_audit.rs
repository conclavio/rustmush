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
use std::fmt::{self, Write as _};
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};

use mush_core::{AccountId, CanonicalHash, SeatAudit};
use rand::TryRngCore as _;

/// Maximum total attributed seat tokens held in memory per process.
///
/// Once full across all accounts, [`SeatAuditLog::record`] fails closed
/// (`None`) rather than evicting another account's active seat or allowing
/// unbounded RAM growth.
pub const MAX_AUDIT_ROWS: usize = 4_096;

/// Maximum unforgotten seat tokens held per account.
///
/// Bounding per account prevents a single caller (for instance via failed
/// `put_seat_grant` attempts that never call `forget`) from exhausting the
/// global table or evicting other accounts' rows; only the caller's own oldest
/// entry is evicted once this limit is reached.
pub const MAX_ROWS_PER_ACCOUNT: usize = 64;

/// The process-scoped nonce source. It is minted once per process and never
/// rendered; two servers, and two logs in one process, mint disjoint tokens.
static NONCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy)]
struct AuditRow {
    account: AccountId,
    seq: u64,
}

/// The in-process token → account log.
pub struct SeatAuditLog {
    rows: RwLock<BTreeMap<String, AuditRow>>,
    /// The 256-bit per-log CSPRNG salt, minted at construction from the OS
    /// entropy source. Never rendered, never persisted; `None` fails closed.
    salt: Option<[u8; 32]>,
    max_rows: usize,
    max_rows_per_account: usize,
}

impl fmt::Debug for SeatAuditLog {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SeatAuditLog")
            .field("len", &self.len())
            .field("salt", &self.salt.as_ref().map(|_| "[redacted]"))
            .field("max_rows", &self.max_rows)
            .field("max_rows_per_account", &self.max_rows_per_account)
            .finish()
    }
}

impl Default for SeatAuditLog {
    fn default() -> Self {
        Self::new()
    }
}

impl SeatAuditLog {
    /// A fresh, empty log seeded with 256 bits of OS CSPRNG entropy.
    #[must_use]
    pub fn new() -> SeatAuditLog {
        Self::with_limits(MAX_AUDIT_ROWS, MAX_ROWS_PER_ACCOUNT)
    }

    fn with_limits(max_rows: usize, max_rows_per_account: usize) -> SeatAuditLog {
        let mut bytes = [0u8; 32];
        let salt = rand::rngs::OsRng
            .try_fill_bytes(&mut bytes)
            .ok()
            .map(|()| bytes);
        SeatAuditLog {
            rows: RwLock::new(BTreeMap::new()),
            salt,
            max_rows,
            max_rows_per_account,
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

fn hex_encode_32(bytes: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for b in bytes {
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

/// A poisoned lock or missing CSPRNG salt is a refusal, never a panic and never
/// a default: a seat that cannot be attributed is a seat that is not taken.
impl SeatAudit for SeatAuditLog {
    fn record(&self, seed: &str, account: AccountId) -> Option<String> {
        let salt = self.salt.as_ref()?;
        if self.max_rows == 0 || self.max_rows_per_account == 0 {
            return None;
        }
        // The 256-bit CSPRNG salt and monotonic counter are folded into the
        // canonical body, so the token is opaque to every party that holds
        // only the seat's public coordinates.
        let n = NONCE.fetch_add(1, Ordering::Relaxed);
        let body = serde_json::json!({
            "salt": hex_encode_32(salt),
            "n": n,
            "seed": seed,
        });
        let token = CanonicalHash::for_canonical_value("server.seat.driver.v1", 1, &body, 4096)
            .ok()?
            .to_hex();
        let token = token[..token.len().min(32)].to_owned();
        let mut rows = self.rows.write().ok()?;
        let mut account_rows = rows.values().filter(|row| row.account == account).count();
        while account_rows >= self.max_rows_per_account {
            let oldest_key = rows
                .iter()
                .filter(|(_, row)| row.account == account)
                .min_by_key(|(_, row)| row.seq)
                .map(|(key, _)| key.clone());
            let Some(key) = oldest_key else {
                break;
            };
            rows.remove(&key);
            account_rows = account_rows.saturating_sub(1);
        }
        if rows.len() >= self.max_rows && !rows.contains_key(&token) {
            return None;
        }
        rows.insert(token.clone(), AuditRow { account, seq: n });
        Some(token)
    }

    fn knows(&self, token: &str) -> bool {
        self.rows
            .read()
            .map(|rows| rows.contains_key(token))
            .unwrap_or(false)
    }

    fn account_for(&self, token: &str) -> Option<AccountId> {
        self.rows.read().ok()?.get(token).map(|row| row.account)
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

    #[test]
    fn p2_tsw_52_default_uses_distinct_csprng_salt_and_redacts_debug() {
        let a = SeatAuditLog::default();
        let b = SeatAuditLog::default();
        let salt_a = a.salt.expect("os csprng salt");
        let salt_b = b.salt.expect("os csprng salt");
        assert_ne!(salt_a, [0u8; 32], "default salt must never be all zeros");
        assert_ne!(salt_b, [0u8; 32], "default salt must never be all zeros");
        assert_ne!(
            salt_a, salt_b,
            "independent default logs must draw fresh salts"
        );

        let dbg = format!("{a:?}");
        assert!(dbg.contains("[redacted]"));
        assert!(!dbg.contains(&hex_encode_32(&salt_a)));
    }

    #[test]
    fn p2_tsw_52_bounds_per_account_and_global_rows_without_cross_account_eviction() {
        let log = SeatAuditLog::with_limits(4, 2);
        let alice = AccountId::mint("Alice", 1).expect("mints");
        let bob = AccountId::mint("Bob", 2).expect("mints");
        let carol = AccountId::mint("Carol", 3).expect("mints");

        let a1 = log.record("a1", alice).expect("mints a1");
        let a2 = log.record("a2", alice).expect("mints a2");
        let a3 = log.record("a3", alice).expect("mints a3");
        // Alice is capped at 2 rows: her oldest row (a1) is evicted, a2 and a3 survive.
        assert!(!log.knows(&a1));
        assert!(log.knows(&a2));
        assert!(log.knows(&a3));
        assert_eq!(log.len(), 2);

        let b1 = log.record("b1", bob).expect("mints b1");
        let b2 = log.record("b2", bob).expect("mints b2");
        assert_eq!(log.len(), 4);

        // Global table is full: Carol is refused fail-closed, and Alice/Bob rows remain intact.
        assert!(log.record("c1", carol).is_none());
        assert!(log.knows(&a2));
        assert!(log.knows(&a3));
        assert!(log.knows(&b1));
        assert!(log.knows(&b2));

        // After forgetting one row, Carol succeeds.
        assert!(log.forget(&b1));
        let c1 = log.record("c1", carol).expect("mints c1 after space frees");
        assert!(log.knows(&c1));
    }
}

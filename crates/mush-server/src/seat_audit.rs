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

use std::collections::BTreeMap;
use std::sync::RwLock;

use mush_core::{AccountId, SeatAudit};

/// The in-process token → account log.
#[derive(Debug, Default)]
pub struct SeatAuditLog {
    rows: RwLock<BTreeMap<String, AccountId>>,
}

impl SeatAuditLog {
    /// A fresh, empty log.
    #[must_use]
    pub fn new() -> SeatAuditLog {
        SeatAuditLog::default()
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
    fn record(&self, token: &str, account: AccountId) -> bool {
        match self.rows.write() {
            Ok(mut rows) => {
                rows.insert(token.to_owned(), account);
                true
            }
            Err(_) => false,
        }
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
        assert!(log.record("abc", account));
        assert!(log.knows("abc"));
        assert_eq!(log.account_for("abc"), Some(account));
        assert!(log.forget("abc"));
        assert!(!log.knows("abc"), "a forgotten token is unattributed again");
        assert!(!log.forget("abc"), "forgetting twice is not an error");
    }

    /// **Two seats for one player produce two rows nothing correlates.** The log
    /// is keyed by token, and a token carries no account material at all, so the
    /// only thing that could join them is this log — which no projection reads.
    #[test]
    fn e7c10_b3_two_tokens_for_one_account_are_independent_rows() {
        let log = SeatAuditLog::new();
        let account = AccountId::mint("Nick", 1).expect("mints");
        assert!(log.record("aaa", account));
        assert!(log.record("bbb", account));
        assert!(log.forget("aaa"));
        assert!(
            log.knows("bbb"),
            "revoking one seat leaves the other attributed"
        );
    }
}

//! E7c2 B4 — the first `AuthorityGuardKind::Reservation` validator on the tree.
//!
//! G3, as re-grounded in 10b §4.0: `AuthorityGuardKind::Reservation` appears
//! under `crates/*/src` only as the enum arm and its routing, and exactly two
//! production `authority_guard_validator` registrations exist —
//! `crates/mush-sim/src/lib.rs`'s `ControlLeaseGuardValidator` and
//! `crates/mush-tactics/src/module.rs`'s `ContactGuardValidator`. This copies
//! those two and is the first `Reservation`.
//!
//! **Why cut 4 registering the same kind is legal by construction, not by luck:**
//! `AuthorityGuardValidatorRegistry` is keyed `(OwnerId, AuthorityGuardKind)` and
//! routed per key, and `Registrar::authority_guard_validator` stamps the *calling
//! module's* owner. `mush-industry` registering `Reservation` for capacity under
//! its own `OwnerId` therefore lands on a different key; it cannot collide with
//! this one, and neither can validate the other's guards — `is_current` below
//! refuses any guard whose `provider` is not this crate's own owner.

use std::fmt;

use mush_core::{
    ActionError, ActionErrorCode, AuthorityGuard, AuthorityGuardContext, AuthorityGuardValidator,
    ReservationBook, ReservationBookId, ReservationState, StateFamilyHandle,
};

/// Re-reads the book and answers `false` on a missing record, a bumped revision
/// or a terminal state.
///
/// `witness` stays the default `None`: a reservation guard is checked, never
/// issued as private evidence, and the only two `AuthorityWitnessRequest` arms
/// are `ControlLease` and `Contact`.
pub struct ReservationGuardValidator {
    books: StateFamilyHandle<ReservationBookId, ReservationBook>,
}

impl fmt::Debug for ReservationGuardValidator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReservationGuardValidator")
            .field("owner", self.books.owner())
            .field("family", self.books.descriptor().id())
            .finish()
    }
}

impl ReservationGuardValidator {
    /// Wrap the book family handle the module registered.
    #[must_use]
    pub const fn new(books: StateFamilyHandle<ReservationBookId, ReservationBook>) -> Self {
        Self { books }
    }
}

impl AuthorityGuardValidator for ReservationGuardValidator {
    fn is_current(
        &self,
        context: &AuthorityGuardContext<'_>,
        guard: &AuthorityGuard,
    ) -> Result<bool, ActionError> {
        let AuthorityGuard::Reservation {
            provider,
            id,
            revision,
        } = guard
        else {
            return Ok(false);
        };
        if provider != self.books.owner() {
            return Ok(false);
        }
        // §3.2 asserts once, in a unit test, that a `ReservationId` always
        // renders as canonical `IdempotencyKeyText` (`CanonicalHash::to_hex` is
        // 64 lowercase hex bytes, inside `MAX_IDEMPOTENCY_KEY_BYTES` = 128 and
        // inside the byte class). It is deliberately NOT re-checked here: guard
        // time is the wrong place to discover an unconstructible id.
        let Some(reservation) = crate::pool::reservation_id_from_text(id) else {
            return Ok(false);
        };
        // The book a reservation lives in is keyed by its own resource, which
        // the guard does not name — so the search is over the *pages* of this
        // owner's book family, bounded on records visited.
        let mut after: Option<smol_str::SmolStr> = None;
        for _ in 0..crate::store::MAX_BOOT_PAGES {
            let page = context
                .scan_action_state(&self.books, after.as_deref(), crate::record::INV_PAGE)
                .map_err(|_| {
                    ActionError::new(
                        ActionErrorCode::Integrity,
                        false,
                        "inventory reservation authority state is unavailable",
                    )
                    .expect("static inventory guard read error is bounded")
                })?;
            for entry in page.entries() {
                if let Some(record) = entry
                    .value()
                    .records()
                    .iter()
                    .find(|record| record.id() == reservation)
                {
                    return Ok(
                        record.state() == ReservationState::Held && record.revision() == *revision
                    );
                }
            }
            match page.next_after().cloned() {
                Some(next) => after = Some(next),
                None => return Ok(false),
            }
        }
        // A budget that ran out is not evidence of currency.
        Ok(false)
    }
}

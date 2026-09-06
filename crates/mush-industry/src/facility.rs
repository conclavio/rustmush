//! E7c4 B1 — facilities, the capacity resource, and cut 4's `Reservation`
//! validator.
//!
//! # The facility record is the exempt catalog/policy half (OPEN 1)
//!
//! [`FacilityRecord`] is **not** authoritative typed state: it is the pack- and
//! seed-authored *definition* half (§4.1's cut-4 row, amended in place —
//! contradiction (iii)). It carries no `active_slots` counter at all, because a
//! number nobody may read as authority is the fail-open shape this programme
//! avoids. **Availability is derived at every read** from the live capacity
//! reservations on [`ReservationBook`], exactly as `held_amount` is a re-derived
//! projection in E4 (08c D-E4-1). §3.4's "corrupt `active_slots` and re-run every
//! availability path" probe is therefore a *structural* assertion — the field
//! does not exist — plus one test that no availability path reads a stored
//! counter.
//!
//! # The capacity book is authoritative, and cut 4's `Reservation` validator is
//! legal by construction
//!
//! A facility slot is one hold on K's [`ReservationBook`] under a per-facility
//! [`ResourceRef`] whose subject is `SubjectKey::Scoped { owner: <industry>,
//! class: "industry.facility", id: "f<obj>" }` and whose id is `"slots"`. The
//! `(inventory.pool, inventory.book)` pattern is copied wholesale: the slot count
//! is a `BoundedScalar` bounded `0..=maximum_slots` with `current == maximum`
//! (slots are **held and released**, never consumed), and availability is
//! `checked_available` — the count minus every live hold.
//!
//! Cut 4 registers the tree's **second** `AuthorityGuardKind::Reservation`
//! validator, under `mush-industry`'s own `OwnerId`. The registry is keyed
//! `(OwnerId, AuthorityGuardKind)` and `Registrar::authority_guard_validator`
//! stamps the calling module's owner, so this lands on a different key from cut
//! 2's inventory validator; it cannot collide, and neither validates the other's
//! guards ([`CapacityGuardValidator::is_current`] refuses any guard whose
//! `provider` is not this crate's own owner). Cut 2 pinned exactly that pair.

use std::collections::BTreeSet;
use std::fmt;

use mush_core::{
    ActionError, ActionErrorCode, AuthorityGuard, AuthorityGuardContext, AuthorityGuardValidator,
    CanonicalId, IdempotencyKeyText, ObjId, OwnerId, ReservationBook, ReservationBookId,
    ReservationId, ReservationState, ResourceRef, StateFamilyHandle, SubjectKey,
};
use mush_org::PrincipalId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::families::IndustryError;
use crate::record::{IndustryRecordError, owner};

/// §7: slots one facility may run at once. `MAX_WORK_ORDER_INPUTS` (10) plus the
/// capacity and destination reservations still fit `MAX_RESERVATIONS_PER_ORDER`
/// (12) with headroom, so this cap is reachable rather than dead.
pub const MAX_FACILITY_SLOTS: u32 = 8;
/// Largest capability set one facility carries. A capability is a canonical id a
/// recipe's `requires` is matched against.
pub const MAX_FACILITY_CAPABILITIES: usize = 32;
/// The `SubjectKey::Scoped` class every facility's capacity resource is scoped
/// by — the industry sibling of cut 2's `inventory.endpoint`.
pub const FACILITY_CLASS: &str = "industry.facility";
/// The `ResourceRef.id` of a facility's slot resource.
pub const FACILITY_SLOTS_ID: &str = "slots";

/// A facility's **definition/policy half** — pack- and seed-authored, exempt,
/// `legacy-post-image` (family `industry.facility`), read at offer projection and
/// pinned into a work order at draft. It gates action legality only through the
/// values a work order **pins** and re-validates against, never through a live
/// read inside a provider (which has no world).
///
/// **No `active_slots`.** Availability is `checked_available` over the live
/// capacity book; storing an occupancy counter would be the fail-open shape OPEN
/// 1 forbids.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FacilityRecord {
    object: ObjId,
    owner: PrincipalId,
    capabilities: BTreeSet<CanonicalId>,
    maximum_slots: u32,
    capacity_revision: u64,
    policy_revision: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FacilityRecordWire {
    object: ObjId,
    owner: PrincipalId,
    capabilities: BTreeSet<CanonicalId>,
    maximum_slots: u32,
    capacity_revision: u64,
    policy_revision: u64,
}

impl<'de> Deserialize<'de> for FacilityRecord {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = FacilityRecordWire::deserialize(deserializer)?;
        Self::from_parts(
            wire.object,
            wire.owner,
            wire.capabilities,
            wire.maximum_slots,
            wire.capacity_revision,
            wire.policy_revision,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl FacilityRecord {
    /// Author one facility at revision 1/1.
    ///
    /// # Errors
    /// [`IndustryRecordError`] for zero/oversized slots or an oversized
    /// capability set.
    pub fn new(
        object: ObjId,
        owner: PrincipalId,
        capabilities: BTreeSet<CanonicalId>,
        maximum_slots: u32,
    ) -> Result<Self, IndustryRecordError> {
        Self::from_parts(object, owner, capabilities, maximum_slots, 1, 1)
    }

    fn from_parts(
        object: ObjId,
        owner: PrincipalId,
        capabilities: BTreeSet<CanonicalId>,
        maximum_slots: u32,
        capacity_revision: u64,
        policy_revision: u64,
    ) -> Result<Self, IndustryRecordError> {
        if maximum_slots == 0 || maximum_slots > MAX_FACILITY_SLOTS {
            return Err(IndustryRecordError::FacilitySlots {
                max: MAX_FACILITY_SLOTS,
            });
        }
        if capabilities.len() > MAX_FACILITY_CAPABILITIES {
            return Err(IndustryRecordError::FacilityCapabilities {
                max: MAX_FACILITY_CAPABILITIES,
            });
        }
        if capacity_revision == 0 || policy_revision == 0 {
            return Err(IndustryRecordError::ZeroRevision);
        }
        Ok(Self {
            object,
            owner,
            capabilities,
            maximum_slots,
            capacity_revision,
            policy_revision,
        })
    }

    /// The world object this facility is.
    #[must_use]
    pub const fn object(&self) -> ObjId {
        self.object
    }
    /// Who owns it — the principal a work order's org/role scope is checked
    /// against.
    #[must_use]
    pub const fn owner(&self) -> &PrincipalId {
        &self.owner
    }
    /// The capabilities a recipe's `requires` is matched against.
    #[must_use]
    pub const fn capabilities(&self) -> &BTreeSet<CanonicalId> {
        &self.capabilities
    }
    /// Whether this facility offers every capability a recipe requires.
    #[must_use]
    pub fn offers(&self, requires: &BTreeSet<CanonicalId>) -> bool {
        requires.is_subset(&self.capabilities)
    }
    /// The concurrent-slot cap.
    #[must_use]
    pub const fn maximum_slots(&self) -> u32 {
        self.maximum_slots
    }
    /// The staleness stamp a work order pins and re-checks: a bump interrupts
    /// every order holding a slot here.
    #[must_use]
    pub const fn capacity_revision(&self) -> u64 {
        self.capacity_revision
    }
    /// The staleness stamp a licence/policy change bumps.
    #[must_use]
    pub const fn policy_revision(&self) -> u64 {
        self.policy_revision
    }
}

/// The document key one facility's definition lives at (the `industry` namespace,
/// exactly as cut 3's `industry:survey:<deposit>`).
#[must_use]
pub fn facility_key(object: ObjId) -> SmolStr {
    SmolStr::new(format!("facility:{}", object.0))
}

/// The facility a document key names, if it is one.
#[must_use]
pub fn facility_of_key(key: &str) -> Option<ObjId> {
    key.strip_prefix("facility:")
        .and_then(|body| body.parse::<u32>().ok())
        .map(ObjId)
}

/// The industry `OwnerId` — re-exported here so the capacity resource reads in
/// one place.
#[must_use]
fn industry_owner() -> OwnerId {
    owner()
}

/// The `ResourceRef` one facility's slot capacity is keyed by.
///
/// # Errors
/// [`IndustryError::Identity`] if the object cannot render as canonical
/// idempotency text (structurally unreachable: `f<decimal>` is inside the byte
/// class).
pub fn facility_resource(object: ObjId) -> Result<ResourceRef, IndustryError> {
    let id = IdempotencyKeyText::parse_canonical(format!("f{}", object.0))
        .map_err(|_| IndustryError::Identity)?;
    let class = CanonicalId::new(FACILITY_CLASS).map_err(|_| IndustryError::Identity)?;
    let slots = CanonicalId::new(FACILITY_SLOTS_ID).map_err(|_| IndustryError::Identity)?;
    Ok(ResourceRef::new(
        industry_owner(),
        SubjectKey::Scoped {
            owner: industry_owner(),
            class,
            id,
        },
        slots,
    ))
}

/// The book/scalar key one facility's capacity is stored under.
///
/// # Errors
/// [`IndustryError`] if the resource cannot be derived or hashed.
pub fn facility_capacity_key(object: ObjId) -> Result<ReservationBookId, IndustryError> {
    let resource = facility_resource(object)?;
    ReservationBookId::derive(&resource).map_err(|_| IndustryError::Identity)
}

/// The facility an object a capacity subject names, if it is one.
#[must_use]
pub fn facility_of_subject(subject: &SubjectKey) -> Option<ObjId> {
    let SubjectKey::Scoped { owner, class, id } = subject else {
        return None;
    };
    if owner != &industry_owner() || class.as_str() != FACILITY_CLASS {
        return None;
    }
    id.as_str()
        .strip_prefix('f')
        .and_then(|body| body.parse::<u32>().ok())
        .map(ObjId)
}

/// Re-reads the capacity book and answers `false` on a missing record, a bumped
/// revision or a terminal state. The tree's **second** `Reservation` validator,
/// under `mush-industry`'s own `OwnerId`.
///
/// `witness` stays the default `None`: a reservation guard is checked, never
/// issued as private evidence.
pub struct CapacityGuardValidator {
    books: StateFamilyHandle<ReservationBookId, ReservationBook>,
    /// One [`ReservationId`] → its [`ReservationBookId`], written in the same
    /// bundle as the hold (cut 2's fold B10 mechanism). The guard names the
    /// reservation, the book is keyed by its resource, so the index makes finding
    /// the book two point lookups rather than a walk with a budget cliff.
    index: StateFamilyHandle<ReservationId, ReservationBookId>,
}

impl fmt::Debug for CapacityGuardValidator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CapacityGuardValidator")
            .field("owner", self.books.owner())
            .field("family", self.books.descriptor().id())
            .finish()
    }
}

impl CapacityGuardValidator {
    /// Wrap the capacity book family and its reservation index.
    #[must_use]
    pub const fn new(
        books: StateFamilyHandle<ReservationBookId, ReservationBook>,
        index: StateFamilyHandle<ReservationId, ReservationBookId>,
    ) -> Self {
        Self { books, index }
    }
}

impl AuthorityGuardValidator for CapacityGuardValidator {
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
        // Legal by construction, not by luck: this validator answers only for
        // guards under its own owner, so cut 2's inventory guards route to cut
        // 2's validator and never here.
        if provider != self.books.owner() {
            return Ok(false);
        }
        let Some(reservation) = mush_inventory::reservation_id_from_text(id) else {
            return Ok(false);
        };
        let unavailable = || {
            ActionError::new(
                ActionErrorCode::Integrity,
                false,
                "industry capacity authority state is unavailable",
            )
            .expect("static capacity guard read error is bounded")
        };
        let Some(book_id) = context
            .action_state(&self.index, &reservation)
            .map_err(|_| unavailable())?
            .copied()
        else {
            return Ok(false);
        };
        let Some(book) = context
            .action_state(&self.books, &book_id)
            .map_err(|_| unavailable())?
        else {
            return Ok(false);
        };
        Ok(book
            .records()
            .iter()
            .find(|record| record.id() == reservation)
            .is_some_and(|record| {
                record.state() == ReservationState::Held && record.revision() == *revision
            }))
    }
}

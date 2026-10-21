//! E7c2 B2/B3 — the records, their caps, and their strict wires.
//!
//! Every persisted type here re-enters through a `TryFrom` wire that re-runs its
//! constructor invariants on load (§4 rule 1, D-E7-33): a derived `Deserialize`
//! would walk straight past every cap below and make §7 advisory, which is the
//! E5a finding this programme is not allowed to re-learn.
//!
//! **Q2D fixes the lot's fields exactly: not `legal`, not `custody`.** Custody is
//! the *endpoint's* — a lot has one location and one custodian because its vault
//! does. Stolen or disputed status follows the **title** record
//! (`mush_economy::Encumbrance::Disputed`), embargo legality is a jurisdiction
//! test at transaction time, and licensing is the acting principal's entitlement
//! (Q1A/Q14A). None of the three is a field that travels with matter.

use mush_core::{CanonicalId, IdempotencyKeyText, ObjId, OwnerId, SubjectKey};
use mush_economy::{ExternalRef, VaultId};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use thiserror::Error;

/// Owner/module namespace. One `OwnerId`, one fact namespace, one book family.
pub const NS: &str = "inventory";

// ---- E7c2: the constants (§7) ----
/// Largest parent set one `Lineage::Merge` may name (§7; E6's figure, 08f §9).
pub const MAX_LOT_PARENTS: usize = 16;
/// Largest recorded lineage depth. Enforced on the *stored* depth so no verb
/// ever walks an unbounded chain to decide whether the next link is legal.
pub const MAX_LINEAGE_DEPTH: u32 = 32;
/// Largest number of distinct commodities one endpoint may pool (§7).
pub const MAX_POOLS_PER_ENDPOINT: usize = 64;
/// Reservation time-to-live, in IC days (§7). Stamped from `Ctx::world_ic_now`.
pub const RESERVATION_TTL_IC_DAYS: i64 = 7;
/// The same figure in IC seconds, which is what a `ClockStamp` counts.
pub const RESERVATION_TTL_SECS: i64 = RESERVATION_TTL_IC_DAYS * 86_400;
/// Ceiling of one pool's `BoundedScalar`. Well inside `i64`, so every checked
/// addition in a transfer has headroom on both sides.
pub const MAX_POOL_QUANTITY: i64 = 1_000_000_000_000;
/// Ceiling of one lot's quantity.
pub const MAX_LOT_QUANTITY: u64 = 1_000_000_000;
/// Largest lot set one tranche records absorbing.
pub const MAX_TRANCHE_ABSORBED: usize = 256;
/// Largest set of foreign roots one tranche records inheriting (E7c2 fold C2).
/// A pool-to-pool transfer records the source slot's root here, so the
/// destination's lineage reaches the source's `absorbed` set transitively.
pub const MAX_TRANCHE_INHERITED: usize = 64;
/// Largest tranche generation one `(endpoint, commodity)` slot may reach
/// (E7c2 fold B2). The root is a **sequence**, not a singleton: a full
/// `absorbed` set, a full `inherited` set or a depth-pinning dissolve rolls to
/// `generation + 1` and names its predecessor, so no cap is an absorbing state.
pub const MAX_TRANCHE_GENERATIONS: u32 = 64;
/// How many terminal reservation records one book keeps before a fresh hold
/// prunes the oldest through K's own `ReservationBook::checked_prune_terminal`
/// (E7c2 fold C3). Well inside `MAX_RESERVATION_BOOK_RECORDS` (256), so a slot
/// never reaches the book's own record limit.
pub const MAX_RETAINED_TERMINAL_RESERVATIONS: usize = 32;
/// Largest number of live lot records one endpoint may hold (E7c2 fold B8).
pub const MAX_LOTS_PER_ENDPOINT: u32 = 512;
/// Largest quality grade. `0` is legal (slag is still matter).
pub const MAX_GRADE: u8 = 100;
/// Page size for every `@inv` listing, counted on records **visited**.
pub const INV_PAGE: usize = 32;
/// Largest applied-marker mirror one world keeps in module state.
pub const MAX_APPLIED_MARKERS: usize = 4_096;
/// Largest bytes one applied-marker id may occupy.
pub const MAX_MARKER_ID_BYTES: usize = 96;

// ---- Doc 18 §9 risk 19, cut IC14 row 1: the per-endpoint marker ring ----
/// The ring's floor (doc 18 cut IC14, row 1): a source endpoint's
/// [`MarkerRing`] at or under this many entries evicts nothing, however old.
/// It is what keeps every endpoint's newest moves answerable as facts long
/// after the horizon, and what bounds the durable `inventory.applied` family
/// per endpoint once its markers age.
pub const MAX_RETAINED_MARKERS_PER_ENDPOINT: usize = 32;
/// The real-clock retention horizon, in **real** seconds (doc 18 cut IC14,
/// row 1): an entry younger than this is never evicted. It must be at least
/// the longest real window in which a marker is read or its request replayed
/// — a sale's draw window (3,600 s) + the executor's first re-stage (90 s) +
/// its settle grace (160 s) + two market sweeps (120 s) = 3,970 s, and a
/// re-driven offer after a reboot (E7c2 owner decision D1) at the first fires
/// after the boot. One real day is that with a wide margin. This crate sits
/// below the market and TacSim and cannot name their constants, so TacSim
/// asserts the inequality. Real, not IC: the IC clock runs at up to 3,600×.
///
/// A **disputed** sale stays in flight past every one of those windows (the
/// economy pages a dispute after three days), so its marker can be retired
/// while the contract still waits on staff. The market therefore reads an
/// absent sale marker as "not moved" only inside this horizon from the deal's
/// acceptance, and as unconfirmed past it (paged, the bid never restored; cut
/// IC14 review F1).
pub const MARKER_RETENTION_SECS: i64 = 86_400;
/// The ring's hard cap (doc 18 cut IC14, row 1): a ring of this many entries
/// none of which may yet be evicted refuses the next marker-writing move out
/// of that endpoint — fail closed, never evict a young marker.
pub const MAX_MARKER_RING: usize = 128;
/// How many aged entries one push may evict (doc 18 cut IC14, row 1), so a
/// bundle's write set stays small; an aged ring drains over a few moves.
pub const MAX_RING_EVICTIONS_PER_PUSH: usize = 8;

/// The canonical class of a quantity endpoint inside `ResourceRef.subject`.
pub const ENDPOINT_CLASS: &str = "inventory.endpoint";

/// This crate's mechanics owner id.
///
/// # Panics
/// Never: `"inventory"` is a canonical id by construction, asserted by a test.
#[must_use]
pub fn owner() -> OwnerId {
    OwnerId::new(NS).expect("static inventory owner id is canonical")
}

// ---- E7c2: EndpointRef (OPEN 1) ----

/// A quantity endpoint: **an E3 vault**.
///
/// 10b §3.2 drafted `EndpointRef { provider: OwnerId, object: ObjId }`. The tree's
/// endpoint is a [`mush_economy::Vault`] whose `backing` is
/// `CustodyRef::{Container, SpatialInterior, Registered}` and whose provider is a
/// `&str`, not an `OwnerId` — so the drafted field list can name neither a
/// `Registered` backing nor its provider. **The vault *is* the endpoint**: its
/// `backing` answers *where*, its `status`/`owner`/`scope` answer custody and
/// compartment, and a `Registered` backing is nameable like any other.
///
/// This is also what makes locality free: a hold that moves with a ship needs no
/// second mechanism, because the **backing** moved and the pool did not.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct EndpointRef(pub VaultId);

impl EndpointRef {
    /// The vault this endpoint is.
    #[must_use]
    pub const fn vault(self) -> VaultId {
        self.0
    }

    /// The endpoint's canonical text inside a `SubjectKey::Scoped`.
    ///
    /// # Errors
    /// Never in practice: `v<decimal>` is inside `IdempotencyKeyText`'s byte
    /// class and far inside `MAX_IDEMPOTENCY_KEY_BYTES`. Asserted once.
    pub fn key_text(self) -> Result<IdempotencyKeyText, InventoryRecordError> {
        IdempotencyKeyText::parse_canonical(format!("v{}", self.0.0))
            .map_err(|_| InventoryRecordError::EndpointKey)
    }

    /// The `SubjectKey` every pool of this endpoint is scoped by.
    ///
    /// # Errors
    /// [`InventoryRecordError::EndpointKey`] if the endpoint cannot render.
    pub fn subject(self) -> Result<SubjectKey, InventoryRecordError> {
        Ok(SubjectKey::Scoped {
            owner: owner(),
            class: CanonicalId::new(ENDPOINT_CLASS).expect("static endpoint class is canonical"),
            id: self.key_text()?,
        })
    }
}

impl std::fmt::Display for EndpointRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "vault#{}", self.0.0)
    }
}

// ---- E7c2: Grade, LotId, TrancheId (contradiction (vii)) ----

/// A lot's quality band, `0..=`[`MAX_GRADE`].
///
/// Deliberately a scalar and not a ruleset concept: `mush-inventory` is neutral,
/// and what a grade *means* for a commodity is the ruleset's (Q2D).
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Grade(u8);

#[derive(Deserialize)]
#[serde(transparent)]
struct GradeWire(u8);

impl<'de> Deserialize<'de> for Grade {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = GradeWire::deserialize(deserializer)?;
        Self::new(wire.0).map_err(serde::de::Error::custom)
    }
}

impl Grade {
    /// Build a grade.
    ///
    /// # Errors
    /// [`InventoryRecordError::Grade`] above [`MAX_GRADE`].
    pub const fn new(value: u8) -> Result<Self, InventoryRecordError> {
        if value > MAX_GRADE {
            return Err(InventoryRecordError::Grade { max: MAX_GRADE });
        }
        Ok(Self(value))
    }

    /// The scalar.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

macro_rules! inventory_id {
    ($(#[$m:meta])* $name:ident, $prefix:literal) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(pub u64);

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, concat!($prefix, "#{:016x}"), self.0)
            }
        }

        impl $name {
            /// Parse the rendered form (`prefix#<16 hex>`), or a bare hex id.
            #[must_use]
            pub fn parse(text: &str) -> Option<Self> {
                let body = text
                    .trim()
                    .strip_prefix(concat!($prefix, "#"))
                    .unwrap_or_else(|| text.trim());
                (body.len() == 16)
                    .then(|| u64::from_str_radix(body, 16).ok())
                    .flatten()
                    .map(Self)
            }
        }
    };
}

inventory_id!(
    /// A lot's identity. **Derived**, never allocated: an allocator whose retry
    /// behaviour could diverge after a restart would mint a second lot for one
    /// replayed action. See `crate::pool::derive_lot_id`.
    LotId,
    "lot"
);
inventory_id!(
    /// A pool tranche's identity, derived the same way as a [`LotId`].
    TrancheId,
    "tranche"
);

// ---- E7c2: Lineage (D-E7-4, D-E7-5, T5) ----

/// Where a lot came from. Append-only and acyclic by construction: every arm
/// names strictly *older* records, and the depth carried beside it is the only
/// thing any verb has to read to know the chain is still bounded.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Lineage {
    /// Minted from outside the inventory — an extraction cycle, a delivery, a
    /// seeded fixture. `source` is E2's own [`ExternalRef`], never a new shape.
    Origin {
        /// The domain journal entry that produced it.
        source: ExternalRef,
    },
    /// Split off one parent lot.
    Split {
        /// The lot it came out of.
        parent: LotId,
    },
    /// Merged from several parent lots (`1..=`[`MAX_LOT_PARENTS`]).
    Merge {
        /// The lots it absorbed, canonically ordered and duplicate-free.
        parents: Vec<LotId>,
    },
    /// Crystallized out of a pool tranche. **The laundering axis**: this arm is
    /// what keeps a `dissolve` → `crystallize` round trip from erasing
    /// provenance, because the tranche it names still records every lot it
    /// absorbed.
    Tranche {
        /// The immutable lineage root it was minted from.
        tranche: TrancheId,
    },
}

impl Lineage {
    /// Every parent lot this arm names, in canonical order.
    #[must_use]
    pub fn parents(&self) -> Vec<LotId> {
        match self {
            Self::Origin { .. } | Self::Tranche { .. } => Vec::new(),
            Self::Split { parent } => vec![*parent],
            Self::Merge { parents } => parents.clone(),
        }
    }

    fn check(&self) -> Result<(), InventoryRecordError> {
        let Self::Merge { parents } = self else {
            return Ok(());
        };
        if parents.is_empty() || parents.len() > MAX_LOT_PARENTS {
            return Err(InventoryRecordError::LotParents {
                max: MAX_LOT_PARENTS,
            });
        }
        let mut previous: Option<LotId> = None;
        for parent in parents {
            if previous.is_some_and(|previous| previous >= *parent) {
                return Err(InventoryRecordError::NonCanonicalParents);
            }
            previous = Some(*parent);
        }
        Ok(())
    }
}

// ---- E7c2: Lot ----

/// One identified parcel of matter at one endpoint.
///
/// §3.2's field list is `{ id, commodity, quantity, grade, lineage }`. Two fields
/// are added and both are recorded as deviations in §3.2's "As built":
/// **`endpoint`**, because OPEN 3 makes the lot's endpoint a field so that moving
/// a ship moves no lot record at all; and **`depth`**, because
/// [`MAX_LINEAGE_DEPTH`] must be enforceable without walking an unbounded chain
/// (and a walk is exactly what the bound exists to forbid).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Lot {
    id: LotId,
    endpoint: EndpointRef,
    commodity: CanonicalId,
    quantity: u64,
    grade: Grade,
    lineage: Lineage,
    depth: u32,
    revision: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LotWire {
    id: LotId,
    endpoint: EndpointRef,
    commodity: CanonicalId,
    quantity: u64,
    grade: Grade,
    lineage: Lineage,
    depth: u32,
    revision: u64,
}

impl<'de> Deserialize<'de> for Lot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = LotWire::deserialize(deserializer)?;
        Self::from_parts(
            wire.id,
            wire.endpoint,
            wire.commodity,
            wire.quantity,
            wire.grade,
            wire.lineage,
            wire.depth,
            wire.revision,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl Lot {
    /// Build a lot.
    ///
    /// # Errors
    /// [`InventoryRecordError`] for a zero or oversized quantity, a malformed
    /// lineage, or a depth past [`MAX_LINEAGE_DEPTH`].
    pub fn new(
        id: LotId,
        endpoint: EndpointRef,
        commodity: CanonicalId,
        quantity: u64,
        grade: Grade,
        lineage: Lineage,
        depth: u32,
    ) -> Result<Self, InventoryRecordError> {
        Self::from_parts(id, endpoint, commodity, quantity, grade, lineage, depth, 1)
    }

    #[allow(clippy::too_many_arguments)]
    fn from_parts(
        id: LotId,
        endpoint: EndpointRef,
        commodity: CanonicalId,
        quantity: u64,
        grade: Grade,
        lineage: Lineage,
        depth: u32,
        revision: u64,
    ) -> Result<Self, InventoryRecordError> {
        if quantity == 0 {
            return Err(InventoryRecordError::ZeroQuantity);
        }
        if quantity > MAX_LOT_QUANTITY {
            return Err(InventoryRecordError::LotQuantity {
                max: MAX_LOT_QUANTITY,
            });
        }
        if revision == 0 {
            return Err(InventoryRecordError::ZeroRevision);
        }
        if depth > MAX_LINEAGE_DEPTH {
            return Err(InventoryRecordError::LineageDepth {
                max: MAX_LINEAGE_DEPTH,
            });
        }
        lineage.check()?;
        if lineage.parents().contains(&id) {
            return Err(InventoryRecordError::LineageCycle);
        }
        Ok(Self {
            id,
            endpoint,
            commodity,
            quantity,
            grade,
            lineage,
            depth,
            revision,
        })
    }

    /// Identity.
    #[must_use]
    pub const fn id(&self) -> LotId {
        self.id
    }

    /// Where it is. The lot moves by having this rewritten; the *matter* moves
    /// with the endpoint's backing and needs no record change at all.
    #[must_use]
    pub const fn endpoint(&self) -> EndpointRef {
        self.endpoint
    }

    /// What it is.
    #[must_use]
    pub const fn commodity(&self) -> &CanonicalId {
        &self.commodity
    }

    /// How much.
    #[must_use]
    pub const fn quantity(&self) -> u64 {
        self.quantity
    }

    /// Quality band.
    #[must_use]
    pub const fn grade(&self) -> Grade {
        self.grade
    }

    /// Where it came from.
    #[must_use]
    pub const fn lineage(&self) -> &Lineage {
        &self.lineage
    }

    /// Recorded lineage depth.
    #[must_use]
    pub const fn depth(&self) -> u32 {
        self.depth
    }

    /// Staleness stamp.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// One lot with a smaller quantity and one revision more.
    ///
    /// # Errors
    /// [`InventoryRecordError`] on a stale revision or an invalid remainder.
    pub fn checked_resize(
        &self,
        expected_revision: u64,
        quantity: u64,
    ) -> Result<Self, InventoryRecordError> {
        if self.revision != expected_revision {
            return Err(InventoryRecordError::StaleRevision {
                expected: expected_revision,
                actual: self.revision,
            });
        }
        Self::from_parts(
            self.id,
            self.endpoint,
            self.commodity.clone(),
            quantity,
            self.grade,
            self.lineage.clone(),
            self.depth,
            self.revision
                .checked_add(1)
                .ok_or(InventoryRecordError::RevisionExhausted)?,
        )
    }

    /// The same lot at another endpoint.
    ///
    /// # Errors
    /// [`InventoryRecordError`] on a stale revision.
    pub fn checked_relocate(
        &self,
        expected_revision: u64,
        endpoint: EndpointRef,
    ) -> Result<Self, InventoryRecordError> {
        if self.revision != expected_revision {
            return Err(InventoryRecordError::StaleRevision {
                expected: expected_revision,
                actual: self.revision,
            });
        }
        Self::from_parts(
            self.id,
            endpoint,
            self.commodity.clone(),
            self.quantity,
            self.grade,
            self.lineage.clone(),
            self.depth,
            self.revision
                .checked_add(1)
                .ok_or(InventoryRecordError::RevisionExhausted)?,
        )
    }
}

// ---- E7c2: PoolTranche — the immutable lineage root ----

/// What a `dissolve` pours a lot into, and what a `crystallize` mints out of.
///
/// It is the **immutable lineage root** of the round trip: `absorbed` records
/// every lot poured in and is never rewritten to drop one, so pouring a stolen
/// lot into a pool and re-minting it cannot erase provenance. The child's
/// `Lineage::Tranche { tranche }` names this record and this record still names
/// the original `LotId`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PoolTranche {
    id: TrancheId,
    endpoint: EndpointRef,
    commodity: CanonicalId,
    generation: u32,
    predecessor: Option<TrancheId>,
    inherited: Vec<TrancheId>,
    absorbed: Vec<LotId>,
    quantity: u64,
    remaining: u64,
    depth: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PoolTrancheWire {
    id: TrancheId,
    endpoint: EndpointRef,
    commodity: CanonicalId,
    generation: u32,
    predecessor: Option<TrancheId>,
    inherited: Vec<TrancheId>,
    absorbed: Vec<LotId>,
    quantity: u64,
    remaining: u64,
    depth: u32,
}

impl<'de> Deserialize<'de> for PoolTranche {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = PoolTrancheWire::deserialize(deserializer)?;
        Self::from_parts(
            wire.id,
            wire.endpoint,
            wire.commodity,
            wire.generation,
            wire.predecessor,
            wire.inherited,
            wire.absorbed,
            wire.quantity,
            wire.remaining,
            wire.depth,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl PoolTranche {
    /// Build a tranche.
    ///
    /// # Errors
    /// [`InventoryRecordError`] for an oversized or non-canonical absorbed or
    /// inherited set, a root that names itself, a zero quantity, a `remaining`
    /// past `quantity`, a generation past [`MAX_TRANCHE_GENERATIONS`], or a
    /// depth past [`MAX_LINEAGE_DEPTH`].
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: TrancheId,
        endpoint: EndpointRef,
        commodity: CanonicalId,
        generation: u32,
        predecessor: Option<TrancheId>,
        inherited: Vec<TrancheId>,
        absorbed: Vec<LotId>,
        quantity: u64,
        remaining: u64,
        depth: u32,
    ) -> Result<Self, InventoryRecordError> {
        Self::from_parts(
            id,
            endpoint,
            commodity,
            generation,
            predecessor,
            inherited,
            absorbed,
            quantity,
            remaining,
            depth,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn from_parts(
        id: TrancheId,
        endpoint: EndpointRef,
        commodity: CanonicalId,
        generation: u32,
        predecessor: Option<TrancheId>,
        inherited: Vec<TrancheId>,
        absorbed: Vec<LotId>,
        quantity: u64,
        remaining: u64,
        depth: u32,
    ) -> Result<Self, InventoryRecordError> {
        // A root records matter it absorbed, matter it inherited from another
        // endpoint's root, or both — but never neither.
        if absorbed.is_empty() && inherited.is_empty() {
            return Err(InventoryRecordError::TrancheAbsorbed {
                max: MAX_TRANCHE_ABSORBED,
            });
        }
        if absorbed.len() > MAX_TRANCHE_ABSORBED {
            return Err(InventoryRecordError::TrancheAbsorbed {
                max: MAX_TRANCHE_ABSORBED,
            });
        }
        let mut previous: Option<LotId> = None;
        for lot in &absorbed {
            if previous.is_some_and(|previous| previous >= *lot) {
                return Err(InventoryRecordError::NonCanonicalParents);
            }
            previous = Some(*lot);
        }
        if inherited.len() > MAX_TRANCHE_INHERITED {
            return Err(InventoryRecordError::TrancheInherited {
                max: MAX_TRANCHE_INHERITED,
            });
        }
        let mut previous: Option<TrancheId> = None;
        for root in &inherited {
            if previous.is_some_and(|previous| previous >= *root) {
                return Err(InventoryRecordError::NonCanonicalParents);
            }
            if *root == id {
                return Err(InventoryRecordError::LineageCycle);
            }
            previous = Some(*root);
        }
        if predecessor == Some(id) {
            return Err(InventoryRecordError::LineageCycle);
        }
        if generation > MAX_TRANCHE_GENERATIONS {
            return Err(InventoryRecordError::TrancheGeneration {
                max: MAX_TRANCHE_GENERATIONS,
            });
        }
        if quantity == 0 {
            return Err(InventoryRecordError::ZeroQuantity);
        }
        if remaining > quantity {
            return Err(InventoryRecordError::TrancheRemaining);
        }
        if depth > MAX_LINEAGE_DEPTH {
            return Err(InventoryRecordError::LineageDepth {
                max: MAX_LINEAGE_DEPTH,
            });
        }
        Ok(Self {
            id,
            endpoint,
            commodity,
            generation,
            predecessor,
            inherited,
            absorbed,
            quantity,
            remaining,
            depth,
        })
    }

    /// Identity.
    #[must_use]
    pub const fn id(&self) -> TrancheId {
        self.id
    }

    /// Where it pooled.
    #[must_use]
    pub const fn endpoint(&self) -> EndpointRef {
        self.endpoint
    }

    /// What it pooled.
    #[must_use]
    pub const fn commodity(&self) -> &CanonicalId {
        &self.commodity
    }

    /// Every lot it absorbed. **Never shortened.**
    #[must_use]
    pub fn absorbed(&self) -> &[LotId] {
        &self.absorbed
    }

    /// How much went in, cumulatively.
    #[must_use]
    pub const fn quantity(&self) -> u64 {
        self.quantity
    }

    /// How much of that a `crystallize` may still claim descent from.
    ///
    /// E7c2 fold C7: a lineage claim is bounded by the matter that established
    /// it. `dissolve` and an inbound `transfer` add to it; `crystallize` draws
    /// it down. One dissolved unit licenses exactly one minted unit.
    #[must_use]
    pub const fn remaining(&self) -> u64 {
        self.remaining
    }

    /// Which generation of this `(endpoint, commodity)` root it is.
    #[must_use]
    pub const fn generation(&self) -> u32 {
        self.generation
    }

    /// The full predecessor generation this one succeeded, if any.
    #[must_use]
    pub const fn predecessor(&self) -> Option<TrancheId> {
        self.predecessor
    }

    /// Foreign roots whose matter arrived here by transfer. **Never shortened.**
    #[must_use]
    pub fn inherited(&self) -> &[TrancheId] {
        &self.inherited
    }

    /// The deepest lineage it absorbed, so a crystallized child is one deeper.
    #[must_use]
    pub const fn depth(&self) -> u32 {
        self.depth
    }

    /// Absorb one more lot. **Append-only**: nothing already recorded is lost.
    ///
    /// # Errors
    /// [`InventoryRecordError`] on a duplicate lot, the absorbed cap, overflow
    /// or the depth cap.
    pub fn checked_absorb(
        &self,
        lot: LotId,
        quantity: u64,
        depth: u32,
    ) -> Result<Self, InventoryRecordError> {
        if self.absorbed.contains(&lot) {
            return Err(InventoryRecordError::DuplicateParent);
        }
        let mut absorbed = self.absorbed.clone();
        absorbed.push(lot);
        absorbed.sort_unstable();
        Self::from_parts(
            self.id,
            self.endpoint,
            self.commodity.clone(),
            self.generation,
            self.predecessor,
            self.inherited.clone(),
            absorbed,
            self.quantity
                .checked_add(quantity)
                .ok_or(InventoryRecordError::QuantityOverflow)?,
            self.remaining
                .checked_add(quantity)
                .ok_or(InventoryRecordError::QuantityOverflow)?,
            self.depth.max(depth),
        )
    }

    /// Record one foreign root whose matter arrived here, and the quantity it
    /// brought. **Append-only and deduplicated**, exactly like `absorbed`.
    ///
    /// # Errors
    /// [`InventoryRecordError::TrancheInherited`] past
    /// [`MAX_TRANCHE_INHERITED`], or [`InventoryRecordError::QuantityOverflow`].
    pub fn checked_inherit(
        &self,
        root: TrancheId,
        quantity: u64,
    ) -> Result<Self, InventoryRecordError> {
        let mut inherited = self.inherited.clone();
        if !inherited.contains(&root) {
            if inherited.len() >= MAX_TRANCHE_INHERITED {
                return Err(InventoryRecordError::TrancheInherited {
                    max: MAX_TRANCHE_INHERITED,
                });
            }
            inherited.push(root);
            inherited.sort_unstable();
        }
        Self::from_parts(
            self.id,
            self.endpoint,
            self.commodity.clone(),
            self.generation,
            self.predecessor,
            inherited,
            self.absorbed.clone(),
            self.quantity
                .checked_add(quantity)
                .ok_or(InventoryRecordError::QuantityOverflow)?,
            self.remaining
                .checked_add(quantity)
                .ok_or(InventoryRecordError::QuantityOverflow)?,
            self.depth,
        )
    }

    /// Draw one mint's worth of lineage claim off this root.
    ///
    /// # Errors
    /// [`InventoryRecordError::TrancheDraw`] when the root never held that much.
    pub fn checked_draw(&self, amount: u64) -> Result<Self, InventoryRecordError> {
        let remaining =
            self.remaining
                .checked_sub(amount)
                .ok_or(InventoryRecordError::TrancheDraw {
                    remaining: self.remaining,
                    requested: amount,
                })?;
        Self::from_parts(
            self.id,
            self.endpoint,
            self.commodity.clone(),
            self.generation,
            self.predecessor,
            self.inherited.clone(),
            self.absorbed.clone(),
            self.quantity,
            remaining,
            self.depth,
        )
    }
}

// ---- E7c2 fold B8: the per-endpoint census the published bounds are counted on ----

/// How many distinct commodities one endpoint pools.
///
/// §7 publishes [`MAX_POOLS_PER_ENDPOINT`] as a shipped bound and, before the
/// E7c2 fold, **nothing anywhere counted an endpoint's pools** — the constant was
/// declared, re-exported twice and enforced nowhere. `ActionTxn` has no scan, so
/// the count that enforces it is a record written **in the same bundle** as the
/// pool it counts: never a walk, and never an unenforced constant.
///
/// It is a separate family from [`LotCensus`] for a mechanical reason: one
/// `ActionTxn` may mutate one state record only once, and `dissolve` both creates
/// a pool and consumes a lot.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct PoolCensus(u32);

#[derive(Deserialize)]
#[serde(transparent)]
struct PoolCensusWire(u32);

impl<'de> Deserialize<'de> for PoolCensus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = PoolCensusWire::deserialize(deserializer)?;
        Self::new(wire.0).map_err(serde::de::Error::custom)
    }
}

impl PoolCensus {
    /// Build a pool count.
    ///
    /// # Errors
    /// [`InventoryRecordError::EndpointPools`] past [`MAX_POOLS_PER_ENDPOINT`].
    pub const fn new(pools: u32) -> Result<Self, InventoryRecordError> {
        if pools as usize > MAX_POOLS_PER_ENDPOINT {
            return Err(InventoryRecordError::EndpointPools {
                max: MAX_POOLS_PER_ENDPOINT,
            });
        }
        Ok(Self(pools))
    }

    /// The count.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// One more pool. **Refuses at [`MAX_POOLS_PER_ENDPOINT`]**, on the create
    /// path only — crediting one of the existing pools counts nothing.
    ///
    /// # Errors
    /// [`InventoryRecordError::EndpointPools`] past the cap.
    pub const fn checked_add(self) -> Result<Self, InventoryRecordError> {
        self.checked_add_n(1)
    }

    /// `n` more pools at once (doc 20 cut SM.6, lead ruling 5): the count one
    /// bundle that opens several new pools at one endpoint stages **once**,
    /// because an `ActionTxn` mutates one census key once
    /// ([`InventoryFamilies::transfer_many`](crate::InventoryFamilies::transfer_many)).
    /// **Refuses past [`MAX_POOLS_PER_ENDPOINT`]**.
    ///
    /// # Errors
    /// [`InventoryRecordError::EndpointPools`] past the cap.
    pub const fn checked_add_n(self, n: u32) -> Result<Self, InventoryRecordError> {
        let Some(pools) = self.0.checked_add(n) else {
            return Err(InventoryRecordError::EndpointPools {
                max: MAX_POOLS_PER_ENDPOINT,
            });
        };
        Self::new(pools)
    }
}

/// How many live lot records one endpoint holds ([`MAX_LOTS_PER_ENDPOINT`]).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct LotCensus(u32);

#[derive(Deserialize)]
#[serde(transparent)]
struct LotCensusWire(u32);

impl<'de> Deserialize<'de> for LotCensus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = LotCensusWire::deserialize(deserializer)?;
        Self::new(wire.0).map_err(serde::de::Error::custom)
    }
}

impl LotCensus {
    /// Build a lot count.
    ///
    /// # Errors
    /// [`InventoryRecordError::EndpointLots`] past [`MAX_LOTS_PER_ENDPOINT`].
    pub const fn new(lots: u32) -> Result<Self, InventoryRecordError> {
        if lots > MAX_LOTS_PER_ENDPOINT {
            return Err(InventoryRecordError::EndpointLots {
                max: MAX_LOTS_PER_ENDPOINT,
            });
        }
        Ok(Self(lots))
    }

    /// The count.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// `delta` more live lots here (negative to consume).
    ///
    /// A census is a **bound**, not a ledger: a count that would go negative
    /// (a lot seeded outside the exported verbs, then consumed through them)
    /// clamps at zero rather than refusing an otherwise legal act.
    ///
    /// # Errors
    /// [`InventoryRecordError::EndpointLots`] past the cap.
    pub const fn checked_move(self, delta: i64) -> Result<Self, InventoryRecordError> {
        let mut lots = self.0 as i64 + delta;
        if lots < 0 {
            lots = 0;
        }
        if lots > MAX_LOTS_PER_ENDPOINT as i64 {
            return Err(InventoryRecordError::EndpointLots {
                max: MAX_LOTS_PER_ENDPOINT,
            });
        }
        Self::new(lots as u32)
    }
}

// ---- E7c2: Pool — a projection, never a third stored record (OPEN 2) ----

/// §3.2's `Pool`, as a **projection** of the `(inventory.pool, inventory.book)`
/// pair.
///
/// `ReservationBook::checked_apply_resource` returns `(book, BoundedScalar)` and
/// the *owner* persists both; the book keeps only `resource_revision`. Storing a
/// third `Pool { quantity, revision }` record beside them would be a second
/// quantity authority — exactly what Q13A exists to prevent — so this type is
/// built on demand and never written.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Pool {
    /// Where.
    pub endpoint: EndpointRef,
    /// What.
    pub commodity: CanonicalId,
    /// The scalar's `current`.
    pub quantity: u64,
    /// The scalar's `revision`.
    pub revision: u64,
    /// Balance minus every live hold.
    pub available: u64,
    /// How many live holds there are.
    pub holds: usize,
}

// ---- E7c2: the applied marker (OPEN 5, §10) ----

/// The committed bundle's own record that one `inventory.*` fact happened.
///
/// §10's E4c finding is the reason this exists: `jobs.resolved` read a *status* a
/// staffer could set by hand. An inventory fact must name the moment the domain
/// journal recorded the effect, so it is written **inside the authoritative
/// bundle** by the provider that did the work, and a bundle that did not commit
/// leaves no marker at all — the fact then answers `None`, which is unsatisfied.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AppliedMarker {
    kind: SmolStr,
    from: Option<EndpointRef>,
    to: Option<EndpointRef>,
    commodity: CanonicalId,
    quantity: u64,
    actor: ObjId,
    at: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AppliedMarkerWire {
    kind: SmolStr,
    from: Option<EndpointRef>,
    to: Option<EndpointRef>,
    commodity: CanonicalId,
    quantity: u64,
    actor: ObjId,
    at: i64,
}

impl<'de> Deserialize<'de> for AppliedMarker {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = AppliedMarkerWire::deserialize(deserializer)?;
        Self::from_parts(
            wire.kind,
            wire.from,
            wire.to,
            wire.commodity,
            wire.quantity,
            wire.actor,
            wire.at,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// `inventory.transferred { from, to, commodity, quantity }` (§10, D-E7-30).
pub const FACT_TRANSFERRED: &str = "inventory.transferred";
/// `inventory.lot.created` (§10, D-E7-30).
pub const FACT_LOT_CREATED: &str = "inventory.lot.created";
/// `inventory.salvage.recovered { from, to, commodity, quantity }` — the **third**
/// kind (E7 residue pass 2, P4; 10b §15.3 item 26, OPEN 5 taken).
///
/// A **kind, never a namespace**: `EconomyModule::with_facts` stays at five
/// namespaces and a duplicate panics at composition. `tactics.salvage.recovered` is
/// decided never to exist (10b:1714) and is not added.
///
/// Item 26's only stated blocker was §15.1 item 4 — *"nothing writes it"* — and P2's
/// salvage bundle is the producer that lifts it. Like its two siblings it is answered
/// from the applied marker the committed bundle itself wrote and **never** from a
/// status: a recovery that merely started leaves no marker, so the fact answers
/// unsatisfied and no settlement pays early.
pub const FACT_SALVAGE_RECOVERED: &str = "inventory.salvage.recovered";

impl AppliedMarker {
    /// Build a marker.
    ///
    /// # Errors
    /// [`InventoryRecordError::UnknownFactKind`] for a kind outside the **three**
    /// this crate owns (E7r2 P4 added [`FACT_SALVAGE_RECOVERED`]), or
    /// [`InventoryRecordError::ZeroQuantity`].
    pub fn new(
        kind: &str,
        from: Option<EndpointRef>,
        to: Option<EndpointRef>,
        commodity: CanonicalId,
        quantity: u64,
        actor: ObjId,
        at: i64,
    ) -> Result<Self, InventoryRecordError> {
        Self::from_parts(SmolStr::new(kind), from, to, commodity, quantity, actor, at)
    }

    fn from_parts(
        kind: SmolStr,
        from: Option<EndpointRef>,
        to: Option<EndpointRef>,
        commodity: CanonicalId,
        quantity: u64,
        actor: ObjId,
        at: i64,
    ) -> Result<Self, InventoryRecordError> {
        // E7r2 P4: the refusal is still a closed list and still the one place the
        // vocabulary lives — it gained one entry and no structure. A hand-edited
        // document naming a fourth kind is refused **on load**, never truncated.
        if kind != FACT_TRANSFERRED && kind != FACT_LOT_CREATED && kind != FACT_SALVAGE_RECOVERED {
            return Err(InventoryRecordError::UnknownFactKind);
        }
        if quantity == 0 {
            return Err(InventoryRecordError::ZeroQuantity);
        }
        Ok(Self {
            kind,
            from,
            to,
            commodity,
            quantity,
            actor,
            at,
        })
    }

    /// Which of this crate's three facts it records.
    #[must_use]
    pub fn kind(&self) -> &str {
        self.kind.as_str()
    }

    /// Source endpoint, when the fact has one.
    #[must_use]
    pub const fn from(&self) -> Option<EndpointRef> {
        self.from
    }

    /// Destination endpoint, when the fact has one.
    #[must_use]
    pub const fn to(&self) -> Option<EndpointRef> {
        self.to
    }

    /// The commodity moved or minted.
    #[must_use]
    pub const fn commodity(&self) -> &CanonicalId {
        &self.commodity
    }

    /// How much.
    #[must_use]
    pub const fn quantity(&self) -> u64 {
        self.quantity
    }

    /// Who acted.
    #[must_use]
    pub const fn actor(&self) -> ObjId {
        self.actor
    }

    /// When, in IC seconds on the world timeline.
    #[must_use]
    pub const fn at(&self) -> i64 {
        self.at
    }
}

// ---- Doc 18 §9 risk 19, cut IC14 row 1: the marker ring ----

/// One applied marker a source endpoint wrote (doc 18 cut IC14, row 1): the
/// marker id and the **real** second its offer was drafted (the offer's
/// `"wall"` fixed parameter, so two replays of one request stamp it
/// identically).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RingEntry {
    /// The applied-marker id (the `inventory.applied` key).
    pub key: SmolStr,
    /// The real (OOC) second the writing offer was drafted.
    pub wall: i64,
}

/// The applied-marker ids one endpoint wrote as a marker's source, oldest
/// first (doc 18 §9 risk 19, cut IC14 row 1). The shape is
/// `mush-industry`'s `MarkerRing`; its count-only replay argument is **not**
/// borrowed (E7c2 owner decision D1: the inventory marker is the only replay
/// guard that survives a reboot), so [`MarkerRing::push`] evicts only past
/// [`MAX_RETAINED_MARKERS_PER_ENDPOINT`] **and** past
/// [`MARKER_RETENTION_SECS`] of real age, and a ring of
/// [`MAX_MARKER_RING`] young entries refuses rather than evicts.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct MarkerRing(Vec<RingEntry>);

#[derive(Deserialize)]
#[serde(transparent)]
struct MarkerRingWire(Vec<RingEntry>);

impl<'de> Deserialize<'de> for MarkerRing {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = MarkerRingWire::deserialize(deserializer)?;
        Self::new(wire.0).map_err(serde::de::Error::custom)
    }
}

impl MarkerRing {
    /// Build a ring from its entries, oldest first.
    ///
    /// # Errors
    /// [`InventoryRecordError::MarkerRingFull`] past [`MAX_MARKER_RING`],
    /// [`InventoryRecordError::MarkerRingKey`] for a key failing the
    /// applied-marker key rule, [`InventoryRecordError::MarkerRingDuplicate`]
    /// for a key listed twice.
    pub fn new(entries: Vec<RingEntry>) -> Result<Self, InventoryRecordError> {
        if entries.len() > MAX_MARKER_RING {
            return Err(InventoryRecordError::MarkerRingFull {
                max: MAX_MARKER_RING,
            });
        }
        let mut seen = std::collections::BTreeSet::new();
        for entry in &entries {
            if !crate::pool::is_marker_id(&entry.key) {
                return Err(InventoryRecordError::MarkerRingKey);
            }
            if !seen.insert(entry.key.as_str()) {
                return Err(InventoryRecordError::MarkerRingDuplicate);
            }
        }
        Ok(Self(entries))
    }

    /// An empty ring (an endpoint that has written no marker yet).
    #[must_use]
    pub const fn empty() -> Self {
        Self(Vec::new())
    }

    /// The entries, oldest first.
    #[must_use]
    pub fn entries(&self) -> &[RingEntry] {
        &self.0
    }

    /// How many entries the ring holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the ring holds nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Append `entry`, then evict aged entries from the front: while the ring
    /// holds more than [`MAX_RETAINED_MARKERS_PER_ENDPOINT`], the front entry
    /// is at least [`MARKER_RETENTION_SECS`] old at `now_wall`, and fewer than
    /// [`MAX_RING_EVICTIONS_PER_PUSH`] were removed. Returns the new ring and
    /// the evicted keys (the caller removes them from `inventory.applied` in
    /// the same bundle). A `wall` in the future (a clock stepped back) simply
    /// never ages: safe.
    ///
    /// # Errors
    /// [`InventoryRecordError::MarkerRingFull`] when, after eviction, the ring
    /// would still hold more than [`MAX_MARKER_RING`] (every entry too young
    /// to evict); [`InventoryRecordError::MarkerRingKey`] /
    /// [`InventoryRecordError::MarkerRingDuplicate`] for a bad or repeated key.
    pub fn push(
        &self,
        entry: RingEntry,
        now_wall: i64,
    ) -> Result<(Self, Vec<SmolStr>), InventoryRecordError> {
        if !crate::pool::is_marker_id(&entry.key) {
            return Err(InventoryRecordError::MarkerRingKey);
        }
        if self.0.iter().any(|e| e.key == entry.key) {
            return Err(InventoryRecordError::MarkerRingDuplicate);
        }
        let mut entries: std::collections::VecDeque<RingEntry> = self.0.iter().cloned().collect();
        entries.push_back(entry);
        let mut evicted = Vec::new();
        while entries.len() > MAX_RETAINED_MARKERS_PER_ENDPOINT
            && evicted.len() < MAX_RING_EVICTIONS_PER_PUSH
            && entries
                .front()
                .is_some_and(|front| front.wall.saturating_add(MARKER_RETENTION_SECS) <= now_wall)
        {
            if let Some(front) = entries.pop_front() {
                evicted.push(front.key);
            }
        }
        if entries.len() > MAX_MARKER_RING {
            return Err(InventoryRecordError::MarkerRingFull {
                max: MAX_MARKER_RING,
            });
        }
        Ok((Self(entries.into_iter().collect()), evicted))
    }
}

/// A record refused its own invariants.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum InventoryRecordError {
    /// The endpoint cannot render as canonical idempotency text.
    #[error("an inventory endpoint must render as canonical idempotency text")]
    EndpointKey,
    /// Above [`MAX_GRADE`].
    #[error("a grade is at most {max}")]
    Grade {
        /// The cap.
        max: u8,
    },
    /// Quantities are non-zero everywhere.
    #[error("an inventory quantity must be non-zero")]
    ZeroQuantity,
    /// Above [`MAX_LOT_QUANTITY`].
    #[error("a lot holds at most {max}")]
    LotQuantity {
        /// The cap.
        max: u64,
    },
    /// Revision zero is reserved.
    #[error("revision zero is reserved")]
    ZeroRevision,
    /// A record was rewritten from a stale pre-image.
    #[error("inventory revision is stale: expected {expected}, actual {actual}")]
    StaleRevision {
        /// What the caller believed.
        expected: u64,
        /// What is stored.
        actual: u64,
    },
    /// The revision counter is exhausted.
    #[error("inventory revision is exhausted")]
    RevisionExhausted,
    /// Outside `1..=`[`MAX_LOT_PARENTS`].
    #[error("a merge names 1 to {max} parents")]
    LotParents {
        /// The cap.
        max: usize,
    },
    /// Parents and absorbed sets are strictly ordered and duplicate-free.
    #[error("lineage parents must be strictly ordered and duplicate-free")]
    NonCanonicalParents,
    /// A tranche already absorbed that lot.
    #[error("that lot was already absorbed")]
    DuplicateParent,
    /// Outside `1..=`[`MAX_TRANCHE_ABSORBED`].
    #[error("a tranche absorbs 1 to {max} lots")]
    TrancheAbsorbed {
        /// The cap.
        max: usize,
    },
    /// Past [`MAX_TRANCHE_INHERITED`].
    #[error("a tranche inherits at most {max} foreign roots")]
    TrancheInherited {
        /// The cap.
        max: usize,
    },
    /// Past [`MAX_TRANCHE_GENERATIONS`].
    #[error("that pool's lineage root has rolled {max} times")]
    TrancheGeneration {
        /// The cap.
        max: u32,
    },
    /// `remaining` may never exceed the matter that established the root.
    #[error("a tranche cannot have more left to claim than it absorbed")]
    TrancheRemaining,
    /// A mint asked for more lineage than the root ever held.
    #[error("that lineage root has {remaining} left and {requested} was asked for")]
    TrancheDraw {
        /// What the root may still license.
        remaining: u64,
        /// What was asked for.
        requested: u64,
    },
    /// Past [`MAX_POOLS_PER_ENDPOINT`].
    #[error("an endpoint pools at most {max} distinct commodities")]
    EndpointPools {
        /// The cap.
        max: usize,
    },
    /// Past [`MAX_LOTS_PER_ENDPOINT`].
    #[error("an endpoint holds at most {max} lots")]
    EndpointLots {
        /// The cap.
        max: u32,
    },
    /// Past [`MAX_LINEAGE_DEPTH`].
    #[error("lineage depth is at most {max}")]
    LineageDepth {
        /// The cap.
        max: u32,
    },
    /// A lot named itself as a parent.
    #[error("lineage is acyclic")]
    LineageCycle,
    /// Checked addition overflowed.
    #[error("inventory quantity overflowed")]
    QuantityOverflow,
    /// A marker named a fact this crate does not own.
    #[error("that is not an inventory fact")]
    UnknownFactKind,
    /// A marker ring past [`MAX_MARKER_RING`] (doc 18 cut IC14, row 1).
    #[error("an endpoint's marker ring holds at most {max} entries")]
    MarkerRingFull {
        /// The cap.
        max: usize,
    },
    /// A marker ring entry whose key fails the applied-marker key rule.
    #[error("a marker ring entry names an applied-marker key")]
    MarkerRingKey,
    /// A marker ring listing one key twice.
    #[error("a marker ring lists each key once")]
    MarkerRingDuplicate,
}

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
/// Largest quality grade. `0` is legal (slag is still matter).
pub const MAX_GRADE: u8 = 100;
/// Page size for every `@inv` listing, counted on records **visited**.
pub const INV_PAGE: usize = 32;
/// Largest applied-marker mirror one world keeps in module state.
pub const MAX_APPLIED_MARKERS: usize = 4_096;
/// Largest bytes one applied-marker id may occupy.
pub const MAX_MARKER_ID_BYTES: usize = 96;

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
    absorbed: Vec<LotId>,
    quantity: u64,
    depth: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PoolTrancheWire {
    id: TrancheId,
    endpoint: EndpointRef,
    commodity: CanonicalId,
    absorbed: Vec<LotId>,
    quantity: u64,
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
            wire.absorbed,
            wire.quantity,
            wire.depth,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl PoolTranche {
    /// Build a tranche.
    ///
    /// # Errors
    /// [`InventoryRecordError`] for an empty, oversized or non-canonical
    /// absorbed set, a zero quantity, or a depth past [`MAX_LINEAGE_DEPTH`].
    pub fn new(
        id: TrancheId,
        endpoint: EndpointRef,
        commodity: CanonicalId,
        absorbed: Vec<LotId>,
        quantity: u64,
        depth: u32,
    ) -> Result<Self, InventoryRecordError> {
        Self::from_parts(id, endpoint, commodity, absorbed, quantity, depth)
    }

    fn from_parts(
        id: TrancheId,
        endpoint: EndpointRef,
        commodity: CanonicalId,
        absorbed: Vec<LotId>,
        quantity: u64,
        depth: u32,
    ) -> Result<Self, InventoryRecordError> {
        if absorbed.is_empty() || absorbed.len() > MAX_TRANCHE_ABSORBED {
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
        if quantity == 0 {
            return Err(InventoryRecordError::ZeroQuantity);
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
            absorbed,
            quantity,
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
            absorbed,
            self.quantity
                .checked_add(quantity)
                .ok_or(InventoryRecordError::QuantityOverflow)?,
            self.depth.max(depth),
        )
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

impl AppliedMarker {
    /// Build a marker.
    ///
    /// # Errors
    /// [`InventoryRecordError::UnknownFactKind`] for a kind outside the two this
    /// crate owns, or [`InventoryRecordError::ZeroQuantity`].
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
        if kind != FACT_TRANSFERRED && kind != FACT_LOT_CREATED {
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

    /// Which of this crate's two facts it records.
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
}

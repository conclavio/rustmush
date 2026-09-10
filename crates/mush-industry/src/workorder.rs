//! E7c4 B2/B3 — the recipe reference, the pinned lines, the recipe authority
//! seam, and the work order + production schedule state machine.
//!
//! # Authoritative, not exempt (contradiction (iii))
//!
//! §4.1's cut-4 row read "exempt, legacy-post-image" for `WorkOrder`,
//! `ProductionSchedule` and `FacilityRecord`. Only the last two of those three —
//! the **facility definition half** ([`crate::facility::FacilityRecord`]) and the
//! recipe **pack record** — stay exempt. A `WorkOrder`'s state transition **gates
//! action legality** (a `Running` order holds a capacity slot a competitor cannot
//! take) and its `reservations`/`holds` lists are a denormalized index of
//! authoritative records, so [`WorkOrder`] and [`ProductionSchedule`] are
//! **authoritative typed state** (family `industry.workorder`, prepared path) —
//! the same amendment cut 3 made for `OreDeposit`. §4.1's cut-4 row and §4.0's
//! log are amended in place.
//!
//! # The recipe reaches this crate by composition, not by an edge
//!
//! A [`WorkOrder`] carries `recipe: RecipeRef` **plus** the [`PinnedRecipe`] lines
//! resolved at draft. One `Arc<dyn RecipeAuthority>` handed to `IndustryModule` at
//! the composition root answers `digest_of` from the ruleset's published pack
//! digest, so `mush-industry` gains **no** `ruleset-tacsim` edge, holds **no**
//! recipe of its own, and a retired recipe version "fails closed" as one `Option`
//! returning `None` (D-E7-24, Q18A).

use std::collections::BTreeSet;

use mush_core::{
    CanonicalHash, CanonicalId, ClockStamp, CommitToken, DefinitionDigest, IdempotencyKey,
    IdempotencyKeyText, IdempotencyScope, ObjId, PrincipalId as CorePrincipalId, ReservationId,
    ScheduleId, ScopedIdempotencyKey, World,
};
use mush_economy::{AccountId, ApprovalRequestId, EconomicVisibility, HoldId};
use mush_inventory::EndpointRef;
use mush_org::PrincipalId;
use serde::{Deserialize, Serialize};
use serde_json::json;
use smol_str::SmolStr;

use crate::families::IndustryError;
use crate::record::IndustryRecordError;

/// §7 (superseded in place): material input lines one work order may name. `10`
/// so `10 material + 1 capacity + 1 destination = 12 == MAX_RESERVATIONS_PER_ORDER`
/// is reachable with headroom, leaving `MAX_RECIPE_LINES` (16) as the pack cap
/// (contradiction (vii): §7's `MAX_WORK_ORDER_INPUTS` 16 against
/// `MAX_RESERVATIONS_PER_ORDER` 12 made three checks dead).
pub const MAX_WORK_ORDER_INPUTS: usize = 10;
/// §7: output lines one work order may name (the pack cap).
pub const MAX_WORK_ORDER_OUTPUTS: usize = 16;
/// §7 (cited, not redefined): reservations one order may hold at once.
pub const MAX_RESERVATIONS_PER_ORDER: usize = 12;
/// §7: recipe capabilities a work order may pin.
pub const MAX_WORK_ORDER_REQUIRES: usize = 16;
/// E7c4 C2/D2 (m2, independent verify fold): non-terminal work orders one
/// requester may hold at once. Terminal orders are never pruned (the family grows
/// one record per draft for ever), so without this cap a single actor could grow
/// `industry.workorder` without bound with cheap drafts, and hold a competitor's
/// finite facility capacity indefinitely. Bounds the requester, not the family.
pub const MAX_WORK_ORDERS_PER_REQUESTER: usize = 16;
/// Largest order quantity — inside cut 2's `MAX_POOL_QUANTITY` when multiplied
/// through a recipe line.
pub const MAX_WORK_ORDER_QUANTITY: u32 = 1_000_000;
/// Largest total work units a pinned recipe may name (matches cut 1's
/// `MAX_RECIPE_UNITS` by value).
pub const MAX_RECIPE_WORK_UNITS: u64 = 1_000_000_000;
/// IC seconds between production occurrences (a policy figure; `[tacsim.time]`
/// tunes the rate, this is the fixed cadence one work unit takes).
pub const PRODUCTION_CADENCE_SECS: i64 = 3_600;
/// Work units one production occurrence adds.
pub const PRODUCTION_UNITS_PER_OCCURRENCE: u64 = 1;
/// The production schedule's catch-up bound (inside K's `MAX_SCHEDULE_CATCH_UP`).
pub const PRODUCTION_MAX_CATCH_UP: u32 = 256;

/// One work order.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct WorkOrderId(pub u64);

impl std::fmt::Display for WorkOrderId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "wo#{:016x}", self.0)
    }
}

impl WorkOrderId {
    /// Parse the rendered form (`wo#<16 hex>`) or a bare hex id.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let body = text
            .trim()
            .strip_prefix("wo#")
            .unwrap_or_else(|| text.trim());
        (body.len() == 16)
            .then(|| u64::from_str_radix(body, 16).ok())
            .flatten()
            .map(Self)
    }
}

/// Derive one work-order identity from durable causal input — never an allocator
/// (whose retry behaviour could diverge after restart and mint a second order).
///
/// # Errors
/// [`IndustryError::Identity`] if the canonical value cannot be hashed.
pub fn derive_work_order_id(
    request: mush_core::RequestUid,
    ordinal: u32,
) -> Result<WorkOrderId, IndustryError> {
    let digest = CanonicalHash::for_canonical_value(
        "industry.id.v1",
        1,
        &json!({ "class": "workorder", "request": request, "ordinal": ordinal }),
        8 * 1024,
    )
    .map_err(|_| IndustryError::Identity)?;
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&digest.as_bytes()[..8]);
    Ok(WorkOrderId(u64::from_be_bytes(bytes)))
}

/// The **deterministic holder key** every hold and reservation of one work order
/// carries, so the terminal transition (a later, different request) can re-derive
/// exactly the reservations submit placed. `wo/{order}` inside
/// `MAX_IDEMPOTENCY_KEY_BYTES` (128), asserted once.
///
/// # Errors
/// [`IndustryError::Identity`] if the key cannot be built.
pub fn wo_holder(order: WorkOrderId) -> Result<ScopedIdempotencyKey, IndustryError> {
    let id = IdempotencyKeyText::parse_canonical(format!("wo{:016x}", order.0))
        .map_err(|_| IndustryError::Identity)?;
    let principal = CorePrincipalId::Scoped {
        owner: crate::record::owner(),
        class: CanonicalId::new("workorder").map_err(|_| IndustryError::Identity)?,
        id,
    };
    let scope = IdempotencyScope {
        principal,
        owner: crate::record::owner(),
        namespace: CanonicalId::new(crate::record::NS).map_err(|_| IndustryError::Identity)?,
    };
    let key = CanonicalHash::for_canonical_value(
        "industry.wo_holder.v1",
        1,
        &json!({ "order": order }),
        1024,
    )
    .map_err(|_| IndustryError::Identity)?;
    Ok(ScopedIdempotencyKey::new(
        scope,
        0,
        IdempotencyKey::Deterministic(key),
    ))
}

/// The `wo/{order}` text form, asserted once to be inside
/// `MAX_IDEMPOTENCY_KEY_BYTES`.
#[must_use]
pub fn wo_holder_text(order: WorkOrderId) -> String {
    format!("wo/{order}")
}

/// One recipe line — commodity and per-unit count — pinned into a work order.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeLine {
    commodity: CanonicalId,
    units: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecipeLineWire {
    commodity: CanonicalId,
    units: u64,
}

impl<'de> Deserialize<'de> for RecipeLine {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = RecipeLineWire::deserialize(deserializer)?;
        Self::new(wire.commodity, wire.units).map_err(serde::de::Error::custom)
    }
}

impl RecipeLine {
    /// Build a line.
    ///
    /// # Errors
    /// [`IndustryRecordError::ZeroQuantity`] for zero units, or
    /// [`IndustryRecordError::QuantityCap`] above `MAX_RECIPE_WORK_UNITS`.
    pub fn new(commodity: CanonicalId, units: u64) -> Result<Self, IndustryRecordError> {
        if units == 0 {
            return Err(IndustryRecordError::ZeroQuantity);
        }
        if units > MAX_RECIPE_WORK_UNITS {
            return Err(IndustryRecordError::QuantityCap {
                max: MAX_WORK_ORDER_QUANTITY,
            });
        }
        Ok(Self { commodity, units })
    }

    /// The commodity.
    #[must_use]
    pub const fn commodity(&self) -> &CanonicalId {
        &self.commodity
    }
    /// Per-unit count.
    #[must_use]
    pub const fn units(&self) -> u64 {
        self.units
    }
}

/// A recipe addressed by **the pack's** digest (OPEN 2).
///
/// Cut 1 authors recipes as pack entries with no per-entry digest, and §9 q3
/// forbids a second one, so a `RecipeRef` names the pack (`CanonicalId` + version)
/// and carries **the pack's** `DefinitionDigest`, read through
/// `TacsimStore::published_digest`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeRef {
    pack: CanonicalId,
    version: u32,
    id: CanonicalId,
    digest: DefinitionDigest,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecipeRefWire {
    pack: CanonicalId,
    version: u32,
    id: CanonicalId,
    digest: DefinitionDigest,
}

impl<'de> Deserialize<'de> for RecipeRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = RecipeRefWire::deserialize(deserializer)?;
        Self::new(wire.pack, wire.version, wire.id, wire.digest).map_err(serde::de::Error::custom)
    }
}

impl RecipeRef {
    /// Build a recipe reference.
    ///
    /// # Errors
    /// [`IndustryRecordError::ZeroRevision`] for version 0.
    pub fn new(
        pack: CanonicalId,
        version: u32,
        id: CanonicalId,
        digest: DefinitionDigest,
    ) -> Result<Self, IndustryRecordError> {
        if version == 0 {
            return Err(IndustryRecordError::ZeroRevision);
        }
        Ok(Self {
            pack,
            version,
            id,
            digest,
        })
    }

    /// The pack that authors it.
    #[must_use]
    pub const fn pack(&self) -> &CanonicalId {
        &self.pack
    }
    /// The pack version.
    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }
    /// The recipe id within the pack.
    #[must_use]
    pub const fn id(&self) -> &CanonicalId {
        &self.id
    }
    /// The pack's digest, pinned at draft.
    #[must_use]
    pub const fn digest(&self) -> DefinitionDigest {
        self.digest
    }
}

/// The recipe lines a work order pins at draft.
///
/// Pinning is what makes production deterministic and the terminal transition a
/// pure function of recorded inputs: the authority's `digest_of` detects a drift,
/// but the arithmetic runs off the pin. `inputs` is bounded by
/// [`MAX_WORK_ORDER_INPUTS`] and **refused on load** past it, never truncated.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PinnedRecipe {
    inputs: Vec<RecipeLine>,
    outputs: Vec<RecipeLine>,
    requires: BTreeSet<CanonicalId>,
    work_units: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PinnedRecipeWire {
    inputs: Vec<RecipeLine>,
    outputs: Vec<RecipeLine>,
    requires: BTreeSet<CanonicalId>,
    work_units: u64,
}

impl<'de> Deserialize<'de> for PinnedRecipe {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = PinnedRecipeWire::deserialize(deserializer)?;
        Self::new(wire.inputs, wire.outputs, wire.requires, wire.work_units)
            .map_err(serde::de::Error::custom)
    }
}

impl PinnedRecipe {
    /// Build the pin.
    ///
    /// # Errors
    /// [`IndustryRecordError::WorkOrderInputs`] past [`MAX_WORK_ORDER_INPUTS`],
    /// [`IndustryRecordError::QuantityCap`] for empty/oversized outputs or work,
    /// [`IndustryRecordError::ReservationCap`] for oversized `requires`.
    pub fn new(
        inputs: Vec<RecipeLine>,
        outputs: Vec<RecipeLine>,
        requires: BTreeSet<CanonicalId>,
        work_units: u64,
    ) -> Result<Self, IndustryRecordError> {
        if inputs.len() > MAX_WORK_ORDER_INPUTS {
            return Err(IndustryRecordError::WorkOrderInputs {
                max: MAX_WORK_ORDER_INPUTS,
            });
        }
        if outputs.is_empty() || outputs.len() > MAX_WORK_ORDER_OUTPUTS {
            return Err(IndustryRecordError::QuantityCap {
                max: MAX_WORK_ORDER_QUANTITY,
            });
        }
        // C1 (independent verify fold): a recipe that mints output while consuming
        // no input is minting matter from nothing — the one thing no player may do
        // (`provider.rs`'s deposit-open reason). Refused here, so a forged
        // zero-input/inflated-output pinned cannot even be constructed, drafted, or
        // decoded off the wire.
        if inputs.is_empty() {
            return Err(IndustryRecordError::RecipeMintsFromNothing);
        }
        if requires.len() > MAX_WORK_ORDER_REQUIRES {
            return Err(IndustryRecordError::ReservationCap {
                max: MAX_WORK_ORDER_REQUIRES,
            });
        }
        if work_units == 0 || work_units > MAX_RECIPE_WORK_UNITS {
            return Err(IndustryRecordError::QuantityCap {
                max: MAX_WORK_ORDER_QUANTITY,
            });
        }
        Ok(Self {
            inputs,
            outputs,
            requires,
            work_units,
        })
    }

    /// The consumed lines.
    #[must_use]
    pub fn inputs(&self) -> &[RecipeLine] {
        &self.inputs
    }
    /// The produced lines.
    #[must_use]
    pub fn outputs(&self) -> &[RecipeLine] {
        &self.outputs
    }
    /// The facility capabilities it needs.
    #[must_use]
    pub const fn requires(&self) -> &BTreeSet<CanonicalId> {
        &self.requires
    }
    /// The total work units one run takes.
    #[must_use]
    pub const fn work_units(&self) -> u64 {
        self.work_units
    }
}

/// The composition-root seam that answers a `RecipeRef` from the ruleset's
/// published packs, **without** a `mush-industry → ruleset` edge (the impl lives
/// at the composition root, reading `TacsimStore` — which is why `digest_of`
/// takes a `&World`, a documented deviation from the brief's world-free
/// signature: `TacsimStore` is per-world module state).
pub trait RecipeAuthority: Send + Sync {
    /// The pack's current published digest for this recipe, or `None` if the pack
    /// version is not published (a **retired** version fails closed here).
    fn digest_of(&self, world: &World, recipe: &RecipeRef) -> Option<DefinitionDigest>;

    /// Resolve a recipe id in a pack to its reference and pinned lines, for a
    /// draft. `None` if the pack version is not published or the id is unknown.
    fn resolve(
        &self,
        world: &World,
        pack: &CanonicalId,
        version: u32,
        id: &CanonicalId,
    ) -> Option<(RecipeRef, PinnedRecipe)>;

    /// **What generation of the catalog [`RecipeAuthority::catalog`] would answer
    /// from** — a value that moves when, and only when, a pack publish changes the
    /// answer.
    ///
    /// It exists so the work-order draft offer can cache the hashed catalog:
    /// `project_legal_actions` runs every provider on **every** projection pass,
    /// including every authoritative command submission and every decision
    /// capture, so walking and hashing every published recipe once per projection
    /// is not affordable. The offer keys its cache on this, so the walk happens
    /// once per publish instead.
    fn catalog_revision(&self, world: &World) -> u64;

    /// **Every published recipe this world can run**, in a deterministic order
    /// (`(pack, version, id)`), each with the pinned lines a draft would carry.
    ///
    /// A retired pack version contributes nothing — the same fail-closed rule
    /// [`RecipeAuthority::resolve`] uses. Two calls against one unchanged world
    /// answer the identical sequence, because the composition root's impl reads
    /// `BTreeMap`s all the way down; the offer's cache and its byte-identical
    /// projections both rest on that.
    fn catalog(&self, world: &World) -> Vec<(RecipeRef, PinnedRecipe)>;
}

/// Where a completed unit is delivered.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DeliveryTarget {
    /// An inventory endpoint the output lot is minted or credited into.
    Endpoint {
        /// The destination endpoint (cut 2's `EndpointRef(VaultId)`).
        endpoint: EndpointRef,
    },
}

impl DeliveryTarget {
    /// The destination endpoint.
    #[must_use]
    pub const fn endpoint(&self) -> EndpointRef {
        match self {
            DeliveryTarget::Endpoint { endpoint } => *endpoint,
        }
    }
}

/// Why a running order paused (the G lens and the drift interrupts).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InterruptionCause {
    /// The world timeline opened a new generation and no restamp policy is
    /// registered — the D-E7-8 default.
    TimelineChanged,
    /// The facility's capacity revision was bumped.
    CapacityRevised,
    /// The facility's policy revision was bumped (a licence change).
    PolicyRevised,
    /// The pinned recipe drifted from the authority's published digest.
    RecipeDrifted,
    /// Facility access was lost.
    FacilityLost,
}

/// One work order's lifecycle. The transition **gates action legality**, which is
/// why the record is authoritative (contradiction (iii)).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkOrderState {
    /// Drafted; no economic effect yet.
    Draft,
    /// Above the org threshold: waiting on the existing org approval, reserving
    /// **only after** it.
    AwaitingApproval {
        /// The approval request.
        request: ApprovalRequestId,
    },
    /// Acquired (hold + reservations placed), not yet running.
    Ready,
    /// Running on a production schedule.
    Running,
    /// Paused, revalidated before resuming.
    Interrupted {
        /// Why.
        cause: InterruptionCause,
    },
    /// Produced; the output is waiting to be placed at a full destination.
    AwaitingDelivery,
    /// Completed under one commit.
    Completed {
        /// The commit that materialized the output.
        commit: CommitToken,
    },
    /// Cancelled before completion; every hold released once.
    Cancelled,
    /// Failed; every hold released once.
    Failed,
}

impl WorkOrderState {
    /// Whether this state holds live reservations/holds a competitor is denied.
    #[must_use]
    pub const fn holds_resources(&self) -> bool {
        matches!(
            self,
            WorkOrderState::Ready
                | WorkOrderState::Running
                | WorkOrderState::Interrupted { .. }
                | WorkOrderState::AwaitingDelivery
        )
    }

    /// Whether this state is terminal — no live reservation may remain (M2/M3).
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(
            self,
            WorkOrderState::Completed { .. } | WorkOrderState::Cancelled | WorkOrderState::Failed
        )
    }
}

/// The production schedule half of a running order (contradiction (v)).
///
/// It carries the DurableSchedule id, `started`, `total_units`, `progress_units`
/// and the `clock_policy`, and **reads `due` from the `DurableSchedule`** rather
/// than storing a second copy that could drift.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProductionSchedule {
    schedule: ScheduleId,
    started: ClockStamp,
    clock_policy: CanonicalId,
    total_units: u64,
    progress_units: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProductionScheduleWire {
    schedule: ScheduleId,
    started: ClockStamp,
    clock_policy: CanonicalId,
    total_units: u64,
    progress_units: u64,
}

impl<'de> Deserialize<'de> for ProductionSchedule {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = ProductionScheduleWire::deserialize(deserializer)?;
        Self::new(
            wire.schedule,
            wire.started,
            wire.clock_policy,
            wire.total_units,
            wire.progress_units,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl ProductionSchedule {
    /// Open a schedule at zero progress.
    ///
    /// # Errors
    /// [`IndustryRecordError::QuantityCap`] for zero/oversized total units or
    /// progress past the total.
    pub fn new(
        schedule: ScheduleId,
        started: ClockStamp,
        clock_policy: CanonicalId,
        total_units: u64,
        progress_units: u64,
    ) -> Result<Self, IndustryRecordError> {
        if total_units == 0 || total_units > MAX_RECIPE_WORK_UNITS {
            return Err(IndustryRecordError::QuantityCap {
                max: MAX_WORK_ORDER_QUANTITY,
            });
        }
        if progress_units > total_units {
            return Err(IndustryRecordError::QuantityCap {
                max: MAX_WORK_ORDER_QUANTITY,
            });
        }
        Ok(Self {
            schedule,
            started,
            clock_policy,
            total_units,
            progress_units,
        })
    }

    /// The DurableSchedule this order runs on — `due` is read from it.
    #[must_use]
    pub const fn schedule(&self) -> ScheduleId {
        self.schedule
    }
    /// When it started (one exact generation).
    #[must_use]
    pub const fn started(&self) -> &ClockStamp {
        &self.started
    }
    /// The catch-up/clock policy id.
    #[must_use]
    pub const fn clock_policy(&self) -> &CanonicalId {
        &self.clock_policy
    }
    /// Total work units.
    #[must_use]
    pub const fn total_units(&self) -> u64 {
        self.total_units
    }
    /// Work units done so far.
    #[must_use]
    pub const fn progress_units(&self) -> u64 {
        self.progress_units
    }
    /// Whether production is complete.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.progress_units >= self.total_units
    }

    /// The post-image of one occurrence's progress, checked and bounded by
    /// `total_units` (the G lens: progress is driven only by an occurrence).
    ///
    /// # Errors
    /// [`IndustryRecordError::QuantityCap`] on overflow.
    pub fn advanced(&self, units: u64) -> Result<Self, IndustryRecordError> {
        let progress = self
            .progress_units
            .checked_add(units)
            .ok_or(IndustryRecordError::QuantityCap {
                max: MAX_WORK_ORDER_QUANTITY,
            })?
            .min(self.total_units);
        Ok(Self {
            progress_units: progress,
            ..self.clone()
        })
    }
}

/// One work order — **authoritative** typed state (family `industry.workorder`).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkOrder {
    id: WorkOrderId,
    revision: u64,
    requester: PrincipalId,
    beneficiary: PrincipalId,
    facility: ObjId,
    // The facility **definition/policy** half, pinned at draft (a deviation from
    // §3.4's field list, with the reason): `prepare_authoritative` has no world,
    // so it cannot read the exempt facility document — the draft command reads it
    // (world available) and pins the concurrent-slot cap and the two revisions.
    // Submit revalidates the pinned revisions against the live document before it
    // reserves; a bump interrupts.
    facility_slots: u32,
    capacity_revision: u64,
    policy_revision: u64,
    recipe: RecipeRef,
    pinned: PinnedRecipe,
    quantity: u32,
    budget_account: AccountId,
    material_source: Option<EndpointRef>,
    destination: DeliveryTarget,
    holds: Vec<HoldId>,
    reservations: Vec<ReservationId>,
    schedule: Option<ProductionSchedule>,
    state: WorkOrderState,
    visibility: EconomicVisibility,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkOrderWire {
    id: WorkOrderId,
    revision: u64,
    requester: PrincipalId,
    beneficiary: PrincipalId,
    facility: ObjId,
    facility_slots: u32,
    capacity_revision: u64,
    policy_revision: u64,
    recipe: RecipeRef,
    pinned: PinnedRecipe,
    quantity: u32,
    budget_account: AccountId,
    material_source: Option<EndpointRef>,
    destination: DeliveryTarget,
    holds: Vec<HoldId>,
    reservations: Vec<ReservationId>,
    schedule: Option<ProductionSchedule>,
    state: WorkOrderState,
    visibility: EconomicVisibility,
}

impl<'de> Deserialize<'de> for WorkOrder {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = WorkOrderWire::deserialize(deserializer)?;
        Self::from_parts(
            wire.id,
            wire.revision,
            wire.requester,
            wire.beneficiary,
            wire.facility,
            wire.facility_slots,
            wire.capacity_revision,
            wire.policy_revision,
            wire.recipe,
            wire.pinned,
            wire.quantity,
            wire.budget_account,
            wire.material_source,
            wire.destination,
            wire.holds,
            wire.reservations,
            wire.schedule,
            wire.state,
            wire.visibility,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl WorkOrder {
    /// Draft one work order.
    ///
    /// # Errors
    /// [`IndustryRecordError`] for an out-of-range quantity or over-cap lines.
    #[allow(clippy::too_many_arguments)]
    pub fn draft(
        id: WorkOrderId,
        requester: PrincipalId,
        beneficiary: PrincipalId,
        facility: ObjId,
        facility_slots: u32,
        capacity_revision: u64,
        policy_revision: u64,
        recipe: RecipeRef,
        pinned: PinnedRecipe,
        quantity: u32,
        budget_account: AccountId,
        material_source: Option<EndpointRef>,
        destination: DeliveryTarget,
        visibility: EconomicVisibility,
    ) -> Result<Self, IndustryRecordError> {
        Self::from_parts(
            id,
            1,
            requester,
            beneficiary,
            facility,
            facility_slots,
            capacity_revision,
            policy_revision,
            recipe,
            pinned,
            quantity,
            budget_account,
            material_source,
            destination,
            Vec::new(),
            Vec::new(),
            None,
            WorkOrderState::Draft,
            visibility,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn from_parts(
        id: WorkOrderId,
        revision: u64,
        requester: PrincipalId,
        beneficiary: PrincipalId,
        facility: ObjId,
        facility_slots: u32,
        capacity_revision: u64,
        policy_revision: u64,
        recipe: RecipeRef,
        pinned: PinnedRecipe,
        quantity: u32,
        budget_account: AccountId,
        material_source: Option<EndpointRef>,
        destination: DeliveryTarget,
        holds: Vec<HoldId>,
        reservations: Vec<ReservationId>,
        schedule: Option<ProductionSchedule>,
        state: WorkOrderState,
        visibility: EconomicVisibility,
    ) -> Result<Self, IndustryRecordError> {
        if revision == 0 {
            return Err(IndustryRecordError::ZeroRevision);
        }
        if facility_slots == 0 || facility_slots > crate::facility::MAX_FACILITY_SLOTS {
            return Err(IndustryRecordError::FacilitySlots {
                max: crate::facility::MAX_FACILITY_SLOTS,
            });
        }
        if capacity_revision == 0 || policy_revision == 0 {
            return Err(IndustryRecordError::ZeroRevision);
        }
        if quantity == 0 || quantity > MAX_WORK_ORDER_QUANTITY {
            return Err(IndustryRecordError::QuantityCap {
                max: MAX_WORK_ORDER_QUANTITY,
            });
        }
        // A stored list over the cap is **refused on load, never truncated**.
        if reservations.len() > MAX_RESERVATIONS_PER_ORDER {
            return Err(IndustryRecordError::ReservationCap {
                max: MAX_RESERVATIONS_PER_ORDER,
            });
        }
        Ok(Self {
            id,
            revision,
            requester,
            beneficiary,
            facility,
            facility_slots,
            capacity_revision,
            policy_revision,
            recipe,
            pinned,
            quantity,
            budget_account,
            material_source,
            destination,
            holds,
            reservations,
            schedule,
            state,
            visibility,
        })
    }

    /// Identity.
    #[must_use]
    pub const fn id(&self) -> WorkOrderId {
        self.id
    }
    /// Staleness stamp.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }
    /// Who drafted/submitted it.
    #[must_use]
    pub const fn requester(&self) -> &PrincipalId {
        &self.requester
    }
    /// Who the output titles to.
    #[must_use]
    pub const fn beneficiary(&self) -> &PrincipalId {
        &self.beneficiary
    }
    /// The facility it runs at.
    #[must_use]
    pub const fn facility(&self) -> ObjId {
        self.facility
    }
    /// The facility's pinned concurrent-slot cap (read at draft; the provider
    /// reserves a slot against it without a world read).
    #[must_use]
    pub const fn facility_slots(&self) -> u32 {
        self.facility_slots
    }
    /// The facility's pinned capacity revision — a bump interrupts.
    #[must_use]
    pub const fn capacity_revision(&self) -> u64 {
        self.capacity_revision
    }
    /// The facility's pinned policy revision — a bump interrupts and re-runs the
    /// licence predicate.
    #[must_use]
    pub const fn policy_revision(&self) -> u64 {
        self.policy_revision
    }
    /// The recipe reference (revalidated against the authority).
    #[must_use]
    pub const fn recipe(&self) -> &RecipeRef {
        &self.recipe
    }
    /// The pinned lines the arithmetic runs off.
    #[must_use]
    pub const fn pinned(&self) -> &PinnedRecipe {
        &self.pinned
    }
    /// How many units.
    #[must_use]
    pub const fn quantity(&self) -> u32 {
        self.quantity
    }
    /// The account the money hold is placed on.
    #[must_use]
    pub const fn budget_account(&self) -> AccountId {
        self.budget_account
    }
    /// Where materials are drawn from.
    #[must_use]
    pub const fn material_source(&self) -> Option<EndpointRef> {
        self.material_source
    }
    /// Where the output is delivered.
    #[must_use]
    pub const fn destination(&self) -> &DeliveryTarget {
        &self.destination
    }
    /// The money holds (a denormalized index of E4 holds).
    #[must_use]
    pub fn holds(&self) -> &[HoldId] {
        &self.holds
    }
    /// The material and capacity reservations (a denormalized index).
    #[must_use]
    pub fn reservations(&self) -> &[ReservationId] {
        &self.reservations
    }
    /// The production schedule, if running.
    #[must_use]
    pub const fn schedule(&self) -> Option<&ProductionSchedule> {
        self.schedule.as_ref()
    }
    /// The lifecycle state.
    #[must_use]
    pub const fn state(&self) -> &WorkOrderState {
        &self.state
    }
    /// Who may see it.
    #[must_use]
    pub const fn visibility(&self) -> &EconomicVisibility {
        &self.visibility
    }

    /// The post-image of a state transition, with a bumped revision.
    ///
    /// # Errors
    /// [`IndustryRecordError::RevisionOverflow`] at `u64::MAX`.
    pub fn with_state(&self, state: WorkOrderState) -> Result<Self, IndustryRecordError> {
        let mut next = self.clone();
        next.state = state;
        next.revision = self.next_revision()?;
        Ok(next)
    }

    /// The post-image of an acquisition: holds, reservations and the schedule,
    /// then the state. One transition, one revision bump.
    ///
    /// # Errors
    /// [`IndustryRecordError`] on an over-cap reservation list or a revision
    /// overflow.
    pub fn acquired(
        &self,
        holds: Vec<HoldId>,
        reservations: Vec<ReservationId>,
        schedule: Option<ProductionSchedule>,
        state: WorkOrderState,
    ) -> Result<Self, IndustryRecordError> {
        if reservations.len() > MAX_RESERVATIONS_PER_ORDER {
            return Err(IndustryRecordError::ReservationCap {
                max: MAX_RESERVATIONS_PER_ORDER,
            });
        }
        let mut next = self.clone();
        next.holds = holds;
        next.reservations = reservations;
        next.schedule = schedule;
        next.state = state;
        next.revision = self.next_revision()?;
        Ok(next)
    }

    /// The post-image of one schedule advance's progress.
    ///
    /// # Errors
    /// [`IndustryRecordError`] on overflow or if there is no schedule.
    pub fn with_progress(&self, units: u64) -> Result<Self, IndustryRecordError> {
        let schedule = self
            .schedule
            .as_ref()
            .ok_or(IndustryRecordError::QuantityCap {
                max: MAX_WORK_ORDER_QUANTITY,
            })?
            .advanced(units)?;
        let mut next = self.clone();
        next.schedule = Some(schedule);
        next.revision = self.next_revision()?;
        Ok(next)
    }

    fn next_revision(&self) -> Result<u64, IndustryRecordError> {
        self.revision
            .checked_add(1)
            .ok_or(IndustryRecordError::RevisionOverflow)
    }
}

/// The schedule sidecar keyed by [`ScheduleId`] (the `HandoffJob` pattern): it
/// binds a production schedule back to its work order, so the adapter's `plan`
/// re-derives the candidate and sets the invocation's actor to the requester.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProductionJob {
    schedule: ScheduleId,
    order: WorkOrderId,
    actor: ObjId,
    units_per_occurrence: u64,
    expected_generation: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProductionJobWire {
    schedule: ScheduleId,
    order: WorkOrderId,
    actor: ObjId,
    units_per_occurrence: u64,
    expected_generation: u64,
}

impl<'de> Deserialize<'de> for ProductionJob {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = ProductionJobWire::deserialize(deserializer)?;
        Self::new(
            wire.schedule,
            wire.order,
            wire.actor,
            wire.units_per_occurrence,
            wire.expected_generation,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl ProductionJob {
    /// Build a job.
    ///
    /// # Errors
    /// [`IndustryRecordError::QuantityCap`] for zero units per occurrence.
    pub fn new(
        schedule: ScheduleId,
        order: WorkOrderId,
        actor: ObjId,
        units_per_occurrence: u64,
        expected_generation: u64,
    ) -> Result<Self, IndustryRecordError> {
        if units_per_occurrence == 0 {
            return Err(IndustryRecordError::QuantityCap {
                max: MAX_WORK_ORDER_QUANTITY,
            });
        }
        Ok(Self {
            schedule,
            order,
            actor,
            units_per_occurrence,
            expected_generation,
        })
    }

    /// The schedule.
    #[must_use]
    pub const fn schedule(&self) -> ScheduleId {
        self.schedule
    }
    /// The work order.
    #[must_use]
    pub const fn order(&self) -> WorkOrderId {
        self.order
    }
    /// The requester's object — the invocation's actor for a scheduled advance.
    #[must_use]
    pub const fn actor(&self) -> ObjId {
        self.actor
    }
    /// Units of work one occurrence adds.
    #[must_use]
    pub const fn units_per_occurrence(&self) -> u64 {
        self.units_per_occurrence
    }
    /// The schedule generation the job was created against.
    #[must_use]
    pub const fn expected_generation(&self) -> u64 {
        self.expected_generation
    }

    /// Re-run structural invariants (the family validator).
    ///
    /// # Errors
    /// [`String`] on a zero unit count.
    pub fn validate(&self) -> Result<(), String> {
        if self.units_per_occurrence == 0 {
            Err("a production job adds a non-zero unit count per occurrence".to_owned())
        } else {
            Ok(())
        }
    }
}

/// The applied marker `industry.workorder.completed` is answered from — the
/// committed terminal bundle's own record, never a status (§10's E4c finding).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkOrderMarker {
    kind: SmolStr,
    order: WorkOrderId,
    actor: ObjId,
    at: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkOrderMarkerWire {
    kind: SmolStr,
    order: WorkOrderId,
    actor: ObjId,
    at: i64,
}

impl<'de> Deserialize<'de> for WorkOrderMarker {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = WorkOrderMarkerWire::deserialize(deserializer)?;
        Self::new(wire.kind.as_str(), wire.order, wire.actor, wire.at)
            .map_err(serde::de::Error::custom)
    }
}

impl WorkOrderMarker {
    /// Build a completion marker.
    ///
    /// # Errors
    /// [`IndustryRecordError::MarkerKind`] for any kind other than
    /// `industry.workorder.completed`.
    pub fn new(
        kind: &str,
        order: WorkOrderId,
        actor: ObjId,
        at: i64,
    ) -> Result<Self, IndustryRecordError> {
        if kind != crate::record::FACT_WORKORDER_COMPLETED {
            return Err(IndustryRecordError::MarkerKind);
        }
        Ok(Self {
            kind: SmolStr::new(kind),
            order,
            actor,
            at,
        })
    }

    /// Which fact it answers.
    #[must_use]
    pub fn kind(&self) -> &str {
        self.kind.as_str()
    }
    /// Which order.
    #[must_use]
    pub const fn order(&self) -> WorkOrderId {
        self.order
    }
    /// Who completed it.
    #[must_use]
    pub const fn actor(&self) -> ObjId {
        self.actor
    }
    /// When (IC seconds).
    #[must_use]
    pub const fn at(&self) -> i64 {
        self.at
    }
}

/// The deterministic price of a quote — a **pure function of recorded inputs**
/// (the P lens): the total input units the run consumes, so a quote is never an
/// inventory, capacity or reserve oracle and two worlds differing only in another
/// principal's holdings quote identically. Saturating, never panicking.
#[must_use]
pub fn quote_price(pinned: &PinnedRecipe, quantity: u32) -> u64 {
    pinned.inputs().iter().fold(0_u64, |total, line| {
        total.saturating_add(line.units().saturating_mul(u64::from(quantity)))
    })
}

/// The deterministic duration of a quote in work units — `work_units × quantity`,
/// a pure function of the pinned recipe.
#[must_use]
pub fn quote_duration(pinned: &PinnedRecipe, quantity: u32) -> u64 {
    pinned.work_units().saturating_mul(u64::from(quantity))
}

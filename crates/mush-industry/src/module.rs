//! E7c3 B1/B7 — the module: six typed families, one custody provider, one fact
//! namespace, five command lanes, one sweep.
//!
//! **One `OwnerId`.** Cut 4's facilities, recipes, work orders and capacity land
//! on this same module and this same owner; cut 3 registers **no**
//! `AuthorityGuardKind::Reservation` validator at all, because it holds against
//! cut 2's inventory book under cut 2's validator. Cut 4's capacity book is the
//! one that registers `mush-industry`'s, and the registry is keyed
//! `(OwnerId, AuthorityGuardKind)`, so that registration is legal by
//! construction rather than by luck (cut 2 pinned exactly that).

use std::sync::Arc;
use std::time::Duration;

use mush_core::{
    AuthorityGuardKind, BoundedScalar, CanonicalId, DurableSchedule, DurableScheduleStateCodec,
    GameModule, ModError, ModuleBootData, ModuleCtx, ObjId, Registrar, ReservationBook,
    ReservationBookId, ReservationBookStateCodec, ReservationId, ScheduleId, StateFamilyDescriptor,
    StateFamilyId, TickCadence, durable_schedule_state_family_descriptor,
    reservation_book_state_family_descriptor,
};
use mush_economy::InstrumentId;
use serde_json::Value;
use smol_str::SmolStr;

use crate::facility::{CapacityGuardValidator, MAX_FACILITY_SLOTS};
use crate::families::IndustryFamilies;
use crate::record::{
    AppliedMarker, ClaimId, ClaimIndex, ClaimRecord, ExtractionRun, MAX_MARKER_ID_BYTES, NS,
    OreDeposit, OreDepositId, PlaceCensus, TenureRecord,
};
use crate::store::IndustryStore;

/// The coarse reconciliation cadence. It exists for one reason: the fact mirror
/// must catch up with the authoritative markers even in a world where nobody
/// types `@deposit`.
pub const SWEEP_TAG: &str = "industry.sweep";
/// Seconds between sweeps.
pub const SWEEP_PERIOD_SECS: u64 = 60;

/// The default catalog coordinate a claim's titled object carries.
///
/// A `CatalogRef` is *opaque to the economy* and is two strings; naming the
/// corridor's ruleset here is a **coordinate**, not a dependency — this crate
/// has no `tacsim` edge and never reads a pack. A deployment that wants
/// another coordinate passes one to [`IndustryModule::with_claim_catalog`].
pub const CLAIM_CATALOG_RULESET: &str = "tacsim";
/// The catalog code a claim's titled object carries.
pub const CLAIM_CATALOG_CODE: &str = "claim";

/// The system principal this crate issues claim titles under.
///
/// It must be named in **two** places that have to agree — here and
/// `EconomyModule::with_title_issuers` — so a typo is a loud refusal at the
/// verb rather than a silent capability (the housing/market precedent).
///
/// # Panics
/// Never: `industry.claims` is a canonical system principal id.
#[must_use]
pub fn industry_title_issuer() -> mush_org::SystemPrincipalId {
    mush_org::SystemPrincipalId::new("industry.claims")
        .expect("static industry title issuer id is well-formed")
}

/// The crate's family handles, published in module state.
///
/// A handle is a capability, not authority: writing one still needs an
/// `ActionTxn` this owner may stamp. Publishing them is what lets cut 4's work
/// orders call the exported seams without owning the registration — the same
/// shape cut 2 published for this cut.
#[derive(Clone, Debug)]
pub struct IndustryHandles(pub IndustryFamilies);

/// The neutral industry authority (doc 10 §14, D-E7-3).
#[derive(Default)]
pub struct IndustryModule {
    families: Option<IndustryFamilies>,
    licence: Option<InstrumentId>,
    catalog: Option<(SmolStr, SmolStr)>,
    inventory: Option<mush_inventory::InventoryHandleCell>,
    recipes: Option<Arc<dyn crate::workorder::RecipeAuthority>>,
    core_origins: Option<Box<crate::core_production::CoreOriginConfig>>,
    core_origin_runtime: Option<Box<crate::core_production::Runtime>>,
    core_factory: Option<Box<crate::core_production::integration::CoreFactoryConfig>>,
    /// Doc 20 cut SM.6: the composition root's cell this module fills.
    cell: Option<crate::seams::IndustryHandleCell>,
    /// Doc 20 cut SM.6 (§3.4): the composed system opener.
    opener: Option<Arc<dyn crate::seams::SystemOpener>>,
    /// Doc 20 cut SM.6 (§3.4): the composed external-draws answer.
    draws: Option<Arc<dyn crate::seams::ExternalDraws>>,
    /// Doc 20 cut SM.6 (§9.6): the composed jurisdiction row.
    jurisdiction: Option<Arc<dyn crate::seams::JurisdictionAuthority>>,
    /// Doc 20 cut SM.10 (§9, D-20-12): the composed tenure authority.
    tenure: Option<Arc<dyn crate::seams::TenureAuthority>>,
    /// Doc 20 cut SM.6: the composed owners this crate delegates its typed
    /// families to.
    delegates: Vec<SmolStr>,
}

impl std::fmt::Debug for IndustryModule {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IndustryModule")
            .field("has_recipes", &self.recipes.is_some())
            .field("has_system_opener", &self.opener.is_some())
            .field("has_external_draws", &self.draws.is_some())
            .field("has_tenure_authority", &self.tenure.is_some())
            .field("delegates", &self.delegates)
            .finish_non_exhaustive()
    }
}

impl IndustryModule {
    /// A fresh module.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            families: None,
            licence: None,
            catalog: None,
            inventory: None,
            recipes: None,
            core_origins: None,
            core_origin_runtime: None,
            core_factory: None,
            cell: None,
            opener: None,
            draws: None,
            jurisdiction: None,
            tenure: None,
            delegates: Vec::new(),
        }
    }

    /// Explicitly enable the configured certified Origin producer. No defaults
    /// or ordinary inventory authority imply this capability.
    #[must_use]
    pub fn with_core_origins(mut self, config: crate::core_production::CoreOriginConfig) -> Self {
        self.core_origins = Some(Box::new(config));
        self
    }

    /// Enable the closed certified-input factory alongside the explicit Origin owner.
    #[must_use]
    pub fn with_core_factory(
        mut self,
        config: crate::core_production::integration::CoreFactoryConfig,
    ) -> Self {
        self.core_factory = Some(Box::new(config));
        self
    }

    /// Publish this crate's family handles into a composition-root cell (doc 20
    /// cut SM.6; `InventoryModule::with_handle_cell`'s shape): `register` fills
    /// it right after the families are built and before the providers register,
    /// so a [`SystemOpener`](crate::seams::SystemOpener) composed onto this
    /// module, and any provider composed after it, can read deposits.
    #[must_use]
    pub fn with_handle_cell(mut self, cell: crate::seams::IndustryHandleCell) -> Self {
        self.cell = Some(cell);
        self
    }

    /// Compose the system opener that admits a non-staff actor to
    /// `DepositOpen` for exactly the one server-authored opening it makes next
    /// (doc 20 §3.4, D-20-5). Unset, `DepositOpen` stays staff-only.
    #[must_use]
    pub fn with_system_opener(mut self, opener: Arc<dyn crate::seams::SystemOpener>) -> Self {
        self.opener = Some(opener);
        self
    }

    /// Compose the answer to "what has the external provider drawn" the 60 s
    /// sweep checks every externally worked body against (doc 20 §3.4). Unset,
    /// every such body is quarantined `ExternalUnwired`.
    #[must_use]
    pub fn with_external_draws(mut self, draws: Arc<dyn crate::seams::ExternalDraws>) -> Self {
        self.draws = Some(draws);
        self
    }

    /// Compose the contest offer's jurisdiction row (doc 20 §9.6, D-20-12).
    /// Unset, the module uses [`ActsForJurisdiction`](crate::seams::ActsForJurisdiction)
    /// on every roster: a deposit with no `jurisdiction` never matches it, so a
    /// roster with no such deposit sees no change.
    #[must_use]
    pub fn with_jurisdiction_authority(
        mut self,
        authority: Arc<dyn crate::seams::JurisdictionAuthority>,
    ) -> Self {
        self.jurisdiction = Some(authority);
        self
    }

    /// Compose the tenure authority (doc 20 §9, D-20-12; cut SM.10) that admits
    /// an actor to `industry_claim_tenure` for exactly the ONE server-authored
    /// [`TenureAct`](crate::seams::TenureAct) it vouches for, and stages the
    /// composing game's own record writes in the same bundle. Unset, the verb
    /// registers and offers nothing.
    #[must_use]
    pub fn with_tenure_authority(
        mut self,
        authority: Arc<dyn crate::seams::TenureAuthority>,
    ) -> Self {
        self.tenure = Some(authority);
        self
    }

    /// Name the composed owners this crate delegates its typed families to
    /// (doc 20 cut SM.6, D1; the shape of `InventoryModule::with_quantity_delegates`):
    /// a grantee may **read** this crate's deposits and claims through its own
    /// module context. A delegation also grants write inside the grantee's
    /// bundle (`Registrar::delegate_state_families`), so a grantee must reach
    /// this crate's records only through the exported seams on
    /// [`IndustryFamilies`] — tacsim's source tripwire holds it to reads. The
    /// delegation widens the bundle, never the gate.
    #[must_use]
    pub fn with_state_delegates<I, S>(mut self, delegates: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<SmolStr>,
    {
        self.delegates = delegates.into_iter().map(Into::into).collect();
        self
    }

    /// Hand this module the one recipe authority (the composition-root seam that
    /// answers a `RecipeRef` from the ruleset's published packs, `digest_of` and
    /// `resolve`). Unset, `@wo/draft` refuses: a server that has said nothing
    /// about what a recipe *is* must not let anyone pin one. Exactly the shape of
    /// `EconomyModule::{with_custody, with_facts, with_title_issuers}`.
    #[must_use]
    pub fn with_recipes(mut self, recipes: Arc<dyn crate::workorder::RecipeAuthority>) -> Self {
        self.recipes = Some(recipes);
        self
    }

    /// Take cut 2's family handles from the composition root's cell.
    ///
    /// Module state is installed after every module has registered, so the
    /// handles cannot be read from `&World` while this module is registering —
    /// and the extraction provider needs them then. The composition root fills
    /// the cell by composing `mush-inventory` first, which is the order the
    /// boot pin already fixes.
    #[must_use]
    pub fn with_inventory(mut self, cell: mush_inventory::InventoryHandleCell) -> Self {
        self.inventory = Some(cell);
        self
    }

    /// Name the `InstrumentKind::Entitlement` instrument an extraction licence
    /// is an account in.
    ///
    /// Composition-root data, like `with_title_issuers`. Unset, **every**
    /// extraction refuses: a server that has not said what a licence *is* must
    /// not let anyone extract without one, and the staff-only reason says so
    /// with the wiring named.
    #[must_use]
    pub fn with_extraction_licence(mut self, instrument: InstrumentId) -> Self {
        self.licence = Some(instrument);
        self
    }

    /// Override the catalog coordinate a claim's titled object carries.
    #[must_use]
    pub fn with_claim_catalog(mut self, ruleset: &str, code: &str) -> Self {
        self.catalog = Some((SmolStr::new(ruleset), SmolStr::new(code)));
        self
    }

    /// The catalog coordinate this composition uses.
    #[must_use]
    pub fn claim_catalog(&self) -> (SmolStr, SmolStr) {
        self.catalog.clone().unwrap_or((
            SmolStr::new_static(CLAIM_CATALOG_RULESET),
            SmolStr::new_static(CLAIM_CATALOG_CODE),
        ))
    }
}

fn family_id(name: &str) -> Result<StateFamilyId, ModError> {
    StateFamilyId::new(name).map_err(|error| ModError::msg(error.to_string()))
}

fn canonical(name: &str) -> Result<CanonicalId, ModError> {
    CanonicalId::new(name).map_err(|error| ModError::msg(error.to_string()))
}

struct ExtractionFamilies {
    deposits: mush_core::StateFamilyHandle<OreDepositId, OreDeposit>,
    claims: mush_core::StateFamilyHandle<ClaimId, ClaimRecord>,
    claim_index: mush_core::StateFamilyHandle<SmolStr, ClaimIndex>,
    runs: mush_core::StateFamilyHandle<OreDepositId, ExtractionRun>,
    applied: mush_core::StateFamilyHandle<SmolStr, AppliedMarker>,
    place_census: mush_core::StateFamilyHandle<ObjId, PlaceCensus>,
    tenure: mush_core::StateFamilyHandle<ObjId, TenureRecord>,
}
#[inline(never)]
fn register_extraction_families(reg: &mut Registrar<'_>) -> Result<ExtractionFamilies, ModError> {
    // ---- E7c3: B2 — the ore body's definition half ----
    // **Authoritative, not exempt.** §4.1's cut-3 row said "exempt,
    // legacy-post-image" for the whole record; §4.1's own closing rule
    // ("not exempt, ever: … anything that changes action legality")
    // overrides it, because `accessibility` decides whether a cycle is
    // legal at all and the curve decides its yield — and the extraction
    // provider re-reads both at prepare time under a checked revision,
    // which a conventional document cannot serve at all.
    let deposits = reg.state_family::<OreDepositId, OreDeposit, _, _, _, _>(
        StateFamilyDescriptor::new(
            family_id("industry_deposits")?,
            canonical("industry.deposit_id.v1")?,
            canonical("industry.deposit.v1")?,
            1,
            canonical("industry.deposit.decode_json.v1")?,
            canonical("industry.deposit.invariants.v1")?,
            canonical("industry.deposit.json.v1")?,
        )
        .map_err(|error| ModError::msg(error.to_string()))?,
        |key: &OreDepositId| Ok(format!("{:016x}", key.0)),
        |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
        // The wire already re-ran every constructor invariant; this is the
        // descriptor's own second look, which is what makes a hand-edited
        // document fail at BOTH gates.
        |value: &OreDeposit| {
            if value.initial_reserve() == 0 {
                Err("an ore body has a non-zero authored reserve".to_owned())
            } else {
                Ok(())
            }
        },
        |value: &OreDeposit| serde_json::to_value(value).map_err(|e| e.to_string()),
    )?;
    // ---- E7c3: B4 — tenure ----
    let claims = reg.state_family::<ClaimId, ClaimRecord, _, _, _, _>(
        StateFamilyDescriptor::new(
            family_id("industry_claims")?,
            canonical("industry.claim_id.v1")?,
            canonical("industry.claim.v1")?,
            1,
            canonical("industry.claim.decode_json.v1")?,
            canonical("industry.claim.invariants.v1")?,
            canonical("industry.claim.json.v1")?,
        )
        .map_err(|error| ModError::msg(error.to_string()))?,
        |key: &ClaimId| Ok(format!("{:016x}", key.0)),
        |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
        |value: &ClaimRecord| {
            if value.revision() == 0 {
                Err("a claim revision starts at 1".to_owned())
            } else {
                Ok(())
            }
        },
        |value: &ClaimRecord| serde_json::to_value(value).map_err(|e| e.to_string()),
    )?;
    let claim_index = reg.state_family::<SmolStr, ClaimIndex, _, _, _, _>(
        StateFamilyDescriptor::new(
            family_id("industry_claim_index")?,
            canonical("industry.subject_id.v1")?,
            canonical("industry.claim_index.v1")?,
            1,
            canonical("industry.claim_index.decode_json.v1")?,
            canonical("industry.claim_index.bounds.v1")?,
            canonical("industry.claim_index.json.v1")?,
        )
        .map_err(|error| ModError::msg(error.to_string()))?,
        |key: &SmolStr| {
            if key.is_empty() || key.len() > 40 {
                Err("a claim-subject token is 1..=40 bytes".to_owned())
            } else if key
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
            {
                Ok(key.to_string())
            } else {
                Err("a claim-subject token is lowercase alphanumeric".to_owned())
            }
        },
        |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
        |value: &ClaimIndex| {
            if value.len() > crate::record::MAX_CLAIMS_PER_SUBJECT {
                Err("a subject carries at most MAX_CLAIMS_PER_SUBJECT claims".to_owned())
            } else {
                Ok(())
            }
        },
        |value: &ClaimIndex| serde_json::to_value(value).map_err(|e| e.to_string()),
    )?;
    // ---- E7c3: B6 — the extraction run ----
    let runs = reg.state_family::<OreDepositId, ExtractionRun, _, _, _, _>(
        StateFamilyDescriptor::new(
            family_id("industry_runs")?,
            canonical("industry.deposit_id.v1")?,
            canonical("industry.run.v1")?,
            1,
            canonical("industry.run.decode_json.v1")?,
            canonical("industry.run.invariants.v1")?,
            canonical("industry.run.json.v1")?,
        )
        .map_err(|error| ModError::msg(error.to_string()))?,
        |key: &OreDepositId| Ok(format!("{:016x}", key.0)),
        |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
        |value: &ExtractionRun| {
            if value.next_cycle() > crate::record::MAX_EXTRACTION_CYCLES {
                Err("a run reaches at most MAX_EXTRACTION_CYCLES cycles".to_owned())
            } else {
                Ok(())
            }
        },
        |value: &ExtractionRun| serde_json::to_value(value).map_err(|e| e.to_string()),
    )?;
    // ---- E7c3: B7 — the applied marker the fact answers from ----
    let applied = reg.state_family::<SmolStr, AppliedMarker, _, _, _, _>(
        StateFamilyDescriptor::new(
            family_id("industry_applied")?,
            canonical("industry.marker_id.v1")?,
            canonical("industry.applied.v1")?,
            1,
            canonical("industry.applied.decode_json.v1")?,
            canonical("industry.applied.invariants.v1")?,
            canonical("industry.applied.json.v1")?,
        )
        .map_err(|error| ModError::msg(error.to_string()))?,
        |key: &SmolStr| {
            if key.is_empty() || key.len() > MAX_MARKER_ID_BYTES {
                Err("an applied-marker id is 1..=96 bytes".to_owned())
            } else if key
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
            {
                Ok(key.to_string())
            } else {
                Err("an applied-marker id is lowercase hex".to_owned())
            }
        },
        |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
        |value: &AppliedMarker| {
            if value.quantity() == 0 {
                Err("an applied marker names a non-zero quantity".to_owned())
            } else {
                Ok(())
            }
        },
        |value: &AppliedMarker| serde_json::to_value(value).map_err(|e| e.to_string()),
    )?;
    // ---- E7c3: the per-place census `MAX_ORE_DEPOSITS_PER_PLACE` is counted on ----
    let place_census = reg.state_family::<ObjId, PlaceCensus, _, _, _, _>(
        StateFamilyDescriptor::new(
            family_id("industry_place_census")?,
            canonical("obj_id.decimal.v1")?,
            canonical("industry.place_census.v1")?,
            1,
            canonical("industry.place_census.decode_json.v1")?,
            canonical("industry.place_census.bounds.v1")?,
            canonical("industry.place_census.json.v1")?,
        )
        .map_err(|error| ModError::msg(error.to_string()))?,
        |key: &ObjId| Ok(format!("{:010}", key.0)),
        |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
        |value: &PlaceCensus| {
            if value.get() as usize > crate::record::MAX_ORE_DEPOSITS_PER_PLACE {
                Err("a place carries at most MAX_ORE_DEPOSITS_PER_PLACE ore bodies".to_owned())
            } else {
                Ok(())
            }
        },
        |value: &PlaceCensus| serde_json::to_value(value).map_err(|e| e.to_string()),
    )?;
    // ---- doc 20 cut SM.6: one tenure record per block place (§9.2–§9.4) ----
    let tenure = reg.state_family::<ObjId, TenureRecord, _, _, _, _>(
        StateFamilyDescriptor::new(
            family_id("industry_tenure")?,
            canonical("obj_id.decimal.v1")?,
            canonical("industry.tenure.v1")?,
            1,
            canonical("industry.tenure.decode_json.v1")?,
            canonical("industry.tenure.bounds.v1")?,
            canonical("industry.tenure.json.v1")?,
        )
        .map_err(|error| ModError::msg(error.to_string()))?,
        |key: &ObjId| Ok(format!("{:010}", key.0)),
        |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
        // The wire already re-ran `check`; this is the descriptor's own
        // second look.
        |value: &TenureRecord| value.check().map_err(|e| e.to_string()),
        |value: &TenureRecord| serde_json::to_value(value).map_err(|e| e.to_string()),
    )?;

    Ok(ExtractionFamilies {
        deposits,
        claims,
        claim_index,
        runs,
        applied,
        place_census,
        tenure,
    })
}

struct WorkFamilies {
    facility_slots: mush_core::StateFamilyHandle<ReservationBookId, BoundedScalar>,
    facility_books: mush_core::StateFamilyHandle<ReservationBookId, ReservationBook>,
    capacity_index: mush_core::StateFamilyHandle<ReservationId, ReservationBookId>,
    work_orders:
        mush_core::StateFamilyHandle<crate::workorder::WorkOrderId, crate::workorder::WorkOrder>,
    schedules: mush_core::StateFamilyHandle<mush_core::ScheduleId, mush_core::DurableSchedule>,
    jobs: mush_core::StateFamilyHandle<mush_core::ScheduleId, crate::workorder::ProductionJob>,
    wo_applied: mush_core::StateFamilyHandle<SmolStr, crate::workorder::WorkOrderMarker>,
    marker_ring: mush_core::StateFamilyHandle<OreDepositId, crate::record::MarkerRing>,
    wo_marker_ring: mush_core::StateFamilyHandle<ObjId, crate::record::MarkerRing>,
}
#[inline(never)]
fn register_work_families(reg: &mut Registrar<'_>) -> Result<WorkFamilies, ModError> {
    // ---- E7c4: B1 — the facility capacity pair and its reservation index ----
    // `industry.facility_slots` is K's `BoundedScalar` bounded
    // `0..=maximum_slots` with `current == maximum` (its `Deserialize`
    // re-runs `BoundedScalar::new`, so the wire re-checks every bound), and
    // `industry.facility_book` is K's own descriptor and codec, so cut 4
    // defines no schema identity of its own for it — exactly cut 2's pool
    // pair, applied to capacity.
    let facility_slots = reg.state_family::<ReservationBookId, BoundedScalar, _, _, _, _>(
        StateFamilyDescriptor::new(
            family_id("industry_facility_slots")?,
            canonical("reservation_book_id.v1")?,
            canonical("industry.facility_slots.v1")?,
            1,
            canonical("industry.facility_slots.decode_json.v1")?,
            canonical("industry.facility_slots.bounds.v1")?,
            canonical("industry.facility_slots.json.v1")?,
        )
        .map_err(|error| ModError::msg(error.to_string()))?,
        |key: &ReservationBookId| Ok(key.canonical_hash().to_hex()),
        |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
        |value: &BoundedScalar| {
            if value.minimum() == 0
                && value.maximum() >= 1
                && value.maximum() <= i64::from(MAX_FACILITY_SLOTS)
                && value.current() == value.maximum()
            {
                Ok(())
            } else {
                Err(
                    "a facility-slots scalar is bounded 0..=MAX_FACILITY_SLOTS with \
                     current == maximum"
                        .to_owned(),
                )
            }
        },
        |value: &BoundedScalar| serde_json::to_value(value).map_err(|e| e.to_string()),
    )?;
    let facility_books = reg.state_family::<ReservationBookId, ReservationBook, _, _, _, _>(
        reservation_book_state_family_descriptor(family_id("industry_facility_book")?)
            .map_err(|error| ModError::msg(error.to_string()))?,
        ReservationBookStateCodec,
        ReservationBookStateCodec,
        ReservationBookStateCodec,
        ReservationBookStateCodec,
    )?;
    let capacity_index = reg.state_family::<ReservationId, ReservationBookId, _, _, _, _>(
        StateFamilyDescriptor::new(
            family_id("industry_capacity_index")?,
            canonical("industry.reservation_id.v1")?,
            canonical("industry.capacity_index.v1")?,
            1,
            canonical("industry.capacity_index.decode_json.v1")?,
            canonical("industry.capacity_index.bounds.v1")?,
            canonical("industry.capacity_index.json.v1")?,
        )
        .map_err(|error| ModError::msg(error.to_string()))?,
        |key: &ReservationId| Ok(key.0.to_hex()),
        |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
        |_value: &ReservationBookId| Ok(()),
        |value: &ReservationBookId| serde_json::to_value(value).map_err(|e| e.to_string()),
    )?;

    // ---- E7c4: B3 — the work-order family (authoritative, contradiction (iii)) ----
    let work_orders = reg
        .state_family::<crate::workorder::WorkOrderId, crate::workorder::WorkOrder, _, _, _, _>(
            StateFamilyDescriptor::new(
                family_id("industry_work_orders")?,
                canonical("industry.work_order_id.v1")?,
                canonical("industry.work_order.v1")?,
                1,
                canonical("industry.work_order.decode_json.v1")?,
                canonical("industry.work_order.invariants.v1")?,
                canonical("industry.work_order.json.v1")?,
            )
            .map_err(|error| ModError::msg(error.to_string()))?,
            |key: &crate::workorder::WorkOrderId| Ok(format!("{:016x}", key.0)),
            |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
            |value: &crate::workorder::WorkOrder| {
                if value.revision() == 0 {
                    Err("a work order revision starts at 1".to_owned())
                } else {
                    Ok(())
                }
            },
            |value: &crate::workorder::WorkOrder| {
                serde_json::to_value(value).map_err(|e| e.to_string())
            },
        )?;

    // ---- E7c4: B5 — the production schedule family and its job sidecar ----
    let schedules = reg.state_family::<ScheduleId, DurableSchedule, _, _, _, _>(
        durable_schedule_state_family_descriptor(family_id("industry_production_schedule")?)
            .map_err(|error| ModError::msg(error.to_string()))?,
        DurableScheduleStateCodec,
        DurableScheduleStateCodec,
        DurableScheduleStateCodec,
        DurableScheduleStateCodec,
    )?;
    let jobs = reg.state_family::<ScheduleId, crate::workorder::ProductionJob, _, _, _, _>(
        StateFamilyDescriptor::new(
            family_id("industry_production_job")?,
            canonical("durable_schedule_id.v1")?,
            canonical("industry.production_job.v1")?,
            1,
            canonical("industry.production_job.decode_json.v1")?,
            canonical("industry.production_job.invariants.v1")?,
            canonical("industry.production_job.json.v1")?,
        )
        .map_err(|error| ModError::msg(error.to_string()))?,
        |key: &ScheduleId| Ok(key.canonical_hash().to_hex()),
        |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
        crate::workorder::ProductionJob::validate,
        |value: &crate::workorder::ProductionJob| {
            serde_json::to_value(value).map_err(|e| e.to_string())
        },
    )?;
    // ---- E7c4: B7 — the work-order completion marker family ----
    let wo_applied = reg.state_family::<SmolStr, crate::workorder::WorkOrderMarker, _, _, _, _>(
        StateFamilyDescriptor::new(
            family_id("industry_wo_applied")?,
            canonical("industry.marker_id.v1")?,
            canonical("industry.wo_applied.v1")?,
            1,
            canonical("industry.wo_applied.decode_json.v1")?,
            canonical("industry.wo_applied.invariants.v1")?,
            canonical("industry.wo_applied.json.v1")?,
        )
        .map_err(|error| ModError::msg(error.to_string()))?,
        |key: &SmolStr| {
            if key.is_empty() || key.len() > MAX_MARKER_ID_BYTES {
                Err("an applied-marker id is 1..=96 bytes".to_owned())
            } else if key
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
            {
                Ok(key.to_string())
            } else {
                Err("an applied-marker id is lowercase hex".to_owned())
            }
        },
        |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
        |_value: &crate::workorder::WorkOrderMarker| Ok(()),
        |value: &crate::workorder::WorkOrderMarker| {
            serde_json::to_value(value).map_err(|e| e.to_string())
        },
    )?;
    // ---- E7c4: D2 discharged — the per-deposit marker ring ----
    let marker_ring = reg.state_family::<OreDepositId, crate::record::MarkerRing, _, _, _, _>(
        StateFamilyDescriptor::new(
            family_id("industry_marker_ring")?,
            canonical("industry.deposit_id.v1")?,
            canonical("industry.marker_ring.v1")?,
            1,
            canonical("industry.marker_ring.decode_json.v1")?,
            canonical("industry.marker_ring.bounds.v1")?,
            canonical("industry.marker_ring.json.v1")?,
        )
        .map_err(|error| ModError::msg(error.to_string()))?,
        |key: &OreDepositId| Ok(format!("{:016x}", key.0)),
        |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
        |value: &crate::record::MarkerRing| {
            if value.len() > crate::record::MAX_RETAINED_MARKERS_PER_DEPOSIT {
                Err(
                    "a marker ring retains at most MAX_RETAINED_MARKERS_PER_DEPOSIT keys"
                        .to_owned(),
                )
            } else {
                Ok(())
            }
        },
        |value: &crate::record::MarkerRing| serde_json::to_value(value).map_err(|e| e.to_string()),
    )?;
    // ---- E7c4 R3 (independent verify fold) — the work-order completion ring ----
    // The sibling of `industry_marker_ring`, keyed per facility, so
    // `industry.wo_applied` is bounded exactly as the extraction family is.
    let wo_marker_ring = reg.state_family::<ObjId, crate::record::MarkerRing, _, _, _, _>(
        StateFamilyDescriptor::new(
            family_id("industry_wo_marker_ring")?,
            canonical("obj_id.decimal.v1")?,
            canonical("industry.wo_marker_ring.v1")?,
            1,
            canonical("industry.wo_marker_ring.decode_json.v1")?,
            canonical("industry.wo_marker_ring.bounds.v1")?,
            canonical("industry.wo_marker_ring.json.v1")?,
        )
        .map_err(|error| ModError::msg(error.to_string()))?,
        |key: &ObjId| Ok(format!("{:010}", key.0)),
        |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
        |value: &crate::record::MarkerRing| {
            if value.len() > crate::record::MAX_RETAINED_MARKERS_PER_DEPOSIT {
                Err(
                    "a marker ring retains at most MAX_RETAINED_MARKERS_PER_DEPOSIT keys"
                        .to_owned(),
                )
            } else {
                Ok(())
            }
        },
        |value: &crate::record::MarkerRing| serde_json::to_value(value).map_err(|e| e.to_string()),
    )?;

    Ok(WorkFamilies {
        facility_slots,
        facility_books,
        capacity_index,
        work_orders,
        schedules,
        jobs,
        wo_applied,
        marker_ring,
        wo_marker_ring,
    })
}

impl IndustryModule {
    #[inline(never)]
    fn register_prepared_paths(
        &mut self,
        reg: &mut Registrar<'_>,
        families: &IndustryFamilies,
    ) -> Result<(), ModError> {
        // ---- E7c3: B6 — the prepared path ----
        if self.core_factory.is_some() && self.core_origins.is_none() {
            return Err(ModError::msg(
                "Core factory requires its configured certified Origin owner",
            ));
        }
        let jurisdiction: Arc<dyn crate::seams::JurisdictionAuthority> = self
            .jurisdiction
            .clone()
            .unwrap_or_else(|| Arc::new(crate::seams::ActsForJurisdiction));
        crate::provider::register_with_core_origins(
            reg,
            families,
            self.recipes.clone(),
            crate::provider::Seams {
                opener: self.opener.clone(),
                jurisdiction,
                tenure: self.tenure.clone(),
            },
            self.core_origins.as_deref(),
            self.core_factory.as_deref(),
        )?;
        if let Some(config) = self.core_origins.as_deref().cloned() {
            self.core_origin_runtime = Some(crate::core_production::Runtime::register(
                reg,
                config,
                families.clone(),
                self.core_factory.as_deref().cloned(),
            )?);
        }

        Ok(())
    }
}

impl GameModule for IndustryModule {
    fn name(&self) -> &'static str {
        NS
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        // ---- doc 20 cut SM.6 (D1): the typed-state delegations, first ----
        for grantee in &self.delegates {
            reg.delegate_state_families(grantee.as_str())?;
        }
        let ExtractionFamilies {
            deposits,
            claims,
            claim_index,
            runs,
            applied,
            place_census,
            tenure,
        } = register_extraction_families(reg)?;
        let WorkFamilies {
            facility_slots,
            facility_books,
            capacity_index,
            work_orders,
            schedules,
            jobs,
            wo_applied,
            marker_ring,
            wo_marker_ring,
        } = register_work_families(reg)?;
        // Cut 2's handles, taken from the composition root's cell. Cut 3 never
        // re-registers an inventory family and never mints a handle: the
        // capability is cut 2's, and the *delegation* that lets this owner's
        // transaction use it is cut 2's statement too
        // (`InventoryModule::with_quantity_delegates`).
        let inventory = self
            .inventory
            .as_ref()
            .and_then(|cell| cell.get().cloned())
            .ok_or_else(|| {
                ModError::msg(
                    "industry: mush-inventory must be composed before mush-industry with \
                     `with_handle_cell`, and must delegate its families to \"industry\"",
                )
            })?;

        let families = IndustryFamilies {
            deposits,
            claims,
            claim_index,
            runs,
            applied,
            place_census,
            inventory,
            facility_slots,
            facility_books: facility_books.clone(),
            capacity_index: capacity_index.clone(),
            work_orders,
            schedules,
            jobs,
            wo_applied,
            marker_ring,
            wo_marker_ring,
            tenure,
        };
        // Doc 20 cut SM.6: publish the handles to the composition root's cell
        // before anything that may read it registers.
        if let Some(cell) = &self.cell
            && cell.set(families.clone()).is_err()
        {
            tracing::warn!(
                target: "industry",
                "industry: the handle cell was already filled; the second fill is ignored"
            );
        }

        // ---- E7c4: B1 — the tree's SECOND `Reservation` validator, under
        // `mush-industry`'s own `OwnerId`. The registry is keyed
        // `(OwnerId, AuthorityGuardKind)` and stamps this module's owner, so it
        // lands on a different key from cut 2's inventory validator: legal by
        // construction, not by luck (cut 2 pinned exactly this composition).
        reg.authority_guard_validator(
            AuthorityGuardKind::Reservation,
            CapacityGuardValidator::new(facility_books, capacity_index),
        )?;

        self.register_prepared_paths(reg, &families)?;

        let mut store = IndustryStore::default();
        store.set_licence(self.licence.clone());
        reg.state(store);
        reg.state(IndustryHandles(families.clone()));

        let sweep_families = families.clone();
        // WS02-S5b: the sweep asks the recipe authority whether a `Draft`'s pinned
        // lines are still the pack's. Cloned into the closure because the tick
        // outlives `register`, and `None` on a server that composed no authority.
        let sweep_recipes = self.recipes.clone();
        // Doc 20 cut SM.6: the composed draws answer the sweep's invariant reads.
        let sweep_draws = self.draws.clone();
        reg.tick(
            SWEEP_TAG,
            TickCadence::Coarse(Duration::from_secs(SWEEP_PERIOD_SECS)),
            Arc::new(
                move |ctx: &mut ModuleCtx<'_>, _tag: &str, _scene: Option<mush_core::SceneId>| {
                    crate::store::reconcile(
                        ctx,
                        &sweep_families,
                        sweep_recipes.as_deref(),
                        sweep_draws.as_deref(),
                    );
                    // ---- E7r1: R1 — the budget settlement arm ----
                    //
                    // A SECOND ARM on this EXISTING Coarse tick, never a second
                    // tick, clock, pulse or schedule source: `wo_deliver` runs in
                    // `prepare_authoritative` with no `Ctx`, so the terminal capture
                    // cannot happen at the transition. It stages no authoritative
                    // submission — every post rides the economy's own carve-out lane
                    // — so the tick-cadence rule cannot bind this tick, and it
                    // carries its own visit/settle budgets so it can never starve
                    // the reconcile above.
                    crate::money::sweep(ctx, &sweep_families);
                },
            ),
        )?;

        // ---- WS02-S4: the open-order index, maintained on open/terminate ----
        //
        // Interface plan I13's `ActionStateChanged` is the only seam that says a
        // typed record went live *with a world in hand*, which is what the draft
        // offer needs: it must read the per-requester cap without walking
        // `industry.workorder` on every projection pass. The sweep's `reconcile`
        // rebuilds the same index from the family, so a missed announcement heals.
        reg.subscribe(
            mush_core::WorldEventKind::ActionStateChanged,
            Arc::new(crate::store::WorkOrderIndex {
                work_orders: families.work_orders.clone(),
            }),
        );

        // ---- E7c3/E7c4: B7/B8 — commands ----
        crate::commands::register(reg, &families, self.claim_catalog(), self.recipes.clone())?;
        self.families = Some(families);
        Ok(())
    }

    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        let Some(families) = self.families.clone() else {
            return Err(ModError::msg("industry: register did not run"));
        };
        if let Some(runtime) = &self.core_origin_runtime {
            runtime.load(ctx, &data)?;
        } else if data.kv.iter().any(|(key, _)| {
            key.as_str() == crate::core_production::INDEX_KEY
                || key.as_str() == crate::core_production::integration::records::INDEX_KEY
                || key.as_str() == crate::core_production::integration::delivery::INDEX_KEY
                || key.as_str() == crate::core_production::refit::records::INDEX_KEY
        }) {
            return Err(ModError::msg(
                "certified Origin history requires its configured owner",
            ));
        }
        // The one conventional document, skipped LOUDLY on a wire failure and
        // its body marked — never silently defaulted.
        if let Some(store) = ctx.w.module_state_mut::<IndustryStore>() {
            crate::store::hydrate(store, &data.kv);
        }
        crate::store::reconcile(
            ctx,
            &families,
            self.recipes.as_deref(),
            self.draws.as_deref(),
        );
        Ok(())
    }

    fn finalize_owner_composition(&mut self, ctx: &mut ModuleCtx<'_>) -> Result<(), ModError> {
        if let Some(runtime) = &self.core_origin_runtime {
            runtime.finalize(ctx)?;
        }
        Ok(())
    }
}

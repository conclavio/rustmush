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
    CanonicalId, GameModule, ModError, ModuleBootData, ModuleCtx, ObjId, Registrar,
    StateFamilyDescriptor, StateFamilyId, TickCadence,
};
use mush_economy::InstrumentId;
use serde_json::Value;
use smol_str::SmolStr;

use crate::families::IndustryFamilies;
use crate::record::{
    AppliedMarker, ClaimId, ClaimIndex, ClaimRecord, ExtractionRun, MAX_MARKER_ID_BYTES, NS,
    OreDeposit, OreDepositId, PlaceCensus,
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
/// has no `ruleset-tacsim` edge and never reads a pack. A deployment that wants
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
#[derive(Debug, Default)]
pub struct IndustryModule {
    families: Option<IndustryFamilies>,
    licence: Option<InstrumentId>,
    catalog: Option<(SmolStr, SmolStr)>,
    inventory: Option<mush_inventory::InventoryHandleCell>,
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
        }
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

impl GameModule for IndustryModule {
    fn name(&self) -> &'static str {
        NS
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
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
        };

        // ---- E7c3: B6 — the prepared path ----
        crate::provider::register(reg, &families)?;

        let mut store = IndustryStore::default();
        store.set_licence(self.licence.clone());
        reg.state(store);
        reg.state(IndustryHandles(families.clone()));

        let sweep_families = families.clone();
        reg.tick(
            SWEEP_TAG,
            TickCadence::Coarse(Duration::from_secs(SWEEP_PERIOD_SECS)),
            Arc::new(
                move |ctx: &mut ModuleCtx<'_>, _tag: &str, _scene: Option<mush_core::SceneId>| {
                    crate::store::reconcile(ctx, &sweep_families);
                },
            ),
        )?;

        // ---- E7c3: B7 — commands ----
        crate::commands::register(reg, &families, self.claim_catalog())?;
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
        // The one conventional document, skipped LOUDLY on a wire failure and
        // its body marked — never silently defaulted.
        if let Some(store) = ctx.w.module_state_mut::<IndustryStore>() {
            crate::store::hydrate(store, &data.kv);
        }
        crate::store::reconcile(ctx, &families);
        Ok(())
    }
}

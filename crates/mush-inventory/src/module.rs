//! E7c2 B2/B7 — the module: five typed families, one guard validator, one
//! command lane, one sweep.

use std::sync::Arc;
use std::time::Duration;

use mush_core::{
    BoundedScalar, CanonicalId, GameModule, ModError, ModuleBootData, ModuleCtx, Registrar,
    ReservationBook, ReservationBookId, ReservationBookStateCodec, ReservationId,
    StateFamilyDescriptor, StateFamilyId, TickCadence, reservation_book_state_family_descriptor,
};
use serde_json::Value;
use smol_str::SmolStr;

use crate::guard::ReservationGuardValidator;
use crate::pool::InventoryFamilies;
use crate::record::{
    AppliedMarker, EndpointRef, Lot, LotCensus, LotId, MAX_MARKER_ID_BYTES, NS, PoolCensus,
    PoolTranche, TrancheId,
};
use crate::store::InventoryStore;

/// The coarse reconciliation cadence. It exists for one reason: the fact mirror
/// (see `crate::facts`) must catch up with the authoritative markers even in a
/// world where nobody types `@inv`.
pub const SWEEP_TAG: &str = "inventory.sweep";
/// Seconds between sweeps.
pub const SWEEP_PERIOD_SECS: u64 = 60;

/// The crate's family handles, published in module state.
///
/// A handle is a capability, not authority: writing one still needs an
/// [`ActionTxn`](mush_core::ActionTxn) stamped with this crate's own `OwnerId`.
/// Publishing them is what lets a consumer (cut 3's extraction, cut 4's work
/// orders) call the exported verbs without owning the registration — 08i answer
/// 16's "owner-exported `pub fn` seams", with the handles they take reachable.
#[derive(Clone, Debug)]
pub struct InventoryHandles(pub InventoryFamilies);

/// The neutral quantity authority (Q13A/Q2D, D-E7-3).
#[derive(Debug, Default)]
pub struct InventoryModule {
    families: Option<InventoryFamilies>,
}

impl InventoryModule {
    /// A fresh module.
    #[must_use]
    pub const fn new() -> Self {
        Self { families: None }
    }
}

fn family_id(name: &str) -> Result<StateFamilyId, ModError> {
    StateFamilyId::new(name).map_err(|error| ModError::msg(error.to_string()))
}

fn canonical(name: &str) -> Result<CanonicalId, ModError> {
    CanonicalId::new(name).map_err(|error| ModError::msg(error.to_string()))
}

impl GameModule for InventoryModule {
    fn name(&self) -> &'static str {
        NS
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        // ---- E7c2: B2 — the pool pair (OPEN 2) ----
        // Two families under one `OwnerId`, written in ONE bundle and never
        // separately. `inventory.pool` is K's own `BoundedScalar` (its
        // `Deserialize` re-runs `BoundedScalar::new`, so the wire re-checks every
        // bound); `inventory.book` is K's own descriptor and codec, so cut 2
        // defines no schema identity of its own for it.
        let pools = reg.state_family::<ReservationBookId, BoundedScalar, _, _, _, _>(
            StateFamilyDescriptor::new(
                family_id("inventory_pools")?,
                canonical("reservation_book_id.v1")?,
                canonical("inventory.pool.v1")?,
                1,
                canonical("inventory.pool.decode_json.v1")?,
                canonical("inventory.pool.bounds.v1")?,
                canonical("inventory.pool.json.v1")?,
            )
            .map_err(|error| ModError::msg(error.to_string()))?,
            |key: &ReservationBookId| Ok(key.canonical_hash().to_hex()),
            |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
            |value: &BoundedScalar| {
                if value.minimum() == 0 && value.maximum() == crate::record::MAX_POOL_QUANTITY {
                    Ok(())
                } else {
                    Err("a pool scalar is bounded 0..=MAX_POOL_QUANTITY".to_owned())
                }
            },
            |value: &BoundedScalar| serde_json::to_value(value).map_err(|e| e.to_string()),
        )?;
        let books = reg.state_family::<ReservationBookId, ReservationBook, _, _, _, _>(
            reservation_book_state_family_descriptor(family_id("inventory_books")?)
                .map_err(|error| ModError::msg(error.to_string()))?,
            ReservationBookStateCodec,
            ReservationBookStateCodec,
            ReservationBookStateCodec,
            ReservationBookStateCodec,
        )?;
        // ---- E7c2: B3 — lots and tranches (OPEN 3: no world object per lot) ----
        let lots = reg.state_family::<LotId, Lot, _, _, _, _>(
            StateFamilyDescriptor::new(
                family_id("inventory_lots")?,
                canonical("inventory.lot_id.v1")?,
                canonical("inventory.lot.v1")?,
                1,
                canonical("inventory.lot.decode_json.v1")?,
                canonical("inventory.lot.invariants.v1")?,
                canonical("inventory.lot.json.v1")?,
            )
            .map_err(|error| ModError::msg(error.to_string()))?,
            |key: &LotId| Ok(format!("{:016x}", key.0)),
            |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
            // The wire already re-ran every constructor invariant; this is the
            // descriptor's own second look, which is what makes a hand-edited
            // document fail at BOTH gates.
            |value: &Lot| {
                if value.quantity() == 0 {
                    Err("a lot holds a non-zero quantity".to_owned())
                } else {
                    Ok(())
                }
            },
            |value: &Lot| serde_json::to_value(value).map_err(|e| e.to_string()),
        )?;
        let tranches = reg.state_family::<TrancheId, PoolTranche, _, _, _, _>(
            StateFamilyDescriptor::new(
                family_id("inventory_tranches")?,
                canonical("inventory.tranche_id.v1")?,
                canonical("inventory.tranche.v1")?,
                1,
                canonical("inventory.tranche.decode_json.v1")?,
                canonical("inventory.tranche.invariants.v1")?,
                canonical("inventory.tranche.json.v1")?,
            )
            .map_err(|error| ModError::msg(error.to_string()))?,
            |key: &TrancheId| Ok(format!("{:016x}", key.0)),
            |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
            |value: &PoolTranche| {
                // E7c2 fold C2: a root records matter it absorbed, matter it
                // inherited from another endpoint's root, or both — never
                // neither.
                if value.absorbed().is_empty() && value.inherited().is_empty() {
                    Err("a tranche absorbed or inherited at least one root".to_owned())
                } else {
                    Ok(())
                }
            },
            |value: &PoolTranche| serde_json::to_value(value).map_err(|e| e.to_string()),
        )?;
        // ---- E7c2: OPEN 5 — the applied marker the facts answer from ----
        let applied = reg.state_family::<SmolStr, AppliedMarker, _, _, _, _>(
            StateFamilyDescriptor::new(
                family_id("inventory_applied")?,
                canonical("inventory.marker_id.v1")?,
                canonical("inventory.applied.v1")?,
                1,
                canonical("inventory.applied.decode_json.v1")?,
                canonical("inventory.applied.invariants.v1")?,
                canonical("inventory.applied.json.v1")?,
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

        // ---- E7c2 fold B8 — the per-endpoint census §7's bounds are counted on ----
        let endpoint_key = |key: &EndpointRef| {
            key.key_text()
                .map(|text| text.as_str().to_owned())
                .map_err(|error| error.to_string())
        };
        let census = reg.state_family::<EndpointRef, PoolCensus, _, _, _, _>(
            StateFamilyDescriptor::new(
                family_id("inventory_census")?,
                canonical("inventory.endpoint_id.v1")?,
                canonical("inventory.census.v1")?,
                1,
                canonical("inventory.census.decode_json.v1")?,
                canonical("inventory.census.bounds.v1")?,
                canonical("inventory.census.json.v1")?,
            )
            .map_err(|error| ModError::msg(error.to_string()))?,
            endpoint_key,
            |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
            |value: &PoolCensus| {
                if value.get() as usize > crate::record::MAX_POOLS_PER_ENDPOINT {
                    Err("an endpoint pools at most MAX_POOLS_PER_ENDPOINT commodities".to_owned())
                } else {
                    Ok(())
                }
            },
            |value: &PoolCensus| serde_json::to_value(value).map_err(|e| e.to_string()),
        )?;
        let lot_census = reg.state_family::<EndpointRef, LotCensus, _, _, _, _>(
            StateFamilyDescriptor::new(
                family_id("inventory_lot_census")?,
                canonical("inventory.endpoint_id.v1")?,
                canonical("inventory.lot_census.v1")?,
                1,
                canonical("inventory.lot_census.decode_json.v1")?,
                canonical("inventory.lot_census.bounds.v1")?,
                canonical("inventory.lot_census.json.v1")?,
            )
            .map_err(|error| ModError::msg(error.to_string()))?,
            endpoint_key,
            |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
            |value: &LotCensus| {
                if value.get() > crate::record::MAX_LOTS_PER_ENDPOINT {
                    Err("an endpoint holds at most MAX_LOTS_PER_ENDPOINT lots".to_owned())
                } else {
                    Ok(())
                }
            },
            |value: &LotCensus| serde_json::to_value(value).map_err(|e| e.to_string()),
        )?;
        // ---- E7c2 fold B10 — the reservation → book index the guard reads ----
        let reservations = reg.state_family::<ReservationId, ReservationBookId, _, _, _, _>(
            StateFamilyDescriptor::new(
                family_id("inventory_reservation_index")?,
                canonical("inventory.reservation_id.v1")?,
                canonical("inventory.reservation_index.v1")?,
                1,
                canonical("inventory.reservation_index.decode_json.v1")?,
                canonical("inventory.reservation_index.bounds.v1")?,
                canonical("inventory.reservation_index.json.v1")?,
            )
            .map_err(|error| ModError::msg(error.to_string()))?,
            |key: &ReservationId| Ok(key.0.to_hex()),
            |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
            |_value: &ReservationBookId| Ok(()),
            |value: &ReservationBookId| serde_json::to_value(value).map_err(|e| e.to_string()),
        )?;

        let families = InventoryFamilies {
            pools,
            books: books.clone(),
            lots,
            tranches,
            applied,
            census,
            lot_census,
            reservations: reservations.clone(),
        };

        // ---- E7c2: B4 — the first `Reservation` validator on the tree ----
        // `mush-sim`'s `register` is the pattern: state family handle →
        // validator → `authority_guard_validator(kind, ..)`. The registry is
        // keyed `(OwnerId, AuthorityGuardKind)` and the registrar stamps THIS
        // module's owner, so cut 4 registering the same kind under
        // `mush-industry`'s owner is legal by construction, not by luck.
        reg.authority_guard_validator(
            mush_core::AuthorityGuardKind::Reservation,
            ReservationGuardValidator::new(books, reservations),
        )?;

        // ---- E7c2: B5 — the prepared path ----
        crate::provider::register(reg, &families)?;

        reg.state(InventoryStore::default());
        // 08i answer 16: the exported verbs are the ONLY way a consumer touches
        // quantity, and a consumer that does not own the registration still
        // needs the handles they take. Publishing them in module state is that
        // seam — read-only capabilities, useless without an `ActionTxn` this
        // owner stamped, and the shape cut 3's extraction writes through.
        reg.state(InventoryHandles(families.clone()));
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

        // ---- E7c2: B7 — commands ----
        crate::commands::register(reg, &families)?;
        self.families = Some(families);
        Ok(())
    }

    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        _data: ModuleBootData,
    ) -> Result<(), ModError> {
        // Cut 2 owns no conventional document, so `_data` is empty by design:
        // every family it writes is authoritative typed state the kernel has
        // already hydrated (strictly — a corrupt record refused this boot).
        // What is left is the cross-family pair check the kernel cannot make.
        let Some(families) = self.families.clone() else {
            return Err(ModError::msg("inventory: register did not run"));
        };
        crate::store::reconcile(ctx, &families);
        Ok(())
    }
}

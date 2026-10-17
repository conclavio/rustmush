//! **`mush-inventory` (E7 cut 2) — the single source of quantity truth.**
//!
//! 10b §3.2 / Q13A / Q2D / D-E7-3: a neutral quantity authority bound to **E3
//! custody endpoints**, which industry, markets, contracts and simulation
//! *consume* — and **none of them keeps a private stock number**. The proposal's
//! "material vault" quantities live here and its `material_source` becomes an
//! [`EndpointRef`].
//!
//! # What this crate is not
//!
//! Nothing TacSim-shaped, industry-shaped or market-shaped goes in it: no
//! commodity *definition*, no recipe, no deposit, no facility, no route, no
//! corridor fact, no listing. It holds **quantity, locality and lineage**, and
//! the *ruleset* decides representation per commodity (Q2D).
//!
//! # What it consumes verbatim and never rebuilds
//!
//! * **K's contention kernel** — `ResourceRef`, `BoundedScalar`,
//!   `ResourceOperation`, `checked_resource_availability`, `ReservationId`,
//!   `ReservationState`, `ReservationRecord`, `ReservationBookId`,
//!   `ReservationBookStateCodec`, `reservation_book_state_family_descriptor` and
//!   `ReservationBook`. There is **no `InventoryReservation`**: 10b §3.2 drafted
//!   one and K delivered exactly that record as `ReservationRecord`.
//! * **E3's custody** — `Vault`, `VaultStatus`, `CustodyRef`, `CustodyProvider`,
//!   `resolve`'s exactly-one-claiming-provider rule and validate-then-mutate
//!   `container_move`, asked through `mush_economy::endpoint_state`. Cut 2 adds
//!   **no second custody model and registers no custody provider**.
//! * **E6c's catalog** and the whole of `mush-market`, which it does not touch:
//!   a `Supply::Market` listing's `quantity` stays the market's own advertised
//!   counter and becomes no [`record::Pool`] claim, and the standing refusal of a
//!   `Representation::Lot` listing is lifted by cut 3 or cut 11, not here.
//! * **Money stays an E4 `Hold`** and never grows a resource field (T4, D-E7-2).
//!
//! # The three lenses cut 2 is verified on
//!
//! **C (conservation and lineage)** — every mutation rides
//! `ReservationBook::checked_apply_resource`, and a `dissolve` → `crystallize`
//! round trip leaves the [`record::PoolTranche`] as the child's parent with the
//! original [`record::LotId`] still reachable, so provenance survives it.
//! **L (locality)** — a pool is keyed by `(endpoint, commodity)` and the endpoint
//! is an E3 vault, so there is no endpoint-free balance and a hold that moves
//! with a ship needs no second mechanism: the backing moved, the pool did not.
//! **R (reservation)** — `checked_hold` subtracts every live hold before
//! admitting the next, and the pool scalar, the book and every lot/tranche
//! post-image ride **one** `AuthoritativeBundle`.

// WS01-S12: the panic-shaped restriction lints this crate measures zero of,
// held at zero. `crates/mush-contract-tests/tests/restriction_lints.rs` carries
// the measurement this set was cut from — including the counts of the lints
// NOT named here — and fails if this attribute loses one of them.
#![deny(
    clippy::unwrap_used,
    clippy::panic,
    clippy::unreachable,
    clippy::panic_in_result_fn
)]

pub mod commands;
pub mod facts;
pub mod guard;
pub mod module;
pub mod pool;
pub mod provider;
pub mod record;
pub mod salvage;
pub mod store;

pub use facts::InventoryFactProvider;
pub use guard::ReservationGuardValidator;
pub use module::{
    InventoryHandleCell, InventoryHandles, InventoryModule, SWEEP_PERIOD_SECS, SWEEP_TAG,
};
pub use pool::{
    InventoryError, InventoryFamilies, LotSale, MARKER_CAUSAL_MAX_BYTES, Moved, SettledSale,
    derive_lot_id, derive_tranche_id, derived_marker_id, pool_key, pool_resource,
    reservation_deadline, reservation_id_from_text, reservation_id_text, tranche_key,
};
pub use provider::{
    InventoryVerb, MAX_ENDPOINT_CHOICES, action_ref, endpoint_status, endpoint_visible,
    endpoints_truncated, live_endpoints, may_take,
};
pub use record::{
    AppliedMarker, EndpointRef, FACT_LOT_CREATED, FACT_SALVAGE_RECOVERED, FACT_TRANSFERRED, Grade,
    INV_PAGE, InventoryRecordError, Lineage, Lot, LotCensus, LotId, MAX_APPLIED_MARKERS, MAX_GRADE,
    MAX_LINEAGE_DEPTH, MAX_LOT_PARENTS, MAX_LOT_QUANTITY, MAX_LOTS_PER_ENDPOINT, MAX_POOL_QUANTITY,
    MAX_POOLS_PER_ENDPOINT, MAX_RETAINED_TERMINAL_RESERVATIONS, MAX_TRANCHE_ABSORBED,
    MAX_TRANCHE_GENERATIONS, MAX_TRANCHE_INHERITED, NS, Pool, PoolCensus, PoolTranche,
    RESERVATION_TTL_IC_DAYS, RESERVATION_TTL_SECS, TrancheId, owner,
};
pub use salvage::{
    ACTION_SALVAGE, MAX_SALVAGE_ENDPOINTS, MAX_SALVAGE_SITES, MAX_SALVAGE_WRECKS, NO_SUCH_WRECK,
    SalvageHandle, SalvageSource, WreckReport, salvage_holder, salvage_holder_text, site_token,
    sites,
};
pub use store::{InventoryStore, Quarantine};

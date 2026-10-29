//! E7c2 B2/B3/B4 — the twelve owner-exported verbs, and the one arithmetic gate.
//!
//! **Cut 2 writes no second arithmetic.** Every quantity mutation goes through
//! [`ReservationBook::checked_apply_resource`] or
//! [`ReservationBook::checked_consume`], which re-check bounds, checked
//! arithmetic, the scalar revision and post-mutation availability in one call —
//! and the book's own doc comment states the contract: *persist the returned book
//! and scalar in the same authoritative bundle; the stored scalar revision
//! detects an owner path that bypassed this gate.*
//!
//! **The pool is a pair, not a record** (OPEN 2): `inventory.pool` holds the
//! [`BoundedScalar`], `inventory.book` holds the [`ReservationBook`], both keyed
//! by the same [`ReservationBookId`], and every verb writes both inside one
//! [`ActionTxn`] — which is one authoritative bundle. §3.2's
//! `Pool { quantity, revision }` is the projection of that pair.
//!
//! **Cut 2 writes no state machine**: `Held → Consumed { commit } | Released |
//! Expired` is K's, through the book's own checked transitions, and there is no
//! `InventoryReservation`, no second `ReservationId` and no second id text.

use mush_core::{
    ActionStateError, ActionTxn, BoundedScalar, CanonicalHash, CanonicalId, ClockStamp,
    CommitToken, RequestUid, ReservationBook, ReservationBookError, ReservationBookId,
    ReservationId, ReservationRecord, ReservationState, ResourceOperation, ResourceRef,
    ScopedIdempotencyKey, StateFamilyHandle,
};
use serde_json::json;
use thiserror::Error;

use crate::record::{
    EndpointRef, Grade, InventoryRecordError, Lineage, Lot, LotId, MAX_LINEAGE_DEPTH,
    MAX_LOT_PARENTS, MAX_POOL_QUANTITY, MAX_RETAINED_TERMINAL_RESERVATIONS, MAX_TRANCHE_ABSORBED,
    MAX_TRANCHE_GENERATIONS, PoolTranche, RESERVATION_TTL_SECS, TrancheId, owner,
};

/// Domain tag of every derived inventory id. One domain, one encoder.
const ID_DOMAIN: &str = "inventory.id.v1";
/// Canonical-encoding ceiling for an id derivation.
const ID_BYTES: usize = 8 * 1024;

/// The typed families this crate owns. Handles are capabilities: nothing outside
/// an [`ActionTxn`] can write one, and every one is stamped with this crate's own
/// `OwnerId`.
#[derive(Clone, Debug)]
pub struct InventoryFamilies {
    /// `inventory.pool` — the [`BoundedScalar`] half of every pool.
    pub pools: StateFamilyHandle<ReservationBookId, BoundedScalar>,
    /// `inventory.book` — the [`ReservationBook`] half, on K's own descriptor.
    pub books: StateFamilyHandle<ReservationBookId, ReservationBook>,
    /// `inventory.lot` — identified parcels (OPEN 3: **no world object per lot**).
    pub lots: StateFamilyHandle<LotId, Lot>,
    /// `inventory.tranche` — the immutable lineage roots.
    pub tranches: StateFamilyHandle<TrancheId, PoolTranche>,
    /// `inventory.applied` — the committed bundle's own fact markers.
    pub applied: StateFamilyHandle<smol_str::SmolStr, crate::record::AppliedMarker>,
    /// `inventory.census` — the per-endpoint pool/lot counts §7's published
    /// bounds are actually enforced on (E7c2 fold B8). `ActionTxn` has no scan,
    /// so the count rides the same bundle as the record it counts.
    pub census: StateFamilyHandle<EndpointRef, crate::record::PoolCensus>,
    /// `inventory.lot_census` - the live-lot count per endpoint. A separate
    /// family from `census` because one `ActionTxn` may mutate one record only
    /// once and `dissolve` both creates a pool and consumes a lot.
    pub lot_census: StateFamilyHandle<EndpointRef, crate::record::LotCensus>,
    /// `inventory.reservation_index` — one [`ReservationId`] → its
    /// [`ReservationBookId`], written in the same bundle as the hold (E7c2 fold
    /// B10). It turns the guard validator's O(books) walk, and its permanent
    /// budget cliff, into one point lookup.
    pub reservations: StateFamilyHandle<ReservationId, ReservationBookId>,
    /// `inventory.marker_ring` — the applied-marker ids each endpoint wrote as
    /// a marker's source, so the durable `inventory.applied` family is bounded
    /// per endpoint (doc 18 §9 risk 19, cut IC14) without retiring a marker
    /// any read or replay could still need (E7c2 owner decision D1).
    pub marker_ring: StateFamilyHandle<EndpointRef, crate::record::MarkerRing>,
    pub(crate) core_inputs: Option<crate::core_input::CoreInputHandle>,
}

/// The `ResourceRef` one `(endpoint, commodity)` pool is keyed by.
///
/// # Errors
/// [`InventoryError::Record`] if the endpoint cannot render as canonical
/// idempotency text.
pub fn pool_resource(
    endpoint: EndpointRef,
    commodity: &CanonicalId,
) -> Result<ResourceRef, InventoryError> {
    Ok(ResourceRef::new(
        owner(),
        endpoint.subject()?,
        commodity.clone(),
    ))
}

/// The key both halves of one pool are stored under.
///
/// # Errors
/// [`InventoryError`] if the resource cannot be derived or hashed.
pub fn pool_key(
    endpoint: EndpointRef,
    commodity: &CanonicalId,
) -> Result<ReservationBookId, InventoryError> {
    let resource = pool_resource(endpoint, commodity)?;
    ReservationBookId::derive(&resource).map_err(|_| InventoryError::Identity)
}

/// Derive one lot identity from durable causal input.
///
/// Never an allocator: an allocator whose retry behaviour diverges after a
/// restart would mint a second lot for one replayed action, and the whole
/// crash-mid-commit argument rests on a replay reaching the *identical*
/// post-image.
///
/// # Errors
/// [`InventoryError::Identity`] if the canonical value cannot be hashed.
pub fn derive_lot_id(request: RequestUid, ordinal: u32) -> Result<LotId, InventoryError> {
    derive_u64("lot", request, ordinal).map(LotId)
}

/// Derive one tranche identity the same way.
///
/// # Errors
/// [`InventoryError::Identity`] if the canonical value cannot be hashed.
pub fn derive_tranche_id(request: RequestUid, ordinal: u32) -> Result<TrancheId, InventoryError> {
    derive_u64("tranche", request, ordinal).map(TrancheId)
}

fn derive_u64(class: &str, request: RequestUid, ordinal: u32) -> Result<u64, InventoryError> {
    let digest = CanonicalHash::for_canonical_value(
        ID_DOMAIN,
        1,
        &json!({ "class": class, "request": request, "ordinal": ordinal }),
        ID_BYTES,
    )
    .map_err(|_| InventoryError::Identity)?;
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&digest.as_bytes()[..8]);
    Ok(u64::from_be_bytes(bytes))
}

/// The cap on the **canonical JSON input** of a [`derived_marker_id`] (doc 18
/// §4.3, cut IC10): the causal value's canonical bytes, not the digest — the
/// digest is always 32 bytes, so a derived id is always 80 hex characters.
pub const MARKER_CAUSAL_MAX_BYTES: usize = 512;

/// Domain tag of a caller-derived applied-marker id (cut IC10).
const MARKER_DOMAIN: &str = "inventory.marker.derived.v1";

/// ---- Doc 18 §4.3 / D-18-26, cut IC10: an offer-time-derivable marker id ----
///
/// The applied-marker id a caller names **before** the bundle that writes it
/// exists: `{at}{digest}`, the IC second order-preserved in 16 hex bytes (the
/// `InventoryProvider::marker_id` shape, so the mirror's newest-kept order holds)
/// followed by the 64-hex SHA-256 of `causal` under its own domain. It is a pure
/// function of its causal input, so the market can gate a sale's contract on
/// `Fact{inventory, inventory.transferred, <id>}` at offer time and
/// [`InventoryFamilies::transfer_marked`] writes the marker under exactly that
/// id when the goods move. Always 16 + 64 = 80 lowercase hex characters, inside
/// the `inventory_applied` family's 1..=96 key rule.
///
/// # Errors
/// [`InventoryError::Identity`] if `causal` cannot be hashed — including a
/// causal value whose canonical bytes exceed [`MARKER_CAUSAL_MAX_BYTES`].
pub fn derived_marker_id(
    at_ic: i64,
    causal: &serde_json::Value,
) -> Result<smol_str::SmolStr, InventoryError> {
    let digest =
        CanonicalHash::for_canonical_value(MARKER_DOMAIN, 1, causal, MARKER_CAUSAL_MAX_BYTES)
            .map_err(|_| InventoryError::Identity)?;
    let ordered = (at_ic as u64) ^ (1_u64 << 63);
    Ok(smol_str::SmolStr::new(format!(
        "{ordered:016x}{}",
        digest.to_hex()
    )))
}

/// Whether `id` is an applied-marker key: 1..=96 bytes of lowercase hex.
///
/// The **one** marker-id rule (doc 18 cut IC14, row 17): the market's sale
/// path and its contract-side checks call this instead of keeping a copy.
#[must_use]
pub fn is_marker_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= crate::record::MAX_MARKER_ID_BYTES
        && id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// The deadline a fresh hold carries: [`RESERVATION_TTL_SECS`] past *now* on the
/// caller's own clock generation.
///
/// Every later comparison is `ClockStamp::checked_cmp`, so a stamp minted on
/// another generation **refuses comparison rather than expiring early** (K4a).
///
/// # Errors
/// [`InventoryError::ClockArithmetic`] if the deadline overflows.
pub fn reservation_deadline(now: &ClockStamp) -> Result<ClockStamp, InventoryError> {
    Ok(ClockStamp {
        clock: now.clock.clone(),
        at: now
            .at
            .checked_add(RESERVATION_TTL_SECS)
            .ok_or(InventoryError::ClockArithmetic)?,
    })
}

/// ---- E7r2: P1 — what one sale asks [`InventoryFamilies::settle_sale`] for ----
///
/// A parameter object rather than eight positional arguments, because the two lot
/// ids and the two endpoints are trivially transposable and a transposed pair would
/// deliver the buyer's goods to the seller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LotSale {
    /// Where the buyer's lot is created. Must differ from the source endpoint.
    pub destination: EndpointRef,
    /// How much of the hold the buyer bought.
    pub sold: u64,
    /// The grade the listing advertised, carried onto both lots.
    pub grade: Grade,
    /// The buyer's lot id — derived from the settling request, never allocated.
    pub buyer_lot: LotId,
    /// The seller's remainder lot id, used only when `sold` is short of the hold.
    pub remainder_lot: LotId,
}

/// What [`InventoryFamilies::settle_sale`] moved.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SettledSale {
    /// The lot the buyer now holds.
    pub buyer: Lot,
    /// The lot the seller keeps when the sale was short of the hold.
    pub remainder: Option<Lot>,
    /// The source pool's balance after the consume.
    pub balance: u64,
}

/// What one quantity mutation moved, for the caller's outcome and marker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Moved {
    /// The pool's balance after the mutation.
    pub balance: u64,
    /// Its availability after the mutation (balance minus every live hold).
    pub available: u64,
    /// How much quantity this call **actually moved**, as
    /// `credit`/`debit`/`transfer` report it (the reservation verbs leave it
    /// zero — they have their own record). Zero for a same-endpoint transfer,
    /// which deviation (g) makes a no-op HIT — and (E7c2 fold C6) a fact that
    /// names a movement must not exist for a bundle that moved nothing, so the
    /// caller reads this before writing a marker.
    pub moved: u64,
}

/// A marker ring's push decided in memory and not yet staged (doc 20 cut SM.8): whether the
/// ring existed, the ring after every push, and the ids the pushes evicted.
struct RingAfter {
    existed: bool,
    next: crate::record::MarkerRing,
    evicted: Vec<smol_str::SmolStr>,
}

impl InventoryFamilies {
    // ---- the pair ----

    fn load_pair(
        &self,
        txn: &mut ActionTxn<'_>,
        endpoint: EndpointRef,
        commodity: &CanonicalId,
    ) -> Result<(ReservationBookId, Option<(BoundedScalar, ReservationBook)>), InventoryError> {
        let key = pool_key(endpoint, commodity)?;
        let scalar = txn.get(&self.pools, &key)?.copied();
        let book = txn.get(&self.books, &key)?.cloned();
        match (scalar, book) {
            (Some(scalar), Some(book)) => Ok((key, Some((scalar, book)))),
            (None, None) => Ok((key, None)),
            // A pool without its book, or a book without its pool, is exactly the
            // half-state boot quarantines. Reaching it inside a transaction means
            // an owner path bypassed the pair, so refuse rather than repair.
            _ => Err(InventoryError::HalfPool),
        }
    }

    fn write_pair(
        &self,
        txn: &mut ActionTxn<'_>,
        key: ReservationBookId,
        existed: bool,
        scalar: BoundedScalar,
        book: ReservationBook,
    ) -> Result<(), InventoryError> {
        if existed {
            txn.replace(&self.pools, &key, scalar)?;
            txn.replace(&self.books, &key, book)?;
        } else {
            txn.insert(&self.pools, &key, scalar)?;
            txn.insert(&self.books, &key, book)?;
        }
        Ok(())
    }

    fn fresh_pair(
        &self,
        endpoint: EndpointRef,
        commodity: &CanonicalId,
    ) -> Result<(BoundedScalar, ReservationBook), InventoryError> {
        let resource = pool_resource(endpoint, commodity)?;
        let scalar =
            BoundedScalar::new(0, MAX_POOL_QUANTITY, 0, 1).map_err(|_| InventoryError::Identity)?;
        let book = ReservationBook::new(resource, scalar)?;
        Ok((scalar, book))
    }

    /// The `(endpoint, commodity)` pool as §3.2's projection, or `None`.
    ///
    /// # Errors
    /// [`InventoryError`] on a half pool or an unreadable family.
    pub fn pool(
        &self,
        txn: &mut ActionTxn<'_>,
        endpoint: EndpointRef,
        commodity: &CanonicalId,
    ) -> Result<Option<crate::record::Pool>, InventoryError> {
        let (_, pair) = self.load_pair(txn, endpoint, commodity)?;
        let Some((scalar, book)) = pair else {
            return Ok(None);
        };
        Ok(Some(project(endpoint, commodity, &scalar, &book)?))
    }

    /// Availability: balance minus **every** live hold, at every read.
    ///
    /// # Errors
    /// [`InventoryError`] on a half pool or an unreadable family.
    pub fn available(
        &self,
        txn: &mut ActionTxn<'_>,
        endpoint: EndpointRef,
        commodity: &CanonicalId,
    ) -> Result<u64, InventoryError> {
        Ok(self
            .pool(txn, endpoint, commodity)?
            .map_or(0, |pool| pool.available))
    }

    /// Add quantity to one pool.
    ///
    /// # Errors
    /// [`InventoryError`] on any refusal; **nothing is written on an error**.
    pub fn credit(
        &self,
        txn: &mut ActionTxn<'_>,
        endpoint: EndpointRef,
        commodity: &CanonicalId,
        amount: u64,
    ) -> Result<Moved, InventoryError> {
        self.apply(
            txn,
            endpoint,
            commodity,
            ResourceOperation::Increase { amount },
        )
    }

    /// Take quantity out of one pool. Refused below **available** — the balance
    /// minus every live hold — because `checked_apply_resource` re-runs
    /// availability over the whole contention set after the arithmetic.
    ///
    /// # Errors
    /// [`InventoryError`] on any refusal; **nothing is written on an error**.
    pub fn debit(
        &self,
        txn: &mut ActionTxn<'_>,
        endpoint: EndpointRef,
        commodity: &CanonicalId,
        amount: u64,
    ) -> Result<Moved, InventoryError> {
        self.apply(
            txn,
            endpoint,
            commodity,
            ResourceOperation::Decrease { amount },
        )
    }

    fn apply(
        &self,
        txn: &mut ActionTxn<'_>,
        endpoint: EndpointRef,
        commodity: &CanonicalId,
        operation: ResourceOperation,
    ) -> Result<Moved, InventoryError> {
        self.apply_counted(txn, endpoint, commodity, operation, CensusWrite::Write)
    }

    /// [`Self::apply`] with the census write named: [`CensusWrite::Write`]
    /// counts a new pool here (every single-pool verb), [`CensusWrite::Staged`]
    /// creates a new pair **without** touching `inventory.census`, because the
    /// caller ([`Self::transfer_many`]) has already staged the count for every
    /// new pool of its bundle (doc 20 cut SM.6, lead ruling 5).
    fn apply_counted(
        &self,
        txn: &mut ActionTxn<'_>,
        endpoint: EndpointRef,
        commodity: &CanonicalId,
        operation: ResourceOperation,
        census: CensusWrite,
    ) -> Result<Moved, InventoryError> {
        let (key, pair) = self.load_pair(txn, endpoint, commodity)?;
        let existed = pair.is_some();
        let (scalar, book) = match pair {
            Some(pair) => pair,
            None => {
                // E7c2 fold B8: `MAX_POOLS_PER_ENDPOINT` is counted HERE, on the
                // create path only, so crediting one of the endpoint's existing
                // pools still succeeds past the cap.
                match census {
                    CensusWrite::Write => self.add_pool(txn, endpoint)?,
                    CensusWrite::Staged => {}
                }
                self.fresh_pair(endpoint, commodity)?
            }
        };
        // THE gate. One call re-checks bounds, checked arithmetic, the scalar
        // revision and post-mutation availability; cut 2 adds no second one.
        let (book, scalar) = book.checked_apply_resource(book.revision(), scalar, operation)?;
        let available = book.checked_available(scalar)?;
        self.write_pair(txn, key, existed, scalar, book)?;
        Ok(Moved {
            balance: balance_of(scalar)?,
            available,
            moved: match operation {
                ResourceOperation::Increase { amount } | ResourceOperation::Decrease { amount } => {
                    amount
                }
                // Cut 2 never issues one; a `Set` moves an unknowable delta and
                // must not be reported as a movement.
                ResourceOperation::Set { .. } => 0,
            },
        })
    }

    /// One more pool at that endpoint, bounded by
    /// [`MAX_POOLS_PER_ENDPOINT`](crate::record::MAX_POOLS_PER_ENDPOINT).
    fn add_pool(
        &self,
        txn: &mut ActionTxn<'_>,
        endpoint: EndpointRef,
    ) -> Result<(), InventoryError> {
        let existing = txn.get(&self.census, &endpoint)?.copied();
        let next = existing.unwrap_or_default().checked_add()?;
        if existing.is_some() {
            txn.replace(&self.census, &endpoint, next)?;
        } else {
            txn.insert(&self.census, &endpoint, next)?;
        }
        Ok(())
    }

    /// `delta` more live lots at that endpoint, bounded by
    /// [`MAX_LOTS_PER_ENDPOINT`](crate::record::MAX_LOTS_PER_ENDPOINT).
    fn move_lots(
        &self,
        txn: &mut ActionTxn<'_>,
        endpoint: EndpointRef,
        delta: i64,
    ) -> Result<(), InventoryError> {
        let existing = txn.get(&self.lot_census, &endpoint)?.copied();
        let next = existing.unwrap_or_default().checked_move(delta)?;
        if existing.is_some() {
            txn.replace(&self.lot_census, &endpoint, next)?;
        } else {
            txn.insert(&self.lot_census, &endpoint, next)?;
        }
        Ok(())
    }

    /// Move quantity between two endpoints in **one** bundle.
    ///
    /// A transfer whose source and destination are the same endpoint is a
    /// **no-op HIT, not a double credit**: crediting and debiting one pool would
    /// bump the scalar revision twice for no movement and let a replay observe a
    /// balance that never existed. It reports [`Moved::moved`] `== 0`, which is
    /// what keeps the caller from publishing a transferred fact for it (E7c2
    /// fold C6).
    ///
    /// **A transfer carries lineage** (E7c2 fold C2). §3.2's anti-laundering
    /// claim used to hold only *within one endpoint*: a pool-to-pool hop named
    /// nothing, so dissolve-at-A → move A→B → crystallize-at-B broke the chain
    /// in one ordinary act. The source slot's root is now drawn down by what
    /// left and recorded on the destination's root through
    /// [`PoolTranche::checked_inherit`], so `crystallize` at B names a root that
    /// transitively reaches A's `absorbed` set.
    ///
    /// # Errors
    /// [`InventoryError`] on any refusal; **nothing is written on an error**.
    pub fn transfer(
        &self,
        txn: &mut ActionTxn<'_>,
        from: EndpointRef,
        to: EndpointRef,
        commodity: &CanonicalId,
        amount: u64,
    ) -> Result<Moved, InventoryError> {
        if from == to {
            let (_, pair) = self.load_pair(txn, from, commodity)?;
            let Some((scalar, book)) = pair else {
                return Err(InventoryError::NoPool);
            };
            if amount == 0 {
                return Err(InventoryError::Record(InventoryRecordError::ZeroQuantity));
            }
            let available = book.checked_available(scalar)?;
            if available < amount {
                return Err(InventoryError::Insufficient {
                    available,
                    requested: amount,
                });
            }
            return Ok(Moved {
                balance: balance_of(scalar)?,
                available,
                moved: 0,
            });
        }
        // Debit first: a refusal there must leave the destination untouched, and
        // `ActionTxn` discards the whole mutation set on any error anyway.
        self.debit(txn, from, commodity, amount)?;
        let moved = self.credit(txn, to, commodity, amount)?;
        self.carry_lineage(txn, from, to, commodity, amount)?;
        Ok(Moved {
            moved: amount,
            ..moved
        })
    }

    /// Move several commodities between two endpoints in ONE bundle, staging the
    /// destination's pool census once for every new pool (doc 20 cut SM.6, lead
    /// ruling 5; SM.3b's round F3).
    ///
    /// **Why it exists.** [`Self::transfer`] counts a new destination pool by
    /// writing `inventory.census` for that endpoint, and an [`ActionTxn`] refuses
    /// a second mutation of one key (`ActionTxn::require_fresh_mutation`,
    /// `mush-core/src/action_state.rs`, `ActionStateError::DuplicateMutation`).
    /// So two `transfer`s that each open a new pool at one endpoint can never
    /// share a bundle: a two-ore hopper dumped onto a fresh spoil refused. Here
    /// the new pools are counted first and the census is staged **once**
    /// (`PoolCensus::checked_add_n`), then each move runs as `transfer` does —
    /// debit, credit (a new pair created without a second census write), carry
    /// the lineage.
    ///
    /// Refused before anything is staged: `from == to`
    /// ([`InventoryError::SameEndpoint`]), an empty list or a zero amount
    /// ([`InventoryRecordError::ZeroQuantity`]), a commodity named twice
    /// ([`InventoryError::DuplicateCommodity`]), more than
    /// [`MAX_POOLS_PER_ENDPOINT`](crate::record::MAX_POOLS_PER_ENDPOINT) moves
    /// or new pools past the destination's cap
    /// ([`InventoryRecordError::EndpointPools`]).
    ///
    /// # Errors
    /// Those, plus every refusal of the debit, the credit and the lineage carry;
    /// **nothing is written on an error** (`ActionTxn` discards the bundle).
    pub fn transfer_many(
        &self,
        txn: &mut ActionTxn<'_>,
        from: EndpointRef,
        to: EndpointRef,
        moves: &[(CanonicalId, u64)],
    ) -> Result<Vec<Moved>, InventoryError> {
        if from == to {
            return Err(InventoryError::SameEndpoint);
        }
        if moves.is_empty() || moves.iter().any(|(_, amount)| *amount == 0) {
            return Err(InventoryError::Record(InventoryRecordError::ZeroQuantity));
        }
        if moves.len() > crate::record::MAX_POOLS_PER_ENDPOINT {
            return Err(InventoryError::Record(
                InventoryRecordError::EndpointPools {
                    max: crate::record::MAX_POOLS_PER_ENDPOINT,
                },
            ));
        }
        let mut seen = std::collections::BTreeSet::new();
        for (commodity, _) in moves {
            if !seen.insert(commodity) {
                return Err(InventoryError::DuplicateCommodity);
            }
        }
        // Which destination pairs are new: counted before any write, so the
        // census is one staged mutation for the whole bundle.
        let mut fresh = Vec::with_capacity(moves.len());
        for (commodity, _) in moves {
            let (_, pair) = self.load_pair(txn, to, commodity)?;
            fresh.push(pair.is_none());
        }
        let new_pools = fresh.iter().filter(|is_new| **is_new).count();
        if new_pools > 0 {
            let n = u32::try_from(new_pools).map_err(|_| {
                InventoryError::Record(InventoryRecordError::EndpointPools {
                    max: crate::record::MAX_POOLS_PER_ENDPOINT,
                })
            })?;
            let existing = txn.get(&self.census, &to)?.copied();
            let next = existing.unwrap_or_default().checked_add_n(n)?;
            if existing.is_some() {
                txn.replace(&self.census, &to, next)?;
            } else {
                txn.insert(&self.census, &to, next)?;
            }
        }
        let mut out = Vec::with_capacity(moves.len());
        for ((commodity, amount), is_new) in moves.iter().zip(fresh) {
            self.debit(txn, from, commodity, *amount)?;
            let census = if is_new {
                CensusWrite::Staged
            } else {
                CensusWrite::Write
            };
            let moved = self.apply_counted(
                txn,
                to,
                commodity,
                ResourceOperation::Increase { amount: *amount },
                census,
            )?;
            self.carry_lineage(txn, from, to, commodity, *amount)?;
            out.push(Moved {
                moved: *amount,
                ..moved
            });
        }
        Ok(out)
    }

    /// ---- Doc 18 §4.3 / D-18-26, cut IC10 — a transfer that writes its own fact ----
    ///
    /// [`Self::transfer`] plus the `inventory.transferred` applied marker under a
    /// **caller-derived** id ([`derived_marker_id`]) in the same bundle. It exists
    /// because the market gates a sale into a standing bid on that fact, names the
    /// id in the contract at offer time, and the provider's own `write_marker` is
    /// private (doc 18 §4.3: "the verb is exported by inventory").
    ///
    /// * `from == to` refuses [`InventoryError::SameEndpoint`]: a marker for a
    ///   non-move is the E7c2 fold-C6 bug.
    /// * A marker id that is not 1..=96 lowercase hex refuses
    ///   [`InventoryError::Identity`].
    /// * An existing marker under `marker` refuses [`InventoryError::AlreadyExists`]
    ///   — **the replay guard**, `write_marker`'s rule — read before any write.
    ///
    /// `wall` is the offer's real-clock stamp (doc 18 cut IC14, row 1: the
    /// `"wall"` fixed parameter the caller's prepare read); the marker is
    /// recorded in `from`'s [`MarkerRing`](crate::record::MarkerRing) in the
    /// same bundle ([`Self::retain_marker`]).
    ///
    /// # Errors
    /// Those three, plus every refusal of [`Self::transfer`], of
    /// [`AppliedMarker::new`](crate::record::AppliedMarker::new) and of
    /// [`Self::retain_marker`] ([`InventoryError::MarkerRingFull`]); **nothing
    /// is written on an error** (`ActionTxn` discards).
    #[allow(clippy::too_many_arguments)]
    pub fn transfer_marked(
        &self,
        txn: &mut ActionTxn<'_>,
        from: EndpointRef,
        to: EndpointRef,
        commodity: &CanonicalId,
        amount: u64,
        marker: &smol_str::SmolStr,
        actor: mush_core::ObjId,
        now: &ClockStamp,
        wall: i64,
    ) -> Result<Moved, InventoryError> {
        if from == to {
            return Err(InventoryError::SameEndpoint);
        }
        if !is_marker_id(marker) {
            return Err(InventoryError::Identity);
        }
        if txn.get(&self.applied, marker)?.is_some() {
            return Err(InventoryError::AlreadyExists);
        }
        let record = crate::record::AppliedMarker::new(
            crate::record::FACT_TRANSFERRED,
            Some(from),
            Some(to),
            commodity.clone(),
            amount,
            actor,
            now.at,
        )?;
        let moved = self.transfer(txn, from, to, commodity, amount)?;
        txn.insert(&self.applied, marker, record)?;
        self.retain_marker(txn, from, marker, wall)?;
        Ok(moved)
    }

    /// ---- Doc 20 §7.1, cut SM.8 — several marked moves in one bundle ----
    ///
    /// [`Self::transfer_many`] plus one `inventory.transferred` applied marker a
    /// move, under caller-derived ids ([`derived_marker_id`]), in ONE bundle (doc
    /// 20 cut SM.8, §7.1: the unload of a hopper that holds two ores). Two
    /// [`Self::transfer_marked`] calls on one source cannot share a bundle — the
    /// source's marker ring and the destination's pool census are one key each, and
    /// `ActionTxn` refuses a second mutation of one key — so the moves run as
    /// `transfer_many` does (the census staged once) and `from`'s ring is written
    /// once with every id ([`Self::retain_markers`]).
    ///
    /// Every check is made before anything is staged: `from == to`
    /// ([`InventoryError::SameEndpoint`]); an empty list or a zero amount
    /// ([`InventoryRecordError::ZeroQuantity`]); a commodity named twice
    /// ([`InventoryError::DuplicateCommodity`]); a marker id not 1..=96 lowercase
    /// hex, or one named twice ([`InventoryError::Identity`]); a marker already in
    /// `inventory.applied` ([`InventoryError::AlreadyExists`] — the replay guard,
    /// `transfer_marked`'s rule); **the source's ring full of young markers**
    /// ([`InventoryError::MarkerRingFull`] — the push is made in memory first, so
    /// a caller may answer this refusal as an outcome: nothing is staged, the cut
    /// SM.8 fix round's correctness/F2). Then the moves, each marker inserted, and
    /// the ring written once.
    ///
    /// # Errors
    /// Those, plus every refusal of [`Self::transfer_many`], of
    /// [`AppliedMarker::new`](crate::record::AppliedMarker::new) and of the ring's
    /// write; **nothing is written on an error** (`ActionTxn` discards the
    /// bundle), and none of the checks above has staged anything when it refuses.
    #[allow(clippy::too_many_arguments)]
    pub fn transfer_many_marked(
        &self,
        txn: &mut ActionTxn<'_>,
        from: EndpointRef,
        to: EndpointRef,
        moves: &[(CanonicalId, u64, smol_str::SmolStr)],
        actor: mush_core::ObjId,
        now: &ClockStamp,
        wall: i64,
    ) -> Result<Vec<Moved>, InventoryError> {
        self.transfer_many_marked_with(txn, (from, to), moves, (actor, now), (wall, None))
    }

    /// **[`Self::transfer_many_marked`] with its markers kept `keep_s` real seconds** (doc 20 cut
    /// SM.12c, decision (c); the owner's O-22 default): the same moves, markers and checks, each
    /// ring entry stamped with its own horizon ([`RingEntry::keep_s`](crate::record::RingEntry::keep_s)),
    /// so the source's ring may evict it once it is `keep_s` old instead of
    /// [`MARKER_RETENTION_SECS`](crate::record::MARKER_RETENTION_SECS). For a writer whose durable
    /// replay guard lies elsewhere (TacSim's unload finish: the rig record's ordinal in the same
    /// bundle). A horizon may only shorten.
    ///
    /// # Errors
    /// [`InventoryError::MarkerHorizon`] when `keep_s` is outside
    /// `1..=MARKER_RETENTION_SECS`, with nothing staged; else [`Self::transfer_many_marked`]'s.
    #[allow(clippy::too_many_arguments)]
    pub fn transfer_many_marked_keeping(
        &self,
        txn: &mut ActionTxn<'_>,
        from: EndpointRef,
        to: EndpointRef,
        moves: &[(CanonicalId, u64, smol_str::SmolStr)],
        actor: mush_core::ObjId,
        now: &ClockStamp,
        wall: i64,
        keep_s: i64,
    ) -> Result<Vec<Moved>, InventoryError> {
        if !(1..=crate::record::MARKER_RETENTION_SECS).contains(&keep_s) {
            return Err(InventoryError::MarkerHorizon);
        }
        self.transfer_many_marked_with(txn, (from, to), moves, (actor, now), (wall, Some(keep_s)))
    }

    /// [`Self::transfer_many_marked`]'s body, the ring entries' horizon given.
    fn transfer_many_marked_with(
        &self,
        txn: &mut ActionTxn<'_>,
        (from, to): (EndpointRef, EndpointRef),
        moves: &[(CanonicalId, u64, smol_str::SmolStr)],
        (actor, now): (mush_core::ObjId, &ClockStamp),
        (wall, keep_s): (i64, Option<i64>),
    ) -> Result<Vec<Moved>, InventoryError> {
        if from == to {
            return Err(InventoryError::SameEndpoint);
        }
        if moves.is_empty() || moves.iter().any(|(_, amount, _)| *amount == 0) {
            return Err(InventoryError::Record(InventoryRecordError::ZeroQuantity));
        }
        let mut commodities = std::collections::BTreeSet::new();
        let mut ids = std::collections::BTreeSet::new();
        for (commodity, _, marker) in moves {
            if !commodities.insert(commodity) {
                return Err(InventoryError::DuplicateCommodity);
            }
            if !is_marker_id(marker) || !ids.insert(marker) {
                return Err(InventoryError::Identity);
            }
        }
        let mut records = Vec::with_capacity(moves.len());
        for (commodity, amount, marker) in moves {
            if txn.get(&self.applied, marker)?.is_some() {
                return Err(InventoryError::AlreadyExists);
            }
            records.push(crate::record::AppliedMarker::new(
                crate::record::FACT_TRANSFERRED,
                Some(from),
                Some(to),
                commodity.clone(),
                *amount,
                actor,
                now.at,
            )?);
        }
        let marked: Vec<smol_str::SmolStr> =
            moves.iter().map(|(_, _, marker)| marker.clone()).collect();
        // The ring's push decided before anything is staged (correctness/F2).
        let ring = self.ring_after(txn, from, &marked, (wall, keep_s))?;
        let plain: Vec<(CanonicalId, u64)> = moves
            .iter()
            .map(|(commodity, amount, _)| (commodity.clone(), *amount))
            .collect();
        let moved = self.transfer_many(txn, from, to, &plain)?;
        for ((_, _, marker), record) in moves.iter().zip(records) {
            txn.insert(&self.applied, marker, record)?;
        }
        self.stage_ring(txn, from, ring)?;
        Ok(moved)
    }

    /// ---- Doc 18 §9 risk 19, cut IC14 row 1 — record a marker in its ring ----
    ///
    /// Record the applied marker `id`, just written in this bundle, in
    /// `endpoint`'s [`MarkerRing`](crate::record::MarkerRing), and remove from
    /// `inventory.applied` every marker the push evicts — all in the same
    /// bundle. `wall` is the offer's real-clock stamp, which is also "now" for
    /// the prepare (a prepare reads no clock). Since doc 20 cut SM.8 it is
    /// [`Self::retain_markers`] of one id: one ring rule, never two.
    ///
    /// **The replay argument.** E7c2 owner decision D1: the applied marker is
    /// the only replay guard that survives a reboot, and for a sale also the
    /// Deliver leg's guard, its settlement Fact and the re-stage's read. A
    /// marker is therefore retired only when its endpoint holds more than
    /// [`MAX_RETAINED_MARKERS_PER_ENDPOINT`](crate::record::MAX_RETAINED_MARKERS_PER_ENDPOINT)
    /// and it is older than its own horizon of real time — past every window
    /// in which a Fact, an over-sell guard, a re-stage or a re-driven offer
    /// reads it. The horizon is
    /// [`MARKER_RETENTION_SECS`](crate::record::MARKER_RETENTION_SECS) for every
    /// marker this writes; since doc 20 cut SM.12c (decision (c); the owner's
    /// O-22 default) a writer whose act carries a durable replay guard of its
    /// own may shorten it through [`Self::transfer_many_marked_keeping`] (the
    /// surface unload finish: one real hour), recorded on the ring entry
    /// ([`RingEntry::keep_s`](crate::record::RingEntry::keep_s)). A request
    /// replayed after its horizon is not guarded by the marker: that is the
    /// owner's D1 trade, recorded as doc 18 cut IC14 owner item O7, and O-22
    /// for the shortened horizon.
    ///
    /// # Errors
    /// [`InventoryError::MarkerRingFull`] when the ring holds
    /// [`MAX_MARKER_RING`](crate::record::MAX_MARKER_RING) entries none of
    /// which may yet be evicted (fail closed: the whole bundle is refused and
    /// nothing is written); [`InventoryError::State`] /
    /// [`InventoryError::Record`] when the ring cannot be read or written.
    pub fn retain_marker(
        &self,
        txn: &mut ActionTxn<'_>,
        endpoint: EndpointRef,
        id: &smol_str::SmolStr,
        wall: i64,
    ) -> Result<(), InventoryError> {
        self.retain_markers(txn, endpoint, std::slice::from_ref(id), wall)
    }

    /// ---- Doc 20 §7.1, cut SM.8 — record several markers in one ring write ----
    ///
    /// Record the applied markers `ids`, just written in this bundle, in
    /// `endpoint`'s [`MarkerRing`](crate::record::MarkerRing) with ONE write:
    /// each pushed in order in memory (`MarkerRing::push`, the eviction rule
    /// [`Self::retain_marker`] documents), every evicted id removed from
    /// `inventory.applied`, the ring staged once — a second ring write in one
    /// bundle would refuse `DuplicateMutation` (doc 20 cut SM.8's two-ore
    /// unload). An evicted id that is one of `ids` (impossible under the push
    /// rule: a just-pushed entry is never older than the horizon) fails closed as
    /// [`InventoryError::Identity`].
    ///
    /// # Errors
    /// [`InventoryError::MarkerRingFull`] as [`Self::retain_marker`]; the
    /// `Identity` case above; [`InventoryError::State`] /
    /// [`InventoryError::Record`] when the ring cannot be read or written.
    pub fn retain_markers(
        &self,
        txn: &mut ActionTxn<'_>,
        endpoint: EndpointRef,
        ids: &[smol_str::SmolStr],
        wall: i64,
    ) -> Result<(), InventoryError> {
        let ring = self.ring_after(txn, endpoint, ids, (wall, None))?;
        self.stage_ring(txn, endpoint, ring)
    }

    /// **[`Self::retain_markers`]' push, in memory** (doc 20 cut SM.8): `endpoint`'s ring
    /// read, `ids` pushed in order, the evictions collected — nothing staged, so
    /// [`InventoryError::MarkerRingFull`] here leaves the bundle untouched. Since doc 20 cut SM.12c
    /// the entries carry `keep_s` (`None`: the default horizon).
    fn ring_after(
        &self,
        txn: &mut ActionTxn<'_>,
        endpoint: EndpointRef,
        ids: &[smol_str::SmolStr],
        (wall, keep_s): (i64, Option<i64>),
    ) -> Result<RingAfter, InventoryError> {
        let current = txn.get(&self.marker_ring, &endpoint)?.cloned();
        let mut next = current
            .clone()
            .unwrap_or_else(crate::record::MarkerRing::empty);
        let mut evicted: Vec<smol_str::SmolStr> = Vec::new();
        for id in ids {
            let pushed = next.push(
                crate::record::RingEntry {
                    key: id.clone(),
                    wall,
                    keep_s,
                },
                wall,
            );
            let (ring, gone) = match pushed {
                Ok(pushed) => pushed,
                Err(InventoryRecordError::MarkerRingFull { .. }) => {
                    tracing::error!(
                        %endpoint,
                        "inventory: this endpoint's marker ring is full of markers younger than \
                         the retention horizon; the move is refused (doc 18 §9 risk 19)"
                    );
                    return Err(InventoryError::MarkerRingFull);
                }
                Err(error) => return Err(InventoryError::Record(error)),
            };
            next = ring;
            evicted.extend(gone);
        }
        if evicted.iter().any(|key| ids.contains(key)) {
            return Err(InventoryError::Identity);
        }
        Ok(RingAfter {
            existed: current.is_some(),
            next,
            evicted,
        })
    }

    /// **[`Self::retain_markers`]' write**: every evicted id removed from `inventory.applied`
    /// and the ring staged once.
    fn stage_ring(
        &self,
        txn: &mut ActionTxn<'_>,
        endpoint: EndpointRef,
        ring: RingAfter,
    ) -> Result<(), InventoryError> {
        for key in &ring.evicted {
            if txn.get(&self.applied, key)?.is_some() {
                txn.remove(&self.applied, key)?;
            }
        }
        if ring.existed {
            txn.replace(&self.marker_ring, &endpoint, ring.next)?;
        } else {
            txn.insert(&self.marker_ring, &endpoint, ring.next)?;
        }
        Ok(())
    }

    /// Record, on the destination's root, that `amount` arrived from the
    /// source's — and draw the source's claim down by exactly that much, so no
    /// unit is licensed twice.
    ///
    /// A source slot with no root carries nothing: there is no provenance to
    /// carry, and inventing one would be the laundering this closes.
    fn carry_lineage(
        &self,
        txn: &mut ActionTxn<'_>,
        from: EndpointRef,
        to: EndpointRef,
        commodity: &CanonicalId,
        amount: u64,
    ) -> Result<(), InventoryError> {
        let Some((source_id, source)) = self.tranche_for(txn, from, commodity)? else {
            return Ok(());
        };
        let carried = source.remaining().min(amount);
        if carried == 0 {
            return Ok(());
        }
        let drawn = source.checked_draw(carried)?;
        txn.replace(&self.tranches, &source_id, drawn)?;
        match self.tranche_for(txn, to, commodity)? {
            Some((id, current)) => match current.checked_inherit(source_id, carried) {
                Ok(next) => {
                    txn.replace(&self.tranches, &id, next)?;
                }
                // A full inherited set is not an absorbing state either: roll to
                // the next generation, which names the full predecessor.
                Err(InventoryRecordError::TrancheInherited { .. }) => {
                    let generation =
                        current
                            .generation()
                            .checked_add(1)
                            .ok_or(InventoryError::Record(
                                InventoryRecordError::TrancheGeneration {
                                    max: MAX_TRANCHE_GENERATIONS,
                                },
                            ))?;
                    let id = tranche_key(to, commodity, generation)?;
                    let next = PoolTranche::new(
                        id,
                        to,
                        commodity.clone(),
                        generation,
                        Some(current.id()),
                        vec![source_id],
                        Vec::new(),
                        carried,
                        carried,
                        current.depth(),
                    )?;
                    txn.insert(&self.tranches, &id, next)?;
                }
                Err(other) => return Err(other.into()),
            },
            None => {
                let id = tranche_key(to, commodity, 0)?;
                let next = PoolTranche::new(
                    id,
                    to,
                    commodity.clone(),
                    0,
                    None,
                    vec![source_id],
                    Vec::new(),
                    carried,
                    carried,
                    0,
                )?;
                txn.insert(&self.tranches, &id, next)?;
            }
        }
        Ok(())
    }

    // ---- reservations: K's lifecycle, written, never rebuilt ----

    /// Place one hold. `checked_hold` subtracts every live hold before admitting
    /// the next, so **two holders cannot spend one unit** and cut 2's whole job
    /// is never to route around it.
    ///
    /// # Errors
    /// [`InventoryError`] on any refusal; **nothing is written on an error**.
    pub fn reserve(
        &self,
        txn: &mut ActionTxn<'_>,
        endpoint: EndpointRef,
        commodity: &CanonicalId,
        holder: &ScopedIdempotencyKey,
        amount: u64,
        now: &ClockStamp,
    ) -> Result<ReservationId, InventoryError> {
        let (key, pair) = self.load_pair(txn, endpoint, commodity)?;
        let Some((scalar, book)) = pair else {
            return Err(InventoryError::NoPool);
        };
        let resource = pool_resource(endpoint, commodity)?;
        let id = ReservationId::derive(holder, &resource).map_err(|_| InventoryError::Identity)?;
        // E7c2 fold C3: terminal records were never retired, so one holder could
        // hold **once** per slot for the life of the world and the 257th hold at
        // a slot refused for ever. Retention is K's own `checked_prune_terminal`
        // in this same bundle — no second lifecycle, no state machine.
        let book = retire_terminal(book, scalar, id)?;
        let record = ReservationRecord::held(
            id,
            resource,
            holder.clone(),
            amount,
            reservation_deadline(now)?,
        )
        .map_err(InventoryError::Reservation)?;
        let book = book.checked_hold(book.revision(), scalar, record, now)?;
        self.write_pair(txn, key, true, scalar, book)?;
        // E7c2 fold B10: the guard validator cannot key the book from the guard,
        // so it walked the whole family and answered "not current" when its page
        // budget ran out. This index makes it a point lookup, and it rides the
        // one bundle the hold is already in.
        if txn.get(&self.reservations, &id)?.is_some() {
            txn.replace(&self.reservations, &id, key)?;
        } else {
            txn.insert(&self.reservations, &id, key)?;
        }
        Ok(id)
    }

    /// Consume one hold and decrement the pool **atomically**: `checked_consume`
    /// returns the book and the scalar together and neither is a valid partial
    /// result.
    ///
    /// # Errors
    /// [`InventoryError`] on any refusal; **nothing is written on an error**.
    #[allow(clippy::too_many_arguments)]
    pub fn consume(
        &self,
        txn: &mut ActionTxn<'_>,
        endpoint: EndpointRef,
        commodity: &CanonicalId,
        reservation: ReservationId,
        expected_reservation_revision: u64,
        now: &ClockStamp,
        commit: CommitToken,
    ) -> Result<Moved, InventoryError> {
        let (key, pair) = self.load_pair(txn, endpoint, commodity)?;
        let Some((scalar, book)) = pair else {
            return Err(InventoryError::NoPool);
        };
        let (book, scalar) = book.checked_consume(
            book.revision(),
            scalar,
            reservation,
            expected_reservation_revision,
            now,
            commit,
        )?;
        let available = book.checked_available(scalar)?;
        self.write_pair(txn, key, true, scalar, book)?;
        Ok(Moved {
            balance: balance_of(scalar)?,
            available,
            moved: 0,
        })
    }

    /// ---- E7r2: P1 — settle one hold **as matter**, not as a number ----
    ///
    /// **Consume one live hold and hand the quantity it held to `destination` as a
    /// lot**, parented to the source endpoint's own lineage root, in ONE bundle.
    ///
    /// This is an owner-exported seam (08i answer 16), **not** a new
    /// [`InventoryVerb`](crate::InventoryVerb) and not a new arithmetic: the
    /// consume is K's `checked_consume` and the lineage draw is
    /// [`PoolTranche::checked_draw`], exactly as `crystallize` does them. It
    /// exists because a sale needs both halves and cannot have them separately:
    /// `consume` writes the source pool pair and `crystallize` writes it again
    /// (through `debit`), and an [`ActionTxn`] mutates one key **once**
    /// (`ActionStateError::DuplicateMutation`). E7 residue pass 1's [`Self::renew`] is
    /// the precedent for the shape — one method, because one `write_pair`.
    ///
    /// **It mints nothing.** The pool is debited by the hold's own amount and the
    /// same amount reappears as lots, so `pool + lots` at the two endpoints is
    /// conserved to the unit; the buyer's lot carries `Lineage::Tranche`, whose
    /// root still names every lot ever poured into that pool, so provenance
    /// survives and no `Lineage::Origin` is created. `checked_draw` is the second
    /// gate and the one that matters for a market: **a sale cannot draw more
    /// lineage than the matter that established the root**, so a pool credited
    /// from somewhere with no root refuses with [`InventoryError::NoTranche`]
    /// rather than laundering.
    ///
    /// `sold` may be less than the hold: the remainder returns to `from` as its
    /// own tranche-rooted lot rather than to the pool, because the pool pair is
    /// already written. `destination` must differ from `from` — one endpoint's
    /// `lot_census` is one key.
    ///
    /// # Errors
    /// [`InventoryError`] on any refusal; **nothing is written on an error**.
    #[allow(clippy::too_many_arguments)]
    pub fn settle_sale(
        &self,
        txn: &mut ActionTxn<'_>,
        from: EndpointRef,
        commodity: &CanonicalId,
        reservation: ReservationId,
        expected_reservation_revision: u64,
        now: &ClockStamp,
        commit: CommitToken,
        sale: LotSale,
    ) -> Result<SettledSale, InventoryError> {
        if sale.destination == from {
            return Err(InventoryError::MergeMismatch);
        }
        let (key, pair) = self.load_pair(txn, from, commodity)?;
        let Some((scalar, book)) = pair else {
            return Err(InventoryError::NoPool);
        };
        let held = book
            .records()
            .iter()
            .find(|record| record.id() == reservation)
            .map(mush_core::ReservationRecord::amount)
            .ok_or(InventoryError::NoReservation)?;
        if sale.sold == 0 || sale.sold > held {
            return Err(InventoryError::Insufficient {
                available: held,
                requested: sale.sold,
            });
        }
        // Validate-then-mutate: every refusal below runs before the first write.
        let (book, scalar) = book.checked_consume(
            book.revision(),
            scalar,
            reservation,
            expected_reservation_revision,
            now,
            commit,
        )?;
        let (tranche_id, tranche) = self
            .tranche_for(txn, from, commodity)?
            .ok_or(InventoryError::NoTranche)?;
        // The whole hold leaves the pool, so the whole hold is drawn from the root.
        let drawn = tranche
            .checked_draw(held)
            .map_err(|_| InventoryError::NoTranche)?;
        let depth = tranche
            .depth()
            .checked_add(1)
            .ok_or(InventoryError::Record(InventoryRecordError::LineageDepth {
                max: MAX_LINEAGE_DEPTH,
            }))?;
        let buyer = Lot::new(
            sale.buyer_lot,
            sale.destination,
            commodity.clone(),
            sale.sold,
            sale.grade,
            Lineage::Tranche {
                tranche: tranche_id,
            },
            depth,
        )?;
        let left = held.checked_sub(sale.sold).ok_or(InventoryError::Record(
            InventoryRecordError::QuantityOverflow,
        ))?;
        let remainder = if left == 0 {
            None
        } else {
            Some(Lot::new(
                sale.remainder_lot,
                from,
                commodity.clone(),
                left,
                sale.grade,
                Lineage::Tranche {
                    tranche: tranche_id,
                },
                depth,
            )?)
        };
        if txn.get(&self.lots, &sale.buyer_lot)?.is_some() {
            return Err(InventoryError::AlreadyExists);
        }
        self.require_new_lot(txn, sale.buyer_lot)?;
        if remainder.is_some() {
            self.require_new_lot(txn, sale.remainder_lot)?;
        }
        if remainder.is_some() && txn.get(&self.lots, &sale.remainder_lot)?.is_some() {
            return Err(InventoryError::AlreadyExists);
        }
        self.write_pair(txn, key, true, scalar, book)?;
        txn.replace(&self.tranches, &tranche_id, drawn)?;
        txn.insert(&self.lots, &sale.buyer_lot, buyer.clone())?;
        self.move_lots(txn, sale.destination, 1)?;
        if let Some(record) = remainder.clone() {
            txn.insert(&self.lots, &sale.remainder_lot, record)?;
            self.move_lots(txn, from, 1)?;
        }
        Ok(SettledSale {
            buyer,
            remainder,
            balance: balance_of(scalar)?,
        })
    }

    /// **Re-anchor one live hold's deadline onto `now`'s clock**, keeping its
    /// amount, its id and its place in the contention set (E7 residue pass 1
    /// independent verify fold, Z3).
    ///
    /// A reservation's deadline is `reservation_deadline(now)`, stamped with the
    /// generation that placed it, and `ReservationBook::checked_consume` runs
    /// `require_future(expires, now)` — whose `ClockStamp::checked_cmp` refuses two
    /// **incomparable** generations outright. So a world-time rate change, which
    /// opens a new `WorldIc` generation, left every live reservation in the world
    /// unconsumable: an in-flight work order could be un-paused by `@wo/restamp` and
    /// still refuse to deliver, one layer below the pause.
    ///
    /// This is K's own [`ReservationBook::checked_replace`] — "replace amount and
    /// deadline for one live hold and re-run contention over the entire proposed
    /// set" — with the amount left exactly where it was, so no quantity authority
    /// moves here and contention is re-proved rather than assumed. It is one
    /// `write_pair`, which is why it is a method rather than a release/re-reserve
    /// pair in the caller: an `ActionTxn` writes one key once per bundle.
    ///
    /// # Errors
    /// [`InventoryError`] on any refusal; **nothing is written on an error**.
    pub fn renew(
        &self,
        txn: &mut ActionTxn<'_>,
        endpoint: EndpointRef,
        commodity: &CanonicalId,
        reservation: ReservationId,
        expected_reservation_revision: u64,
        now: &ClockStamp,
    ) -> Result<(), InventoryError> {
        let (key, pair) = self.load_pair(txn, endpoint, commodity)?;
        let Some((scalar, book)) = pair else {
            return Err(InventoryError::NoPool);
        };
        let amount = book
            .records()
            .iter()
            .find(|record| record.id() == reservation)
            .map(mush_core::ReservationRecord::amount)
            .ok_or(InventoryError::NoReservation)?;
        let book = book.checked_replace(
            book.revision(),
            scalar,
            mush_core::ReservationReplacement {
                reservation,
                expected_revision: expected_reservation_revision,
                amount,
                expires: reservation_deadline(now)?,
            },
            now,
        )?;
        self.write_pair(txn, key, true, scalar, book)
    }

    /// Release one live hold. Refused after `consume` — the record is terminal
    /// and `require_held` says so.
    ///
    /// # Errors
    /// [`InventoryError`] on any refusal; **nothing is written on an error**.
    pub fn release(
        &self,
        txn: &mut ActionTxn<'_>,
        endpoint: EndpointRef,
        commodity: &CanonicalId,
        reservation: ReservationId,
        expected_reservation_revision: u64,
    ) -> Result<(), InventoryError> {
        let (key, pair) = self.load_pair(txn, endpoint, commodity)?;
        let Some((scalar, book)) = pair else {
            return Err(InventoryError::NoPool);
        };
        let book = book.checked_release(
            book.revision(),
            scalar,
            reservation,
            expected_reservation_revision,
        )?;
        self.write_pair(txn, key, true, scalar, book)
    }

    /// Expire one elapsed hold. Idempotent by construction: a second attempt
    /// finds a terminal record and is refused, never re-applied.
    ///
    /// # Errors
    /// [`InventoryError`] on any refusal; **nothing is written on an error**.
    pub fn expire(
        &self,
        txn: &mut ActionTxn<'_>,
        endpoint: EndpointRef,
        commodity: &CanonicalId,
        reservation: ReservationId,
        expected_reservation_revision: u64,
        now: &ClockStamp,
    ) -> Result<(), InventoryError> {
        let (key, pair) = self.load_pair(txn, endpoint, commodity)?;
        let Some((scalar, book)) = pair else {
            return Err(InventoryError::NoPool);
        };
        let book = book.checked_expire(
            book.revision(),
            scalar,
            reservation,
            expected_reservation_revision,
            now,
        )?;
        self.write_pair(txn, key, true, scalar, book)
    }

    /// One live hold's exact record, if the pool has it.
    ///
    /// # Errors
    /// [`InventoryError`] on a half pool or an unreadable family.
    pub fn reservation(
        &self,
        txn: &mut ActionTxn<'_>,
        endpoint: EndpointRef,
        commodity: &CanonicalId,
        reservation: ReservationId,
    ) -> Result<Option<ReservationRecord>, InventoryError> {
        let (_, pair) = self.load_pair(txn, endpoint, commodity)?;
        Ok(pair.and_then(|(_, book)| {
            book.records()
                .iter()
                .find(|record| record.id() == reservation)
                .cloned()
        }))
    }

    // ---- lots, tranches, lineage ----

    /// Split `amount` off one lot into a new one at the same endpoint.
    ///
    /// `endpoint` is **the endpoint the offer authorised** (E7c2 fold C1): a lot
    /// at any other endpoint answers [`InventoryError::NoLot`], byte-identically
    /// to a lot that does not exist, so the check adds no oracle. Before the
    /// fold the three lot verbs read no endpoint at all and derived it from the
    /// record itself, which meant the `Choice` list authorised endpoint A while
    /// the mechanic mutated endpoint B.
    ///
    /// # Errors
    /// [`InventoryError`] on a missing lot, a lot at another endpoint, a zero or
    /// oversized split, or the depth cap. **Nothing is written on an error.**
    pub fn split(
        &self,
        txn: &mut ActionTxn<'_>,
        endpoint: EndpointRef,
        lot: LotId,
        amount: u64,
        child: LotId,
    ) -> Result<Lot, InventoryError> {
        let parent = txn
            .get(&self.lots, &lot)?
            .cloned()
            .ok_or(InventoryError::NoLot)?;
        if parent.endpoint() != endpoint {
            return Err(InventoryError::NoLot);
        }
        self.require_unheld_lot(txn, lot)?;
        self.require_new_lot(txn, child)?;
        if amount == 0 {
            return Err(InventoryError::Record(InventoryRecordError::ZeroQuantity));
        }
        if amount >= parent.quantity() {
            return Err(InventoryError::SplitTooLarge {
                quantity: parent.quantity(),
                requested: amount,
            });
        }
        let remainder = parent
            .quantity()
            .checked_sub(amount)
            .ok_or(InventoryError::Record(
                InventoryRecordError::QuantityOverflow,
            ))?;
        let depth = parent.depth().checked_add(1).ok_or(InventoryError::Record(
            InventoryRecordError::LineageDepth {
                max: crate::record::MAX_LINEAGE_DEPTH,
            },
        ))?;
        let created = Lot::new(
            child,
            parent.endpoint(),
            parent.commodity().clone(),
            amount,
            parent.grade(),
            Lineage::Split { parent: lot },
            depth,
        )?;
        let shrunk = parent.checked_resize(parent.revision(), remainder)?;
        self.move_lots(txn, endpoint, 1)?;
        txn.replace(&self.lots, &lot, shrunk)?;
        txn.insert(&self.lots, &child, created.clone())?;
        Ok(created)
    }

    /// ---- E7r2: P2 — **move one lot to another endpoint, unchanged** ----
    ///
    /// [`Lot::checked_relocate`]'s **first caller in the entire workspace** (10b
    /// §15.1 item 4's blocker cell, confirmed exactly true and now discharged), and
    /// the one seam salvage moves matter through.
    ///
    /// *A relocated lot is the SAME lot.* `checked_relocate` carries `id`,
    /// `commodity`, `quantity`, `grade`, `lineage` and `depth` through untouched and
    /// moves only `endpoint` and `revision` — so provenance is intact by
    /// construction rather than by care, and there is no arithmetic here at all to
    /// get wrong. **Nothing is minted, split or merged**: the two `lot_census`
    /// counters move by one in opposite directions and the pools are not touched,
    /// because a lot is not pooled quantity.
    ///
    /// It is deliberately **not** an [`InventoryVerb`](crate::InventoryVerb): no
    /// player-reachable relocate verb exists, and that absence is what stops a seller
    /// walking a listed lot out from under the market (10b §15.5's constraint (3)).
    /// The only caller is the salvage action's own `prepare_authoritative`, whose
    /// authority is `may_take_from_vault` over the wreck's endpoint, decided in
    /// `project_offer`.
    ///
    /// # Errors
    /// [`InventoryError::NoLot`] when the lot does not exist or is not at `from`,
    /// [`InventoryError::MergeMismatch`] when `from == to` (one endpoint's
    /// `lot_census` is one key, and a self-move would write it twice), and
    /// [`InventoryError::Record`] on a stale revision. **Nothing is written on an
    /// error.**
    pub fn relocate(
        &self,
        txn: &mut ActionTxn<'_>,
        lot: LotId,
        from: EndpointRef,
        to: EndpointRef,
    ) -> Result<Lot, InventoryError> {
        if from == to {
            return Err(InventoryError::MergeMismatch);
        }
        let record = txn
            .get(&self.lots, &lot)?
            .cloned()
            .ok_or(InventoryError::NoLot)?;
        // The endpoint is re-checked rather than trusted: `from` decides the
        // authority, so a lot that has already moved must not be movable by an offer
        // that named where it used to be.
        if record.endpoint() != from {
            return Err(InventoryError::NoLot);
        }
        self.require_unheld_lot(txn, lot)?;
        let moved = record.checked_relocate(record.revision(), to)?;
        txn.replace(&self.lots, &lot, moved.clone())?;
        self.move_lots(txn, from, -1)?;
        self.move_lots(txn, to, 1)?;
        Ok(moved)
    }

    /// Consume one complete, identified lot into a durable product receipt.
    /// The caller must validate authority, provenance and absence of a sale, and
    /// write its receipt in this same transaction. This does not credit a pool.
    ///
    /// # Errors
    /// Refuses a missing, moved or changed lot; census and removal are atomic.
    pub fn consume_product(
        &self,
        txn: &mut ActionTxn<'_>,
        expected: &Lot,
    ) -> Result<(), InventoryError> {
        let actual = txn
            .get(&self.lots, &expected.id())?
            .ok_or(InventoryError::NoLot)?;
        if actual != expected {
            return Err(InventoryError::NoLot);
        }
        self.move_lots(txn, expected.endpoint(), -1)?;
        txn.remove(&self.lots, &expected.id())?;
        Ok(())
    }

    /// Merge several lots into one. Every parent is consumed; the child records
    /// all of them.
    ///
    /// # Errors
    /// [`InventoryError`] past [`MAX_LOT_PARENTS`], on mixed commodity, grade or
    /// endpoint, on `u64` overflow, or on the depth cap. **Nothing is written on
    /// an error.**
    pub fn merge(
        &self,
        txn: &mut ActionTxn<'_>,
        endpoint: EndpointRef,
        parents: &[LotId],
        child: LotId,
    ) -> Result<Lot, InventoryError> {
        if parents.len() < 2 || parents.len() > MAX_LOT_PARENTS {
            return Err(InventoryError::Record(InventoryRecordError::LotParents {
                max: MAX_LOT_PARENTS,
            }));
        }
        let mut ordered = parents.to_vec();
        ordered.sort_unstable();
        ordered.dedup();
        if ordered.len() != parents.len() {
            return Err(InventoryError::Record(
                InventoryRecordError::NonCanonicalParents,
            ));
        }
        let mut loaded = Vec::with_capacity(ordered.len());
        for id in &ordered {
            loaded.push(
                txn.get(&self.lots, id)?
                    .cloned()
                    .ok_or(InventoryError::NoLot)?,
            );
        }
        let first = loaded.first().ok_or(InventoryError::NoLot)?;
        // E7c2 fold C1: the authorised endpoint decides, not the record.
        if first.endpoint() != endpoint {
            return Err(InventoryError::NoLot);
        }
        let commodity = first.commodity().clone();
        let grade = first.grade();
        let mut quantity = 0_u64;
        let mut depth = 0_u32;
        for lot in &loaded {
            if lot.endpoint() != endpoint || lot.commodity() != &commodity || lot.grade() != grade {
                return Err(InventoryError::MergeMismatch);
            }
            quantity = quantity
                .checked_add(lot.quantity())
                .ok_or(InventoryError::Record(
                    InventoryRecordError::QuantityOverflow,
                ))?;
            depth = depth.max(lot.depth());
        }
        let depth = depth.checked_add(1).ok_or(InventoryError::Record(
            InventoryRecordError::LineageDepth {
                max: crate::record::MAX_LINEAGE_DEPTH,
            },
        ))?;
        let created = Lot::new(
            child,
            endpoint,
            commodity,
            quantity,
            grade,
            Lineage::Merge {
                parents: ordered.clone(),
            },
            depth,
        )?;
        // Validate-then-mutate: every check above ran before the first write.
        for id in &ordered {
            self.require_unheld_lot(txn, *id)?;
        }
        self.require_new_lot(txn, child)?;
        let consumed = i64::try_from(ordered.len()).map_err(|_| InventoryError::Identity)?;
        self.move_lots(txn, endpoint, 1 - consumed)?;
        for id in &ordered {
            txn.remove(&self.lots, id)?;
        }
        txn.insert(&self.lots, &child, created.clone())?;
        Ok(created)
    }

    /// Pour one lot into its endpoint's pool. **One journaled, attributed act**
    /// (Q2D/T5): the tranche is created or extended, the pool is credited, and
    /// the lot record is consumed.
    ///
    /// The tranche is the **immutable lineage root** — `absorbed` keeps the
    /// original `LotId` for ever, which is what makes the round trip below
    /// unable to launder provenance.
    ///
    /// # Errors
    /// [`InventoryError`] on a missing lot or any refusal of the pool gate.
    /// **Nothing is written on an error.**
    pub fn dissolve(
        &self,
        txn: &mut ActionTxn<'_>,
        endpoint: EndpointRef,
        lot: LotId,
    ) -> Result<(TrancheId, Moved), InventoryError> {
        let record = txn
            .get(&self.lots, &lot)?
            .cloned()
            .ok_or(InventoryError::NoLot)?;
        // E7c2 fold C1: the authorised endpoint decides, not the record.
        if record.endpoint() != endpoint {
            return Err(InventoryError::NoLot);
        }
        self.require_unheld_lot(txn, lot)?;
        let commodity = record.commodity().clone();
        // One tranche per `(endpoint, commodity, generation)`, keyed
        // deterministically. That is what lets `crystallize` name a parent
        // without a scan, and it is why `absorbed` is append-only: the root is
        // shared, so dropping an entry would drop somebody else's provenance.
        //
        // E7c2 fold B2: the root is a **sequence**, not a singleton. A full
        // `absorbed` set, or a lot deep enough to pin the shared root at
        // `MAX_LINEAGE_DEPTH` and disable `crystallize` for every other holder,
        // rolls to `generation + 1` naming the full predecessor — so neither cap
        // is an absorbing state and nothing already recorded is dropped.
        let existing = self.tranche_for(txn, endpoint, &commodity)?;
        let roll = existing.as_ref().is_some_and(|(_, current)| {
            current.absorbed().len() >= MAX_TRANCHE_ABSORBED
                || current.depth().max(record.depth()) >= MAX_LINEAGE_DEPTH
        });
        let (id, next, fresh) = match (&existing, roll) {
            (Some((id, current)), false) => (
                *id,
                current.checked_absorb(lot, record.quantity(), record.depth())?,
                false,
            ),
            (Some((_, current)), true) => {
                let generation =
                    current
                        .generation()
                        .checked_add(1)
                        .ok_or(InventoryError::Record(
                            InventoryRecordError::TrancheGeneration {
                                max: MAX_TRANCHE_GENERATIONS,
                            },
                        ))?;
                let id = tranche_key(endpoint, &commodity, generation)?;
                (
                    id,
                    self.fresh_tranche(
                        id,
                        endpoint,
                        &commodity,
                        generation,
                        Some(current.id()),
                        lot,
                        &record,
                    )?,
                    true,
                )
            }
            (None, _) => {
                let id = tranche_key(endpoint, &commodity, 0)?;
                (
                    id,
                    self.fresh_tranche(id, endpoint, &commodity, 0, None, lot, &record)?,
                    true,
                )
            }
        };
        let moved = self.credit(txn, endpoint, &commodity, record.quantity())?;
        if fresh {
            txn.insert(&self.tranches, &id, next)?;
        } else {
            txn.replace(&self.tranches, &id, next)?;
        }
        self.move_lots(txn, endpoint, -1)?;
        txn.remove(&self.lots, &lot)?;
        Ok((id, moved))
    }

    /// A fresh generation of one slot's lineage root.
    ///
    /// The depth it starts at is the absorbed lot's, **clamped one below**
    /// [`MAX_LINEAGE_DEPTH`]: depth is a bound-enforcement device, not a
    /// provenance record, and re-anchoring it here is what keeps one deep lot
    /// from disabling `crystallize` at that slot for ever. The chain across
    /// generations is `predecessor`, itself bounded by
    /// [`MAX_TRANCHE_GENERATIONS`], so no verb walks an unbounded chain.
    #[allow(clippy::too_many_arguments)]
    fn fresh_tranche(
        &self,
        id: TrancheId,
        endpoint: EndpointRef,
        commodity: &CanonicalId,
        generation: u32,
        predecessor: Option<TrancheId>,
        lot: LotId,
        record: &Lot,
    ) -> Result<PoolTranche, InventoryError> {
        Ok(PoolTranche::new(
            id,
            endpoint,
            commodity.clone(),
            generation,
            predecessor,
            Vec::new(),
            vec![lot],
            record.quantity(),
            record.quantity(),
            record.depth().min(MAX_LINEAGE_DEPTH.saturating_sub(1)),
        )?)
    }

    /// Mint one lot out of a pool, **whose parent is that endpoint's tranche**.
    ///
    /// This is the other half of the round trip and the reason the laundering
    /// axis closes: the child's `Lineage::Tranche` names a record that still
    /// names every lot poured in, so the original `LotId` stays reachable.
    ///
    /// # Errors
    /// [`InventoryError::NoTranche`] when the endpoint has never dissolved
    /// anything there — a pool with no lineage root cannot mint a lot with one —
    /// plus every refusal of the pool gate. **Nothing is written on an error.**
    pub fn crystallize(
        &self,
        txn: &mut ActionTxn<'_>,
        endpoint: EndpointRef,
        commodity: &CanonicalId,
        amount: u64,
        grade: Grade,
        child: LotId,
    ) -> Result<(Lot, Moved), InventoryError> {
        self.require_new_lot(txn, child)?;
        let (tranche_id, tranche) = self
            .tranche_for(txn, endpoint, commodity)?
            .ok_or(InventoryError::NoTranche)?;
        // E7c2 fold C7: a lineage claim is bounded by the matter that
        // established it. One dissolved unit used to license every unit that
        // ever arrived in that pool to claim descent from that root.
        let drawn = tranche
            .checked_draw(amount)
            .map_err(|_| InventoryError::NoTranche)?;
        let depth = tranche
            .depth()
            .checked_add(1)
            .ok_or(InventoryError::Record(InventoryRecordError::LineageDepth {
                max: crate::record::MAX_LINEAGE_DEPTH,
            }))?;
        let created = Lot::new(
            child,
            endpoint,
            commodity.clone(),
            amount,
            grade,
            Lineage::Tranche {
                tranche: tranche_id,
            },
            depth,
        )?;
        let moved = self.debit(txn, endpoint, commodity, amount)?;
        self.move_lots(txn, endpoint, 1)?;
        txn.replace(&self.tranches, &tranche_id, drawn)?;
        txn.insert(&self.lots, &child, created.clone())?;
        Ok((created, moved))
    }

    /// Mint one lot of matter that entered the inventory **here** —
    /// `Lineage::Origin { source }`.
    ///
    /// This is the one verb that *creates* quantity rather than moving it, and
    /// it exists because a producer had nowhere else to put its output:
    /// `crystallize` needs a tranche (matter poured out of an existing lot) and
    /// `credit` makes pool, not a lot. E7 cut 3's extraction cycle calls it, and
    /// **conservation is the caller's bundle, not this call's**: the cycle
    /// `consume`s the deposit endpoint's hold in the same `ActionTxn`, so the
    /// unit minted here is exactly the unit that left the ground. `source` is
    /// the domain journal entry that produced it, so the origin is attributable
    /// rather than asserted.
    ///
    /// It deliberately writes **no pool**: crediting the destination and then
    /// debiting it again inside one transaction is a `DuplicateMutation`, and
    /// the intermediate balance never existed anyway.
    ///
    /// # Errors
    /// [`InventoryError`] for a zero or oversized quantity, a lot id already in
    /// use, or an endpoint at [`MAX_LOTS_PER_ENDPOINT`](crate::record::MAX_LOTS_PER_ENDPOINT).
    /// **Nothing is written on an error.**
    #[allow(clippy::too_many_arguments)]
    pub fn mint_origin(
        &self,
        txn: &mut ActionTxn<'_>,
        endpoint: EndpointRef,
        commodity: &CanonicalId,
        amount: u64,
        grade: Grade,
        source: mush_economy::ExternalRef,
        child: LotId,
    ) -> Result<Lot, InventoryError> {
        self.mint_root(
            txn,
            endpoint,
            commodity,
            amount,
            grade,
            Lineage::Origin { source },
            child,
        )
    }

    /// **Mint one lot under any root lineage** (doc 17h cut M8b-2, H1): `mint_origin`'s
    /// verb for the heirloom and grey arms beside `Origin`. The same conservation rule
    /// holds: the caller's bundle answers for the quantity.
    ///
    /// # Errors
    /// [`InventoryError`] as [`mint_origin`](Self::mint_origin), and for a lineage that
    /// names an older lot (a split, a merge or a tranche is never a root).
    /// **Nothing is written on an error.**
    #[allow(clippy::too_many_arguments)]
    pub fn mint_root(
        &self,
        txn: &mut ActionTxn<'_>,
        endpoint: EndpointRef,
        commodity: &CanonicalId,
        amount: u64,
        grade: Grade,
        lineage: Lineage,
        child: LotId,
    ) -> Result<Lot, InventoryError> {
        if !lineage.is_root() {
            return Err(InventoryError::InvalidLineage);
        }
        self.require_new_lot(txn, child)?;
        if txn.get(&self.lots, &child)?.is_some() {
            return Err(InventoryError::AlreadyExists);
        }
        let created = Lot::new(
            child,
            endpoint,
            commodity.clone(),
            amount,
            grade,
            lineage,
            0,
        )?;
        self.move_lots(txn, endpoint, 1)?;
        txn.insert(&self.lots, &child, created.clone())?;
        Ok(created)
    }

    /// One exact generation of the endpoint's root for one commodity.
    ///
    /// **Verifies what it loaded** (E7c2 fold C10): `tranche_key` truncates a
    /// canonical hash to 64 bits, and a key is not proof that the record it
    /// found is the record it names. A tranche whose own `endpoint`, `commodity`
    /// or `generation` disagrees with the key answers `None` — the same fail-
    /// closed rule `store::endpoint_of` already follows.
    fn tranche_at(
        &self,
        txn: &mut ActionTxn<'_>,
        endpoint: EndpointRef,
        commodity: &CanonicalId,
        generation: u32,
    ) -> Result<Option<(TrancheId, PoolTranche)>, InventoryError> {
        let id = tranche_key(endpoint, commodity, generation)?;
        let Some(record) = txn.get(&self.tranches, &id)?.cloned() else {
            return Ok(None);
        };
        if record.endpoint() != endpoint
            || record.commodity() != commodity
            || record.generation() != generation
        {
            return Ok(None);
        }
        Ok(Some((id, record)))
    }

    /// The endpoint's **current** tranche generation for one commodity.
    ///
    /// Bounded on records visited: at most [`MAX_TRANCHE_GENERATIONS`] + 1 point
    /// lookups, never a scan.
    ///
    /// # Errors
    /// [`InventoryError::State`] if the family cannot be read.
    pub fn tranche_for(
        &self,
        txn: &mut ActionTxn<'_>,
        endpoint: EndpointRef,
        commodity: &CanonicalId,
    ) -> Result<Option<(TrancheId, PoolTranche)>, InventoryError> {
        let mut current = None;
        for generation in 0..=MAX_TRANCHE_GENERATIONS {
            match self.tranche_at(txn, endpoint, commodity, generation)? {
                Some(found) => current = Some(found),
                None => break,
            }
        }
        Ok(current)
    }
}

/// Retire terminal reservation records so neither a repeat holder nor the book's
/// own record cap becomes an absorbing state (E7c2 fold C3).
///
/// The holder's own terminal record goes first — that is the (a) half, a holder
/// that could never reserve again after a clean consume — and then the oldest
/// terminal records down to [`MAX_RETAINED_TERMINAL_RESERVATIONS`], which is the
/// (b) half. A **live** duplicate is untouched and still refuses.
fn retire_terminal(
    book: ReservationBook,
    scalar: BoundedScalar,
    holder: ReservationId,
) -> Result<ReservationBook, InventoryError> {
    let terminal = |record: &&ReservationRecord| record.state() != ReservationState::Held;
    let mut book = book;
    if let Some(record) = book
        .records()
        .iter()
        .find(|record| record.id() == holder && terminal(record))
        .cloned()
    {
        book =
            book.checked_prune_terminal(book.revision(), scalar, record.id(), record.revision())?;
    }
    for _ in 0..mush_core::MAX_RESERVATION_BOOK_RECORDS {
        if book.records().iter().filter(terminal).count() <= MAX_RETAINED_TERMINAL_RESERVATIONS {
            break;
        }
        let Some(record) = book.records().iter().find(terminal).cloned() else {
            break;
        };
        book =
            book.checked_prune_terminal(book.revision(), scalar, record.id(), record.revision())?;
    }
    Ok(book)
}

/// The deterministic tranche key for one `(endpoint, commodity, generation)`.
///
/// One tranche per endpoint, commodity and generation is what makes
/// `crystallize` able to name a parent at all without a scan, and it is why
/// `absorbed` is append-only: the root is shared, so losing an entry would lose
/// somebody else's provenance. The **generation** is E7c2 fold B2: a full root
/// rolls forward naming its predecessor instead of refusing for ever.
///
/// # Errors
/// [`InventoryError::Identity`] if the canonical value cannot be hashed.
pub fn tranche_key(
    endpoint: EndpointRef,
    commodity: &CanonicalId,
    generation: u32,
) -> Result<TrancheId, InventoryError> {
    let digest = CanonicalHash::for_canonical_value(
        ID_DOMAIN,
        1,
        &json!({
            "class": "tranche.endpoint",
            "endpoint": endpoint,
            "commodity": commodity,
            "generation": generation,
        }),
        ID_BYTES,
    )
    .map_err(|_| InventoryError::Identity)?;
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&digest.as_bytes()[..8]);
    Ok(TrancheId(u64::from_be_bytes(bytes)))
}

fn balance_of(scalar: BoundedScalar) -> Result<u64, InventoryError> {
    u64::try_from(scalar.current()).map_err(|_| InventoryError::Identity)
}

/// Build §3.2's `Pool` projection from the stored pair.
///
/// # Errors
/// [`InventoryError`] if the pair disagrees or the balance is not representable.
pub fn project(
    endpoint: EndpointRef,
    commodity: &CanonicalId,
    scalar: &BoundedScalar,
    book: &ReservationBook,
) -> Result<crate::record::Pool, InventoryError> {
    let available = book.checked_available(*scalar)?;
    Ok(crate::record::Pool {
        endpoint,
        commodity: commodity.clone(),
        quantity: balance_of(*scalar)?,
        revision: scalar.revision(),
        available,
        holds: book
            .records()
            .iter()
            .filter(|record| record.state() == ReservationState::Held)
            .count(),
    })
}

/// Every refusal this crate can produce. Exhaustive, and none of them leaves a
/// partial write: `ActionTxn` discards the whole mutation set on any error.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum InventoryError {
    /// A permanently retained exact-input head protects this physical identity.
    #[error("that lot is reserved or its identity is retired")]
    ExactInput,
    /// A record refused its own invariants.
    #[error(transparent)]
    Record(#[from] InventoryRecordError),
    /// K's book or scalar refused the transition.
    #[error("inventory contention: {0}")]
    Book(String),
    /// K's reservation record refused the transition.
    #[error("inventory reservation: {0}")]
    Reservation(mush_core::ReservationError),
    /// A typed family could not be read or written.
    #[error("inventory typed state is unavailable")]
    State,
    /// An id could not be derived canonically.
    #[error("inventory identity cannot be derived")]
    Identity,
    /// The endpoint has a pool without its book, or a book without its pool.
    #[error("that endpoint's pool and book disagree and it is quarantined")]
    HalfPool,
    /// No pool exists at that endpoint for that commodity.
    #[error("that endpoint holds none of that")]
    NoPool,
    /// No such lot.
    #[error("no such lot")]
    NoLot,
    /// No live reservation with that id in this pool's book.
    #[error("no such reservation")]
    NoReservation,
    /// A request-derived lot id is already in use — the replay guard on the one
    /// verb that creates matter (E7c3's `mint_origin`).
    #[error("that lot already exists")]
    AlreadyExists,
    /// The endpoint has no lineage root to mint from.
    #[error("nothing has been dissolved into that pool")]
    NoTranche,
    /// A minted lot's lineage named an older lot (doc 17h cut M8b-2's `mint_root`).
    #[error("a minted lot names no older lot")]
    InvalidLineage,
    /// A split must leave a remainder.
    #[error("that lot holds {quantity}; a split of {requested} leaves nothing behind")]
    SplitTooLarge {
        /// The parent's quantity.
        quantity: u64,
        /// What was asked for.
        requested: u64,
    },
    /// A merge mixed endpoints, commodities or grades.
    #[error("those lots are not the same commodity, grade and endpoint")]
    MergeMismatch,
    /// Not enough is available (balance minus every live hold).
    #[error("that endpoint has {available} available and {requested} was asked for")]
    Insufficient {
        /// What may be spent.
        available: u64,
        /// What was asked for.
        requested: u64,
    },
    /// A deadline could not be computed on the caller's clock generation.
    #[error("inventory clock arithmetic overflowed")]
    ClockArithmetic,
    /// A marked transfer named one endpoint for both ends (doc 18 §4.3, cut
    /// IC10): a fact naming a movement must not exist for a bundle that moved
    /// nothing (E7c2 fold C6).
    #[error("a marked transfer moves between two endpoints")]
    SameEndpoint,
    /// The source endpoint's marker ring holds [`MAX_MARKER_RING`] entries
    /// none of which may yet be evicted (doc 18 §9 risk 19, cut IC14 row 1):
    /// the move is refused rather than retire a marker that may still guard a
    /// replay. Its text is player-facing (owner voice, cut IC14 O1).
    ///
    /// [`MAX_MARKER_RING`]: crate::record::MAX_MARKER_RING
    #[error(
        "this endpoint has recorded too many moves in the last day to record another; try again later"
    )]
    MarkerRingFull,
    /// A [`InventoryFamilies::transfer_many`] named one commodity twice (doc 20
    /// cut SM.6): two moves of one pool would mutate one pair twice in the
    /// bundle, so the list is refused before anything is staged.
    #[error("that move names one commodity twice")]
    DuplicateCommodity,
    /// [`InventoryFamilies::transfer_many_marked_keeping`] was asked for a marker
    /// horizon outside `1..=MARKER_RETENTION_SECS` (doc 20 cut SM.12c, decision
    /// (c)): a horizon may only shorten the ring's day, and nothing is staged.
    #[error("that move's marker horizon is out of range")]
    MarkerHorizon,
}

/// Whether a create path counts its new pool in `inventory.census` itself, or
/// its caller staged the count for the whole bundle (doc 20 cut SM.6).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CensusWrite {
    /// Count the new pool here (every single-pool verb).
    Write,
    /// The caller staged the census once for every new pool of its bundle.
    Staged,
}

impl From<ReservationBookError> for InventoryError {
    fn from(source: ReservationBookError) -> Self {
        Self::Book(source.to_string())
    }
}

impl From<ActionStateError> for InventoryError {
    fn from(_source: ActionStateError) -> Self {
        Self::State
    }
}

/// Recover a [`ReservationId`] from the canonical text an
/// `AuthorityGuard::Reservation` carries.
///
/// The pair with `ReservationId::0.to_hex()` is asserted once by a unit test:
/// `CanonicalHash::to_hex` is 64 lowercase hex bytes, which is inside
/// `MAX_IDEMPOTENCY_KEY_BYTES` (128) and inside `IdempotencyKeyText`'s byte
/// class, so the rendering is total. Cut 2 builds **no second id text**.
#[must_use]
pub fn reservation_id_from_text(text: &mush_core::IdempotencyKeyText) -> Option<ReservationId> {
    CanonicalHash::from_hex(text.as_str())
        .ok()
        .map(ReservationId)
}

/// Render a [`ReservationId`] as the canonical text a guard names it by.
///
/// # Errors
/// [`InventoryError::Identity`] if the hex is somehow not canonical text —
/// structurally unreachable, and asserted once rather than checked at guard time.
pub fn reservation_id_text(
    id: ReservationId,
) -> Result<mush_core::IdempotencyKeyText, InventoryError> {
    mush_core::IdempotencyKeyText::parse_canonical(id.0.to_hex())
        .map_err(|_| InventoryError::Identity)
}

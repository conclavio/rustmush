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
        let (key, pair) = self.load_pair(txn, endpoint, commodity)?;
        let existed = pair.is_some();
        let (scalar, book) = match pair {
            Some(pair) => pair,
            None => {
                // E7c2 fold B8: `MAX_POOLS_PER_ENDPOINT` is counted HERE, on the
                // create path only, so crediting one of the endpoint's existing
                // pools still succeeds past the cap.
                self.add_pool(txn, endpoint)?;
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
        if txn.get(&self.lots, &child)?.is_some() {
            return Err(InventoryError::AlreadyExists);
        }
        let created = Lot::new(
            child,
            endpoint,
            commodity.clone(),
            amount,
            grade,
            Lineage::Origin { source },
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
    /// A request-derived lot id is already in use — the replay guard on the one
    /// verb that creates matter (E7c3's `mint_origin`).
    #[error("that lot already exists")]
    AlreadyExists,
    /// The endpoint has no lineage root to mint from.
    #[error("nothing has been dissolved into that pool")]
    NoTranche,
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

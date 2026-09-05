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
    EndpointRef, Grade, InventoryRecordError, Lineage, Lot, LotId, MAX_LOT_PARENTS,
    MAX_POOL_QUANTITY, PoolTranche, RESERVATION_TTL_SECS, TrancheId, owner,
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
            None => self.fresh_pair(endpoint, commodity)?,
        };
        // THE gate. One call re-checks bounds, checked arithmetic, the scalar
        // revision and post-mutation availability; cut 2 adds no second one.
        let (book, scalar) = book.checked_apply_resource(book.revision(), scalar, operation)?;
        let available = book.checked_available(scalar)?;
        self.write_pair(txn, key, existed, scalar, book)?;
        Ok(Moved {
            balance: balance_of(scalar)?,
            available,
        })
    }

    /// Move quantity between two endpoints in **one** bundle.
    ///
    /// A transfer whose source and destination are the same endpoint is a
    /// **no-op HIT, not a double credit**: crediting and debiting one pool would
    /// bump the scalar revision twice for no movement and let a replay observe a
    /// balance that never existed.
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
            });
        }
        // Debit first: a refusal there must leave the destination untouched, and
        // `ActionTxn` discards the whole mutation set on any error anyway.
        self.debit(txn, from, commodity, amount)?;
        self.credit(txn, to, commodity, amount)
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
    /// # Errors
    /// [`InventoryError`] on a missing lot, a zero or oversized split, or the
    /// depth cap. **Nothing is written on an error.**
    pub fn split(
        &self,
        txn: &mut ActionTxn<'_>,
        lot: LotId,
        amount: u64,
        child: LotId,
    ) -> Result<Lot, InventoryError> {
        let parent = txn
            .get(&self.lots, &lot)?
            .cloned()
            .ok_or(InventoryError::NoLot)?;
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
        let endpoint = first.endpoint();
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
        lot: LotId,
    ) -> Result<(TrancheId, Moved), InventoryError> {
        let record = txn
            .get(&self.lots, &lot)?
            .cloned()
            .ok_or(InventoryError::NoLot)?;
        let endpoint = record.endpoint();
        let commodity = record.commodity().clone();
        // ONE tranche per `(endpoint, commodity)`, keyed deterministically. That
        // is what lets `crystallize` name a parent without a scan, and it is why
        // `absorbed` is append-only: the root is shared, so dropping an entry
        // would drop somebody else's provenance.
        let tranche = tranche_key(endpoint, &commodity)?;
        let existing = self.tranche_for(txn, endpoint, &commodity)?;
        let next = match &existing {
            Some((id, current)) => (
                *id,
                current.checked_absorb(lot, record.quantity(), record.depth())?,
            ),
            None => (
                tranche,
                PoolTranche::new(
                    tranche,
                    endpoint,
                    commodity.clone(),
                    vec![lot],
                    record.quantity(),
                    record.depth(),
                )?,
            ),
        };
        let moved = self.credit(txn, endpoint, &commodity, record.quantity())?;
        if existing.is_some() {
            txn.replace(&self.tranches, &next.0, next.1)?;
        } else {
            txn.insert(&self.tranches, &next.0, next.1)?;
        }
        txn.remove(&self.lots, &lot)?;
        Ok((next.0, moved))
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
        txn.insert(&self.lots, &child, created.clone())?;
        Ok((created, moved))
    }

    /// The endpoint's tranche for one commodity, if it has one.
    ///
    /// Bounded on records **visited**: a tranche family page, never a full scan.
    ///
    /// # Errors
    /// [`InventoryError::State`] if the family cannot be read.
    pub fn tranche_for(
        &self,
        txn: &mut ActionTxn<'_>,
        endpoint: EndpointRef,
        commodity: &CanonicalId,
    ) -> Result<Option<(TrancheId, PoolTranche)>, InventoryError> {
        let id = tranche_key(endpoint, commodity)?;
        Ok(txn.get(&self.tranches, &id)?.cloned().map(|t| (id, t)))
    }
}

/// The deterministic tranche key for one `(endpoint, commodity)`.
///
/// One tranche per endpoint and commodity is what makes `crystallize` able to
/// name a parent at all without a scan, and it is why `absorbed` is append-only:
/// the root is shared, so losing an entry would lose somebody else's provenance.
///
/// # Errors
/// [`InventoryError::Identity`] if the canonical value cannot be hashed.
pub fn tranche_key(
    endpoint: EndpointRef,
    commodity: &CanonicalId,
) -> Result<TrancheId, InventoryError> {
    let digest = CanonicalHash::for_canonical_value(
        ID_DOMAIN,
        1,
        &json!({ "class": "tranche.endpoint", "endpoint": endpoint, "commodity": commodity }),
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

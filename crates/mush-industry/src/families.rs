//! E7c3 B1/B2/B4/B6 — the typed families and the owner-exported seams over them.
//!
//! **Cut 3 writes no quantity arithmetic.** Every mutation of a deposit's
//! remaining reserve or of an extractor's stock goes through cut 2's
//! [`InventoryFamilies`] verbs, which are the only place
//! `ReservationBook::checked_apply_resource` and `checked_hold` live. What this
//! module owns is *tenure and authority state* — the deposit's definition half,
//! the claim, the run, and the applied marker the fact answers from.
//!
//! # One bundle, two owners
//!
//! An extraction cycle debits `inventory`'s pool at the deposit endpoint, mints
//! a lot at the operator's endpoint and advances `industry`'s run — and it does
//! all three in **one** `ActionTxn`. That is possible because `mush-inventory`
//! delegates its families to this owner at composition
//! (`InventoryModule::with_quantity_delegates`, E7c3 B1), which cut 2's own
//! handoff named as cut 3's: *"the cross-owner write — an `ActionTxn` is
//! owner-stamped, so a `mush-industry` provider cannot write `inventory`'s
//! families directly — cut 3 owns that bridge"*. The delegation widens the
//! **bundle**, never the gate: the arithmetic and its checked-apply gate stay
//! inside `mush-inventory`, and each write still persists under its own family
//! owner's namespace (`PreparedTxn::authoritative_replay_writes` keys every
//! write by `record.key.owner`).

use mush_core::{
    ActionStateError, ActionTxn, BoundedScalar, CanonicalHash, CanonicalId, ClockStamp,
    CommitToken, ObjId, RequestUid, ReservationBook, ReservationBookError, ReservationBookId,
    ReservationId, ReservationRecord, ReservationState, ScopedIdempotencyKey, StateFamilyHandle,
};
use mush_economy::ExternalRef;
use mush_inventory::{EndpointRef, Grade, InventoryError, InventoryFamilies, LotId};
use mush_org::PrincipalId;
use serde_json::json;
use smol_str::SmolStr;
use thiserror::Error;

use crate::record::{
    AppliedMarker, ClaimId, ClaimIndex, ClaimRecord, ClaimSubject, ExtractionRun,
    FACT_EXTRACTION_CYCLED, IndustryRecordError, OreDeposit, OreDepositId, PlaceCensus,
};

/// Domain tag of every derived industry id. One domain, one encoder.
const ID_DOMAIN: &str = "industry.id.v1";
/// Canonical-encoding ceiling for an id derivation.
const ID_BYTES: usize = 8 * 1024;

/// The typed families this crate owns, plus the delegated cut-2 handles every
/// quantity mutation rides.
#[derive(Clone, Debug)]
pub struct IndustryFamilies {
    /// `industry.deposit` — the ore body's definition half.
    ///
    /// **Authoritative, not exempt** (§4.1's cut-3 row, amended): the
    /// extraction provider re-reads it at prepare time under a checked
    /// revision, and `accessibility` decides whether a cycle is legal at all —
    /// which §4.1's own closing rule ("not exempt, ever: … anything that
    /// changes action legality") settles.
    pub deposits: StateFamilyHandle<OreDepositId, OreDeposit>,
    /// `industry.claim` — tenure. Authoritative for the same reason.
    pub claims: StateFamilyHandle<ClaimId, ClaimRecord>,
    /// `industry.run` — one extraction run per deposit.
    pub runs: StateFamilyHandle<OreDepositId, ExtractionRun>,
    /// `industry.applied` — the committed bundle's own fact marker.
    pub applied: StateFamilyHandle<SmolStr, AppliedMarker>,
    /// `industry.place_census` — how [`MAX_ORE_DEPOSITS_PER_PLACE`] is counted
    /// rather than merely declared (cut 2's fold B8 mechanism: an `ActionTxn`
    /// has no scan, so the count rides the same bundle as the record it counts).
    pub place_census: StateFamilyHandle<ObjId, PlaceCensus>,
    /// `industry.claim_index` — every claim **ever filed** over one subject,
    /// written in the same bundle as the claim. An `ActionTxn` has no scan, so
    /// this is how a provider knows the set at all (cut 2's fold B10 mechanism).
    /// It is **add-only and never pruned**, so `MAX_CLAIMS_PER_SUBJECT` is a
    /// **lifetime** bound (E7c3 verify fold B5); pruning terminal claims is cut
    /// 4's, with the marker-mirror retirement.
    pub claim_index: StateFamilyHandle<SmolStr, ClaimIndex>,
    /// Cut 2's handles, delegated to this owner at composition. Every quantity
    /// mutation goes through their exported verbs, never through a `txn` write
    /// of this crate's own.
    pub inventory: InventoryFamilies,
    /// `industry.facility_slots` — the [`BoundedScalar`] half of a facility's
    /// capacity, `0..=maximum_slots` with `current == maximum` (slots are held
    /// and released, never consumed). The `(pool, book)` pair of cut 2 applied to
    /// capacity (E7c4 B1, D-E7-2: one C8-conformant reservation shape, two
    /// owners, money is neither).
    pub facility_slots: StateFamilyHandle<ReservationBookId, BoundedScalar>,
    /// `industry.facility_book` — K's [`ReservationBook`] half. **Authoritative**;
    /// the facility *definition* half is the exempt document.
    pub facility_books: StateFamilyHandle<ReservationBookId, ReservationBook>,
    /// `industry.capacity_index` — one [`ReservationId`] → its
    /// [`ReservationBookId`], written in the same bundle as the slot hold so the
    /// [`CapacityGuardValidator`](crate::facility::CapacityGuardValidator) is two
    /// point lookups (cut 2's fold B10).
    pub capacity_index: StateFamilyHandle<ReservationId, ReservationBookId>,
    /// `industry.workorder` — **authoritative** (contradiction (iii)): its state
    /// transition gates capacity legality and its lists index authoritative
    /// records.
    pub work_orders: StateFamilyHandle<crate::workorder::WorkOrderId, crate::workorder::WorkOrder>,
    /// `industry.production_schedule` — K's [`DurableSchedule`](mush_core::DurableSchedule),
    /// created in the same bundle as the transition to `Running` (B5).
    pub schedules: StateFamilyHandle<mush_core::ScheduleId, mush_core::DurableSchedule>,
    /// `industry.production_job` — the schedule sidecar keyed by `ScheduleId`
    /// (the `handoff.rs` `HandoffJob` pattern).
    pub jobs: StateFamilyHandle<mush_core::ScheduleId, crate::workorder::ProductionJob>,
    /// `industry.wo_applied` — the committed terminal bundle's own
    /// `industry.workorder.completed` marker (B7). A second marker family beside
    /// cut 3's `industry.applied`, mirrored into the same store so the one
    /// `IndustryFactProvider` answers both arms (DECIDED (f)).
    pub wo_applied: StateFamilyHandle<SmolStr, crate::workorder::WorkOrderMarker>,
    /// `industry.marker_ring` — a bounded per-deposit FIFO of recent extraction
    /// marker keys, written in each cycle's bundle so the durable
    /// `industry.applied` family is **bounded per deposit** (D2, discharged in cut
    /// 4). Pruning cannot break replay: the run's monotone `next_cycle` refuses a
    /// replayed old cycle first.
    pub marker_ring: StateFamilyHandle<OreDepositId, crate::record::MarkerRing>,
    /// `industry.wo_marker_ring` — the sibling of `marker_ring` for the work-order
    /// completion family (R3, independent verify fold). Cut 4's own
    /// `industry.wo_applied` shipped with **no** ring and was never pruned, so past
    /// `MAX_APPLIED_MARKERS` lifetime completions the RAM mirror truncated and
    /// EVERY industry fact went dark for ever. Keyed **per facility** (a work order
    /// completes exactly once, so the retention key is the facility of its recent
    /// completions), it bounds `industry.wo_applied` the same way and cannot break
    /// replay: the request-derived lot id and the refuse-an-existing-marker guard
    /// remain the durable replay guards, so the ring may retire an old marker the
    /// delivery path will never read again.
    pub wo_marker_ring: StateFamilyHandle<ObjId, crate::record::MarkerRing>,
}

/// The canonical token one claim subject is censused under.
///
/// # Errors
/// Never in practice; the shape is `d<hex>` or `p<decimal>`.
#[must_use]
pub fn claim_subject_token(subject: ClaimSubject) -> SmolStr {
    match subject {
        ClaimSubject::Deposit { deposit } => SmolStr::new(format!("d{:016x}", deposit.0)),
        ClaimSubject::Place { place } => SmolStr::new(format!("p{}", place.0)),
    }
}

/// The marker id one request's fact is stored and answered under.
///
/// `{at}{request}`: the IC second the offer's fixed parameters stamped,
/// order-preserving in 16 hex bytes, then the request digest — cut 2's fold R3
/// shape, so canonical key order **is** time order and the RAM mirror keeps the
/// newest rather than whichever digests sort first. A pure function of durable
/// causal input: two replays of one request name the identical marker.
#[must_use]
pub fn marker_id(request: RequestUid, at: i64) -> SmolStr {
    let ordered = (at as u64) ^ (1_u64 << 63);
    SmolStr::new(format!("{ordered:016x}{}", request.to_hex()))
}

/// Derive one claim identity from durable causal input.
///
/// Never an allocator: an allocator whose retry behaviour diverges after a
/// restart would mint a second claim for one replayed action.
///
/// # Errors
/// [`IndustryError::Identity`] if the canonical value cannot be hashed.
pub fn derive_claim_id(request: RequestUid, ordinal: u32) -> Result<ClaimId, IndustryError> {
    derive_u64("claim", request, ordinal).map(ClaimId)
}

/// Derive one deposit identity the same way.
///
/// # Errors
/// [`IndustryError::Identity`] if the canonical value cannot be hashed.
pub fn derive_deposit_id(request: RequestUid, ordinal: u32) -> Result<OreDepositId, IndustryError> {
    derive_u64("deposit", request, ordinal).map(OreDepositId)
}

fn derive_u64(class: &str, request: RequestUid, ordinal: u32) -> Result<u64, IndustryError> {
    let digest = CanonicalHash::for_canonical_value(
        ID_DOMAIN,
        1,
        &json!({ "class": class, "request": request, "ordinal": ordinal }),
        ID_BYTES,
    )
    .map_err(|_| IndustryError::Identity)?;
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&digest.as_bytes()[..8]);
    Ok(u64::from_be_bytes(bytes))
}

/// What one completed extraction cycle produced.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Cycled {
    /// The cycle ordinal that ran.
    pub cycle: u64,
    /// How much came out of the ground.
    pub quantity: u64,
    /// The lot it became.
    pub lot: LotId,
    /// What is left in the ground afterwards.
    pub remaining: u64,
}

impl IndustryFamilies {
    // ---- deposits ----

    /// One deposit, if it exists.
    ///
    /// # Errors
    /// [`IndustryError::State`] if the family is unreadable.
    pub fn deposit(
        &self,
        txn: &mut ActionTxn<'_>,
        id: OreDepositId,
    ) -> Result<Option<OreDeposit>, IndustryError> {
        Ok(txn.get(&self.deposits, &id)?.cloned())
    }

    /// What is still in the ground: **cut 2's pool at the deposit's endpoint**.
    ///
    /// There is no second number to disagree with it.
    ///
    /// # Errors
    /// [`IndustryError`] if the pool pair is unreadable or half-present.
    pub fn remaining(
        &self,
        txn: &mut ActionTxn<'_>,
        deposit: &OreDeposit,
    ) -> Result<u64, IndustryError> {
        let pool = self
            .inventory
            .pool(txn, deposit.endpoint(), deposit.commodity())?;
        Ok(pool.map_or(0, |pool| pool.quantity))
    }

    /// Open one ore body: write the definition half and seed its endpoint pool
    /// with the authored reserve, **in one bundle**.
    ///
    /// The reserve is credited through cut 2's `credit`, so the deposit's very
    /// first unit of quantity is already under `ReservationBook`'s gate.
    ///
    /// # Errors
    /// [`IndustryError`] on a duplicate id, a place already at
    /// [`MAX_ORE_DEPOSITS_PER_PLACE`], or any inventory refusal. **Nothing is
    /// written on an error** — `ActionTxn` discards the whole mutation set.
    pub fn open_deposit(
        &self,
        txn: &mut ActionTxn<'_>,
        deposit: OreDeposit,
    ) -> Result<(), IndustryError> {
        if txn.get(&self.deposits, &deposit.id())?.is_some() {
            return Err(IndustryError::DepositExists);
        }
        let place = deposit.place();
        let census = txn.get(&self.place_census, &place)?.copied();
        let next = PlaceCensus::new(census.map_or(0, PlaceCensus::get).saturating_add(1))?;
        self.inventory.credit(
            txn,
            deposit.endpoint(),
            deposit.commodity(),
            deposit.initial_reserve(),
        )?;
        if census.is_some() {
            txn.replace(&self.place_census, &place, next)?;
        } else {
            txn.insert(&self.place_census, &place, next)?;
        }
        txn.insert(&self.deposits, &deposit.id(), deposit)?;
        Ok(())
    }

    /// Record an attributed campaign event that moves accessibility.
    ///
    /// It moves the **next** cycle's effective yield and never a completed one:
    /// a completed cycle's quantity is already in a lot and this writes no
    /// quantity at all.
    ///
    /// # Errors
    /// [`IndustryError::NoDeposit`] on a missing deposit,
    /// [`IndustryError::StaleRevision`] on a checked-revision mismatch.
    pub fn set_accessibility(
        &self,
        txn: &mut ActionTxn<'_>,
        id: OreDepositId,
        expected_revision: u64,
        accessibility: crate::record::Accessibility,
    ) -> Result<OreDeposit, IndustryError> {
        let deposit = self.deposit(txn, id)?.ok_or(IndustryError::NoDeposit)?;
        if deposit.revision() != expected_revision {
            return Err(IndustryError::StaleRevision);
        }
        let next = deposit.with_accessibility(accessibility)?;
        txn.replace(&self.deposits, &id, next.clone())?;
        Ok(next)
    }

    // ---- claims ----

    /// One claim, if it exists.
    ///
    /// # Errors
    /// [`IndustryError::State`] if the family is unreadable.
    pub fn claim(
        &self,
        txn: &mut ActionTxn<'_>,
        id: ClaimId,
    ) -> Result<Option<ClaimRecord>, IndustryError> {
        Ok(txn.get(&self.claims, &id)?.cloned())
    }

    /// File one claim.
    ///
    /// # Errors
    /// [`IndustryError::ClaimExists`] on a duplicate id,
    /// [`IndustryError::Record`] when the subject is already at
    /// [`MAX_CLAIMS_PER_SUBJECT`].
    pub fn file_claim(
        &self,
        txn: &mut ActionTxn<'_>,
        claim: ClaimRecord,
    ) -> Result<(), IndustryError> {
        if txn.get(&self.claims, &claim.id())?.is_some() {
            return Err(IndustryError::ClaimExists);
        }
        let token = claim_subject_token(claim.subject());
        let index = txn.get(&self.claim_index, &token)?.cloned();
        let next = index.clone().unwrap_or_default().with(claim.id())?;
        if index.is_some() {
            txn.replace(&self.claim_index, &token, next)?;
        } else {
            txn.insert(&self.claim_index, &token, next)?;
        }
        txn.insert(&self.claims, &claim.id(), claim)?;
        Ok(())
    }

    /// Every live claim over one subject, read through the index written beside
    /// them.
    ///
    /// Bounded by construction: the index cannot hold more than
    /// `MAX_CLAIMS_PER_SUBJECT`, so this is a bounded number of point lookups
    /// and never a scan.
    ///
    /// # Errors
    /// [`IndustryError::State`] if a family is unreadable.
    pub fn claims_over(
        &self,
        txn: &mut ActionTxn<'_>,
        subject: ClaimSubject,
    ) -> Result<Vec<ClaimRecord>, IndustryError> {
        let token = claim_subject_token(subject);
        let Some(index) = txn.get(&self.claim_index, &token)?.cloned() else {
            return Ok(Vec::new());
        };
        let mut out = Vec::with_capacity(index.len());
        for id in index.claims() {
            if let Some(claim) = txn.get(&self.claims, id)?.cloned() {
                out.push(claim);
            }
        }
        Ok(out)
    }

    /// Replace one claim under a checked revision.
    ///
    /// # Errors
    /// [`IndustryError::NoClaim`] on a missing claim,
    /// [`IndustryError::StaleRevision`] on a mismatch.
    pub fn put_claim(
        &self,
        txn: &mut ActionTxn<'_>,
        expected_revision: u64,
        claim: ClaimRecord,
    ) -> Result<(), IndustryError> {
        let live = self.claim(txn, claim.id())?.ok_or(IndustryError::NoClaim)?;
        if live.revision() != expected_revision {
            return Err(IndustryError::StaleRevision);
        }
        txn.replace(&self.claims, &claim.id(), claim)?;
        Ok(())
    }

    // ---- runs ----

    /// One deposit's run, if it has one.
    ///
    /// # Errors
    /// [`IndustryError::State`] if the family is unreadable.
    pub fn run(
        &self,
        txn: &mut ActionTxn<'_>,
        deposit: OreDepositId,
    ) -> Result<Option<ExtractionRun>, IndustryError> {
        Ok(txn.get(&self.runs, &deposit)?.cloned())
    }

    /// Open or restart one run.
    ///
    /// **One run per deposit, and only its own operator may replace a live one**
    /// (E7c3 verify fold C2 / D1(a)). A `Running` run whose `operator()` differs
    /// from the incoming run's is refused with [`IndustryError::RunNotYours`],
    /// which renders byte-identically to the missing-body refusal: without this,
    /// a co-tenant's `@extract/start` silently redirected the deposit's single
    /// run — and every later cycle's minted lot — to their own endpoint. A
    /// `Stopped` run carries no live tenure and may be replaced by anyone the
    /// offer authorized; a same-operator restart is always allowed.
    ///
    /// # Errors
    /// [`IndustryError::RunNotYours`] on a foreign redirect of a live run;
    /// [`IndustryError::State`] if the family is unwritable.
    pub fn put_run(
        &self,
        txn: &mut ActionTxn<'_>,
        run: ExtractionRun,
    ) -> Result<(), IndustryError> {
        if let Some(existing) = txn.get(&self.runs, &run.deposit())?.cloned() {
            if existing.state() == crate::record::RunState::Running
                && existing.operator() != run.operator()
            {
                return Err(IndustryError::RunNotYours);
            }
            txn.replace(&self.runs, &run.deposit(), run)?;
        } else {
            txn.insert(&self.runs, &run.deposit(), run)?;
        }
        Ok(())
    }

    // ---- E7c4 B1: facility capacity, on K's ReservationBook ----

    fn load_capacity_pair(
        &self,
        txn: &mut ActionTxn<'_>,
        object: ObjId,
    ) -> Result<(ReservationBookId, Option<(BoundedScalar, ReservationBook)>), IndustryError> {
        let key = crate::facility::facility_capacity_key(object)?;
        let scalar = txn.get(&self.facility_slots, &key)?.copied();
        let book = txn.get(&self.facility_books, &key)?.cloned();
        match (scalar, book) {
            (Some(scalar), Some(book)) => Ok((key, Some((scalar, book)))),
            (None, None) => Ok((key, None)),
            // Half a capacity pair is the half-state boot quarantines: reaching it
            // inside a transaction means an owner path bypassed the pair, so refuse
            // rather than repair — the fail-closed direction (a slot that vanished
            // must not read as free).
            _ => Err(IndustryError::CapacityHalf),
        }
    }

    fn write_capacity_pair(
        &self,
        txn: &mut ActionTxn<'_>,
        key: ReservationBookId,
        existed: bool,
        scalar: BoundedScalar,
        book: ReservationBook,
    ) -> Result<(), IndustryError> {
        if existed {
            txn.replace(&self.facility_slots, &key, scalar)?;
            txn.replace(&self.facility_books, &key, book)?;
        } else {
            txn.insert(&self.facility_slots, &key, scalar)?;
            txn.insert(&self.facility_books, &key, book)?;
        }
        Ok(())
    }

    fn fresh_capacity_pair(
        &self,
        object: ObjId,
        maximum_slots: u32,
    ) -> Result<(BoundedScalar, ReservationBook), IndustryError> {
        let resource = crate::facility::facility_resource(object)?;
        let max = i64::from(maximum_slots);
        // `current == maximum`: every slot is free at the start, and a slot is
        // *held* (not consumed), so `current` never moves — availability falls and
        // rises with the live holds alone, and there is no stored occupancy counter
        // (OPEN 1).
        let scalar = BoundedScalar::new(0, max, max, 1).map_err(|_| IndustryError::Identity)?;
        let book = ReservationBook::new(resource, scalar)?;
        Ok((scalar, book))
    }

    /// Free slots at this facility, **derived at every read** from the live book.
    ///
    /// A facility with no book yet has every slot free; there is no counter to
    /// read (OPEN 1). This is the only availability path, and a test asserts it
    /// reads no stored `active_slots`.
    ///
    /// # Errors
    /// [`IndustryError`] on a half pair or an unreadable family.
    pub fn available_slots(
        &self,
        txn: &mut ActionTxn<'_>,
        object: ObjId,
        maximum_slots: u32,
    ) -> Result<u64, IndustryError> {
        let (_, pair) = self.load_capacity_pair(txn, object)?;
        match pair {
            None => Ok(u64::from(maximum_slots)),
            Some((scalar, book)) => Ok(book.checked_available(scalar)?),
        }
    }

    /// Hold one slot for the work order's deterministic holder key.
    ///
    /// **Two orders contending for the last slot: exactly one starts**, and the
    /// second refuses at K's `checked_hold` (`InsufficientAvailable`), never at a
    /// cut-4 check. A `Held` slot for the same holder that was released is retired
    /// first, so a holder that finished may reserve again.
    ///
    /// # Errors
    /// [`IndustryError`] on any refusal; **nothing is written on an error**.
    pub fn reserve_slot(
        &self,
        txn: &mut ActionTxn<'_>,
        object: ObjId,
        maximum_slots: u32,
        holder: &ScopedIdempotencyKey,
        now: &ClockStamp,
    ) -> Result<ReservationId, IndustryError> {
        let (key, pair) = self.load_capacity_pair(txn, object)?;
        let (existed, scalar, book) = match pair {
            Some((scalar, book)) => (true, scalar, book),
            None => {
                let (scalar, book) = self.fresh_capacity_pair(object, maximum_slots)?;
                (false, scalar, book)
            }
        };
        let resource = crate::facility::facility_resource(object)?;
        let id = ReservationId::derive(holder, &resource).map_err(|_| IndustryError::Identity)?;
        let book = retire_terminal_capacity(book, scalar, id)?;
        let record = ReservationRecord::held(
            id,
            resource,
            holder.clone(),
            1,
            mush_inventory::reservation_deadline(now).map_err(IndustryError::Inventory)?,
        )?;
        let book = book.checked_hold(book.revision(), scalar, record, now)?;
        self.write_capacity_pair(txn, key, existed, scalar, book)?;
        if txn.get(&self.capacity_index, &id)?.is_some() {
            txn.replace(&self.capacity_index, &id, key)?;
        } else {
            txn.insert(&self.capacity_index, &id, key)?;
        }
        Ok(id)
    }

    /// Release one held slot — the terminal transition's capacity step, and
    /// cancellation's. Idempotent by K's own rule: a second release finds a
    /// terminal record and is refused, never re-applied.
    ///
    /// # Errors
    /// [`IndustryError`] on any refusal; **nothing is written on an error**.
    pub fn release_slot(
        &self,
        txn: &mut ActionTxn<'_>,
        object: ObjId,
        reservation: ReservationId,
        expected_reservation_revision: u64,
    ) -> Result<(), IndustryError> {
        let (key, pair) = self.load_capacity_pair(txn, object)?;
        let Some((scalar, book)) = pair else {
            return Err(IndustryError::NoCapacity);
        };
        let book = book.checked_release(
            book.revision(),
            scalar,
            reservation,
            expected_reservation_revision,
        )?;
        self.write_capacity_pair(txn, key, true, scalar, book)
    }

    /// Expire one elapsed slot hold.
    ///
    /// # Errors
    /// [`IndustryError`] on any refusal.
    pub fn expire_slot(
        &self,
        txn: &mut ActionTxn<'_>,
        object: ObjId,
        reservation: ReservationId,
        expected_reservation_revision: u64,
        now: &ClockStamp,
    ) -> Result<(), IndustryError> {
        let (key, pair) = self.load_capacity_pair(txn, object)?;
        let Some((scalar, book)) = pair else {
            return Err(IndustryError::NoCapacity);
        };
        let book = book.checked_expire(
            book.revision(),
            scalar,
            reservation,
            expected_reservation_revision,
            now,
        )?;
        self.write_capacity_pair(txn, key, true, scalar, book)
    }

    /// One slot hold's exact record, if the facility has it.
    ///
    /// # Errors
    /// [`IndustryError`] on a half pair or an unreadable family.
    pub fn slot_reservation(
        &self,
        txn: &mut ActionTxn<'_>,
        object: ObjId,
        reservation: ReservationId,
    ) -> Result<Option<ReservationRecord>, IndustryError> {
        let (_, pair) = self.load_capacity_pair(txn, object)?;
        Ok(pair.and_then(|(_, book)| {
            book.records()
                .iter()
                .find(|record| record.id() == reservation)
                .cloned()
        }))
    }

    // ---- E7c4 B3: work orders ----

    /// One work order, if it exists.
    ///
    /// # Errors
    /// [`IndustryError::State`] if the family is unreadable.
    pub fn work_order(
        &self,
        txn: &mut ActionTxn<'_>,
        id: crate::workorder::WorkOrderId,
    ) -> Result<Option<crate::workorder::WorkOrder>, IndustryError> {
        Ok(txn.get(&self.work_orders, &id)?.cloned())
    }

    /// Insert one fresh work order (a draft).
    ///
    /// # Errors
    /// [`IndustryError::WorkOrderExists`] on a duplicate id.
    pub fn open_work_order(
        &self,
        txn: &mut ActionTxn<'_>,
        order: crate::workorder::WorkOrder,
    ) -> Result<(), IndustryError> {
        if txn.get(&self.work_orders, &order.id())?.is_some() {
            return Err(IndustryError::WorkOrderExists);
        }
        txn.insert(&self.work_orders, &order.id(), order)?;
        Ok(())
    }

    /// Replace one work order under a checked revision.
    ///
    /// # Errors
    /// [`IndustryError::NoWorkOrder`] on a missing order,
    /// [`IndustryError::StaleRevision`] on a mismatch.
    pub fn put_work_order(
        &self,
        txn: &mut ActionTxn<'_>,
        expected_revision: u64,
        order: crate::workorder::WorkOrder,
    ) -> Result<(), IndustryError> {
        let live = self
            .work_order(txn, order.id())?
            .ok_or(IndustryError::NoWorkOrder)?;
        if live.revision() != expected_revision {
            return Err(IndustryError::StaleRevision);
        }
        txn.replace(&self.work_orders, &order.id(), order)?;
        Ok(())
    }

    /// Tear the production schedule and its job down (R1, independent verify
    /// fold): a repeating cadence schedule never auto-completes, so a terminal or
    /// awaiting-delivery order must remove both halves in the same bundle, or the
    /// next cadence tick discovers a candidate `wo_advance` refuses with a
    /// non-retryable `ProviderRejected` — which the driver classifies as a hard
    /// error and freezes the whole world. Idempotent: a second teardown finds
    /// nothing and removes nothing (the `handoff.rs` cancel-path shape).
    ///
    /// # Errors
    /// [`IndustryError`] on a family read/remove failure.
    pub fn tear_down_production(
        &self,
        txn: &mut ActionTxn<'_>,
        schedule_id: mush_core::ScheduleId,
    ) -> Result<(), IndustryError> {
        if txn.get(&self.jobs, &schedule_id)?.is_some() {
            txn.remove(&self.jobs, &schedule_id)?;
        }
        if txn.get(&self.schedules, &schedule_id)?.is_some() {
            txn.remove(&self.schedules, &schedule_id)?;
        }
        Ok(())
    }

    /// Record one work-order completion marker, keeping `industry.wo_applied`
    /// **bounded per facility** (R3, independent verify fold): the marker rides the
    /// terminal bundle, and an evicted (oldest) marker for the same facility is
    /// deleted, exactly as [`run_cycle`](Self::run_cycle) does for the extraction
    /// family. This cannot break replay idempotency: `wo_deliver` refuses an
    /// existing marker and the request-derived lot id is the durable replay guard,
    /// so the ring may retire an old marker the delivery path will never read again.
    ///
    /// # Errors
    /// [`IndustryError`] on any family write failure; the caller has already
    /// refused a duplicate marker.
    pub fn record_wo_completion(
        &self,
        txn: &mut ActionTxn<'_>,
        facility: ObjId,
        marker_key: SmolStr,
        marker: crate::workorder::WorkOrderMarker,
    ) -> Result<(), IndustryError> {
        txn.insert(&self.wo_applied, &marker_key, marker)?;
        let ring = txn
            .get(&self.wo_marker_ring, &facility)?
            .cloned()
            .unwrap_or_else(crate::record::MarkerRing::empty);
        let ring_existed = txn.get(&self.wo_marker_ring, &facility)?.is_some();
        let (ring, evicted) = ring.push(marker_key);
        if let Some(old) = evicted
            && txn.get(&self.wo_applied, &old)?.is_some()
        {
            txn.remove(&self.wo_applied, &old)?;
        }
        if ring_existed {
            txn.replace(&self.wo_marker_ring, &facility, ring)?;
        } else {
            txn.insert(&self.wo_marker_ring, &facility, ring)?;
        }
        Ok(())
    }

    // ---- the extraction cycle (B6) ----

    /// Run **one** extraction cycle as one bundle.
    ///
    /// The exact write set, in order: the deposit endpoint's
    /// `(inventory.pool, inventory.book)` pair debited by this cycle's yield,
    /// one `Lot` minted at the operator's endpoint with
    /// `Lineage::Origin { source }`, that endpoint's `inventory.lot_census`,
    /// the run's advanced post-image, and the applied marker
    /// [`FACT_EXTRACTION_CYCLED`] is answered from — **two owners' families in
    /// one `ActionTxn`**. All of it is transaction scratch until the receipt
    /// installs it, so a crash mid-cycle produces **no lot or exactly one** and
    /// never a debited reserve with no lot.
    ///
    /// Cut 3 performs **no arithmetic**: the one quantity mutation is cut 2's
    /// `debit`, which rides `ReservationBook::checked_apply_resource` — the
    /// call that re-checks bounds, checked arithmetic, the scalar revision
    /// **and post-mutation availability** in one place, where availability is
    /// balance minus every live hold. A cycle therefore cannot draw into
    /// somebody else's reservation, and "two reservations against one remaining
    /// unit" stays K's `checked_hold` refusal rather than a check here.
    ///
    /// # Errors
    /// [`IndustryError`] for a depleted or unworkable body, a stale revision, a
    /// duplicate request, or any inventory refusal. **Nothing is written on an
    /// error.**
    #[allow(clippy::too_many_arguments)]
    pub fn run_cycle(
        &self,
        txn: &mut ActionTxn<'_>,
        deposit: &OreDeposit,
        run: &ExtractionRun,
        actor: ObjId,
        request: RequestUid,
        // The acting principal's scoped key and this commit's token. Neither is
        // read by the cycle itself — the pool is debited once and there is no
        // hold to hold — and both are taken so cut 4's work order, which DOES
        // hold capacity across time, calls the same shape.
        _holder: &ScopedIdempotencyKey,
        now: &ClockStamp,
        _commit: CommitToken,
        source: ExternalRef,
    ) -> Result<Cycled, IndustryError> {
        // The pre-image the whole cycle is computed from, read once.
        let live = self
            .deposit(txn, deposit.id())?
            .ok_or(IndustryError::NoDeposit)?;
        if live.revision() != deposit.revision() {
            return Err(IndustryError::StaleRevision);
        }
        let live_run = self.run(txn, run.deposit())?.ok_or(IndustryError::NoRun)?;
        if live_run.revision() != run.revision() || live_run.next_cycle() != run.next_cycle() {
            return Err(IndustryError::StaleRevision);
        }
        if live_run.state() != crate::record::RunState::Running {
            return Err(IndustryError::RunStopped);
        }
        // E7c3 verify fold C2 / D1(a): the cycling actor must be the run's own
        // operator, so a shared-claim co-tenant cannot cycle a run that is not
        // theirs and mint under the incumbent's name. The run's operator is the
        // object that opened it (`start` builds `PrincipalId::Object(actor)`), so
        // this is a world-free equality — the offer's licence/claim gate lives in
        // `project_offer`; this closes the run itself. Renders as the missing-run
        // refusal.
        if live_run.operator() != &PrincipalId::Object(actor) {
            return Err(IndustryError::RunNotYours);
        }
        if !live.accessibility().is_workable() {
            return Err(IndustryError::Unworkable);
        }
        let remaining = self.remaining(txn, &live)?;
        // Arithmetic, not a roll: the curve at the current cumulative
        // extraction, scaled by accessibility, clamped to what is left.
        let quantity = live.next_yield(remaining);
        if quantity == 0 {
            return Err(IndustryError::Depleted);
        }

        // The marker is the epoch-free replay guard (cut 2's fold R1(a)): it
        // REFUSES an existing marker rather than rewriting it, so a retry after
        // a reboot cannot extract twice.
        let marker_key = marker_id(request, now.at);
        if txn.get(&self.applied, &marker_key)?.is_some() {
            return Err(IndustryError::AlreadyApplied);
        }

        // Every quantity mutation is cut 2's, under cut 2's gate — and there
        // is exactly **one** of them, because cut 2 writes the
        // `(inventory.pool, inventory.book)` pair together and an `ActionTxn`
        // admits one mutation per key per transaction. A `reserve` followed by
        // a `consume` inside one bundle would be a `DuplicateMutation`, and it
        // would not be a hold either: a hold exists to carry quantity across
        // *time*, between an order and its fulfilment, and a cycle has no such
        // gap.
        //
        // The over-draw gate is unchanged and is still K's:
        // `ReservationBook::checked_apply_resource` re-checks bounds, checked
        // arithmetic, the scalar revision **and post-mutation availability** in
        // one call, and availability is balance minus every live hold — so a
        // cycle can never draw into somebody else's reservation, and two
        // concurrent cycles serialize on the store's one pending-intent slot
        // with the second reading the already-debited pool.
        self.inventory
            .debit(txn, live.endpoint(), live.commodity(), quantity)?;
        // The lot IS the delivery: `mint_origin` puts the extracted matter
        // straight into a lot at the operator's endpoint, so the cycle never
        // credits and then debits one pool inside one transaction (which is a
        // `DuplicateMutation` and an intermediate balance that never existed).
        // Conservation is this bundle: the unit consumed above is the unit
        // minted here.
        let lot = derive_lot(request)?;
        self.inventory.mint_origin(
            txn,
            live_run.facility_endpoint(),
            live.commodity(),
            quantity,
            live.grade(),
            source,
            lot,
        )?;

        let advanced = live_run.cycled()?;
        txn.replace(&self.runs, &live_run.deposit(), advanced)?;
        let marker = AppliedMarker::new(
            FACT_EXTRACTION_CYCLED,
            live.id(),
            live_run.next_cycle(),
            live.commodity().clone(),
            quantity,
            live_run.facility_endpoint(),
            actor,
            now.at,
        )?;
        txn.insert(&self.applied, &marker_key, marker)?;
        // D2 (discharged in cut 4): keep the durable `industry.applied` family
        // bounded per deposit. The ring rides this same bundle; an evicted
        // (oldest) marker is deleted. This cannot break replay idempotency: a
        // replayed old cycle refuses at the `next_cycle`/revision check above long
        // before its marker would be read, so the marker is a fast guard the ring
        // may retire while the run's monotonicity remains the durable one.
        let ring = txn
            .get(&self.marker_ring, &live.id())?
            .cloned()
            .unwrap_or_else(crate::record::MarkerRing::empty);
        let ring_existed = txn.get(&self.marker_ring, &live.id())?.is_some();
        let (ring, evicted) = ring.push(marker_key);
        if let Some(old) = evicted
            && txn.get(&self.applied, &old)?.is_some()
        {
            txn.remove(&self.applied, &old)?;
        }
        if ring_existed {
            txn.replace(&self.marker_ring, &live.id(), ring)?;
        } else {
            txn.insert(&self.marker_ring, &live.id(), ring)?;
        }
        Ok(Cycled {
            cycle: live_run.next_cycle(),
            quantity,
            lot,
            remaining: remaining.saturating_sub(quantity),
        })
    }
}

/// Retire the terminal capacity record for one holder before re-holding, and
/// keep the book's terminal tail bounded (cut 2's fold C3, applied to capacity):
/// without it a facility that has run and released `MAX_RESERVATION_BOOK_RECORDS`
/// orders would refuse the next slot for ever.
fn retire_terminal_capacity(
    book: ReservationBook,
    scalar: BoundedScalar,
    holder: ReservationId,
) -> Result<ReservationBook, IndustryError> {
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
        if book.records().iter().filter(terminal).count()
            <= mush_inventory::MAX_RETAINED_TERMINAL_RESERVATIONS
        {
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

/// Derive the lot identity one cycle mints.
///
/// # Errors
/// [`IndustryError::Identity`] if the canonical value cannot be hashed.
pub fn derive_lot(request: RequestUid) -> Result<LotId, IndustryError> {
    mush_inventory::derive_lot_id(request, 0).map_err(IndustryError::Inventory)
}

/// The commodity a `CanonicalId` names, parsed once.
///
/// # Errors
/// [`IndustryError::Identity`] for a non-canonical id.
pub fn commodity(text: &str) -> Result<CanonicalId, IndustryError> {
    CanonicalId::new(text).map_err(|_| IndustryError::Identity)
}

/// The grade a scalar names, checked once.
///
/// # Errors
/// [`IndustryError::Record`] above cut 2's `MAX_GRADE`.
pub fn grade(value: u8) -> Result<Grade, IndustryError> {
    Grade::new(value).map_err(|_| IndustryError::Identity)
}

/// One endpoint, from a vault id.
#[must_use]
pub const fn endpoint(vault: mush_economy::VaultId) -> EndpointRef {
    EndpointRef(vault)
}

/// Every refusal this crate can produce. Exhaustive, and none of them leaves a
/// partial write.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum IndustryError {
    /// A record refused its own invariants.
    #[error(transparent)]
    Record(#[from] IndustryRecordError),
    /// Cut 2 refused the quantity mutation.
    #[error(transparent)]
    Inventory(#[from] InventoryError),
    /// Typed state was unreadable or unwritable.
    #[error("industry typed state is unavailable")]
    State,
    /// An id could not be derived.
    #[error("an industry identity could not be derived")]
    Identity,
    /// No such ore body.
    #[error("no such ore body")]
    NoDeposit,
    /// That ore body already exists.
    #[error("that ore body already exists")]
    DepositExists,
    /// No such claim.
    #[error("no such claim")]
    NoClaim,
    /// That claim already exists.
    #[error("that claim already exists")]
    ClaimExists,
    /// No extraction run at that body.
    #[error("no extraction run at that ore body")]
    NoRun,
    /// The run is stopped.
    #[error("that extraction run is stopped")]
    RunStopped,
    /// A live run at that body belongs to a different operator (E7c3 verify
    /// fold C2): starting would redirect it, cycling would mint under its
    /// operator's name. Renders as the missing-run refusal.
    #[error("that extraction run is not yours")]
    RunNotYours,
    /// The body is collapsed, suspended or otherwise unworkable right now.
    #[error("that ore body cannot be worked right now")]
    Unworkable,
    /// The body is exhausted.
    #[error("that ore body is exhausted")]
    Depleted,
    /// A checked revision disagreed with live state.
    #[error("that record changed underneath this action")]
    StaleRevision,
    /// The request already has an applied marker.
    #[error("that request has already been applied")]
    AlreadyApplied,
    /// A capacity book/scalar pair was half-present — the fail-closed shape.
    #[error("that facility's capacity state is half-present")]
    CapacityHalf,
    /// No capacity book at that facility (a release/expire with no live book).
    #[error("that facility has no capacity book")]
    NoCapacity,
    /// K refused a capacity book transition (contention, revision, deadline).
    #[error(transparent)]
    Book(#[from] ReservationBookError),
    /// K refused a reservation-record transition.
    #[error(transparent)]
    Reservation(#[from] mush_core::ReservationError),
    /// No such work order.
    #[error("no such work order")]
    NoWorkOrder,
    /// That work order already exists.
    #[error("that work order already exists")]
    WorkOrderExists,
}

impl From<ActionStateError> for IndustryError {
    fn from(_source: ActionStateError) -> Self {
        Self::State
    }
}

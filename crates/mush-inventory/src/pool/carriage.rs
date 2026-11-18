//! **Carriage: goods moved under a job's hold** (doc 48 cut EX.6; Q-48-42, doc 48b W-48-3b
//! and W-48-3c).
//!
//! A haul's cargo travels reserved to the job. Inventory had no call that moves a pool hold
//! from one endpoint to another: a hold belongs to one endpoint's book, and an [`ActionTxn`]
//! writes one key once, so a `consume` then a `transfer` (or a `credit` then a `reserve_for`)
//! of the same pool cannot share a bundle. [`InventoryFamilies::carry`] is the one call that
//! does each leg in a single write per pool:
//!
//! - **the source**: the job's hold there is consumed (the pool decremented by the hold), or,
//!   with no hold named, the amount is debited from what is available. A hold whose deadline
//!   has passed is expired and the amount debited, so goods held past their deadline still
//!   move back (the deadline return).
//! - **the destination**: the amount credited and, when the leg keeps the hold, placed under
//!   the job's holder with a deadline that outlives the job ([`reservation_deadline_after`]).
//! - **the fact**: one `inventory.transferred` marker under a caller-derived id
//!   ([`derived_marker_id`](crate::derived_marker_id)), retained in the source's ring. An
//!   existing marker refuses (the replay guard), so a leg lands once.
//!
//! It mints nothing: the source loses exactly what the destination gains, and the lineage is
//! carried as [`InventoryFamilies::transfer`] carries it.

use mush_core::{
    ActionTxn, CanonicalId, ClockStamp, CommitToken, ReservationId, ReservationRecord,
};
use mush_core::{ReservationState, ScopedIdempotencyKey};

use super::{
    InventoryError, InventoryFamilies, is_marker_id, pool_resource, reservation_deadline_after,
    retire_terminal,
};
use crate::record::{EndpointRef, InventoryRecordError};

/// **One carriage leg** (doc 48 cut EX.6): what [`InventoryFamilies::carry`] moves.
#[derive(Clone, Debug)]
pub struct Carriage<'a> {
    /// Where the goods are.
    pub from: EndpointRef,
    /// Where they go. Must differ from `from`.
    pub to: EndpointRef,
    /// The good.
    pub commodity: &'a CanonicalId,
    /// How much.
    pub amount: u64,
    /// The job's hold at `from`, consumed by the move; `None` debits what is available.
    pub held: Option<ReservationId>,
    /// The holder to keep the goods reserved to at `to`, and the IC seconds of work the hold
    /// must outlive (on top of the inventory TTL); `None` lands them free.
    pub keep: Option<(&'a ScopedIdempotencyKey, i64)>,
    /// The applied marker's caller-derived id.
    pub marker: &'a smol_str::SmolStr,
    /// Who moved them.
    pub actor: mush_core::ObjId,
    /// The bundle's clock.
    pub now: &'a ClockStamp,
    /// The offer's real-clock stamp, for the marker ring.
    pub wall: i64,
    /// The bundle's commit token, for a consume.
    pub commit: CommitToken,
}

impl InventoryFamilies {
    /// **Carry one leg** (doc 48 cut EX.6): the source's hold consumed (or the amount debited),
    /// the destination credited and, when `keep` names a holder, reserved to it, and the
    /// marker written, in one bundle. Answers the destination's hold when one was placed.
    ///
    /// # Errors
    ///
    /// [`InventoryError::SameEndpoint`] for `from == to`; [`InventoryError::Identity`] for a
    /// malformed marker id; [`InventoryError::AlreadyExists`] for a marker already applied;
    /// [`InventoryError::NoPool`] for no pool at the source; [`InventoryError::NoReservation`]
    /// for a named hold the source does not have; [`InventoryError::Insufficient`] when the
    /// hold is not of `amount`, or (with no hold) less than `amount` is available; every
    /// refusal of the book, the credit, the hold, the lineage and the marker ring. **Nothing
    /// is written on an error.**
    pub fn carry(
        &self,
        txn: &mut ActionTxn<'_>,
        leg: &Carriage<'_>,
    ) -> Result<Option<ReservationId>, InventoryError> {
        if leg.from == leg.to {
            return Err(InventoryError::SameEndpoint);
        }
        if leg.amount == 0 {
            return Err(InventoryError::Record(InventoryRecordError::ZeroQuantity));
        }
        if !is_marker_id(leg.marker) {
            return Err(InventoryError::Identity);
        }
        if txn.get(&self.applied, leg.marker)?.is_some() {
            return Err(InventoryError::AlreadyExists);
        }
        let marker = crate::record::AppliedMarker::new(
            crate::record::FACT_TRANSFERRED,
            Some(leg.from),
            Some(leg.to),
            leg.commodity.clone(),
            leg.amount,
            leg.actor,
            leg.now.at,
        )?;
        // 1. The source: one write of its pair.
        let (from_key, pair) = self.load_pair(txn, leg.from, leg.commodity)?;
        let Some((scalar, book)) = pair else {
            return Err(InventoryError::NoPool);
        };
        let (book, scalar) = match leg.held {
            Some(id) => {
                let record: ReservationRecord = book
                    .records()
                    .iter()
                    .find(|r| r.id() == id && r.state() == ReservationState::Held)
                    .cloned()
                    .ok_or(InventoryError::NoReservation)?;
                if record.amount() != leg.amount {
                    return Err(InventoryError::Insufficient {
                        available: record.amount(),
                        requested: leg.amount,
                    });
                }
                let lapsed = record
                    .expires()
                    .checked_cmp(leg.now)
                    .map_err(|_| InventoryError::ClockArithmetic)?
                    != std::cmp::Ordering::Greater;
                if lapsed {
                    // The deadline return: the hold has lapsed, so it is expired and the goods
                    // it held are debited as available stock, in the same write.
                    let book = book.checked_expire(
                        book.revision(),
                        scalar,
                        id,
                        record.revision(),
                        leg.now,
                    )?;
                    book.checked_apply_resource(
                        book.revision(),
                        scalar,
                        mush_core::ResourceOperation::Decrease { amount: leg.amount },
                    )?
                } else {
                    book.checked_consume(
                        book.revision(),
                        scalar,
                        id,
                        record.revision(),
                        leg.now,
                        leg.commit,
                    )?
                }
            }
            None => book.checked_apply_resource(
                book.revision(),
                scalar,
                mush_core::ResourceOperation::Decrease { amount: leg.amount },
            )?,
        };
        book.checked_available(scalar)?;
        self.write_pair(txn, from_key, true, scalar, book)?;
        // 2. The destination: one write of its pair, credited and (when kept) held.
        let (to_key, pair) = self.load_pair(txn, leg.to, leg.commodity)?;
        let existed = pair.is_some();
        let (scalar, book) = match pair {
            Some(pair) => pair,
            None => {
                self.add_pool(txn, leg.to)?;
                self.fresh_pair(leg.to, leg.commodity)?
            }
        };
        let (book, scalar) = book.checked_apply_resource(
            book.revision(),
            scalar,
            mush_core::ResourceOperation::Increase { amount: leg.amount },
        )?;
        let (book, placed) = match leg.keep {
            Some((holder, lead)) => {
                let resource = pool_resource(leg.to, leg.commodity)?;
                let id = ReservationId::derive(holder, &resource)
                    .map_err(|_| InventoryError::Identity)?;
                let book = retire_terminal(book, scalar, id)?;
                let record = ReservationRecord::held(
                    id,
                    resource,
                    holder.clone(),
                    leg.amount,
                    reservation_deadline_after(leg.now, lead)?,
                )
                .map_err(InventoryError::Reservation)?;
                (
                    book.checked_hold(book.revision(), scalar, record, leg.now)?,
                    Some(id),
                )
            }
            None => (book, None),
        };
        book.checked_available(scalar)?;
        self.write_pair(txn, to_key, existed, scalar, book)?;
        if let Some(id) = placed {
            if txn.get(&self.reservations, &id)?.is_some() {
                txn.replace(&self.reservations, &id, to_key)?;
            } else {
                txn.insert(&self.reservations, &id, to_key)?;
            }
        }
        // 3. The lineage, as a transfer carries it; 4. the fact.
        self.carry_lineage(txn, leg.from, leg.to, leg.commodity, leg.amount)?;
        txn.insert(&self.applied, leg.marker, marker)?;
        self.retain_marker(txn, leg.from, leg.marker, leg.wall)?;
        Ok(placed)
    }
}

#[cfg(test)]
mod tests {
    use super::Carriage;
    use crate::pool::{InventoryError, InventoryFamilies};
    use crate::record::EndpointRef;
    use crate::{InventoryHandles, InventoryModule};
    use mush_core::{
        ActionTxn, CanonicalHash, CanonicalId, ClockStamp, CommitToken, Engine, IdempotencyKey,
        IdempotencyKeyText, IdempotencyScope, PrincipalId, ScopedIdempotencyKey,
    };
    use mush_economy::VaultId;
    use mush_testkit::TestWorld;

    fn bundle<T>(
        engine: &mut Engine,
        families: &InventoryFamilies,
        body: impl FnOnce(&mut ActionTxn<'_>) -> Result<T, InventoryError>,
    ) -> Result<T, InventoryError> {
        let (out, prepared) = {
            let mut txn = engine
                .action_transaction(&families.pools)
                .expect("inventory transaction");
            let out = body(&mut txn)?;
            (out, txn.prepare().expect("prepares"))
        };
        let (_, install) = prepared.into_parts();
        engine.install_prepared_action(install).expect("installs");
        Ok(out)
    }

    fn holder() -> ScopedIdempotencyKey {
        let owner = crate::record::owner();
        let principal = PrincipalId::Scoped {
            owner: owner.clone(),
            class: CanonicalId::new("haul").expect("class"),
            id: IdempotencyKeyText::parse_canonical("haul0000000000000007").expect("id"),
        };
        let scope = IdempotencyScope {
            principal,
            owner,
            namespace: CanonicalId::new("inventory").expect("namespace"),
        };
        let key = CanonicalHash::for_canonical_value(
            "test.haul_holder.v1",
            1,
            &serde_json::json!({"job": 7}),
            1024,
        )
        .expect("key");
        ScopedIdempotencyKey::new(scope, 0, IdempotencyKey::Deterministic(key))
    }

    fn marker(n: u64) -> smol_str::SmolStr {
        crate::derived_marker_id(1, &serde_json::json!({"test": n})).expect("marker")
    }

    fn quantity(
        engine: &mut Engine,
        families: &InventoryFamilies,
        at: EndpointRef,
        good: &CanonicalId,
    ) -> (u64, u64) {
        let mut txn = engine
            .action_transaction(&families.pools)
            .expect("inventory transaction");
        families
            .pool(&mut txn, at, good)
            .expect("the pool reads")
            .map_or((0, 0), |pool| (pool.quantity, pool.available))
    }

    /// **Doc 48 cut EX.6: a haul's three legs.** The load debits the shipper's locker and lands
    /// the cargo held in the hold; the hauler cannot debit it; the delivery consumes the hold
    /// and lands it free at the consignee; a replay of a marker refuses and writes nothing; and
    /// a hold past its deadline still carries back (the deadline return).
    #[test]
    fn ex6_a_held_cargo_moves_only_by_its_legs() {
        let mut world = TestWorld::with_modules(vec![Box::new(InventoryModule::new())]);
        let families = world
            .world()
            .module_state::<InventoryHandles>()
            .expect("inventory publishes its handles")
            .0
            .clone();
        let now = world.world().world_ic_now();
        let engine = world.engine_mut();
        let ore = CanonicalId::new("ex6.ore").expect("good");
        let (locker, hold, dest) = (
            EndpointRef(VaultId(1)),
            EndpointRef(VaultId(2)),
            EndpointRef(VaultId(3)),
        );
        bundle(engine, &families, |txn| {
            families.credit(txn, locker, &ore, 20)
        })
        .expect("the shipper's stock");
        let holder = holder();
        let leg = |from, to, held, keep, n| Carriage {
            from,
            to,
            commodity: &ore,
            amount: 6,
            held,
            keep,
            marker: Box::leak(Box::new(marker(n))),
            actor: mush_core::ObjId(9),
            now: Box::leak(Box::new(now.clone())),
            wall: 1,
            commit: CommitToken::from_bytes([n as u8; 16]),
        };
        let placed = bundle(engine, &families, |txn| {
            families.carry(txn, &leg(locker, hold, None, Some((&holder, 86_400)), 1))
        })
        .expect("the load")
        .expect("a hold is placed");
        assert_eq!(quantity(engine, &families, locker, &ore), (14, 14));
        assert_eq!(
            quantity(engine, &families, hold, &ore),
            (6, 0),
            "aboard and held: nothing is the hauler's to spend"
        );
        assert!(
            bundle(engine, &families, |txn| families.debit(txn, hold, &ore, 1)).is_err(),
            "the hauler cannot take held cargo"
        );
        assert!(
            bundle(engine, &families, |txn| {
                families.carry(txn, &leg(locker, hold, None, Some((&holder, 0)), 1))
            })
            .is_err(),
            "a replayed marker refuses, though the stock is there"
        );
        bundle(engine, &families, |txn| {
            families.carry(txn, &leg(hold, dest, Some(placed), None, 2))
        })
        .expect("the delivery");
        assert_eq!(quantity(engine, &families, hold, &ore), (0, 0));
        assert_eq!(quantity(engine, &families, dest, &ore), (6, 6));
        // The deadline return: a hold already past its deadline still carries back.
        let placed = bundle(engine, &families, |txn| {
            let mut stale = leg(dest, hold, None, Some((&holder, 0)), 3);
            let past = Box::leak(Box::new(ClockStamp {
                clock: now.clock.clone(),
                at: now.at - crate::record::RESERVATION_TTL_SECS - 10,
            }));
            stale.now = past;
            families.carry(txn, &stale)
        })
        .expect("a hold placed in the past")
        .expect("placed");
        bundle(engine, &families, |txn| {
            families.carry(txn, &leg(hold, locker, Some(placed), None, 4))
        })
        .expect("the lapsed hold carries back");
        assert_eq!(quantity(engine, &families, locker, &ore), (20, 20));
        assert_eq!(quantity(engine, &families, hold, &ore), (0, 0));
    }
}

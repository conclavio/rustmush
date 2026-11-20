//! **Seizure: a pool drawn down past its holds** (doc 50 unit CR; Q-50-25 and Q-50-44 as
//! corrected, doc 50a A-5).
//!
//! An authority that takes goods takes what is there, held or not. [`InventoryFamilies::debit`]
//! is refused below what is *available*, so goods reserved to a job could not be taken, and
//! because an [`ActionTxn`] writes one key once a `release` then a `debit` of the same pool
//! cannot share a bundle. [`InventoryFamilies::seize`] is the one call that does both in a
//! single write of the pair: it releases the named holds, only as many as the draw needs and
//! in the order given, and then debits the amount from what is available. It answers what it
//! took and which holds it released, so the caller can fail the jobs those holds belonged to in
//! the same transaction.
//!
//! **Where the goods go** (doc 50 unit LW.5; Q-50-44 as corrected): given a destination, the
//! taken units are credited there in the same bundle, with their lineage carried as
//! [`InventoryFamilies::transfer`] carries it, so a seizure is a transfer and mints and destroys
//! nothing. With none, the units leave the world (CR's behavior).

use mush_core::{ActionTxn, CanonicalId, ReservationId, ReservationState};

use super::{InventoryError, InventoryFamilies};
use crate::record::EndpointRef;

/// **What a seizure did** ([`InventoryFamilies::seize`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Seized {
    /// The units taken out of the pool.
    pub taken: u64,
    /// The holds released so they could be taken, in the order released.
    pub released: Vec<ReservationId>,
}

impl InventoryFamilies {
    /// **Seize up to `amount` from one pool** (doc 50 unit CR): release the live holds named in
    /// `release`, in order and only while what is available falls short of `amount`, then
    /// debit `amount`, or what is then available when that is less, in one write of the pair.
    /// A named hold the book does not hold live is passed over. No pool, or nothing available
    /// once the holds are released, takes nothing and writes nothing. With `into`, the taken
    /// units are credited to that endpoint's pool (made when absent) and their lineage carried,
    /// one write of each pair; `into` must differ from `endpoint`.
    ///
    /// # Errors
    ///
    /// [`InventoryError`] on a half pool, an unreadable family, or any refusal of the book's
    /// arithmetic. **Nothing is written on an error.**
    pub fn seize(
        &self,
        txn: &mut ActionTxn<'_>,
        endpoint: EndpointRef,
        commodity: &CanonicalId,
        amount: u64,
        release: &[ReservationId],
        into: Option<EndpointRef>,
    ) -> Result<Seized, InventoryError> {
        if into == Some(endpoint) {
            return Err(InventoryError::SameEndpoint);
        }
        let (key, pair) = self.load_pair(txn, endpoint, commodity)?;
        let Some((scalar, mut book)) = pair else {
            return Ok(Seized::default());
        };
        if amount == 0 {
            return Ok(Seized::default());
        }
        let mut released = Vec::new();
        for id in release {
            if book.checked_available(scalar)? >= amount {
                break;
            }
            let Some(record) = book
                .records()
                .iter()
                .find(|r| r.id() == *id && r.state() == ReservationState::Held)
                .cloned()
            else {
                continue;
            };
            book = book.checked_release(book.revision(), scalar, *id, record.revision())?;
            released.push(*id);
        }
        let taken = amount.min(book.checked_available(scalar)?);
        if taken == 0 {
            return Ok(Seized::default());
        }
        let (book, scalar) = book.checked_apply_resource(
            book.revision(),
            scalar,
            mush_core::ResourceOperation::Decrease { amount: taken },
        )?;
        book.checked_available(scalar)?;
        self.write_pair(txn, key, true, scalar, book)?;
        if let Some(to) = into {
            let (to_key, pair) = self.load_pair(txn, to, commodity)?;
            let existed = pair.is_some();
            let (scalar, book) = match pair {
                Some(pair) => pair,
                None => {
                    self.add_pool(txn, to)?;
                    self.fresh_pair(to, commodity)?
                }
            };
            let (book, scalar) = book.checked_apply_resource(
                book.revision(),
                scalar,
                mush_core::ResourceOperation::Increase { amount: taken },
            )?;
            book.checked_available(scalar)?;
            self.write_pair(txn, to_key, existed, scalar, book)?;
            self.carry_lineage(txn, endpoint, to, commodity, taken)?;
        }
        Ok(Seized { taken, released })
    }
}

#[cfg(test)]
mod tests {
    use crate::pool::carriage::Carriage;
    use crate::pool::{InventoryError, InventoryFamilies};
    use crate::record::EndpointRef;
    use crate::{InventoryHandles, InventoryModule};
    use mush_core::{
        ActionTxn, CanonicalHash, CanonicalId, CommitToken, Engine, IdempotencyKey,
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

    fn holder(job: u64) -> ScopedIdempotencyKey {
        let owner = crate::record::owner();
        let principal = PrincipalId::Scoped {
            owner: owner.clone(),
            class: CanonicalId::new("haul").expect("class"),
            id: IdempotencyKeyText::parse_canonical(format!("haul{job:016x}")).expect("id"),
        };
        let scope = IdempotencyScope {
            principal,
            owner,
            namespace: CanonicalId::new("inventory").expect("namespace"),
        };
        let key = CanonicalHash::for_canonical_value(
            "test.haul_holder.v1",
            1,
            &serde_json::json!({"job": job}),
            1024,
        )
        .expect("key");
        ScopedIdempotencyKey::new(scope, 0, IdempotencyKey::Deterministic(key))
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

    /// **Doc 50 unit CR (Q-50-25 as corrected): a seizure takes held cargo.** A hold carries
    /// 6 held for one job beside 2 free. A debit cannot reach the held 6; a seizure of 4 takes
    /// the free 2 and releases the job's hold to take the rest, in one write; a seizure of the
    /// rest takes all that is left and releases nothing more; a hold it is not told of stays.
    #[test]
    fn cr_a_seizure_releases_the_holds_it_needs_and_takes_held_cargo() {
        let mut world = TestWorld::with_modules(vec![Box::new(InventoryModule::new())]);
        let families = world
            .world()
            .module_state::<InventoryHandles>()
            .expect("inventory publishes its handles")
            .0
            .clone();
        let now = world.world().world_ic_now();
        let engine = world.engine_mut();
        let ore = CanonicalId::new("cr.ore").expect("good");
        let (locker, hold) = (EndpointRef(VaultId(1)), EndpointRef(VaultId(2)));
        bundle(engine, &families, |txn| {
            families.credit(txn, locker, &ore, 9)
        })
        .expect("the shipper's stock");
        let (job, other) = (holder(7), holder(8));
        let mut placed = Vec::new();
        for (n, who, amount) in [(1_u8, &job, 6_u64), (2, &other, 1)] {
            let marker = crate::derived_marker_id(1, &serde_json::json!({"cr": n})).expect("id");
            let leg = Carriage {
                from: locker,
                to: hold,
                commodity: &ore,
                amount,
                held: None,
                keep: Some((who, 86_400)),
                marker: &marker,
                actor: mush_core::ObjId(9),
                now: &now,
                wall: 1,
                commit: CommitToken::from_bytes([n; 16]),
            };
            placed.push(
                bundle(engine, &families, |txn| families.carry(txn, &leg))
                    .expect("the load")
                    .expect("a hold is placed"),
            );
        }
        bundle(engine, &families, |txn| families.credit(txn, hold, &ore, 2)).expect("free ore");
        assert_eq!(quantity(engine, &families, hold, &ore), (9, 2));
        assert!(
            bundle(engine, &families, |txn| families.debit(txn, hold, &ore, 4)).is_err(),
            "a debit cannot reach held cargo"
        );
        let first = bundle(engine, &families, |txn| {
            families.seize(txn, hold, &ore, 4, &placed[..1], None)
        })
        .expect("the seizure");
        assert_eq!((first.taken, first.released.clone()), (4, vec![placed[0]]));
        assert_eq!(
            quantity(engine, &families, hold, &ore),
            (5, 4),
            "the job's hold is released and four are gone; the other hold stands"
        );
        let vault = EndpointRef(VaultId(4));
        let second = bundle(engine, &families, |txn| {
            families.seize(txn, hold, &ore, 9, &placed[..1], Some(vault))
        })
        .expect("the rest");
        assert_eq!(
            (second.taken, second.released.len()),
            (4, 0),
            "a released hold is passed over, and an unnamed one is not taken"
        );
        assert_eq!(quantity(engine, &families, hold, &ore), (1, 0));
        assert_eq!(
            quantity(engine, &families, vault, &ore),
            (4, 4),
            "doc 50 unit LW.5: given a destination, what is taken lands there, free"
        );
        let mut txn = engine
            .action_transaction(&families.pools)
            .expect("inventory transaction");
        let none = families
            .seize(&mut txn, EndpointRef(VaultId(3)), &ore, 1, &[], None)
            .expect("no pool");
        assert_eq!(none, super::Seized::default(), "no pool takes nothing");
    }
}

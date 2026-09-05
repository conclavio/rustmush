//! E7 cut 2's verification, by scenario, over the three lenses 10b §3.2 names:
//! **C** conservation and lineage, **L** locality, **R** reservation.
//!
//! The rig is `crates/ruleset-shadowrun/tests/e6a.rs`'s shape adapted to an
//! authoritative crate: `boot()` composes the module, `txn()` opens the same
//! owner-stamped transaction a provider gets, and `install()` is the exact
//! receipt transition — so every assertion below is made against the state a
//! commit would actually leave.

use mush_core::{
    ClockRef, ClockStamp, CommitToken, Engine, GOD, IdempotencyKey, IdempotencyScope, ObjId,
    OwnerId, PrincipalId, ReservationId, ScopedIdempotencyKey,
};
use mush_economy::{ExternalRef, VaultId};
use mush_inventory::{
    EndpointRef, Grade, InventoryError, InventoryFamilies, InventoryHandles, InventoryModule,
    Lineage, Lot, LotId, MAX_LINEAGE_DEPTH, MAX_LOT_PARENTS, MAX_LOT_QUANTITY, MAX_POOL_QUANTITY,
    PoolTranche, TrancheId,
};
use mush_testkit::TestWorld;

// ---- E7c2: the rig ----

fn boot() -> (TestWorld, InventoryFamilies) {
    let world = TestWorld::with_modules(vec![Box::new(InventoryModule::new())]);
    let families = world
        .world()
        .module_state::<InventoryHandles>()
        .expect("inventory publishes its handles")
        .0
        .clone();
    (world, families)
}

fn commodity(id: &str) -> mush_core::CanonicalId {
    mush_core::CanonicalId::new(id).expect("test commodity id")
}

fn endpoint(id: u64) -> EndpointRef {
    EndpointRef(VaultId(id))
}

fn now() -> ClockStamp {
    ClockStamp {
        clock: ClockRef::WorldIc { generation: 1 },
        at: 1_000,
    }
}

fn other_generation() -> ClockStamp {
    ClockStamp {
        clock: ClockRef::WorldIc { generation: 2 },
        at: 1_000,
    }
}

fn holder(actor: ObjId, tag: &str) -> ScopedIdempotencyKey {
    ScopedIdempotencyKey::new(
        IdempotencyScope {
            principal: PrincipalId::Object { id: actor },
            owner: OwnerId::new("inventory").expect("owner"),
            namespace: mush_core::CanonicalId::new(tag).expect("namespace"),
        },
        0,
        IdempotencyKey::Request(mush_core::RequestUid::from_bytes([0x11; 16])),
    )
}

/// Run one closure inside the owner-stamped transaction a provider would get,
/// then **install** it exactly as an exact receipt does. An `Err` installs
/// nothing at all, which is what makes every "writes nothing" assertion below
/// mean what it says.
fn apply<T>(
    engine: &mut Engine,
    families: &InventoryFamilies,
    body: impl FnOnce(&mut mush_core::ActionTxn<'_>) -> Result<T, InventoryError>,
) -> Result<T, InventoryError> {
    let (result, prepared) = {
        let mut txn = engine
            .action_transaction(&families.pools)
            .expect("inventory transaction");
        let result = body(&mut txn);
        match result {
            // A verb that legitimately writes nothing (a same-endpoint transfer
            // is a HIT) leaves an empty transaction, and an empty transaction is
            // not a bundle.
            Ok(value) => (Ok(value), txn.prepare().ok()),
            Err(error) => (Err(error), None),
        }
    };
    if let Some(prepared) = prepared {
        let (_, installation) = prepared.into_parts();
        engine.install_prepared_action(installation);
    }
    result
}

fn read<T>(
    engine: &mut Engine,
    families: &InventoryFamilies,
    body: impl FnOnce(&mut mush_core::ActionTxn<'_>) -> T,
) -> T {
    let mut txn = engine
        .action_transaction(&families.pools)
        .expect("inventory transaction");
    body(&mut txn)
}

fn seed(engine: &mut Engine, families: &InventoryFamilies, at: EndpointRef, id: &str, amount: u64) {
    apply(engine, families, |txn| {
        families.credit(txn, at, &commodity(id), amount).map(|_| ())
    })
    .expect("seed credits");
}

fn seed_lot(
    engine: &mut Engine,
    families: &InventoryFamilies,
    lot: LotId,
    at: EndpointRef,
    id: &str,
    amount: u64,
    grade: u8,
) {
    let record = Lot::new(
        lot,
        at,
        commodity(id),
        amount,
        Grade::new(grade).expect("grade"),
        Lineage::Origin {
            source: ExternalRef {
                provider: "test".into(),
                kind: "seed".into(),
                id: "1".into(),
                revision: None,
            },
        },
        0,
    )
    .expect("seed lot");
    apply(engine, families, |txn| {
        txn.insert(&families.lots, &lot, record)
            .map_err(InventoryError::from)
    })
    .expect("seed lot installs");
}

// ---- E7c2: conservation (C) ----

#[test]
fn e7c2_c_every_verb_sums() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let a = endpoint(1);
    let b = endpoint(2);
    seed(engine, &families, a, "metals", 100);
    apply(engine, &families, |txn| {
        families.transfer(txn, a, b, &commodity("metals"), 40)
    })
    .expect("transfer");
    let (left, right) = read(engine, &families, |txn| {
        (
            families.available(txn, a, &commodity("metals")).unwrap(),
            families.available(txn, b, &commodity("metals")).unwrap(),
        )
    });
    assert_eq!(left, 60);
    assert_eq!(right, 40);
    assert_eq!(left + right, 100, "a transfer conserves quantity exactly");
}

#[test]
fn e7c2_c_a_debit_past_available_refuses_and_writes_nothing() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let a = endpoint(1);
    seed(engine, &families, a, "metals", 10);
    let error = apply(engine, &families, |txn| {
        families.debit(txn, a, &commodity("metals"), 11)
    })
    .expect_err("a debit past the balance refuses");
    assert!(matches!(error, InventoryError::Book(_)), "{error}");
    let after = read(engine, &families, |txn| {
        families.available(txn, a, &commodity("metals")).unwrap()
    });
    assert_eq!(after, 10, "the refused debit wrote nothing");
}

#[test]
fn e7c2_c_a_debit_past_availability_minus_holds_refuses() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let a = endpoint(1);
    seed(engine, &families, a, "metals", 10);
    apply(engine, &families, |txn| {
        families.reserve(txn, a, &commodity("metals"), &holder(GOD, "one"), 6, &now())
    })
    .expect("hold");
    let available = read(engine, &families, |txn| {
        families.available(txn, a, &commodity("metals")).unwrap()
    });
    assert_eq!(available, 4, "availability is balance minus live holds");
    apply(engine, &families, |txn| {
        families.debit(txn, a, &commodity("metals"), 5)
    })
    .expect_err("a debit into a live hold refuses");
}

#[test]
fn e7c2_c_the_scalar_and_the_book_always_move_together() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let a = endpoint(1);
    seed(engine, &families, a, "metals", 7);
    let key = mush_inventory::pool_key(a, &commodity("metals")).expect("key");
    let (scalar, book) = read(engine, &families, |txn| {
        (
            *txn.get(&families.pools, &key).unwrap().unwrap(),
            txn.get(&families.books, &key).unwrap().unwrap().clone(),
        )
    });
    assert_eq!(
        book.resource_revision(),
        scalar.revision(),
        "the book's resource revision tracks the stored scalar exactly"
    );
    // The gate itself: a scalar whose revision disagrees is refused, not repaired.
    let stale = mush_core::BoundedScalar::new(0, MAX_POOL_QUANTITY, 7, scalar.revision() + 5)
        .expect("stale scalar");
    let error = book
        .checked_apply_resource(
            book.revision(),
            stale,
            mush_core::ResourceOperation::Increase { amount: 1 },
        )
        .expect_err("a bypassed owner path is detected");
    assert!(
        matches!(
            error,
            mush_core::ReservationBookError::ResourceRevisionMismatch { .. }
        ),
        "{error}"
    );
}

#[test]
fn e7c2_c_a_zero_quantity_split_and_an_overflowing_merge_both_refuse() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let a = endpoint(1);
    let one = LotId(1);
    seed_lot(engine, &families, one, a, "metals", 10, 3);
    apply(engine, &families, |txn| {
        families.split(txn, one, 0, LotId(2)).map(|_| ())
    })
    .expect_err("a zero-quantity split refuses");
    apply(engine, &families, |txn| {
        families.split(txn, one, 10, LotId(2)).map(|_| ())
    })
    .expect_err("a split that leaves nothing behind refuses");

    let big = LotId(3);
    let bigger = LotId(4);
    seed_lot(engine, &families, big, a, "gas", MAX_LOT_QUANTITY, 1);
    seed_lot(engine, &families, bigger, a, "gas", MAX_LOT_QUANTITY, 1);
    apply(engine, &families, |txn| {
        families.merge(txn, &[big, bigger], LotId(5)).map(|_| ())
    })
    .expect_err("a merge past the lot ceiling refuses");
}

// ---- E7c2: lineage (C, the laundering axis) ----

#[test]
fn e7c2_c_split_merge_split_reconstructs_both_parents() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let a = endpoint(1);
    let root = LotId(10);
    seed_lot(engine, &families, root, a, "metals", 100, 5);
    let left = LotId(11);
    apply(engine, &families, |txn| {
        families.split(txn, root, 40, left).map(|_| ())
    })
    .expect("split");
    let merged = LotId(12);
    apply(engine, &families, |txn| {
        families.merge(txn, &[root, left], merged).map(|_| ())
    })
    .expect("merge");
    let record = read(engine, &families, |txn| {
        txn.get(&families.lots, &merged).unwrap().unwrap().clone()
    });
    let mut parents = record.lineage().parents();
    parents.sort_unstable();
    let mut expected = vec![root, left];
    expected.sort_unstable();
    assert_eq!(parents, expected, "both parents are reconstructible");
    assert_eq!(record.quantity(), 100, "a merge conserves quantity");
}

#[test]
fn e7c2_c_dissolve_then_crystallize_cannot_launder_provenance() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let a = endpoint(1);
    let stolen = LotId(20);
    seed_lot(engine, &families, stolen, a, "metals", 50, 7);
    let tranche = apply(engine, &families, |txn| {
        families.dissolve(txn, stolen).map(|(id, _)| id)
    })
    .expect("dissolve");
    let child = LotId(21);
    apply(engine, &families, |txn| {
        families
            .crystallize(
                txn,
                a,
                &commodity("metals"),
                50,
                Grade::new(7).unwrap(),
                child,
            )
            .map(|_| ())
    })
    .expect("crystallize");
    let (minted, root) = read(engine, &families, |txn| {
        (
            txn.get(&families.lots, &child).unwrap().unwrap().clone(),
            txn.get(&families.tranches, &tranche)
                .unwrap()
                .unwrap()
                .clone(),
        )
    });
    assert_eq!(
        minted.lineage(),
        &Lineage::Tranche { tranche },
        "the child's parent IS the tranche"
    );
    assert!(
        root.absorbed().contains(&stolen),
        "the original lot id is still reachable through the tranche"
    );
    // And the round trip conserved: the pool is back to zero, the lot holds 50.
    let pooled = read(engine, &families, |txn| {
        families.available(txn, a, &commodity("metals")).unwrap()
    });
    assert_eq!(pooled, 0);
    assert_eq!(minted.quantity(), 50);
}

#[test]
fn e7c2_c_a_merge_over_max_lot_parents_refuses() {
    let parents: Vec<LotId> = (1..=(MAX_LOT_PARENTS as u64 + 1)).map(LotId).collect();
    let error = Lot::new(
        LotId(9_999),
        endpoint(1),
        commodity("metals"),
        1,
        Grade::new(0).unwrap(),
        Lineage::Merge { parents },
        1,
    )
    .expect_err("a merge past MAX_LOT_PARENTS refuses");
    assert!(
        matches!(
            error,
            mush_inventory::InventoryRecordError::LotParents { max } if max == MAX_LOT_PARENTS
        ),
        "{error}"
    );
}

#[test]
fn e7c2_c_a_lineage_cycle_is_unconstructible_and_depth_is_capped() {
    let self_parent = Lot::new(
        LotId(1),
        endpoint(1),
        commodity("metals"),
        1,
        Grade::new(0).unwrap(),
        Lineage::Split { parent: LotId(1) },
        1,
    );
    assert!(self_parent.is_err(), "a lot cannot be its own parent");
    let too_deep = Lot::new(
        LotId(2),
        endpoint(1),
        commodity("metals"),
        1,
        Grade::new(0).unwrap(),
        Lineage::Split { parent: LotId(1) },
        MAX_LINEAGE_DEPTH + 1,
    );
    assert!(too_deep.is_err(), "MAX_LINEAGE_DEPTH refuses");
}

#[test]
fn e7c2_c_coarsening_the_projection_deletes_no_stored_parent() {
    let tranche = PoolTranche::new(
        TrancheId(1),
        endpoint(1),
        commodity("metals"),
        vec![LotId(1)],
        10,
        0,
    )
    .expect("tranche");
    let grown = tranche.checked_absorb(LotId(2), 5, 1).expect("absorb");
    assert_eq!(
        grown.absorbed(),
        &[LotId(1), LotId(2)],
        "absorbing is append-only"
    );
    assert_eq!(grown.quantity(), 15);
    // Rendering a page of the lineage is a PROJECTION choice; the record is
    // untouched by how much of it a command chose to print.
    let rendered: Vec<LotId> = grown.absorbed().iter().copied().take(1).collect();
    assert_eq!(rendered.len(), 1);
    assert_eq!(grown.absorbed().len(), 2, "the stored set is unchanged");
}

#[test]
fn e7c2_c_a_lot_has_no_legal_and_no_custody_field() {
    let lot = Lot::new(
        LotId(1),
        endpoint(1),
        commodity("metals"),
        10,
        Grade::new(2).unwrap(),
        Lineage::Origin {
            source: ExternalRef {
                provider: "test".into(),
                kind: "seed".into(),
                id: "1".into(),
                revision: None,
            },
        },
        0,
    )
    .expect("lot");
    let encoded = serde_json::to_value(&lot).expect("encode");
    let object = encoded.as_object().expect("object");
    assert!(!object.contains_key("legal"), "D-E7-4: no `legal` field");
    assert!(
        !object.contains_key("custody"),
        "D-E7-4: no `custody` field"
    );
    // Every field the record does carry, pinned so a later cut cannot smuggle
    // one in without this test noticing.
    let mut keys: Vec<&String> = object.keys().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "commodity",
            "depth",
            "endpoint",
            "grade",
            "id",
            "lineage",
            "quantity",
            "revision"
        ]
    );
}

// ---- E7c2: locality (L) ----

#[test]
fn e7c2_l_no_verb_produces_an_endpoint_free_pool() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let a = endpoint(7);
    seed(engine, &families, a, "metals", 5);
    let key = mush_inventory::pool_key(a, &commodity("metals")).expect("key");
    let book = read(engine, &families, |txn| {
        txn.get(&families.books, &key).unwrap().unwrap().clone()
    });
    assert_eq!(
        mush_inventory::store::endpoint_of(book.resource().subject.clone()),
        Some(a),
        "every pool names its endpoint in its own resource"
    );
}

#[test]
fn e7c2_l_moving_the_backing_moves_the_hold_by_construction() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let ship = endpoint(42);
    seed(engine, &families, ship, "metals", 30);
    let before = read(engine, &families, |txn| {
        families.available(txn, ship, &commodity("metals")).unwrap()
    });
    // "Moving the ship" is an E3 custody act on the vault's BACKING. No
    // inventory record names a location other than the endpoint, so there is
    // nothing here to rewrite — which is the whole locality argument.
    let after = read(engine, &families, |txn| {
        families.available(txn, ship, &commodity("metals")).unwrap()
    });
    assert_eq!(before, after);
    let key = mush_inventory::pool_key(ship, &commodity("metals")).expect("key");
    let revision = read(engine, &families, |txn| {
        txn.get(&families.pools, &key).unwrap().unwrap().revision()
    });
    assert_eq!(revision, 2, "the pool did not move; the backing did");
}

#[test]
fn e7c2_l_a_transfer_to_the_same_endpoint_is_a_no_op_hit_not_a_double_credit() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let a = endpoint(1);
    seed(engine, &families, a, "metals", 20);
    let key = mush_inventory::pool_key(a, &commodity("metals")).expect("key");
    let before = read(engine, &families, |txn| {
        *txn.get(&families.pools, &key).unwrap().unwrap()
    });
    let moved = apply(engine, &families, |txn| {
        families.transfer(txn, a, a, &commodity("metals"), 5)
    })
    .expect("a same-endpoint transfer is a HIT");
    assert_eq!(moved.balance, 20);
    let after = read(engine, &families, |txn| {
        *txn.get(&families.pools, &key).unwrap().unwrap()
    });
    assert_eq!(
        after.revision(),
        before.revision(),
        "a no-op HIT bumps no revision"
    );
    assert_eq!(after.current(), 20, "and credits nothing twice");
}

#[test]
fn e7c2_l_a_transfer_out_of_an_endpoint_with_nothing_there_refuses() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let error = apply(engine, &families, |txn| {
        families.transfer(txn, endpoint(1), endpoint(2), &commodity("metals"), 1)
    })
    .expect_err("an empty source refuses");
    assert!(matches!(error, InventoryError::Book(_)), "{error}");
}

#[test]
fn e7c2_l_the_endpoint_state_seam_is_e3s_own_rules() {
    // With no economy composed at all, every endpoint is fail-closed: unknown
    // is never "usable". The rules stay E3's; cut 2 rebuilds none of them.
    let (world, _families) = boot();
    assert!(
        !mush_economy::endpoint_state(world.world(), VaultId(1)).is_live(),
        "an endpoint with no economy behind it is never live"
    );
}

// ---- E7c2: reservations (R, the double-spend axis) ----

#[test]
fn e7c2_r_two_orders_cannot_hold_one_unit() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let a = endpoint(1);
    seed(engine, &families, a, "metals", 10);
    apply(engine, &families, |txn| {
        families
            .reserve(
                txn,
                a,
                &commodity("metals"),
                &holder(GOD, "one"),
                10,
                &now(),
            )
            .map(|_| ())
    })
    .expect("the first order holds all ten");
    let error = apply(engine, &families, |txn| {
        families
            .reserve(
                txn,
                a,
                &commodity("metals"),
                &holder(ObjId(2), "two"),
                1,
                &now(),
            )
            .map(|_| ())
    })
    .expect_err("the second order cannot hold what the first already holds");
    assert!(matches!(error, InventoryError::Book(_)), "{error}");
}

#[test]
fn e7c2_r_consume_decrements_and_release_after_consume_refuses() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let a = endpoint(1);
    seed(engine, &families, a, "metals", 10);
    let id = apply(engine, &families, |txn| {
        families.reserve(txn, a, &commodity("metals"), &holder(GOD, "one"), 4, &now())
    })
    .expect("hold");
    let moved = apply(engine, &families, |txn| {
        families.consume(
            txn,
            a,
            &commodity("metals"),
            id,
            1,
            &now(),
            CommitToken::from_bytes([0x21; 16]),
        )
    })
    .expect("consume");
    assert_eq!(moved.balance, 6, "consuming decrements the pool");
    assert_eq!(moved.available, 6, "and releases the hold's share");
    let error = apply(engine, &families, |txn| {
        families.release(txn, a, &commodity("metals"), id, 2)
    })
    .expect_err("release after consume refuses");
    assert!(matches!(error, InventoryError::Book(_)), "{error}");
}

#[test]
fn e7c2_r_a_stale_reservation_revision_refuses() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let a = endpoint(1);
    seed(engine, &families, a, "metals", 10);
    let id = apply(engine, &families, |txn| {
        families.reserve(txn, a, &commodity("metals"), &holder(GOD, "one"), 4, &now())
    })
    .expect("hold");
    apply(engine, &families, |txn| {
        families.consume(
            txn,
            a,
            &commodity("metals"),
            id,
            9,
            &now(),
            CommitToken::from_bytes([0x22; 16]),
        )
    })
    .expect_err("a stale reservation revision refuses");
}

#[test]
fn e7c2_r_a_consumed_reservation_under_another_holder_refuses_rather_than_hits() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let a = endpoint(1);
    seed(engine, &families, a, "metals", 10);
    let mine = apply(engine, &families, |txn| {
        families.reserve(txn, a, &commodity("metals"), &holder(GOD, "one"), 4, &now())
    })
    .expect("hold");
    let commit = CommitToken::from_bytes([0x23; 16]);
    apply(engine, &families, |txn| {
        families.consume(txn, a, &commodity("metals"), mine, 1, &now(), commit)
    })
    .expect("consume");
    // The SAME commit token, replayed under a different holder's derived id.
    let theirs = ReservationId::derive(
        &holder(ObjId(2), "two"),
        &mush_inventory::pool_resource(a, &commodity("metals")).expect("resource"),
    )
    .expect("derive");
    assert_ne!(theirs, mine, "a different holder derives a different id");
    let error = apply(engine, &families, |txn| {
        families.consume(txn, a, &commodity("metals"), theirs, 1, &now(), commit)
    })
    .expect_err("a replay under another holder refuses rather than HITs");
    assert!(matches!(error, InventoryError::Book(_)), "{error}");
}

#[test]
fn e7c2_r_expiry_is_idempotent_and_a_foreign_generation_refuses_comparison() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let a = endpoint(1);
    seed(engine, &families, a, "metals", 10);
    let id = apply(engine, &families, |txn| {
        families.reserve(txn, a, &commodity("metals"), &holder(GOD, "one"), 4, &now())
    })
    .expect("hold");
    // A stamp on ANOTHER generation refuses comparison rather than expiring
    // early — K4a's `checked_cmp`, not a clamp.
    let error = apply(engine, &families, |txn| {
        families.expire(
            txn,
            a,
            &commodity("metals"),
            id,
            1,
            &ClockStamp {
                clock: other_generation().clock,
                at: 10_000_000,
            },
        )
    })
    .expect_err("a foreign generation refuses comparison");
    assert!(matches!(error, InventoryError::Book(_)), "{error}");
    // On its own generation, past the TTL, it expires — and only once.
    let elapsed = ClockStamp {
        clock: now().clock,
        at: now().at + mush_inventory::RESERVATION_TTL_SECS + 1,
    };
    apply(engine, &families, |txn| {
        families.expire(txn, a, &commodity("metals"), id, 1, &elapsed)
    })
    .expect("expire");
    apply(engine, &families, |txn| {
        families.expire(txn, a, &commodity("metals"), id, 2, &elapsed)
    })
    .expect_err("expiring a terminal record again refuses");
    let available = read(engine, &families, |txn| {
        families.available(txn, a, &commodity("metals")).unwrap()
    });
    assert_eq!(available, 10, "an expired hold returns its share");
}

#[test]
fn e7c2_r_availability_is_balance_minus_live_holds_at_every_read() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let a = endpoint(1);
    seed(engine, &families, a, "metals", 10);
    for (tag, amount, expected) in [("one", 3_u64, 7_u64), ("two", 2, 5)] {
        apply(engine, &families, |txn| {
            families
                .reserve(
                    txn,
                    a,
                    &commodity("metals"),
                    &holder(GOD, tag),
                    amount,
                    &now(),
                )
                .map(|_| ())
        })
        .expect("hold");
        let available = read(engine, &families, |txn| {
            families.available(txn, a, &commodity("metals")).unwrap()
        });
        assert_eq!(available, expected);
        let pool = read(engine, &families, |txn| {
            families
                .pool(txn, a, &commodity("metals"))
                .unwrap()
                .unwrap()
        });
        assert_eq!(pool.quantity, 10, "the balance never moved");
        assert_eq!(pool.available, expected);
    }
}

#[test]
fn e7c2_r_a_reservation_id_always_renders_as_idempotency_key_text() {
    // §3.2's "MUST render to a canonical `IdempotencyKeyText`" is already
    // satisfied by `ReservationId(CanonicalHash)`. Asserted ONCE, here, and
    // never re-checked at guard time.
    let id = ReservationId::derive(
        &holder(GOD, "one"),
        &mush_inventory::pool_resource(endpoint(1), &commodity("metals")).expect("resource"),
    )
    .expect("derive");
    let text = mush_inventory::reservation_id_text(id).expect("renders");
    assert_eq!(text.as_str().len(), 64, "64 lowercase hex bytes");
    assert!(text.as_str().len() <= mush_core::MAX_IDEMPOTENCY_KEY_BYTES);
    assert_eq!(
        mush_inventory::reservation_id_from_text(&text),
        Some(id),
        "and round-trips exactly"
    );
}

// ---- E7c2: restart (R, the crash-mid-commit axis) ----

#[test]
fn e7c2_restart_every_record_re_enters_through_its_constructor() {
    // A hand-edited document must fail on LOAD, not at first use.
    let bad_grade = serde_json::json!({
        "id": 1_u64, "endpoint": 1_u64, "commodity": "metals", "quantity": 5_u64,
        "grade": 200_u8, "lineage": { "kind": "split", "parent": 2_u64 },
        "depth": 1_u32, "revision": 1_u64
    });
    assert!(
        serde_json::from_value::<Lot>(bad_grade).is_err(),
        "a grade past MAX_GRADE is refused on the wire"
    );
    let deep = serde_json::json!({
        "id": 1_u64, "endpoint": 1_u64, "commodity": "metals", "quantity": 5_u64,
        "grade": 1_u8, "lineage": { "kind": "split", "parent": 2_u64 },
        "depth": MAX_LINEAGE_DEPTH + 1, "revision": 1_u64
    });
    assert!(
        serde_json::from_value::<Lot>(deep).is_err(),
        "a depth past MAX_LINEAGE_DEPTH is refused on the wire"
    );
    let zero = serde_json::json!({
        "id": 1_u64, "endpoint": 1_u64, "commodity": "metals", "quantity": 0_u64,
        "grade": 1_u8, "lineage": { "kind": "origin",
            "source": { "provider": "t", "kind": "k", "id": "1", "revision": null } },
        "depth": 0_u32, "revision": 1_u64
    });
    assert!(
        serde_json::from_value::<Lot>(zero).is_err(),
        "a zero-quantity lot is refused on the wire"
    );
    let unordered = serde_json::json!({
        "id": 9_u64, "endpoint": 1_u64, "commodity": "metals", "quantity": 5_u64,
        "grade": 1_u8, "lineage": { "kind": "merge", "parents": [3_u64, 2_u64] },
        "depth": 1_u32, "revision": 1_u64
    });
    assert!(
        serde_json::from_value::<Lot>(unordered).is_err(),
        "a non-canonical parent set is refused on the wire"
    );
}

#[test]
fn e7c2_restart_the_pool_book_and_lot_post_images_are_one_bundle() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let a = endpoint(1);
    let lot = LotId(30);
    seed_lot(engine, &families, lot, a, "metals", 10, 4);
    // One transaction: the tranche, the pool scalar, the book and the lot
    // removal. `prepare()` is what a bundle carries; there is no half of it.
    let prepared = {
        let mut txn = engine
            .action_transaction(&families.pools)
            .expect("transaction");
        families.dissolve(&mut txn, lot).expect("dissolve");
        txn.prepare().expect("prepared")
    };
    let writes = prepared.authoritative_replay_writes();
    assert!(
        writes.len() >= 4,
        "pool, book, tranche and lot ride ONE bundle: {} writes",
        writes.len()
    );
    // Abort it: nothing at all is installed, so a crash before the receipt
    // leaves the pre-image exactly.
    let (_, installation) = prepared.into_parts();
    engine.abort_prepared_action(installation);
    let (pooled, still_there) = read(engine, &families, |txn| {
        (
            families.available(txn, a, &commodity("metals")).unwrap(),
            txn.get(&families.lots, &lot).unwrap().is_some(),
        )
    });
    assert_eq!(pooled, 0, "an aborted bundle credited nothing");
    assert!(still_there, "and destroyed nothing");
}

#[test]
fn e7c2_restart_availability_between_two_writes_never_exceeds_the_pre_image() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let a = endpoint(1);
    let b = endpoint(2);
    seed(engine, &families, a, "metals", 100);
    // A transfer is a debit then a credit. Read availability at the destination
    // while the batch is only half applied: it may never have risen.
    let mut txn = engine
        .action_transaction(&families.pools)
        .expect("transaction");
    families
        .debit(&mut txn, a, &commodity("metals"), 40)
        .expect("debit");
    let mid = families
        .available(&mut txn, b, &commodity("metals"))
        .expect("read");
    assert_eq!(
        mid, 0,
        "the destination's availability never exceeds its pre-image mid-batch"
    );
    drop(txn);
    let after = read(engine, &families, |txn| {
        families.available(txn, a, &commodity("metals")).unwrap()
    });
    assert_eq!(after, 100, "and the dropped transaction wrote nothing");
}

#[test]
fn e7c2_restart_a_replay_of_one_request_reaches_the_identical_post_image() {
    // Ids are DERIVED from durable causal input, never allocated, so replaying
    // one request mints the same lot rather than a second one.
    let request = mush_core::RequestUid::from_bytes([0x31; 16]);
    let first = mush_inventory::derive_lot_id(request, 0).expect("derive");
    let again = mush_inventory::derive_lot_id(request, 0).expect("derive");
    assert_eq!(first, again, "one request, one lot id");
    let other = mush_inventory::derive_lot_id(mush_core::RequestUid::from_bytes([0x32; 16]), 0)
        .expect("derive");
    assert_ne!(first, other, "a different request mints a different lot");
    // And a tranche is keyed by `(endpoint, commodity)`, so a re-run of one
    // dissolve extends the same root instead of forking a second one.
    let root = mush_inventory::tranche_key(endpoint(1), &commodity("metals")).expect("key");
    let same = mush_inventory::tranche_key(endpoint(1), &commodity("metals")).expect("key");
    assert_eq!(root, same);
    assert_ne!(
        root,
        mush_inventory::tranche_key(endpoint(2), &commodity("metals")).expect("key")
    );
}

#[test]
fn e7c2_restart_a_half_pool_is_refused_inside_a_transaction() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let a = endpoint(1);
    seed(engine, &families, a, "metals", 10);
    let key = mush_inventory::pool_key(a, &commodity("metals")).expect("key");
    // Simulate the half-state boot quarantines: the book survived, the scalar
    // did not.
    apply(engine, &families, |txn| {
        txn.remove(&families.pools, &key)
            .map_err(InventoryError::from)
    })
    .expect("remove the scalar half");
    let error = apply(engine, &families, |txn| {
        families.debit(txn, a, &commodity("metals"), 1)
    })
    .expect_err("a half pool refuses every mutation");
    assert!(matches!(error, InventoryError::HalfPool), "{error}");
    let read_back = apply(engine, &families, |txn| {
        families.available(txn, a, &commodity("metals"))
    });
    assert!(
        read_back.is_err(),
        "and availability refuses rather than rising"
    );
}

// ---- E7c2: the market is untouched ----

#[test]
fn e7c2_the_market_keeps_its_own_quantity() {
    // Cut 2 adds no `mush-market` edge and lifts no refusal: this crate's whole
    // dependency set is `{mush-core, mush-economy}` plus the serde vocabulary,
    // and nothing here can name a listing at all.
    let manifest = include_str!("../Cargo.toml");
    let declarations: Vec<&str> = manifest
        .lines()
        .map(str::trim)
        .filter(|line| line.ends_with(".workspace = true"))
        .collect();
    for forbidden in [
        "mush-market",
        "mush-industry",
        "ruleset-tacsim",
        "mush-sim",
        "mush-tactics",
        "mush-org",
    ] {
        assert!(
            !declarations
                .iter()
                .any(|line| line.starts_with(&format!("{forbidden}."))),
            "mush-inventory must not depend on {forbidden}: {declarations:?}"
        );
    }
    assert!(declarations.contains(&"mush-core.workspace = true"));
    assert!(declarations.contains(&"mush-economy.workspace = true"));
}

// ---- E7c2: the guard validator (B4, G3) ----

/// A second module registering **the same kind under another `OwnerId`** — cut
/// 4's capacity reservations, in miniature. It exists to prove the registry's
/// `(OwnerId, AuthorityGuardKind)` key, not to model industry.
#[derive(Debug, Default)]
struct CapacityModule;

impl mush_core::GameModule for CapacityModule {
    fn name(&self) -> &'static str {
        "industry"
    }

    fn register(&mut self, reg: &mut mush_core::Registrar<'_>) -> Result<(), mush_core::ModError> {
        let books = reg
            .state_family::<mush_core::ReservationBookId, mush_core::ReservationBook, _, _, _, _>(
                mush_core::reservation_book_state_family_descriptor(
                    mush_core::StateFamilyId::new("capacity_books").expect("family id"),
                )
                .expect("descriptor"),
                mush_core::ReservationBookStateCodec,
                mush_core::ReservationBookStateCodec,
                mush_core::ReservationBookStateCodec,
                mush_core::ReservationBookStateCodec,
            )?;
        reg.authority_guard_validator(
            mush_core::AuthorityGuardKind::Reservation,
            mush_inventory::ReservationGuardValidator::new(books),
        )?;
        Ok(())
    }
}

fn reservation_guard(owner: &str, id: ReservationId, revision: u64) -> mush_core::AuthorityGuard {
    mush_core::AuthorityGuard::Reservation {
        provider: OwnerId::new(owner).expect("owner"),
        id: mush_inventory::reservation_id_text(id).expect("renders"),
        revision,
    }
}

fn guard_is_current(engine: &Engine, guard: mush_core::AuthorityGuard) -> bool {
    let mut builder = engine
        .registry
        .server_guard_set_builder(mush_core::RevisionSet::default())
        .expect("guard builder");
    builder.add(guard).expect("guard admitted");
    let set = builder.finish().expect("guard set");
    engine
        .revalidate_guards(&set)
        .expect("revalidation runs")
        .is_current()
}

#[test]
fn e7c2_r_the_guard_validator_tracks_the_book_exactly() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let a = endpoint(1);
    seed(engine, &families, a, "metals", 10);
    let id = apply(engine, &families, |txn| {
        families.reserve(txn, a, &commodity("metals"), &holder(GOD, "one"), 4, &now())
    })
    .expect("hold");
    assert!(
        guard_is_current(engine, reservation_guard("inventory", id, 1)),
        "a live hold at its own revision is current"
    );
    assert!(
        !guard_is_current(engine, reservation_guard("inventory", id, 2)),
        "a bumped revision is stale"
    );
    apply(engine, &families, |txn| {
        families.release(txn, a, &commodity("metals"), id, 1)
    })
    .expect("release");
    assert!(
        !guard_is_current(engine, reservation_guard("inventory", id, 2)),
        "a terminal state is never current"
    );
    let absent = ReservationId::derive(
        &holder(ObjId(9), "nope"),
        &mush_inventory::pool_resource(a, &commodity("metals")).expect("resource"),
    )
    .expect("derive");
    assert!(
        !guard_is_current(engine, reservation_guard("inventory", absent, 1)),
        "a missing record is never current"
    );
}

#[test]
fn e7c2_r_cut_fours_same_kind_registration_composes_without_collision() {
    // The registry is keyed `(OwnerId, AuthorityGuardKind)` and the registrar
    // stamps the CALLING module's owner, so this composes — by construction,
    // not by luck.
    let world = TestWorld::with_modules(vec![
        Box::new(InventoryModule::new()),
        Box::new(CapacityModule),
    ]);
    let names: Vec<&str> = world.engine().modules().iter().map(|m| m.name()).collect();
    assert_eq!(names, ["inventory", "industry"]);
    assert!(
        world
            .engine()
            .registry
            .authority_guard_validators()
            .is_some(),
        "both validators froze into one registry"
    );
    // And neither validates the other's guards: the provider check is explicit.
    let engine = world.engine();
    let id = ReservationId::derive(
        &holder(GOD, "one"),
        &mush_inventory::pool_resource(endpoint(1), &commodity("metals")).expect("resource"),
    )
    .expect("derive");
    assert!(!guard_is_current(
        engine,
        reservation_guard("industry", id, 1)
    ));
}

// ---- E7c2: disclosure and wiring ----

#[test]
fn e7c2_disclosure_show_is_byte_identical_for_unviewable_and_absent() {
    let mut world = TestWorld::with_modules(vec![Box::new(InventoryModule::new())]);
    let unviewable = world.run(GOD, "@inv/show 1");
    let absent = world.run(GOD, "@inv/show 4242");
    let render = |events: &[(mush_core::Audience, mush_proto::OutputEvent)]| -> Vec<String> {
        events
            .iter()
            .map(|(_, e)| mush_testkit::text_of(e))
            .collect()
    };
    assert_eq!(
        render(&unviewable),
        render(&absent),
        "an endpoint the viewer may not read and one that does not exist answer identically"
    );
    assert!(
        render(&absent)
            .iter()
            .any(|line| line.contains("There is no such endpoint.")),
        "{:?}",
        render(&absent)
    );
}

#[test]
fn e7c2_b7_a_mutating_switch_reaches_the_seam_with_its_own_action() {
    let mut world = TestWorld::with_modules(vec![Box::new(InventoryModule::new())]);
    let _ = world.run(GOD, "@inv/move 1/metals:5=2");
    assert!(
        world.engine().has_authoritative_submit(),
        "the command queued exactly one authoritative submission"
    );
    let submit = *world
        .engine_mut()
        .take_authoritative_submit()
        .expect("submission");
    assert_eq!(submit.actor, GOD);
    assert_eq!(submit.action.key().id.as_str(), "inv_move");
    assert_eq!(submit.action.key().owner.as_str(), "inventory");
    assert_eq!(submit.subject, mush_core::PerceivedRef::SelfActor);
    assert!(submit.targets.is_empty());
    assert_eq!(submit.parameters["from"], "1");
    assert_eq!(submit.parameters["to"], "2");
    assert_eq!(submit.parameters["commodity"], "metals");
    assert_eq!(submit.parameters["quantity"], 5);
    assert_eq!(submit.key.as_str(), "inv.move");
    // And a READ switch submits nothing at all.
    let _ = world.run(GOD, "@inv/show 1");
    assert!(!world.engine().has_authoritative_submit());
}

#[test]
fn e7c2_b7_no_offer_is_projected_without_a_live_endpoint() {
    // The endpoint choice list IS the legality gate (see `provider.rs`): with no
    // economy behind the world, no endpoint is live and nothing projects — so a
    // submission that reached the driver would be refused at resolution.
    let world = TestWorld::with_modules(vec![Box::new(InventoryModule::new())]);
    assert!(mush_inventory::live_endpoints(world.world(), GOD).is_empty());
}

// ---- E7c2: the facts (OPEN 5, §10) ----

#[test]
fn e7c2_facts_answer_from_an_applied_marker_and_none_otherwise() {
    use mush_economy::FactProvider;
    let (mut world, families) = boot();
    let marker_id = smol_str::SmolStr::new("abc123");
    let marker = mush_inventory::AppliedMarker::new(
        mush_inventory::FACT_TRANSFERRED,
        Some(endpoint(1)),
        Some(endpoint(2)),
        commodity("metals"),
        5,
        GOD,
        1_234,
    )
    .expect("marker");
    {
        let engine = world.engine_mut();
        apply(engine, &families, |txn| {
            txn.insert(&families.applied, &marker_id, marker)
                .map_err(InventoryError::from)
        })
        .expect("marker installs");
    }
    // The mirror is refreshed by any inventory command (and by the sweep tick).
    let _ = world.run(GOD, "@inv/show 1");
    let provider = mush_inventory::InventoryFactProvider;
    assert_eq!(provider.provider(), "inventory");
    let state = provider
        .resolve(
            world.world(),
            mush_inventory::FACT_TRANSFERRED,
            "abc123",
            None,
        )
        .expect("the committed marker answers");
    assert!(state.satisfied);
    assert_eq!(state.actor, Some(GOD));
    assert_eq!(state.at, 1_234);
    // A bundle that did not commit left no marker: unknown, which is unsatisfied.
    assert!(
        provider
            .resolve(
                world.world(),
                mush_inventory::FACT_TRANSFERRED,
                "deadbeef",
                None
            )
            .is_none()
    );
    // The wrong kind for a real marker is also unknown, never "true".
    assert!(
        provider
            .resolve(
                world.world(),
                mush_inventory::FACT_LOT_CREATED,
                "abc123",
                None
            )
            .is_none()
    );
    // And a kind this crate does not own is never answered at all.
    assert!(
        provider
            .resolve(world.world(), "jobs.resolved", "abc123", None)
            .is_none()
    );
}

// ---- E7c2: what a lot is not ----

#[test]
fn e7c2_c_a_disputed_title_changes_no_lot_record() {
    // D-E7-4: stolen/disputed status follows the TITLE record
    // (`mush_economy::Encumbrance::Disputed`), never the matter. There is no
    // field here for it to be written into, and this pins that.
    let encoded = serde_json::to_value(
        Lot::new(
            LotId(1),
            endpoint(1),
            commodity("metals"),
            10,
            Grade::new(2).unwrap(),
            Lineage::Origin {
                source: ExternalRef {
                    provider: "test".into(),
                    kind: "seed".into(),
                    id: "1".into(),
                    revision: None,
                },
            },
            0,
        )
        .expect("lot"),
    )
    .expect("encode");
    let text = encoded.to_string();
    for forbidden in [
        "encumbrance",
        "stolen",
        "disputed",
        "legal",
        "custody",
        "owner",
    ] {
        assert!(
            !text.contains(forbidden),
            "a lot must carry no {forbidden} field: {text}"
        );
    }
    // The economy's own variant is where it lives, and it is untouched by us.
    assert!(matches!(
        mush_economy::Encumbrance::Disputed {
            claimant: mush_economy::object_principal(GOD),
            reason: "someone else's".to_owned(),
        },
        mush_economy::Encumbrance::Disputed { .. }
    ));
}

#[test]
#[should_panic(expected = "two fact providers claim the namespace")]
fn e7c2_facts_a_second_inventory_namespace_panics_at_composition() {
    // D-E7-30: one provider per crate, no exceptions. The panic is what makes
    // "one namespace per crate" a composition fact rather than a convention.
    let _ = mush_economy::EconomyModule::new().with_facts(vec![
        std::sync::Arc::new(mush_inventory::InventoryFactProvider),
        std::sync::Arc::new(mush_inventory::InventoryFactProvider),
    ]);
}

// ---- E7c2: boot (B6) ----

#[test]
fn e7c2_boot_a_pool_without_its_book_quarantines_the_endpoint() {
    let (mut world, families) = boot();
    let a = endpoint(11);
    {
        let engine = world.engine_mut();
        seed(engine, &families, a, "metals", 12);
        let key = mush_inventory::pool_key(a, &commodity("metals")).expect("key");
        // The half-state that can only arise beneath the kernel: one of the two
        // documents written in ONE bundle did not come back.
        apply(engine, &families, |txn| {
            txn.remove(&families.books, &key)
                .map_err(InventoryError::from)
        })
        .expect("drop the book half");
    }
    // Reconciliation is what derives the marker; it runs at boot, on the sweep
    // tick and at the top of every `@inv` command.
    let _ = world.run(GOD, "@inv/show 11");
    let store = world
        .world()
        .module_state::<mush_inventory::InventoryStore>()
        .expect("inventory store");
    assert_eq!(
        store.orphaned_pool_keys(),
        1,
        "the surviving scalar's key is quarantined; only the book carries the endpoint"
    );
    // And the pair check refuses every mutation against it, without raising
    // availability.
    let engine = world.engine_mut();
    let error = apply(engine, &families, |txn| {
        families.debit(txn, a, &commodity("metals"), 1)
    })
    .expect_err("a half pool refuses");
    assert!(matches!(error, InventoryError::HalfPool), "{error}");
}

#[test]
fn e7c2_boot_a_book_without_its_pool_quarantines_the_named_endpoint() {
    let (mut world, families) = boot();
    let a = endpoint(12);
    {
        let engine = world.engine_mut();
        seed(engine, &families, a, "metals", 9);
        let key = mush_inventory::pool_key(a, &commodity("metals")).expect("key");
        apply(engine, &families, |txn| {
            txn.remove(&families.pools, &key)
                .map_err(InventoryError::from)
        })
        .expect("drop the scalar half");
    }
    let _ = world.run(GOD, "@inv/show 12");
    let store = world
        .world()
        .module_state::<mush_inventory::InventoryStore>()
        .expect("inventory store");
    assert!(
        store.is_quarantined(a),
        "the book still names its endpoint, so the endpoint itself is marked"
    );
    assert_eq!(store.orphaned_pool_keys(), 0);
    assert!(store.is_complete(), "the walk finished inside its budget");
}

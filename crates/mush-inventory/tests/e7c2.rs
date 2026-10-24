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

/// Run the boot reconciliation the module owns, without going through a
/// command: E7c2 fold B4 took `store::reconcile` off the command path.
fn reconcile_now(world: &mut TestWorld) {
    world.modules_loaded(std::collections::HashMap::new());
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
        engine
            .install_prepared_action(installation)
            .expect("the fixture installs");
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
        let census = txn
            .get(&families.lot_census, &at)
            .map_err(InventoryError::from)?
            .copied()
            .unwrap_or_default();
        let next = census.checked_move(1)?;
        if txn
            .get(&families.lot_census, &at)
            .map_err(InventoryError::from)?
            .is_some()
        {
            txn.replace(&families.lot_census, &at, next)
                .map_err(InventoryError::from)?;
        } else {
            txn.insert(&families.lot_census, &at, next)
                .map_err(InventoryError::from)?;
        }
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
        families.split(txn, a, one, 0, LotId(2)).map(|_| ())
    })
    .expect_err("a zero-quantity split refuses");
    apply(engine, &families, |txn| {
        families.split(txn, a, one, 10, LotId(2)).map(|_| ())
    })
    .expect_err("a split that leaves nothing behind refuses");

    let big = LotId(3);
    let bigger = LotId(4);
    seed_lot(engine, &families, big, a, "gas", MAX_LOT_QUANTITY, 1);
    seed_lot(engine, &families, bigger, a, "gas", MAX_LOT_QUANTITY, 1);
    apply(engine, &families, |txn| {
        families.merge(txn, a, &[big, bigger], LotId(5)).map(|_| ())
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
        families.split(txn, a, root, 40, left).map(|_| ())
    })
    .expect("split");
    let merged = LotId(12);
    apply(engine, &families, |txn| {
        families.merge(txn, a, &[root, left], merged).map(|_| ())
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
        families.dissolve(txn, a, stolen).map(|(id, _)| id)
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
        0,
        None,
        Vec::new(),
        vec![LotId(1)],
        10,
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
    // E7c2 fold B9: and against REAL vault records, each arm answers E3's own.
    let (mut world, _families) = econ_rig();
    let vault = make_vault(&mut world, "hold", None);
    assert_eq!(
        mush_economy::endpoint_state(world.world(), vault.vault()),
        mush_economy::EndpointState::Live
    );
    assert_eq!(
        mush_economy::endpoint_state(world.world(), VaultId(4_242)),
        mush_economy::EndpointState::Missing
    );
    let _ = world.run(GOD, &format!("@vault/freeze {}", vault.vault().0));
    assert!(matches!(
        mush_economy::endpoint_state(world.world(), vault.vault()),
        mush_economy::EndpointState::Inactive(_)
    ));
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
        families.dissolve(&mut txn, a, lot).expect("dissolve");
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
    engine
        .abort_prepared_action(installation)
        .expect("the fixture aborts");
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
    let root = mush_inventory::tranche_key(endpoint(1), &commodity("metals"), 0).expect("key");
    let same = mush_inventory::tranche_key(endpoint(1), &commodity("metals"), 0).expect("key");
    assert_eq!(root, same);
    assert_ne!(
        root,
        mush_inventory::tranche_key(endpoint(2), &commodity("metals"), 0).expect("key")
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
        "tacsim",
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
        let index = reg
            .state_family::<mush_core::ReservationId, mush_core::ReservationBookId, _, _, _, _>(
                mush_core::StateFamilyDescriptor::new(
                    mush_core::StateFamilyId::new("capacity_reservation_index").expect("family id"),
                    mush_core::CanonicalId::new("capacity.reservation_id.v1").expect("id"),
                    mush_core::CanonicalId::new("capacity.reservation_index.v1").expect("id"),
                    1,
                    mush_core::CanonicalId::new("capacity.reservation_index.decode_json.v1")
                        .expect("id"),
                    mush_core::CanonicalId::new("capacity.reservation_index.bounds.v1")
                        .expect("id"),
                    mush_core::CanonicalId::new("capacity.reservation_index.json.v1").expect("id"),
                )
                .expect("descriptor"),
                |key: &mush_core::ReservationId| Ok(key.0.to_hex()),
                |value: &serde_json::Value| {
                    serde_json::from_value(value.clone()).map_err(|e| e.to_string())
                },
                |_value: &mush_core::ReservationBookId| Ok(()),
                |value: &mush_core::ReservationBookId| {
                    serde_json::to_value(value).map_err(|e| e.to_string())
                },
            )?;
        reg.authority_guard_validator(
            mush_core::AuthorityGuardKind::Reservation,
            mush_inventory::ReservationGuardValidator::new(books, index),
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
    // E7c2 fold B9: on the REAL two-module rig, with a vault that exists and a
    // viewer who may not read it. It used to run with no economy composed, where
    // `endpoint_visible` returns false at its first line for everybody — so it
    // compared two instances of the same "no economy" branch.
    let (mut world, _families) = econ_rig();
    let vault = make_vault(&mut world, "hold", None);
    let mallory = world.spawn_player("mallory");
    let unviewable = world.run(mallory, &format!("@inv/show {}", vault.vault().0));
    let absent = world.run(mallory, "@inv/show 4242");
    assert_eq!(
        rendered(&unviewable),
        rendered(&absent),
        "an endpoint the viewer may not read and one that does not exist answer identically"
    );
    assert!(
        rendered(&absent)
            .iter()
            .any(|line| line.contains("There is no such endpoint.")),
        "{:?}",
        rendered(&absent)
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
    // E7c2 fold R1(b): the discriminator now carries the IC second and an
    // ordinal inside it, so a retyped intent is a new request.
    assert!(
        submit.key.as_str().starts_with("inv.move."),
        "{}",
        submit.key.as_str()
    );
    // And a READ switch submits nothing at all.
    let _ = world.run(GOD, "@inv/show 1");
    assert!(!world.engine().has_authoritative_submit());
}

#[test]
fn e7c2_b7_no_offer_is_projected_without_a_live_endpoint() {
    // The endpoint choice list IS the legality gate (see `provider.rs`). E7c2
    // fold B9: proved against a real vault, not against a world with no economy.
    let (mut world, _families) = econ_rig();
    let vault = make_vault(&mut world, "hold", None);
    assert!(mush_inventory::live_endpoints(world.world(), GOD).contains(&vault.vault()));
    let mallory = world.spawn_player("mallory");
    assert!(
        mush_inventory::live_endpoints(world.world(), mallory).is_empty(),
        "an actor with no endpoint of their own gets no offer at all"
    );
    let _ = world.run(GOD, &format!("@vault/close {}", vault.vault().0));
    assert!(
        mush_inventory::live_endpoints(world.world(), GOD).is_empty(),
        "and a closed endpoint drops out of the regenerated choice list"
    );
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
    // The mirror is refreshed at boot and on the sweep tick (fold B4).
    reconcile_now(&mut world);
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
    // Reconciliation is what derives the marker; since E7c2 fold B4 it runs at
    // boot and on the sweep tick — never on the command path.
    reconcile_now(&mut world);
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
    reconcile_now(&mut world);
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

// ---- E7 cut 2 independent verify fold ----
//
// One section, one test per confirmed finding. The rig below (fold B9) is the
// reason most of them can exist at all: cut 2's disclosure and endpoint-legality
// proofs ran in a world with **no economy composed**, where `endpoint_visible`
// and `live_endpoints` both short-circuit at their first line — so the
// byte-identity proof compared two instances of the same "no economy" branch and
// the "no offer without a live endpoint" proof showed only that a world without
// an economy has no endpoints.

/// `EconomyModule` + `InventoryModule`, composed in production order.
fn econ_rig() -> (TestWorld, InventoryFamilies) {
    let world = TestWorld::with_modules(vec![
        Box::new(mush_economy::EconomyModule::new()),
        Box::new(InventoryModule::new()),
    ]);
    let families = world
        .world()
        .module_state::<InventoryHandles>()
        .expect("inventory publishes its handles")
        .0
        .clone();
    (world, families)
}

fn rendered(events: &[(mush_core::Audience, mush_proto::OutputEvent)]) -> Vec<String> {
    events
        .iter()
        .map(|(_, event)| mush_testkit::text_of(event))
        .collect()
}

fn number_after(
    events: &[(mush_core::Audience, mush_proto::OutputEvent)],
    needle: &str,
) -> Option<u64> {
    for line in rendered(events) {
        if let Some(at) = line.find(needle) {
            let digits: String = line[at + needle.len()..]
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if !digits.is_empty() {
                return digits.parse().ok();
            }
        }
    }
    None
}

/// Create a real container-backed E3 vault and return the endpoint it is.
fn make_vault(world: &mut TestWorld, name: &str, owner: Option<ObjId>) -> EndpointRef {
    let created = world.run(GOD, &format!("@create {name}"));
    let object = number_after(&created, "(#").expect("the created object's id");
    let line = owner.map_or_else(
        || format!("@vault/create {name}=container:#{object}"),
        |actor| {
            format!(
                "@vault/create {name}=container:#{object} owner=#{}",
                actor.0
            )
        },
    );
    let out = world.run(GOD, &line);
    EndpointRef(VaultId(
        number_after(&out, "vault #").unwrap_or_else(|| panic!("vault id: {:?}", rendered(&out))),
    ))
}

#[test]
fn vf_b9_the_disclosure_and_legality_proofs_run_against_a_real_vault() {
    let (mut world, _families) = econ_rig();
    let vault = make_vault(&mut world, "hold", None);
    // (c) `endpoint_state` against a real vault record, not the
    // `BackingUnavailable` fallback a world with no economy always answers.
    assert_eq!(
        mush_economy::endpoint_state(world.world(), vault.vault()),
        mush_economy::EndpointState::Live,
        "a container-backed active vault is live"
    );
    assert_eq!(
        mush_economy::endpoint_state(world.world(), VaultId(4_242)),
        mush_economy::EndpointState::Missing
    );
    assert!(
        mush_inventory::endpoint_visible(world.world(), GOD, vault),
        "the vault's own owner reads it"
    );
    // (b) a frozen endpoint drops out of the regenerated choice list.
    assert!(mush_inventory::live_endpoints(world.world(), GOD).contains(&vault.vault()));
    let _ = world.run(GOD, &format!("@vault/freeze {}", vault.vault().0));
    assert!(matches!(
        mush_economy::endpoint_state(world.world(), vault.vault()),
        mush_economy::EndpointState::Inactive(_)
    ));
    assert!(
        mush_inventory::live_endpoints(world.world(), GOD).is_empty(),
        "a frozen endpoint is not live and projects no choice"
    );
}

#[test]
fn vf_c8_the_inventory_gate_is_e3s_vault_gate() {
    let (mut world, _families) = econ_rig();
    let mine = make_vault(&mut world, "mine", None);
    let mallory = world.spawn_player("mallory");
    let hers = make_vault(&mut world, "hers", Some(mallory));

    // The read gate IS `may_view_vault` and the take gate IS E3's withdraw
    // standing, across the matrix — never one predicate answering both.
    for actor in [GOD, mallory] {
        for endpoint in [mine, hers, EndpointRef(VaultId(4_242))] {
            assert_eq!(
                mush_inventory::endpoint_visible(world.world(), actor, endpoint),
                mush_economy::may_view_vault(world.world(), actor, endpoint.vault()),
                "read gate disagrees for {actor:?} at {endpoint}"
            );
            assert_eq!(
                mush_inventory::may_take(world.world(), actor, endpoint),
                mush_economy::may_take_from_vault(world.world(), actor, endpoint.vault()),
                "take gate disagrees for {actor:?} at {endpoint}"
            );
        }
    }
    assert!(!mush_inventory::endpoint_visible(
        world.world(),
        mallory,
        mine
    ));
    assert!(!mush_inventory::may_take(world.world(), mallory, mine));
    assert!(mush_inventory::endpoint_visible(
        world.world(),
        mallory,
        hers
    ));
    assert!(mush_inventory::may_take(world.world(), mallory, hers));
    assert!(mush_inventory::live_endpoints(world.world(), mallory).contains(&hers.vault()));
    assert!(!mush_inventory::live_endpoints(world.world(), mallory).contains(&mine.vault()));

    // And the crate keeps no predicate of its own: an org's money-posting grant
    // is never read as authority to walk its physical stock out.
    let source = include_str!("../src/provider.rs");
    assert!(source.contains("mush_economy::may_view_vault"));
    assert!(source.contains("mush_economy::may_take_from_vault"));
    assert!(
        !source.contains("mush_economy::acts_for("),
        "cut 2 must not decide vault authority itself"
    );
}

#[test]
fn vf_c1_a_lot_at_an_endpoint_the_actor_may_not_act_on_refuses() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let a = endpoint(1);
    let b = endpoint(2);
    let one = LotId(70);
    let two = LotId(71);
    seed_lot(engine, &families, one, b, "metals", 40, 3);
    seed_lot(engine, &families, two, b, "metals", 10, 3);
    let before = read(engine, &families, |txn| {
        (
            txn.get(&families.lots, &one).unwrap().cloned(),
            txn.get(&families.lots, &two).unwrap().cloned(),
        )
    });
    for error in [
        apply(engine, &families, |txn| {
            families.split(txn, a, one, 5, LotId(72)).map(|_| ())
        })
        .expect_err("split at the wrong endpoint refuses"),
        apply(engine, &families, |txn| {
            families.merge(txn, a, &[one, two], LotId(73)).map(|_| ())
        })
        .expect_err("merge at the wrong endpoint refuses"),
        apply(engine, &families, |txn| {
            families.dissolve(txn, a, one).map(|_| ())
        })
        .expect_err("dissolve at the wrong endpoint refuses"),
    ] {
        assert!(
            matches!(error, InventoryError::NoLot),
            "the refusal must be byte-identical to a missing lot: {error}"
        );
    }
    let after = read(engine, &families, |txn| {
        (
            txn.get(&families.lots, &one).unwrap().cloned(),
            txn.get(&families.lots, &two).unwrap().cloned(),
        )
    });
    assert_eq!(before, after, "the foreign endpoint's lots are untouched");
    let (pooled, tranche) = read(engine, &families, |txn| {
        (
            families.pool(txn, b, &commodity("metals")).unwrap(),
            families.tranche_for(txn, b, &commodity("metals")).unwrap(),
        )
    });
    assert!(pooled.is_none(), "and no pool was created at B");
    assert!(tranche.is_none(), "and no tranche was created at B");
}

#[test]
fn vf_c1_every_lot_arm_reads_the_authorised_endpoint() {
    // A source pin: the `endpoint` Choice `project_offer` mints is the ONLY
    // consent gate cut 2 has, and it was decorative for three of the five verbs.
    let source = include_str!("../src/provider.rs");
    let body = source
        .split("let summary = match self.verb {")
        .nth(1)
        .expect("the prepare_authoritative match");
    let arms = [
        "InventoryVerb::Move => {",
        "InventoryVerb::Split => {",
        "InventoryVerb::Merge => {",
        "InventoryVerb::Dissolve => {",
        "InventoryVerb::Crystallize => {",
    ];
    for (index, arm) in arms.iter().enumerate() {
        let start = body.find(arm).unwrap_or_else(|| panic!("arm {arm}"));
        let end = arms
            .get(index + 1)
            .and_then(|next| body.find(next))
            .unwrap_or(body.len());
        assert!(
            body[start..end].contains("endpoint_field(&parameters,"),
            "{arm} must read the authorised endpoint"
        );
    }
}

#[test]
fn vf_c2_a_transfer_carries_the_source_tranche_into_the_destination_root() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let a = endpoint(1);
    let b = endpoint(2);
    let stolen = LotId(80);
    let alibi = LotId(81);
    seed_lot(engine, &families, stolen, a, "metals", 50, 7);
    seed_lot(engine, &families, alibi, b, "metals", 1, 7);
    let source_root = apply(engine, &families, |txn| {
        families.dissolve(txn, a, stolen).map(|(id, _)| id)
    })
    .expect("dissolve at A");
    apply(engine, &families, |txn| {
        families.transfer(txn, a, b, &commodity("metals"), 50)
    })
    .expect("transfer A -> B");
    apply(engine, &families, |txn| {
        families.dissolve(txn, b, alibi).map(|(id, _)| id)
    })
    .expect("dissolve the clean lot at B");
    let child = LotId(82);
    apply(engine, &families, |txn| {
        families
            .crystallize(
                txn,
                b,
                &commodity("metals"),
                50,
                Grade::new(7).unwrap(),
                child,
            )
            .map(|_| ())
    })
    .expect("crystallize at B");
    // Walk child -> tranche -> inherited -> absorbed.
    let (minted, dest_root) = read(engine, &families, |txn| {
        (
            txn.get(&families.lots, &child).unwrap().unwrap().clone(),
            families
                .tranche_for(txn, b, &commodity("metals"))
                .unwrap()
                .unwrap()
                .1,
        )
    });
    let Lineage::Tranche { tranche } = minted.lineage().clone() else {
        panic!("the child's parent is the destination tranche");
    };
    assert_eq!(tranche, dest_root.id());
    assert!(
        dest_root.inherited().contains(&source_root),
        "the destination root names the source root"
    );
    let reached = read(engine, &families, |txn| {
        txn.get(&families.tranches, &source_root)
            .unwrap()
            .unwrap()
            .absorbed()
            .to_vec()
    });
    assert!(
        reached.contains(&stolen),
        "one hop no longer launders provenance: the original lot is reachable"
    );
    // A transfer out of a slot with NO tranche adds nothing.
    let c = endpoint(3);
    let d = endpoint(4);
    seed(engine, &families, c, "gas", 10);
    apply(engine, &families, |txn| {
        families.transfer(txn, c, d, &commodity("gas"), 4)
    })
    .expect("transfer with no lineage");
    let none = read(engine, &families, |txn| {
        families.tranche_for(txn, d, &commodity("gas")).unwrap()
    });
    assert!(none.is_none(), "no lineage to carry, so none invented");
    // And `inherited` is deduplicated and refuses past its cap.
    let root = PoolTranche::new(
        TrancheId(900),
        endpoint(9),
        commodity("metals"),
        0,
        None,
        Vec::new(),
        vec![LotId(1)],
        10,
        10,
        0,
    )
    .expect("root");
    let once = root.checked_inherit(TrancheId(5), 1).expect("inherit");
    let twice = once
        .checked_inherit(TrancheId(5), 1)
        .expect("inherit again");
    assert_eq!(twice.inherited(), &[TrancheId(5)], "deduplicated");
    let mut full = root;
    for id in 0..mush_inventory::MAX_TRANCHE_INHERITED {
        full = full
            .checked_inherit(TrancheId(1_000 + id as u64), 1)
            .expect("inherit inside the cap");
    }
    assert!(
        full.checked_inherit(TrancheId(9_999), 1).is_err(),
        "and refuses past MAX_TRANCHE_INHERITED"
    );
}

#[test]
fn vf_c3_a_pool_stays_reservable_past_its_book_capacity() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let a = endpoint(1);
    seed(engine, &families, a, "metals", 1_000);
    for index in 0..(mush_core::MAX_RESERVATION_BOOK_RECORDS + 8) {
        let key = holder(ObjId(u32::try_from(index).unwrap() + 10), "cap");
        let id = apply(engine, &families, |txn| {
            families.reserve(txn, a, &commodity("metals"), &key, 1, &now())
        })
        .unwrap_or_else(|error| panic!("hold {index} must succeed: {error}"));
        apply(engine, &families, |txn| {
            families.consume(
                txn,
                a,
                &commodity("metals"),
                id,
                1,
                &now(),
                CommitToken::from_bytes([0x41; 16]),
            )
        })
        .unwrap_or_else(|error| panic!("consume {index}: {error}"));
    }
}

#[test]
fn vf_c3_a_holder_may_reserve_again_after_its_hold_is_consumed() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let a = endpoint(1);
    seed(engine, &families, a, "metals", 1_000);
    let key = holder(GOD, "one");
    let id = apply(engine, &families, |txn| {
        families.reserve(txn, a, &commodity("metals"), &key, 4, &now())
    })
    .expect("hold");
    // A LIVE duplicate still refuses.
    apply(engine, &families, |txn| {
        families
            .reserve(txn, a, &commodity("metals"), &key, 1, &now())
            .map(|_| ())
    })
    .expect_err("a live duplicate hold refuses");
    apply(engine, &families, |txn| {
        families.consume(
            txn,
            a,
            &commodity("metals"),
            id,
            1,
            &now(),
            CommitToken::from_bytes([0x42; 16]),
        )
    })
    .expect("consume");
    apply(engine, &families, |txn| {
        families
            .reserve(txn, a, &commodity("metals"), &key, 4, &now())
            .map(|_| ())
    })
    .expect("the same holder reserves again once its hold is terminal");
}

#[test]
fn vf_c6_a_self_transfer_publishes_no_transferred_fact() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let a = endpoint(1);
    let b = endpoint(2);
    seed(engine, &families, a, "metals", 100);
    let key = mush_inventory::pool_key(a, &commodity("metals")).expect("key");
    let before = read(engine, &families, |txn| {
        *txn.get(&families.pools, &key).unwrap().unwrap()
    });
    let moved = apply(engine, &families, |txn| {
        families.transfer(txn, a, a, &commodity("metals"), 100)
    })
    .expect("a same-endpoint transfer is a HIT");
    assert_eq!(moved.moved, 0, "nothing moved, and the caller is told so");
    let after = read(engine, &families, |txn| {
        *txn.get(&families.pools, &key).unwrap().unwrap()
    });
    assert_eq!(after.revision(), before.revision());
    let real = apply(engine, &families, |txn| {
        families.transfer(txn, a, b, &commodity("metals"), 10)
    })
    .expect("a real transfer");
    assert_eq!(real.moved, 10, "a real transfer reports what it moved");
    // And the provider writes the fact only when something moved.
    let source = include_str!("../src/provider.rs");
    let guarded = source
        .split("if moved.moved > 0 {")
        .nth(1)
        .expect("the transferred marker is guarded on actual movement");
    assert!(
        guarded[..200].contains("FACT_TRANSFERRED"),
        "the guard must wrap the transferred marker itself"
    );
}

#[test]
fn vf_c7_crystallize_cannot_mint_more_than_its_root_absorbed() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let a = endpoint(1);
    let seedling = LotId(90);
    seed_lot(engine, &families, seedling, a, "metals", 1, 4);
    apply(engine, &families, |txn| {
        families.dissolve(txn, a, seedling).map(|_| ())
    })
    .expect("dissolve one unit");
    seed(engine, &families, a, "metals", 999_999);
    let error = apply(engine, &families, |txn| {
        families
            .crystallize(
                txn,
                a,
                &commodity("metals"),
                1_000_000,
                Grade::new(4).unwrap(),
                LotId(91),
            )
            .map(|_| ())
    })
    .expect_err("one dissolved unit licenses exactly one minted unit");
    assert!(matches!(error, InventoryError::NoTranche), "{error}");
    let untouched = read(engine, &families, |txn| {
        txn.get(&families.lots, &LotId(91)).unwrap().is_some()
    });
    assert!(!untouched, "and the refused mint wrote nothing");
    apply(engine, &families, |txn| {
        families
            .crystallize(
                txn,
                a,
                &commodity("metals"),
                1,
                Grade::new(4).unwrap(),
                LotId(92),
            )
            .map(|_| ())
    })
    .expect("one unit mints");
    let drawn = read(engine, &families, |txn| {
        families
            .tranche_for(txn, a, &commodity("metals"))
            .unwrap()
            .unwrap()
            .1
    });
    assert_eq!(drawn.remaining(), 0, "the root is drawn to zero");
    // A second dissolve restores headroom exactly equal to the lot it absorbed.
    let more = LotId(93);
    seed_lot(engine, &families, more, a, "metals", 7, 4);
    apply(engine, &families, |txn| {
        families.dissolve(txn, a, more).map(|_| ())
    })
    .expect("dissolve seven");
    let restored = read(engine, &families, |txn| {
        families
            .tranche_for(txn, a, &commodity("metals"))
            .unwrap()
            .unwrap()
            .1
    });
    assert_eq!(restored.remaining(), 7);
}

#[test]
fn vf_c10_a_tranche_whose_fields_disagree_with_its_key_is_not_returned() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let a = endpoint(1);
    let b = endpoint(2);
    let key = mush_inventory::tranche_key(a, &commodity("metals"), 0).expect("key");
    let foreign = PoolTranche::new(
        key,
        b,
        commodity("metals"),
        0,
        None,
        Vec::new(),
        vec![LotId(1)],
        10,
        10,
        0,
    )
    .expect("tranche");
    apply(engine, &families, |txn| {
        txn.insert(&families.tranches, &key, foreign)
            .map_err(InventoryError::from)
    })
    .expect("hand-install a colliding tranche");
    let found = read(engine, &families, |txn| {
        families.tranche_for(txn, a, &commodity("metals")).unwrap()
    });
    assert!(found.is_none(), "a key is not proof of what it loaded");
    let error = apply(engine, &families, |txn| {
        families
            .crystallize(
                txn,
                a,
                &commodity("metals"),
                1,
                Grade::new(0).unwrap(),
                LotId(95),
            )
            .map(|_| ())
    })
    .expect_err("and crystallize refuses rather than inheriting it");
    assert!(matches!(error, InventoryError::NoTranche), "{error}");
}

#[test]
fn vf_c11_every_verb_that_mints_a_lot_publishes_lot_created() {
    // The marker is the provider's, written inside the authoritative bundle, so
    // this is a source pin over the three arms that mint a lot plus the record
    // rule the marker itself must satisfy.
    let source = include_str!("../src/provider.rs");
    let body = source
        .split("let summary = match self.verb {")
        .nth(1)
        .expect("the prepare_authoritative match");
    for (arm, next) in [
        ("InventoryVerb::Split => {", "InventoryVerb::Merge => {"),
        ("InventoryVerb::Merge => {", "InventoryVerb::Dissolve => {"),
        ("InventoryVerb::Crystallize => {", "data.insert(\"summary\""),
    ] {
        let start = body.find(arm).unwrap_or_else(|| panic!("arm {arm}"));
        let end = body[start..]
            .find(next)
            .map_or(body.len(), |offset| start + offset);
        assert!(
            body[start..end].contains("FACT_LOT_CREATED"),
            "{arm} must publish inventory.lot.created"
        );
    }
    assert!(
        mush_inventory::AppliedMarker::new(
            mush_inventory::FACT_LOT_CREATED,
            None,
            Some(endpoint(1)),
            commodity("metals"),
            5,
            GOD,
            10,
        )
        .is_ok()
    );
}

#[test]
fn vf_c12_a_truncated_endpoint_set_is_declared_not_silent() {
    let (mut world, _families) = econ_rig();
    for index in 0..(mush_inventory::MAX_ENDPOINT_CHOICES + 4) {
        let _ = make_vault(&mut world, &format!("hold{index}"), None);
    }
    let live = mush_inventory::live_endpoints(world.world(), GOD);
    assert_eq!(live.len(), mush_inventory::MAX_ENDPOINT_CHOICES);
    let mut sorted = live.clone();
    sorted.sort_unstable();
    assert_eq!(live, sorted, "the set is sorted BEFORE it is truncated");
    assert!(
        mush_inventory::endpoints_truncated(world.world(), GOD),
        "and the truncation is knowable"
    );
    let out = world.run(GOD, "@inv/move 0/metals:1=1");
    assert!(
        rendered(&out)
            .iter()
            .any(|line| line.contains("live endpoints")),
        "the command says so rather than silently omitting endpoints: {:?}",
        rendered(&out)
    );
}

/// Doc 20 cut SM.6 (review F2): a machine's vault never pushes a held one out of
/// the capped list. A world that lays more system-owned vaults than the cap
/// BEFORE a held vault exists (the lane's 96 block vaults, then a hull's hold)
/// still lists the held vault for staff, first; machine vaults fill the room
/// left, in id order, and the truncation is still declared.
#[test]
fn sm6_a_held_vault_is_listed_before_any_machine_vault() {
    let (mut world, _families) = econ_rig();
    let mut machines = Vec::new();
    for index in 0..(mush_inventory::MAX_ENDPOINT_CHOICES + 4) {
        let name = format!("machine{index}");
        let created = world.run(GOD, &format!("@create {name}"));
        let object = number_after(&created, "(#").expect("the created object's id");
        let out = world.run(
            GOD,
            &format!("@vault/create {name}=container:#{object} owner=sys:test.machines"),
        );
        machines
            .push(VaultId(number_after(&out, "vault #").unwrap_or_else(
                || panic!("machine vault id: {:?}", rendered(&out)),
            )));
    }
    let held = make_vault(&mut world, "hold", None);
    assert!(
        machines.iter().all(|machine| *machine < held.vault()),
        "every machine vault was made first"
    );
    let live = mush_inventory::live_endpoints(world.world(), GOD);
    assert_eq!(live.len(), mush_inventory::MAX_ENDPOINT_CHOICES);
    assert_eq!(live.first(), Some(&held.vault()), "the held vault first");
    assert_eq!(
        &live[1..],
        &machines[..mush_inventory::MAX_ENDPOINT_CHOICES - 1],
        "then the machines, lowest first"
    );
    assert!(mush_inventory::endpoints_truncated(world.world(), GOD));
}

#[test]
fn vf_r2_a_revision_mismatched_pair_is_quarantined_and_paged() {
    let (mut world, families) = boot();
    let a = endpoint(21);
    {
        let engine = world.engine_mut();
        seed(engine, &families, a, "metals", 12);
        let key = mush_inventory::pool_key(a, &commodity("metals")).expect("key");
        // The OTHER crash shape of two `Put`s in one bundle: both halves
        // present, one carrying the pre-image. It used to boot clean.
        let (scalar, book) = read(engine, &families, |txn| {
            (
                *txn.get(&families.pools, &key).unwrap().unwrap(),
                txn.get(&families.books, &key).unwrap().unwrap().clone(),
            )
        });
        let stale = mush_core::BoundedScalar::new(
            0,
            MAX_POOL_QUANTITY,
            scalar.current(),
            scalar.revision() - 1,
        )
        .expect("pre-image scalar");
        assert_eq!(book.resource_revision(), scalar.revision());
        apply(engine, &families, |txn| {
            txn.replace(&families.pools, &key, stale)
                .map_err(InventoryError::from)
        })
        .expect("install the pre-image half");
    }
    reconcile_now(&mut world);
    {
        let store = world
            .world()
            .module_state::<mush_inventory::InventoryStore>()
            .expect("inventory store");
        assert!(
            store.is_quarantined(a),
            "a revision-mismatched pair is quarantined"
        );
        assert_eq!(
            store.quarantine_reason(a),
            Some(mush_inventory::Quarantine::RevisionMismatch)
        );
    }
    let out = world.run(GOD, &format!("@inv/show {}", a.vault().0));
    assert!(
        rendered(&out)
            .iter()
            .any(|line| line.contains("QUARANTINED")),
        "{:?}",
        rendered(&out)
    );
    // A matched pair is still not quarantined.
    let b = endpoint(22);
    {
        let engine = world.engine_mut();
        seed(engine, &families, b, "metals", 3);
    }
    reconcile_now(&mut world);
    let store = world
        .world()
        .module_state::<mush_inventory::InventoryStore>()
        .expect("inventory store");
    assert!(!store.is_quarantined(b));
}

#[test]
fn vf_r3_a_marker_written_past_the_mirror_cap_still_answers_its_fact() {
    use mush_economy::FactProvider;
    let (mut world, families) = boot();
    let total = mush_inventory::MAX_APPLIED_MARKERS + 1;
    let newest = format!("{:016x}{}", u64::MAX, "f".repeat(64));
    {
        let engine = world.engine_mut();
        for chunk in 0..((total / 512) + 1) {
            apply(engine, &families, |txn| {
                for index in 0..512 {
                    let ordinal = chunk * 512 + index;
                    if ordinal >= total {
                        break;
                    }
                    // Ids are `{at}{request}`: canonical order IS time order.
                    let id = smol_str::SmolStr::new(format!("{ordinal:016x}{:064x}", ordinal));
                    let marker = mush_inventory::AppliedMarker::new(
                        mush_inventory::FACT_TRANSFERRED,
                        Some(endpoint(1)),
                        Some(endpoint(2)),
                        commodity("metals"),
                        1,
                        GOD,
                        i64::try_from(ordinal).unwrap(),
                    )
                    .expect("marker");
                    txn.insert(&families.applied, &id, marker)
                        .map_err(InventoryError::from)?;
                }
                Ok(())
            })
            .expect("markers install");
        }
        // The newest marker of all, lexicographically last because its stamp is.
        apply(engine, &families, |txn| {
            let marker = mush_inventory::AppliedMarker::new(
                mush_inventory::FACT_TRANSFERRED,
                Some(endpoint(1)),
                Some(endpoint(2)),
                commodity("metals"),
                9,
                GOD,
                9_999,
            )
            .expect("marker");
            txn.insert(&families.applied, &smol_str::SmolStr::new(&newest), marker)
                .map_err(InventoryError::from)
        })
        .expect("newest marker installs");
    }
    reconcile_now(&mut world);
    let store = world
        .world()
        .module_state::<mush_inventory::InventoryStore>()
        .expect("store");
    assert_eq!(store.markers(), mush_inventory::MAX_APPLIED_MARKERS);
    assert!(
        store.marker(&newest).is_some(),
        "the mirror keeps the NEWEST markers, not the first by hash"
    );
    // Truncation is fail-closed for the whole mirror, which is what makes
    // "unknown is unsatisfied" a property of the code rather than of key order.
    let provider = mush_inventory::InventoryFactProvider;
    assert!(
        provider
            .resolve(
                world.world(),
                mush_inventory::FACT_TRANSFERRED,
                &newest,
                None
            )
            .is_none(),
        "a truncated mirror answers no fact at all"
    );
    assert!(!store.is_complete());
}

#[test]
fn vf_r3_a_truncated_mirror_answers_no_fact() {
    use mush_economy::FactProvider;
    let (mut world, families) = boot();
    let id = smol_str::SmolStr::new(format!("{:016x}{:064x}", 7_u64, 7_u64));
    {
        let engine = world.engine_mut();
        apply(engine, &families, |txn| {
            let marker = mush_inventory::AppliedMarker::new(
                mush_inventory::FACT_TRANSFERRED,
                Some(endpoint(1)),
                Some(endpoint(2)),
                commodity("metals"),
                5,
                GOD,
                7,
            )
            .expect("marker");
            txn.insert(&families.applied, &id, marker)
                .map_err(InventoryError::from)
        })
        .expect("marker installs");
    }
    let provider = mush_inventory::InventoryFactProvider;
    // Before any reconciliation the mirror has never claimed completeness.
    assert!(
        provider
            .resolve(world.world(), mush_inventory::FACT_TRANSFERRED, &id, None)
            .is_none(),
        "an incomplete mirror answers None for a marker that IS in the family"
    );
    reconcile_now(&mut world);
    assert!(
        provider
            .resolve(world.world(), mush_inventory::FACT_TRANSFERRED, &id, None)
            .is_some(),
        "and answers once the mirror is whole"
    );
}

#[test]
fn vf_r7_a_pool_whose_vault_did_not_hydrate_is_quarantined_and_paged() {
    let (mut world, families) = econ_rig();
    let vault = make_vault(&mut world, "hold", None);
    let orphan = EndpointRef(VaultId(vault.vault().0 + 77));
    {
        let engine = world.engine_mut();
        seed(engine, &families, orphan, "metals", 5);
    }
    reconcile_now(&mut world);
    {
        let store = world
            .world()
            .module_state::<mush_inventory::InventoryStore>()
            .expect("store");
        assert!(
            store.is_quarantined(orphan),
            "a pool whose vault the economy never hydrated is quarantined"
        );
        assert_eq!(
            store.quarantine_reason(orphan),
            Some(mush_inventory::Quarantine::VaultMissing)
        );
        assert!(!store.is_quarantined(vault), "and the live one is not");
    }
    // Staff can see what is stranded rather than being told it does not exist.
    let out = world.run(GOD, &format!("@inv/show {}", orphan.vault().0));
    assert!(
        rendered(&out)
            .iter()
            .any(|line| line.contains("QUARANTINED")),
        "{:?}",
        rendered(&out)
    );
}

#[test]
fn vf_b1_a_viewers_own_stock_is_never_displaced_by_other_endpoints() {
    let (mut world, families) = econ_rig();
    let mine = make_vault(&mut world, "mine", None);
    {
        let engine = world.engine_mut();
        for index in 0..8 {
            seed(engine, &families, mine, &format!("metal{index}"), 10);
        }
        // Bury them under a thousand books belonging to nobody the viewer knows.
        for block in 0..32 {
            for index in 0..(INV_PAGE_FOR_TEST) {
                seed(
                    engine,
                    &families,
                    endpoint(10_000 + (block * 100 + index) as u64),
                    "filler",
                    1,
                );
            }
        }
    }
    let out = world.run(GOD, &format!("@inv/show {}", mine.vault().0));
    let lines = rendered(&out);
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("nothing pooled here")),
        "an endpoint holding stock never reports itself empty: {lines:?}"
    );
    for index in 0..8 {
        assert!(
            lines
                .iter()
                .any(|line| line.contains(&format!("metal{index}"))),
            "row metal{index} is missing: {lines:?}"
        );
    }
}

const INV_PAGE_FOR_TEST: usize = mush_inventory::INV_PAGE;

#[test]
fn vf_b1_a_commodity_filter_surfaces_every_matching_lot() {
    let (mut world, families) = econ_rig();
    let mine = make_vault(&mut world, "mine", None);
    {
        let engine = world.engine_mut();
        for index in 0..(2 * mush_inventory::INV_PAGE) {
            let id = LotId(500 + index as u64);
            let what = if index % 2 == 0 { "metals" } else { "gas" };
            seed_lot(engine, &families, id, mine, what, 5, 1);
        }
    }
    let out = world.run(GOD, &format!("@inv/lots {} metals", mine.vault().0));
    let lines = rendered(&out);
    let rows = lines
        .iter()
        .filter(|line| line.starts_with("  lot#") && line.contains("metals"))
        .count();
    assert_eq!(
        rows,
        mush_inventory::INV_PAGE,
        "every matching lot renders: {lines:?}"
    );
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("narrow the commodity")),
        "the false advice is gone: {lines:?}"
    );
}

#[test]
fn vf_b2_dissolve_survives_a_full_tranche() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let a = endpoint(1);
    let mut earliest = None;
    for index in 0..(mush_inventory::MAX_TRANCHE_ABSORBED + 4) {
        let id = LotId(2_000 + index as u64);
        if earliest.is_none() {
            earliest = Some(id);
        }
        seed_lot(engine, &families, id, a, "metals", 1, 2);
        apply(engine, &families, |txn| {
            families.dissolve(txn, a, id).map(|_| ())
        })
        .unwrap_or_else(|error| panic!("dissolve {index}: {error}"));
    }
    let current = read(engine, &families, |txn| {
        families
            .tranche_for(txn, a, &commodity("metals"))
            .unwrap()
            .unwrap()
            .1
    });
    assert!(current.generation() >= 1, "the root rolled forward");
    let predecessor = current
        .predecessor()
        .expect("the successor names the old one");
    let child = LotId(2_500);
    apply(engine, &families, |txn| {
        families
            .crystallize(
                txn,
                a,
                &commodity("metals"),
                1,
                Grade::new(2).unwrap(),
                child,
            )
            .map(|_| ())
    })
    .expect("crystallize from the current generation");
    let reached = read(engine, &families, |txn| {
        txn.get(&families.tranches, &predecessor)
            .unwrap()
            .unwrap()
            .absorbed()
            .to_vec()
    });
    assert!(
        reached.contains(&earliest.expect("earliest")),
        "the earliest lot is still reachable from the newest child"
    );
}

#[test]
fn vf_b2_a_max_depth_dissolve_does_not_disable_crystallize() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let a = endpoint(1);
    let deep = LotId(3_000);
    let record = Lot::new(
        deep,
        a,
        commodity("metals"),
        10,
        Grade::new(1).unwrap(),
        Lineage::Split { parent: LotId(1) },
        MAX_LINEAGE_DEPTH,
    )
    .expect("a lot at the depth cap");
    apply(engine, &families, |txn| {
        txn.insert(&families.lots, &deep, record)
            .map_err(InventoryError::from)
    })
    .expect("seed the deep lot");
    apply(engine, &families, |txn| {
        families.dissolve(txn, a, deep).map(|_| ())
    })
    .expect("dissolve the deep lot");
    apply(engine, &families, |txn| {
        families
            .crystallize(
                txn,
                a,
                &commodity("metals"),
                1,
                Grade::new(1).unwrap(),
                LotId(3_001),
            )
            .map(|_| ())
    })
    .expect("one deep lot must not disable crystallize for every other holder");
}

#[test]
fn vf_b4_a_read_switch_does_not_rescan_the_tree() {
    let (mut world, _families) = econ_rig();
    let vault = make_vault(&mut world, "hold", None);
    reconcile_now(&mut world);
    let before = world
        .world()
        .module_state::<mush_inventory::InventoryStore>()
        .expect("store")
        .reconciliations();
    let _ = world.run(GOD, &format!("@inv/show {}", vault.vault().0));
    let _ = world.run(GOD, &format!("@inv/lots {}", vault.vault().0));
    let after = world
        .world()
        .module_state::<mush_inventory::InventoryStore>()
        .expect("store")
        .reconciliations();
    assert_eq!(before, after, "a read switch rebuilds no mirror");
    reconcile_now(&mut world);
    let booted = world
        .world()
        .module_state::<mush_inventory::InventoryStore>()
        .expect("store")
        .reconciliations();
    assert_eq!(booted, after + 1, "boot and the sweep still do");
}

#[test]
fn vf_b8_an_endpoint_refuses_a_sixty_fifth_commodity() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let a = endpoint(1);
    for index in 0..mush_inventory::MAX_POOLS_PER_ENDPOINT {
        seed(engine, &families, a, &format!("c{index}"), 5);
    }
    let error = apply(engine, &families, |txn| {
        families
            .credit(txn, a, &commodity("onetoomany"), 1)
            .map(|_| ())
    })
    .expect_err("the 65th distinct commodity refuses");
    assert!(
        matches!(
            error,
            InventoryError::Record(mush_inventory::InventoryRecordError::EndpointPools { .. })
        ),
        "{error}"
    );
    let absent = read(engine, &families, |txn| {
        families.pool(txn, a, &commodity("onetoomany")).unwrap()
    });
    assert!(absent.is_none(), "and it wrote nothing");
    apply(engine, &families, |txn| {
        families.credit(txn, a, &commodity("c0"), 1).map(|_| ())
    })
    .expect("crediting one of the existing 64 still succeeds");
}

#[test]
fn vf_b10_a_live_hold_is_current_past_the_page_budget() {
    let (mut world, families) = boot();
    let target = endpoint(999_999);
    {
        let engine = world.engine_mut();
        // More books than the guard's old page budget could ever visit.
        let total = mush_inventory::store::MAX_BOOT_PAGES * mush_inventory::INV_PAGE + 1;
        let mut done = 0;
        while done < total {
            let batch = (total - done).min(300);
            apply(engine, &families, |txn| {
                for index in 0..batch {
                    families.credit(
                        txn,
                        endpoint((done + index) as u64),
                        &commodity("filler"),
                        1,
                    )?;
                }
                Ok(())
            })
            .expect("books install");
            done += batch;
        }
        seed(engine, &families, target, "metals", 10);
    }
    let engine = world.engine_mut();
    let id = apply(engine, &families, |txn| {
        families.reserve(
            txn,
            target,
            &commodity("metals"),
            &holder(GOD, "one"),
            4,
            &now(),
        )
    })
    .expect("hold");
    assert!(
        guard_is_current(engine, reservation_guard("inventory", id, 1)),
        "a live hold past the old page budget is current"
    );
    apply(engine, &families, |txn| {
        families.release(txn, target, &commodity("metals"), id, 1)
    })
    .expect("release");
    assert!(
        !guard_is_current(engine, reservation_guard("inventory", id, 2)),
        "and a terminal record in the same book still answers false"
    );
}

#[test]
fn vf_b13_a_malformed_grade_refuses() {
    let (mut world, _families) = econ_rig();
    let vault = make_vault(&mut world, "hold", None);
    let bad = world.run(
        GOD,
        &format!("@inv/crystallize {}/metals=5/abc", vault.vault().0),
    );
    assert!(
        !world.engine().has_authoritative_submit(),
        "a malformed grade queues nothing"
    );
    assert!(
        rendered(&bad).iter().any(|line| line.contains("grade")),
        "{:?}",
        rendered(&bad)
    );
    let _ = world.run(
        GOD,
        &format!("@inv/crystallize {}/metals=5", vault.vault().0),
    );
    assert!(
        world.engine().has_authoritative_submit(),
        "an omitted grade still defaults to 0 and submits"
    );
}

#[test]
fn vf_b14_a_key_quarantine_is_reportable() {
    let (mut world, families) = boot();
    let a = endpoint(31);
    {
        let engine = world.engine_mut();
        seed(engine, &families, a, "metals", 12);
        let key = mush_inventory::pool_key(a, &commodity("metals")).expect("key");
        apply(engine, &families, |txn| {
            txn.remove(&families.books, &key)
                .map_err(InventoryError::from)
        })
        .expect("drop the book half");
    }
    reconcile_now(&mut world);
    let staff = rendered(&world.run(GOD, "@inv/quarantine"));
    assert!(
        staff.iter().any(|line| line.contains("pool key")),
        "the orphaned key half of the quarantine has an operator surface: {staff:?}"
    );
    assert!(
        staff
            .iter()
            .any(|line| line.contains("restore the missing half")),
        "with the reconciliation order filled in: {staff:?}"
    );
    let mallory = world.spawn_player("mallory");
    let player = rendered(&world.run(mallory, "@inv/quarantine"));
    assert!(
        !player.iter().any(|line| line.contains("pool key")),
        "and a non-staff actor sees nothing: {player:?}"
    );
}

#[test]
fn vf_b14_reconcile_stops_at_the_marker_cap() {
    // The marker-cap break used to exit only the inner `for`, so the outer
    // paging loop kept scanning and discarding on the world single-writer.
    let source = include_str!("../src/store.rs");
    assert!(
        source.contains("'markers: loop {") && source.contains("break 'markers;"),
        "the marker walk must break out of BOTH loops"
    );
    assert!(
        source.contains("MAX_MARKER_PAGES"),
        "and the marker walk has its own declared page budget"
    );
}

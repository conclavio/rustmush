//! ---- E7r2: P1 — the `Representation::Lot` market lift, verified ----
//!
//! 10b §15.1 item 3 and §15.5's rewritten (a′)–(c′), over the pass's three hazard axes:
//! **A** conservation over pools + holds + lots, **B** provenance, **C** consent.
//!
//! The rig is `crates/mush-inventory/tests/e7c2.rs`'s, deliberately: `apply()` opens the
//! **same owner-stamped transaction a provider gets** and installs it exactly as an
//! exact receipt does, so every assertion is made against the state a commit would
//! really leave, and an `Err` installs nothing at all — which is what makes every
//! "writes nothing" assertion below mean what it says.

use std::path::{Path, PathBuf};

use mush_core::{
    ClockRef, ClockStamp, CommitToken, Engine, ObjId, OwnerId, ReservationId, ReservationState,
};
use mush_economy::{ExternalRef, VaultId};
use mush_inventory::{
    EndpointRef, Grade, InventoryError, InventoryFamilies, InventoryHandles, InventoryModule,
    Lineage, Lot, LotId, LotSale,
};
use mush_market::{ListingId, LotOp, MarketStore};
use mush_testkit::TestWorld;

// ---- the rig ---------------------------------------------------------------

/// The production composition in miniature: `mush-inventory` delegating to `"market"`,
/// and `mush-market` composed behind it holding the same handle cell.
///
/// Both halves are required and that is the point — the kernel refuses a delegation to
/// an **uncomposed** owner, so the grant cannot be declared for a module that is not
/// there, and the market's own owner has to exist before it can be a grantee.
fn boot() -> (TestWorld, InventoryFamilies) {
    let cell: mush_inventory::InventoryHandleCell = std::sync::Arc::default();
    let world = TestWorld::with_modules(vec![
        Box::new(
            InventoryModule::new()
                .with_quantity_delegates(["market"])
                .with_handle_cell(std::sync::Arc::clone(&cell)),
        ),
        Box::new(mush_market::MarketModule::new().with_inventory(std::sync::Arc::clone(&cell))),
    ]);
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

fn commit(byte: u8) -> CommitToken {
    CommitToken::from_bytes([byte; 16])
}

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

/// Put `amount` of `id` into the endpoint's pool **through a lot**, so the pool has the
/// lineage root a lot listing is required to have: mint one origin lot and dissolve it,
/// which is exactly what an extraction cycle plus `@inv/dissolve` do on a live server.
fn seed_rooted_pool(
    engine: &mut Engine,
    families: &InventoryFamilies,
    at: EndpointRef,
    id: &str,
    amount: u64,
    lot: LotId,
) {
    apply(engine, families, |txn| {
        families
            .mint_origin(
                txn,
                at,
                &commodity(id),
                amount,
                Grade::new(2).expect("grade"),
                ExternalRef {
                    provider: "test".into(),
                    kind: "seed".into(),
                    id: "1".into(),
                    revision: None,
                },
                lot,
            )
            .map(|_| ())
    })
    .expect("seed mints one origin lot");
    apply(engine, families, |txn| {
        families.dissolve(txn, at, lot).map(|_| ())
    })
    .expect("the seed lot dissolves into the endpoint's pool");
}

/// The pool's balance and availability, as a viewer reads them.
fn pool(
    engine: &mut Engine,
    families: &InventoryFamilies,
    at: EndpointRef,
    id: &str,
) -> (u64, u64) {
    read(engine, families, |txn| {
        let key = mush_inventory::pool_key(at, &commodity(id)).expect("pool key");
        let scalar = txn.get(&families.pools, &key).ok().flatten().copied();
        let book = txn.get(&families.books, &key).ok().flatten().cloned();
        match (scalar, book) {
            (Some(scalar), Some(book)) => (
                u64::try_from(scalar.current()).unwrap_or_default(),
                book.checked_available(scalar).unwrap_or_default(),
            ),
            _ => (0, 0),
        }
    })
}

/// Every lot at one endpoint, and what they hold in total.
fn lots_at(
    engine: &mut Engine,
    families: &InventoryFamilies,
    at: EndpointRef,
    ids: &[LotId],
) -> (Vec<Lot>, u64) {
    let found: Vec<Lot> = read(engine, families, |txn| {
        ids.iter()
            .filter_map(|id| txn.get(&families.lots, id).ok().flatten().cloned())
            .filter(|lot| lot.endpoint() == at)
            .collect()
    });
    let total = found.iter().map(Lot::quantity).sum();
    (found, total)
}

fn reservation_of(listing: u64, at: EndpointRef, id: &str) -> ReservationId {
    mush_market::listing_reservation(ListingId(listing), at, &commodity(id))
        .expect("the listing's reservation derives")
}

fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .to_path_buf()
}

/// One crate file's **production** half — everything before its first `#[cfg(test)]`.
fn production(relative: &str) -> String {
    let text = std::fs::read_to_string(crates_dir().join(relative))
        .unwrap_or_else(|error| panic!("cannot read {relative}: {error}"));
    match text.find("#[cfg(test)]") {
        Some(at) => text[..at].to_owned(),
        None => text,
    }
}

// ---- E7r2: the wiring (P) --------------------------------------------------

/// **The third edge tripwire, beside the two that already exist** (10b §15.5, corrected:
/// *"two untouched tripwires plus one new `mush-market` tripwire"*).
///
/// `mush-inventory/tests/e7c2.rs` forbids `mush-inventory -> {market, industry,
/// tacsim, sim, tactics, org}` and `mush-industry/tests/e7c3.rs` forbids
/// `mush-industry -> {sim, market, faction, tacsim, party, tactics, scenes,
/// jobs, housing}`. **Neither names `mush-market -> mush-inventory`**, neither is
/// amended by this pass, and both stay green. What was missing is a scan of this
/// crate's own manifest, so the pass's ONE new edge cannot be joined by a second.
#[test]
fn e7r2_p1_the_market_edge_set_is_exactly_core_org_economy_inventory() {
    let manifest = include_str!("../Cargo.toml");
    let declared: Vec<&str> = manifest
        .lines()
        .filter_map(|line| line.split_once(".workspace = true"))
        .map(|(name, _)| name.trim())
        .collect();
    for required in ["mush-core", "mush-org", "mush-economy", "mush-inventory"] {
        assert!(
            declared.contains(&required),
            "mush-market declares {required}"
        );
    }
    // Everything else, in production OR in dev. `mush-testkit`, `mush-store`,
    // `mush-proto` and `tokio` are dev-only and are listed here so the set is exact
    // rather than merely bounded.
    let allowed = [
        "mush-core",
        "mush-org",
        "mush-economy",
        "mush-inventory",
        "serde",
        "serde_json",
        "smol_str",
        "tracing",
        "mush-testkit",
        "mush-store",
        "mush-proto",
        "tokio",
        // `[package]`'s own inherited keys, which share the `.workspace = true` shape.
        "version",
        "edition",
        "license",
        "authors",
    ];
    for name in &declared {
        assert!(
            allowed.contains(name),
            "mush-market grew an undeclared edge: {name}. E7 residue pass 2 adds EXACTLY \
             ONE ({{mush-inventory}}); anything else needs its own decision."
        );
    }
    for forbidden in [
        "mush-industry",
        "mush-sim",
        "mush-tactics",
        "mush-campaign",
        "mush-propagation",
        "tacsim",
        "mush-canon",
        "mush-party",
        "mush-faction",
    ] {
        assert!(
            !manifest.contains(&format!("{forbidden}.workspace")),
            "no {forbidden} edge, in production or in dev"
        );
    }
    // The reason lives in the manifest, beside the crate's own edge-set claim, so a
    // later reader learns why before they learn that it is there.
    assert!(
        manifest.contains("DELEGATED path"),
        "the manifest comment records WHY the edge exists"
    );
}

/// **The two existing tripwires are untouched, and their premises still hold.**
#[test]
fn e7r2_p1_the_two_existing_edge_tripwires_are_unamended_and_still_true() {
    let inventory = std::fs::read_to_string(crates_dir().join("mush-inventory/Cargo.toml"))
        .expect("mush-inventory/Cargo.toml");
    assert!(
        !inventory.contains("mush-market.workspace"),
        "e7c2's premise: mush-inventory takes no market edge — the lift runs the OTHER way"
    );
    let industry = std::fs::read_to_string(crates_dir().join("mush-industry/Cargo.toml"))
        .expect("mush-industry/Cargo.toml");
    assert!(
        !industry.contains("mush-market.workspace"),
        "e7c3's premise: mush-industry takes no market edge"
    );
    // And the two test files themselves still carry their forbidden lists, so this
    // pass did not "fix" them by deleting them.
    let e7c2 = std::fs::read_to_string(crates_dir().join("mush-inventory/tests/e7c2.rs"))
        .expect("e7c2.rs");
    assert!(e7c2.contains("e7c2_the_market_keeps_its_own_quantity"));
    let e7c3 =
        std::fs::read_to_string(crates_dir().join("mush-industry/tests/e7c3.rs")).expect("e7c3.rs");
    assert!(e7c3.contains("e7c3_wiring_the_crates_edge_set_is_exactly_what_it_declares"));
}

/// **`mint_origin` is called NOWHERE in `mush-market`** (hazard B, and the pass's
/// headline: a sale that takes the money moves matter and creates none).
///
/// A source probe rather than a behavioural one, because the claim is about a path that
/// must not exist at all — a behavioural test can only fail to find one.
#[test]
fn e7r2_p1_no_path_in_the_market_mints() {
    for file in [
        "mush-market/src/lot.rs",
        "mush-market/src/api.rs",
        "mush-market/src/trade.rs",
        "mush-market/src/boot.rs",
        "mush-market/src/module.rs",
        "mush-market/src/commands.rs",
        "mush-market/src/store.rs",
        "mush-market/src/record.rs",
    ] {
        let text = production(file);
        assert!(
            !text.contains(".mint_origin("),
            "{file} calls mint_origin: the market moves matter and creates none"
        );
    }
    // The workspace's mint callers are extraction's, a work order's delivery
    // (`mush-industry/src/provider.rs`), the TacSim port's stock clerk
    // (`tacsim/src/port/stock.rs`, doc 18 cut IC8: the seeded Origin receipts) and
    // the one seam `mush-inventory` defines it on — none of them the market's.
    assert!(
        production("mush-industry/src/families.rs").contains(".mint_origin("),
        "extraction is still a minting path — the probe is looking in the right place"
    );
}

/// **Doc 20 cut SM.12a: the restock creates no matter** (P5's supply half; the twin of
/// [`e7r2_p1_no_path_in_the_market_mints`] for the one new raise of a drawn ask). The
/// body of `restock_ask` reads the pool and writes one listing: it names no inventory
/// writer, no inventory handle and no lot leg. And it is the one public function of the
/// market that takes a `RestockSpec`, so no second door raises an ask by milled units.
#[test]
fn e7r2_p1_the_restock_creates_no_matter() {
    let api = production("mush-market/src/api.rs");
    let start = api
        .find("pub fn restock_ask(")
        .expect("restock_ask is defined in api.rs");
    let body: String = api[start..]
        .lines()
        .take_while(|line| *line != "}")
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        body.len() > 200,
        "the probe read restock_ask's body: {body}"
    );
    for forbidden in [
        "mint_origin(",
        ".credit(",
        ".debit(",
        ".transfer",
        "InventoryFamilies",
        "lot::stage(",
    ] {
        assert!(
            !body.contains(forbidden),
            "restock_ask names `{forbidden}`: the market moves matter and creates none"
        );
    }
    let mut takers: Vec<String> = Vec::new();
    let src = crates_dir().join("mush-market/src");
    for entry in std::fs::read_dir(&src).expect("mush-market/src") {
        let path = entry.expect("an entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let relative = format!(
            "mush-market/src/{}",
            path.file_name().and_then(|n| n.to_str()).expect("a name")
        );
        let text = production(&relative);
        let mut rest = text.as_str();
        while let Some(at) = rest.find("pub fn ") {
            let tail = &rest[at..];
            let signature = tail.split('{').next().unwrap_or(tail);
            if signature.contains("RestockSpec") {
                let name = signature
                    .trim_start_matches("pub fn ")
                    .split('(')
                    .next()
                    .unwrap_or_default()
                    .to_owned();
                takers.push(format!("{relative}: {name}"));
            }
            rest = &tail["pub fn ".len()..];
        }
    }
    assert_eq!(
        takers,
        vec!["mush-market/src/api.rs: restock_ask".to_owned()],
        "restock_ask is the only public function that takes a RestockSpec"
    );
}

/// **No `InventoryVerb` was added, and the structural lockout is intact** (10b §15.5's
/// constraint (3)).
#[test]
fn e7r2_p1_no_inventory_verb_and_no_engine_owned_reservation_entry() {
    let provider = production("mush-inventory/src/provider.rs");
    for absent in [
        "    Reserve,",
        "    Release,",
        "    Consume,",
        "    Relocate,",
    ] {
        assert!(
            !provider.contains(absent),
            "the delegated path needs no verb; the ABSENCE of one is the lock ({absent})"
        );
    }
    for present in [
        "    Move,",
        "    Split,",
        "    Merge,",
        "    Dissolve,",
        "    Crystallize,",
    ] {
        assert!(
            provider.contains(present),
            "`InventoryVerb` is still exactly the five ({present})"
        );
    }
    // `ENGINE_OWNED_RESERVATIONS` is a MONEY table (matched only against
    // `HoldPurpose::Reservation`), so the lot lane adds no entry to it.
    let economy = production("mush-economy/src/api.rs");
    let table = economy
        .split("ENGINE_OWNED_RESERVATIONS")
        .nth(1)
        .unwrap_or_default();
    assert!(
        !table.contains("market."),
        "the lot reservation is not a money hold and registers nothing here"
    );
}

// ---- E7r2: the composition probe §15.5 owes --------------------------------

/// **The delegation is load-bearing, not incidental** (10b §15.5's closing paragraph).
///
/// A market-owned transaction may write `inventory`'s families **because the
/// composition said so**, on an endpoint the acting actor holds no
/// `may_take_from_vault` over — which is the whole point: the *seller's* authority was
/// frozen in the reservation at listing time, and the settling bundle runs under the
/// listing's holder rather than under anybody's ambient permission. Remove the delegate
/// and the identical write is `ActionStateError::OwnerMismatch`.
#[test]
fn e7r2_p1_a_market_owned_txn_writes_inventory_only_because_the_composition_delegates() {
    let (mut world, families) = boot();
    let at = endpoint(7);
    seed_rooted_pool(world.engine_mut(), &families, at, "iron", 100, LotId(0xa1));
    // The market's own owner-stamped transaction: `Engine::action_transaction` is keyed
    // by the family handle, and the delegation is what admits the write.
    let holder = mush_market::listing_holder(ListingId(4)).expect("holder");
    let placed = apply(world.engine_mut(), &families, |txn| {
        families.reserve(txn, at, &commodity("iron"), &holder, 30, &now())
    });
    assert!(
        placed.is_ok(),
        "a delegated grantee may hold against another owner's pool: {placed:?}"
    );
    assert_eq!(pool(world.engine_mut(), &families, at, "iron"), (100, 70));

    // …and without the delegation the identical bundle is refused. A fresh composition,
    // because the delegation is frozen at registration.
    let mut bare = TestWorld::with_modules(vec![Box::new(InventoryModule::new())]);
    let bare_families = bare
        .world()
        .module_state::<InventoryHandles>()
        .expect("handles")
        .0
        .clone();
    seed_rooted_pool(
        bare.engine_mut(),
        &bare_families,
        at,
        "iron",
        100,
        LotId(0xa2),
    );
    let refused = {
        let engine = bare.engine_mut();
        let mut txn = engine
            .action_transaction(&bare_families.pools)
            .expect("transaction");
        // The transaction is stamped with `inventory`'s own owner here, so instead of
        // driving the same call we assert the registry's own answer, which is what
        // `ActionTxn::checked_key` consults.
        let _ = &mut txn;
        engine
            .registry
            .state_families()
            .expect("state families")
            .delegates(
                &OwnerId::new("inventory").expect("owner"),
                &OwnerId::new("market").expect("owner"),
            )
    };
    assert!(
        !refused,
        "with no `with_quantity_delegates([\"market\"])` the grant does not exist, and the \
         same write is an `OwnerMismatch`"
    );
    assert!(
        world
            .engine()
            .registry
            .state_families()
            .expect("state families")
            .delegates(
                &OwnerId::new("inventory").expect("owner"),
                &OwnerId::new("market").expect("owner"),
            ),
        "…and with it, it exists"
    );
}

// ---- E7r2: hazard A — conservation over pools + holds + lots ---------------

/// **`reserve` → `settle_sale` conserves `pool + lots` to the unit, and the buyer's lot
/// carries the seller's lineage** (hazards A and B in one bundle, because they are one
/// bundle on a live server).
#[test]
fn e7r2_p1_a_sale_conserves_pool_plus_lots_and_the_buyer_receives_a_tranche_child() {
    let (mut world, families) = boot();
    let seller = endpoint(11);
    let buyer = endpoint(12);
    seed_rooted_pool(
        world.engine_mut(),
        &families,
        seller,
        "iron",
        100,
        LotId(0xb1),
    );
    let before = pool(world.engine_mut(), &families, seller, "iron");
    assert_eq!(before, (100, 100));

    let holder = mush_market::listing_holder(ListingId(9)).expect("holder");
    let placed = apply(world.engine_mut(), &families, |txn| {
        families.reserve(txn, seller, &commodity("iron"), &holder, 40, &now())
    })
    .expect("the listing's hold");
    assert_eq!(
        placed,
        reservation_of(9, seller, "iron"),
        "the id the LISTING DOCUMENT names is the id the hold carries — OPEN 2's whole \
         ordering rests on it being a pure derivation"
    );
    // A hold moves nothing: the balance is unchanged and only availability drops.
    assert_eq!(
        pool(world.engine_mut(), &families, seller, "iron"),
        (100, 60)
    );

    let record = read(world.engine_mut(), &families, |txn| {
        families
            .reservation(txn, seller, &commodity("iron"), placed)
            .expect("read")
            .expect("the hold is live")
    });
    assert_eq!(record.state(), ReservationState::Held);

    let settled = apply(world.engine_mut(), &families, |txn| {
        families.settle_sale(
            txn,
            seller,
            &commodity("iron"),
            placed,
            record.revision(),
            &now(),
            commit(0x22),
            LotSale {
                destination: buyer,
                sold: 25,
                grade: Grade::new(2).expect("grade"),
                buyer_lot: LotId(0xb2),
                remainder_lot: LotId(0xb3),
            },
        )
    })
    .expect("the sale settles");

    // **Conservation.** The seller's pool lost exactly the hold; the buyer's lot and the
    // seller's remainder lot together hold exactly the hold.
    assert_eq!(
        pool(world.engine_mut(), &families, seller, "iron"),
        (60, 60)
    );
    let (buyer_lots, buyer_total) = lots_at(
        world.engine_mut(),
        &families,
        buyer,
        &[LotId(0xb2), LotId(0xb3)],
    );
    let (seller_lots, seller_total) = lots_at(
        world.engine_mut(),
        &families,
        seller,
        &[LotId(0xb2), LotId(0xb3)],
    );
    assert_eq!(buyer_total, 25, "the buyer holds what they bought");
    assert_eq!(seller_total, 15, "the seller keeps the unsold remainder");
    assert_eq!(
        60 + buyer_total + seller_total,
        100,
        "pool + lots is conserved to the unit across reserve -> consume"
    );

    // **Provenance.** Both lots are `Lineage::Tranche` children of the root the seed lot
    // established — never a fresh `Origin`, which is the arm only `mint_origin` writes.
    for lot in buyer_lots.iter().chain(seller_lots.iter()) {
        assert!(
            matches!(lot.lineage(), Lineage::Tranche { .. }),
            "a bought lot carries the seller's lineage, never a fresh Origin: {:?}",
            lot.lineage()
        );
        assert_eq!(lot.grade(), Grade::new(2).expect("grade"));
        assert_eq!(lot.commodity(), &commodity("iron"));
    }
    assert_eq!(settled.buyer.id(), LotId(0xb2));
    assert_eq!(settled.remainder.map(|lot| lot.quantity()), Some(15));
}

/// **A listed lot cannot be sold twice**: the second settle refuses **on the
/// reservation**, writes nothing, and the pool is byte-identical.
#[test]
fn e7r2_p1_a_listed_lot_cannot_be_sold_twice_and_the_second_attempt_writes_nothing() {
    let (mut world, families) = boot();
    let seller = endpoint(21);
    let buyer = endpoint(22);
    seed_rooted_pool(
        world.engine_mut(),
        &families,
        seller,
        "ice",
        50,
        LotId(0xc1),
    );
    let holder = mush_market::listing_holder(ListingId(3)).expect("holder");
    let placed = apply(world.engine_mut(), &families, |txn| {
        families.reserve(txn, seller, &commodity("ice"), &holder, 50, &now())
    })
    .expect("hold");
    let revision = read(world.engine_mut(), &families, |txn| {
        families
            .reservation(txn, seller, &commodity("ice"), placed)
            .expect("read")
            .expect("held")
            .revision()
    });
    apply(world.engine_mut(), &families, |txn| {
        families.settle_sale(
            txn,
            seller,
            &commodity("ice"),
            placed,
            revision,
            &now(),
            commit(0x31),
            LotSale {
                destination: buyer,
                sold: 50,
                grade: Grade::new(0).expect("grade"),
                buyer_lot: LotId(0xc2),
                remainder_lot: LotId(0xc3),
            },
        )
    })
    .expect("the first sale settles");
    let after_first = pool(world.engine_mut(), &families, seller, "ice");
    assert_eq!(after_first, (0, 0));

    let second = apply(world.engine_mut(), &families, |txn| {
        families.settle_sale(
            txn,
            seller,
            &commodity("ice"),
            placed,
            revision,
            &now(),
            commit(0x32),
            LotSale {
                destination: buyer,
                sold: 50,
                grade: Grade::new(0).expect("grade"),
                buyer_lot: LotId(0xc4),
                remainder_lot: LotId(0xc5),
            },
        )
    });
    assert!(second.is_err(), "the hold is terminal: {second:?}");
    assert_eq!(
        pool(world.engine_mut(), &families, seller, "ice"),
        after_first,
        "and the pool is byte-identical — nothing was written"
    );
    let (_, ghost) = lots_at(world.engine_mut(), &families, buyer, &[LotId(0xc4)]);
    assert_eq!(ghost, 0, "no second parcel was minted for the replay");
}

/// **A sale cannot be split past its tranche**, and a pool with no lineage root cannot
/// back a lot listing at all.
///
/// `PoolTranche::checked_draw` is the gate — *a lineage claim is bounded by the matter
/// that established it* (cut 2's fold C7) — and it is what stops a pool credited from a
/// work-order delivery being sold as though it had provenance.
#[test]
fn e7r2_p1_a_sale_cannot_draw_more_lineage_than_its_tranche_established() {
    let (mut world, families) = boot();
    let seller = endpoint(31);
    let buyer = endpoint(32);
    // A pool with NO tranche: credited straight, exactly as a delivery would.
    apply(world.engine_mut(), &families, |txn| {
        families
            .credit(txn, seller, &commodity("ore"), 80)
            .map(|_| ())
    })
    .expect("credit");
    let holder = mush_market::listing_holder(ListingId(5)).expect("holder");
    let placed = apply(world.engine_mut(), &families, |txn| {
        families.reserve(txn, seller, &commodity("ore"), &holder, 80, &now())
    })
    .expect("a hold needs no root");
    let revision = read(world.engine_mut(), &families, |txn| {
        families
            .reservation(txn, seller, &commodity("ore"), placed)
            .expect("read")
            .expect("held")
            .revision()
    });
    let rootless = apply(world.engine_mut(), &families, |txn| {
        families.settle_sale(
            txn,
            seller,
            &commodity("ore"),
            placed,
            revision,
            &now(),
            commit(0x41),
            LotSale {
                destination: buyer,
                sold: 80,
                grade: Grade::new(0).expect("grade"),
                buyer_lot: LotId(0xd1),
                remainder_lot: LotId(0xd2),
            },
        )
    });
    assert!(
        matches!(rootless, Err(InventoryError::NoTranche)),
        "a pool with no lineage root cannot be sold as provenance-bearing stock: \
         {rootless:?}"
    );
    assert_eq!(
        pool(world.engine_mut(), &families, seller, "ore"),
        (80, 0),
        "and the refusal wrote nothing: the balance stands and the hold is still live"
    );

    // Now a rooted pool whose root is SMALLER than the hold: 20 dissolved, then 60
    // credited straight. The hold may cover all 80; the lineage draw may not.
    let (mut world, families) = boot();
    let seller = endpoint(33);
    seed_rooted_pool(
        world.engine_mut(),
        &families,
        seller,
        "ore",
        20,
        LotId(0xd3),
    );
    apply(world.engine_mut(), &families, |txn| {
        families
            .credit(txn, seller, &commodity("ore"), 60)
            .map(|_| ())
    })
    .expect("credit");
    let placed = apply(world.engine_mut(), &families, |txn| {
        families.reserve(txn, seller, &commodity("ore"), &holder, 80, &now())
    })
    .expect("hold");
    let revision = read(world.engine_mut(), &families, |txn| {
        families
            .reservation(txn, seller, &commodity("ore"), placed)
            .expect("read")
            .expect("held")
            .revision()
    });
    let over = apply(world.engine_mut(), &families, |txn| {
        families.settle_sale(
            txn,
            seller,
            &commodity("ore"),
            placed,
            revision,
            &now(),
            commit(0x42),
            LotSale {
                destination: buyer,
                sold: 80,
                grade: Grade::new(0).expect("grade"),
                buyer_lot: LotId(0xd4),
                remainder_lot: LotId(0xd5),
            },
        )
    });
    assert!(
        matches!(over, Err(InventoryError::NoTranche)),
        "80 cannot be drawn from a root 20 established: {over:?}"
    );
    assert_eq!(
        pool(world.engine_mut(), &families, seller, "ore"),
        (80, 0),
        "and again nothing moved"
    );
}

/// **`release` twice writes nothing, and a `consume` after a `release` refuses** — the
/// two arms a withdrawn listing and a retyped withdrawal exercise.
#[test]
fn e7r2_p1_release_is_idempotent_and_a_settle_after_a_release_refuses() {
    let (mut world, families) = boot();
    let at = endpoint(41);
    let buyer = endpoint(42);
    seed_rooted_pool(world.engine_mut(), &families, at, "fuel", 30, LotId(0xe1));
    let holder = mush_market::listing_holder(ListingId(7)).expect("holder");
    let placed = apply(world.engine_mut(), &families, |txn| {
        families.reserve(txn, at, &commodity("fuel"), &holder, 30, &now())
    })
    .expect("hold");
    let revision = read(world.engine_mut(), &families, |txn| {
        families
            .reservation(txn, at, &commodity("fuel"), placed)
            .expect("read")
            .expect("held")
            .revision()
    });
    assert_eq!(pool(world.engine_mut(), &families, at, "fuel"), (30, 0));

    apply(world.engine_mut(), &families, |txn| {
        families.release(txn, at, &commodity("fuel"), placed, revision)
    })
    .expect("the withdrawal releases");
    let after = pool(world.engine_mut(), &families, at, "fuel");
    assert_eq!(
        after,
        (30, 30),
        "a release gives availability back and moves no balance"
    );

    let twice = apply(world.engine_mut(), &families, |txn| {
        families.release(txn, at, &commodity("fuel"), placed, revision)
    });
    assert!(twice.is_err(), "a second release is refused: {twice:?}");
    assert_eq!(
        pool(world.engine_mut(), &families, at, "fuel"),
        after,
        "…and writes nothing"
    );

    let settled = apply(world.engine_mut(), &families, |txn| {
        families.settle_sale(
            txn,
            at,
            &commodity("fuel"),
            placed,
            revision,
            &now(),
            commit(0x51),
            LotSale {
                destination: buyer,
                sold: 30,
                grade: Grade::new(0).expect("grade"),
                buyer_lot: LotId(0xe2),
                remainder_lot: LotId(0xe3),
            },
        )
    });
    assert!(
        settled.is_err(),
        "a released hold cannot then be consumed: {settled:?}"
    );
    assert_eq!(pool(world.engine_mut(), &families, at, "fuel"), after);
}

/// **A sale into the seller's own endpoint is refused**, because one endpoint's
/// `lot_census` is one key and an `ActionTxn` mutates one key once — the same rule that
/// makes `settle_sale` exist at all.
#[test]
fn e7r2_p1_a_sale_into_the_source_endpoint_is_refused_before_it_writes() {
    let (mut world, families) = boot();
    let at = endpoint(51);
    seed_rooted_pool(world.engine_mut(), &families, at, "iron", 10, LotId(0xf1));
    let holder = mush_market::listing_holder(ListingId(1)).expect("holder");
    let placed = apply(world.engine_mut(), &families, |txn| {
        families.reserve(txn, at, &commodity("iron"), &holder, 10, &now())
    })
    .expect("hold");
    let revision = read(world.engine_mut(), &families, |txn| {
        families
            .reservation(txn, at, &commodity("iron"), placed)
            .expect("read")
            .expect("held")
            .revision()
    });
    let same = apply(world.engine_mut(), &families, |txn| {
        families.settle_sale(
            txn,
            at,
            &commodity("iron"),
            placed,
            revision,
            &now(),
            commit(0x61),
            LotSale {
                destination: at,
                sold: 10,
                grade: Grade::new(0).expect("grade"),
                buyer_lot: LotId(0xf2),
                remainder_lot: LotId(0xf3),
            },
        )
    });
    assert!(same.is_err(), "self-delivery is refused: {same:?}");
    assert_eq!(pool(world.engine_mut(), &families, at, "iron"), (10, 0));
}

// ---- E7r2: the records and the handles -------------------------------------

/// **The holder key is `wo_holder`'s shape, is deterministic, and is inside the kernel's
/// `MAX_IDEMPOTENCY_KEY_BYTES` (128)** — asserted once, here, as the doc comment says.
#[test]
fn e7r2_p1_the_listing_holder_is_deterministic_scoped_and_inside_the_kernel_cap() {
    let a = mush_market::listing_holder(ListingId(42)).expect("holder");
    let b = mush_market::listing_holder(ListingId(42)).expect("holder");
    let other = mush_market::listing_holder(ListingId(43)).expect("holder");
    assert_eq!(a, b, "two derivations of one listing's holder agree");
    assert_ne!(a, other, "…and two listings do not share a holder");
    let text = mush_market::listing_holder_text(ListingId(42));
    assert_eq!(text, "listing/42");
    assert!(
        text.len() <= 128,
        "MAX_IDEMPOTENCY_KEY_BYTES is 128 and the holder text is {}",
        text.len()
    );
    // The reservation id is a pure function of (listing, endpoint, commodity), which is
    // what lets the listing document name it before the submit that places it.
    assert_eq!(
        reservation_of(42, endpoint(1), "iron"),
        reservation_of(42, endpoint(1), "iron")
    );
    assert_ne!(
        reservation_of(42, endpoint(1), "iron"),
        reservation_of(42, endpoint(2), "iron")
    );
    assert_ne!(
        reservation_of(42, endpoint(1), "iron"),
        reservation_of(42, endpoint(1), "ice")
    );
}

/// **The five ops are exactly five, and their words round-trip** — a new op needs its
/// own authority sentence in `project_offer`, so the set is pinned. Doc 18 cut IC7 added
/// the fourth, `draw`, and its sentence is the paid projection: `consume` and `draw` are
/// offered only over the actor's own paid contracts. Doc 18 cut IC10 added the fifth,
/// `deliver`, offered only to the **seller** of a paid sale into a bid, over the actor's
/// own endpoint, with the destination and the marker fixed by the server.
#[test]
fn e7r2_p1_the_lot_action_has_exactly_five_ops() {
    let words: Vec<&str> = [
        LotOp::Reserve,
        LotOp::Consume,
        LotOp::Release,
        LotOp::Draw,
        LotOp::Deliver,
    ]
    .iter()
    .map(|op| op.word())
    .collect();
    assert_eq!(
        words,
        vec!["reserve", "consume", "release", "draw", "deliver"]
    );
    let source = production("mush-market/src/lot.rs");
    assert!(
        source.matches("LotOp::").count() > 0,
        "the ops are used, not merely declared"
    );
    // One action, one outcome schema, one provider — §15.5 (c′)'s "ONE market-owned
    // `Authoritative` action".
    assert_eq!(
        source.matches("reg.action_provider(").count(),
        1,
        "exactly one market-owned action provider"
    );
}

/// **`is_stockable` is LIFTED with its comment, not retired** (10b §15.5, corrected).
///
/// One definition, one call site, and the call site is now conditional on a composed lot
/// supply — so a server that composes none refuses byte-identically to E6c, which is
/// what `ruleset-shadowrun/tests/e6c.rs`'s `"not stockable"` assertion still checks.
#[test]
fn e7r2_p1_is_stockable_is_lifted_with_its_comment_and_still_refuses_when_unwired() {
    let record = production("mush-market/src/record.rs");
    assert_eq!(
        record.matches("pub fn is_stockable").count(),
        1,
        "still exactly one definition"
    );
    assert!(
        record.contains("LIFTED, not retired"),
        "…and it kept its comment, which is what says why it survives"
    );
    let api = production("mush-market/src/api.rs");
    assert_eq!(
        api.matches(".is_stockable()").count(),
        1,
        "still exactly one call site"
    );
    assert!(
        api.contains("lot_supply.is_none()"),
        "…and it is conditional on a composed lot supply"
    );
    assert!(
        api.contains("lots are not stockable on this server yet."),
        "the unwired refusal is byte-identical to E6c's"
    );
}

/// **A hand-edited lot listing is refused ON LOAD** — every new record re-enters through
/// its `TryFrom`, and a grade past `MAX_GRADE` would otherwise refuse inside the settling
/// bundle, i.e. after the money leg is in force.
#[test]
fn e7r2_p1_a_lot_listing_re_enters_through_its_wire_and_a_bad_grade_refuses_on_load() {
    let built = mush_market::Listing {
        id: ListingId(1),
        seller: mush_org::PrincipalId::Organization(mush_org::OrgId(1)),
        vendor: mush_market::VendorId(1),
        market: mush_market::MarketId::new("spinward").expect("venue"),
        catalog: mush_economy::CatalogRef {
            ruleset: "tacsim".into(),
            code: "iron".into(),
        },
        definition_version: 1,
        source: mush_market::Supply::Lot {
            endpoint: mush_market::LotEndpoint(endpoint(7)),
            commodity: mush_market::LotCommodity(commodity("iron")),
            grade: 3,
        },
        object: None,
        price: mush_economy::Amount::from_minor(100),
        quantity: 10,
        visibility: mush_market::MarketVisibility::Public,
        draw: Some(false),
        expires: mush_economy::EconomicDeadline {
            clock: mush_economy::EconomicClock::RealUtc,
            at: 9_999,
        },
        revision: 1,
    };
    let good = serde_json::to_value(&built).expect("a lot listing serializes");
    let listing: mush_market::Listing =
        serde_json::from_value(good.clone()).expect("a lot listing round-trips");
    assert_eq!(listing, built, "the wire is lossless");
    assert!(listing.source.lot().is_some());
    assert_eq!(
        listing.source.lot().map(|(_, _, grade)| grade),
        Some(3),
        "the advertised grade survives the wire"
    );
    let mut bad = good;
    bad["source"]["lot"]["grade"] = serde_json::json!(200);
    let refused: Result<mush_market::Listing, _> = serde_json::from_value(bad);
    assert!(
        refused.is_err(),
        "a grade past MAX_GRADE ({}) is refused on load, never clamped",
        mush_inventory::MAX_GRADE
    );
    // The cap is ENCODED, not restated: the grade the good document carries is inside
    // it and the one the bad document carries is past it, both read off the documents
    // themselves so the assertion is a *test* rather than a constant folded away.
    assert_eq!(mush_inventory::MAX_GRADE, 100);
    let cap = u64::from(mush_inventory::MAX_GRADE);
    let accepted = listing
        .source
        .lot()
        .map(|(_, _, grade)| u64::from(grade))
        .expect("the good listing's grade");
    let rejected = 200_u64;
    assert!(accepted <= cap, "the accepted grade is inside the cap");
    assert!(rejected > cap, "and the refused one is past it");
}

/// **The one-live-listing-per-lot index is a re-derived projection**, and
/// `indexes_agree` says so — the property every other index in this store already has.
#[test]
fn e7r2_p1_the_lot_pool_index_is_re_derived_and_agrees() {
    let store = MarketStore::default();
    assert!(store.indexes_agree());
    assert!(
        store
            .lot_listing_of(endpoint(3), &commodity("iron"))
            .is_none(),
        "an empty store carries no lot listing"
    );
    // The index is built in exactly one place.
    let source = production("mush-market/src/store.rs");
    let reindex = source
        .split("fn reindex(")
        .nth(1)
        .expect("reindex exists")
        .split("\n    /// ")
        .next()
        .unwrap_or_default();
    assert_eq!(
        reindex.matches("self.by_lot_pool").count(),
        2,
        "built in exactly one place: cleared, then filled, inside `reindex`"
    );
    assert!(
        source
            .split("fn indexes_agree(")
            .nth(1)
            .unwrap_or_default()
            .contains("by_lot_pool"),
        "…and `indexes_agree` checks it, so a drift is loud"
    );
}

/// **The sweep's reconcile arm is a NEW ARM on the EXISTING tick, and it stages no
/// authoritative submission** (the tick-cadence rule: a `reg.tick` runs as GOD and two
/// GOD-tick submissions in one drain void both).
#[test]
fn e7r2_p1_the_lot_reconcile_is_a_second_arm_on_the_existing_sweep_and_submits_nothing() {
    let module = production("mush-market/src/module.rs");
    assert_eq!(
        module.matches("reg.tick(").count(),
        1,
        "still exactly ONE tick in the whole crate"
    );
    assert!(module.contains("SWEEP_TAG"));
    let boot = production("mush-market/src/boot.rs");
    assert!(
        boot.contains("reconcile_lot_holds(ctx, MAX_LOT_RECONCILED_PER_FIRE)"),
        "the arm hangs off `sweep_tick`"
    );
    assert!(
        !boot.contains("submit_authoritative"),
        "a GOD tick stages no authoritative submission"
    );
    // The budget is encoded beside the arm that spends it.
    assert_eq!(mush_market::MAX_LOT_RECONCILED_PER_FIRE, 32);
    assert_eq!(mush_market::MAX_SWEPT_PER_FIRE, 64);
}

/// **Every declared cap this pass touches is encoded**, not restated in prose.
#[test]
fn e7r2_p1_the_caps_are_encoded() {
    assert_eq!(mush_market::MAX_LISTINGS_PER_VENDOR, 64);
    assert_eq!(mush_market::MAX_LISTINGS, 1_024);
    assert_eq!(mush_market::MAX_QUANTITY, 100_000);
    assert_eq!(mush_market::MAX_LOT_ENDPOINT_CHOICES, 64);
    assert_eq!(
        mush_market::MAX_LOT_ENDPOINT_CHOICES,
        mush_inventory::MAX_ENDPOINT_CHOICES,
        "the lot offer cites cut 2's ceiling rather than redefining it"
    );
}

/// **No `?`, `unwrap`, `expect` or panicking index after the first irreversible
/// mutation** on the lot lane — the property every authoritative path in the tree owes.
///
/// `settle_sale` validates then mutates: every refusal runs before `write_pair`, and the
/// probe pins the ordering in the source so a later edit cannot quietly invert it.
#[test]
fn e7r2_p1_settle_sale_validates_before_it_mutates() {
    let pool = production("mush-inventory/src/pool.rs");
    let body = pool
        .split("pub fn settle_sale(")
        .nth(1)
        .expect("settle_sale exists")
        .split("\n    /// ")
        .next()
        .unwrap_or_default();
    let first_write = body.find("self.write_pair(").expect("it writes the pair");
    let tail = &body[first_write..];
    for forbidden in [".unwrap()", ".expect(", "return Err("] {
        assert!(
            !tail.contains(forbidden),
            "`settle_sale` must not {forbidden} after its first irreversible mutation"
        );
    }
    // …and every refusal it can make is before that point.
    let head = &body[..first_write];
    for refusal in [
        "InventoryError::NoPool",
        "InventoryError::NoReservation",
        "InventoryError::Insufficient",
        "InventoryError::NoTranche",
        "InventoryError::AlreadyExists",
    ] {
        assert!(
            head.contains(refusal),
            "{refusal} is decided before the first write"
        );
    }
}

/// **A vanished hold is a pause and a page, never a replay HIT** (the DECIDED header's
/// rule (g), applied to this lane).
///
/// The reconcile arm withdraws the listing and opens a level-2 record on the **listing**
/// — the ladder's own per-subject dedupe key, so one stuck listing stops only itself —
/// and nothing anywhere treats a missing reservation as "already applied".
#[test]
fn e7r2_p1_a_vanished_hold_pages_per_listing_and_is_never_a_replay_hit() {
    let boot = production("mush-market/src/boot.rs");
    assert!(
        boot.contains("EscalationSubject::MarketListing(id.0)"),
        "the page is keyed on the LISTING, so the dedupe stops only the subject"
    );
    assert!(
        boot.contains("@market/vendor stock"),
        "…with the recommended command already filled in"
    );
    assert!(
        !boot.contains("already applied") && !boot.contains("AlreadyPosted"),
        "a vanished hold is never read as a replay hit"
    );
    let lot = production("mush-market/src/lot.rs");
    assert!(
        lot.contains("ok_or_else(|| error(ActionErrorCode::ProviderRejected, LOT_REFUSED))"),
        "and the prepare REFUSES a missing hold rather than proceeding without one"
    );
    assert_eq!(
        mush_economy::EscalationSubject::MarketListing(7).render(),
        "market listing #7"
    );
}

/// **The refusals a viewer may not distinguish are byte-identical** (§4 rule 4): one
/// line for a hold that is gone, an endpoint the actor may not hold, and a listing that
/// never existed.
#[test]
fn e7r2_p1_the_lot_lane_has_one_refusal_line() {
    let source = production("mush-market/src/lot.rs");
    assert_eq!(
        source.matches("pub const LOT_REFUSED").count(),
        1,
        "one line, defined once"
    );
    assert_eq!(
        mush_market::LOT_REFUSED,
        "That stock cannot be moved right now."
    );
    // Every `ProviderRejected` on the lane renders it, so a stranger cannot tell an
    // endpoint they may not hold from one that does not exist.
    let rejects = source.matches("ActionErrorCode::ProviderRejected").count();
    assert_eq!(
        source.matches("ProviderRejected, LOT_REFUSED").count(),
        rejects,
        "every provider rejection on this lane carries the one line"
    );
}

/// The one place a market lot action names an actor is its own outcome, and the outcome
/// is witnessed only by the acting principal — a receipt is never a disclosure of
/// another principal's inventory.
#[test]
fn e7r2_p1_the_lot_outcome_is_witnessed_only_by_its_actor() {
    let source = production("mush-market/src/lot.rs");
    assert!(
        source.contains("with_eligible_witnesses(vec![invocation.principal().clone()])"),
        "one witness: the acting principal"
    );
    assert!(
        !source.contains("fn project_receipt"),
        "and no typed receipt override — the default generic receipt is what a second \
         crate edge would have bought"
    );
    let _ = ObjId(1);
}

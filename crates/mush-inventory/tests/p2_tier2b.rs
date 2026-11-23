//! Defensive regression tests for Pass 2 Tier 2B `mush-inventory` findings:
//! - `P2-MOD-50`: Cross-endpoint lot existence & mass/commodity oracles in `inv_dissolve` and `@inv/merge`.
//! - `P2-MOD-51`: Pre-authorization vault-type & status oracle in `@inv/move`.
//! - `P2-MOD-52`: Global `InventoryStore::complete = false` DoS via `MAX_APPLIED_MARKERS` / `MAX_BOOT_PAGES`.
//! - `P2-MOD-53`: Tranche generation rollover stranding prior `remaining` lineage credit.
//! - `P2-MOD-79`: `SalvageProvider` (`@salvage/recover`) bypassing `PoolCapacity`.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use mush_core::{
    ActionError, ActionProviderPrepareError, Audience, CanonicalId, ClockRef, ClockStamp,
    CommitToken, Engine, GOD, IdempotencyKey, IdempotencyScope, ObjId, ObjKind,
    OfferExecutionContext, OfferSubmission, PerceivedRef, PrincipalId, RequestUid,
    ScopedIdempotencyKey, World,
};
use mush_economy::{EconomyModule, EconomyStore, ExternalRef, FactProvider, VaultId};
use mush_inventory::commands::{NO_SUCH_ENDPOINT, NO_SUCH_LOT};
use mush_inventory::lot_hold::IndivisibleGoods;
use mush_inventory::provider::{MovePolicy, PoolCapacity, PoolReadFailed, action_ref};
use mush_inventory::record::{
    Grade, Lineage, Lot, LotId, MAX_APPLIED_MARKERS, MAX_LINEAGE_DEPTH, MAX_POOLS_PER_ENDPOINT,
    MAX_TRANCHE_ABSORBED, MAX_TRANCHE_INHERITED,
};
use mush_inventory::salvage::{NO_SUCH_WRECK, SalvageSource, WreckReport, site_token};
use mush_inventory::{
    EndpointRef, InventoryError, InventoryFactProvider, InventoryFamilies, InventoryHandleCell,
    InventoryHandles, InventoryModule, InventoryStore, InventoryVerb, LotSale, derived_marker_id,
};
use mush_testkit::TestWorld;
use serde_json::{Value, json};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

const T0: i64 = 1_700_000_000;

fn cid(s: &str) -> CanonicalId {
    CanonicalId::new(s).expect("valid canonical id")
}

fn now_ic(at: i64) -> ClockStamp {
    ClockStamp {
        clock: ClockRef::WorldIc { generation: 1 },
        at,
    }
}

fn rendered(events: &[(Audience, mush_proto::OutputEvent)]) -> Vec<String> {
    events
        .iter()
        .map(|(_, event)| mush_testkit::text_of(event))
        .collect()
}

fn apply<T>(
    engine: &mut Engine,
    families: &InventoryFamilies,
    body: impl FnOnce(&mut mush_core::ActionTxn<'_>) -> Result<T, InventoryError>,
) -> Result<T, InventoryError> {
    let (result, prepared) = {
        let mut txn = engine
            .action_transaction(&families.pools)
            .expect("inventory txn");
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
            .expect("install prepared");
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
        .expect("inventory txn");
    body(&mut txn)
}

fn seed_origin_lot(
    engine: &mut Engine,
    families: &InventoryFamilies,
    lot: LotId,
    endpoint: EndpointRef,
    commodity: &str,
    qty: u64,
    grade: u8,
) {
    apply(engine, families, |txn| {
        families
            .mint_origin(
                txn,
                endpoint,
                &cid(commodity),
                qty,
                Grade::new(grade).expect("grade"),
                ExternalRef {
                    provider: "test".into(),
                    kind: "seed".into(),
                    id: format!("{}", lot.0).into(),
                    revision: None,
                },
                lot,
            )
            .map(|_| ())
    })
    .expect("mint origin lot");
}

fn set_offer_keys(w: &mut TestWorld) {
    w.engine_mut().set_offer_keys(
        mush_core::OfferKeyRing::new(
            mush_core::OfferSecret::new(1, [0x55; 32]).expect("secret"),
            [],
        )
        .expect("keyring"),
    );
}

fn make_vault(w: &mut TestWorld, owner: ObjId, name: &str) -> TestResult<EndpointRef> {
    let obj = w.world_mut().create(ObjKind::Thing, name, owner, None)?;
    w.run(
        GOD,
        &format!(
            "@vault/create {name}=container:#{} owner=#{}",
            obj.0, owner.0
        ),
    );
    let id = w
        .world()
        .module_state::<EconomyStore>()
        .ok_or("economy")?
        .snapshot()
        .vaults()
        .find(|v| v.name.as_str() == name)
        .ok_or("vault")?
        .id;
    Ok(EndpointRef(id))
}

fn submit_direct(
    w: &mut TestWorld,
    actor: ObjId,
    action: mush_core::DefinitionRef,
    parameters: Value,
    salt: u8,
) -> Result<(), ActionError> {
    let principal = PrincipalId::Object { id: actor };
    let query = w
        .engine()
        .legal_action_query(principal.clone(), actor, PerceivedRef::SelfActor)
        .expect("query");
    let offers = w.engine().project_legal_actions(&query).expect("offers");
    let offer = offers
        .projected_for_action(&action)
        .expect("offer must be projected")
        .clone();
    let req = RequestUid::from_bytes([salt; 16]);
    let key = ScopedIdempotencyKey::new(
        IdempotencyScope {
            principal: principal.clone(),
            owner: mush_inventory::record::owner(),
            namespace: cid("p2_tier2b_test"),
        },
        1,
        IdempotencyKey::Request(req),
    );
    let resolved = w
        .engine()
        .resolve_legal_action(
            &offers,
            OfferExecutionContext {
                request: req,
                principal,
                actor,
                idempotency: key,
                viewer_scope: query.viewer_scope(),
                now: None,
            },
            OfferSubmission {
                offer: offer.offer(),
                subject: PerceivedRef::SelfActor,
                targets: Vec::new(),
                parameters,
            },
        )
        .expect("resolve");
    let (act, inv, _) = resolved.into_authoritative_parts();
    let (prepared, _) = w
        .engine_mut()
        .prepare_registered_authoritative_action(
            &act,
            inv.invocation(),
            CommitToken::from_bytes([salt; 16]),
        )
        .map_err(|e| match e {
            ActionProviderPrepareError::Provider { source, .. } => *source,
            other => panic!("unexpected prepare error: {other:?}"),
        })?;
    let (_, install) = prepared.into_parts();
    w.engine_mut()
        .install_prepared_action(install)
        .expect("install");
    Ok(())
}

/// Capacity policy that enforces a per-endpoint unit ceiling and optionally refuses a
/// forbidden commodity (so any pre-auth call on a foreign lot would leak commodity/mass!).
#[derive(Debug)]
struct CapPolicy {
    governed: Vec<(EndpointRef, u64)>,
    forbidden: Option<String>,
    checks: Arc<AtomicUsize>,
}

impl PoolCapacity for CapPolicy {
    fn plan(&self, _world: &World, endpoint: EndpointRef) -> Option<Value> {
        self.governed
            .iter()
            .find(|(ep, _)| *ep == endpoint)
            .map(|(_, limit)| json!({ "limit": limit }))
    }

    fn check(
        &self,
        plan: &Value,
        commodity: &CanonicalId,
        quantity: u64,
        read_pool: &mut dyn FnMut(&CanonicalId) -> Result<u64, PoolReadFailed>,
    ) -> Result<(), String> {
        self.checks.fetch_add(1, Ordering::SeqCst);
        if self.forbidden.as_deref() == Some(commodity.as_str()) {
            return Err(format!("Cannot stow forbidden commodity {commodity}."));
        }
        let limit = plan
            .get("limit")
            .and_then(Value::as_u64)
            .ok_or_else(|| "bad plan".to_owned())?;
        let held = read_pool(commodity).map_err(|_| "pool read failed".to_owned())?;
        if held.saturating_add(quantity) > limit {
            return Err(format!(
                "Hold full: {held} + {quantity} exceeds {limit} {commodity}."
            ));
        }
        Ok(())
    }
}

#[derive(Debug)]
struct PartsOnly;

impl IndivisibleGoods for PartsOnly {
    fn indivisible(&self, commodity: &CanonicalId) -> bool {
        commodity.as_str() == "part"
    }
}

/// `P2-MOD-50`: `inv_dissolve` (`room_for`) and `@inv/merge` (`pool::merge` + `commands::merge`)
/// must authorize target lots against the caller's endpoint before disclosing existence,
/// commodity, indivisibility, or mass/capacity errors.
#[test]
fn p2_mod_50_inv_dissolve_and_merge_hide_cross_endpoint_lots() -> TestResult {
    let cell: InventoryHandleCell = Arc::default();
    let checks = Arc::new(AtomicUsize::new(0));
    let cap = Arc::new(CapPolicy {
        governed: vec![(EndpointRef(VaultId(0)), 10)],
        forbidden: Some("uranium".to_owned()),
        checks: Arc::clone(&checks),
    });
    let inventory = InventoryModule::new()
        .with_handle_cell(Arc::clone(&cell))
        .with_pool_capacity(cap)
        .with_indivisible_goods(Arc::new(PartsOnly));
    let mut w = TestWorld::with_modules(vec![Box::new(EconomyModule::new()), Box::new(inventory)]);
    set_offer_keys(&mut w);
    w.world_mut().set_now(T0);

    let alice = w.spawn_player("Alice");
    let bob = w.spawn_player("Bob");
    let alice_a = make_vault(&mut w, alice, "AliceA")?;
    let alice_b = make_vault(&mut w, alice, "AliceB")?;
    let bob_v = make_vault(&mut w, bob, "BobV")?;
    assert_eq!(alice_a.vault().0, 0);

    let families = cell.get().ok_or("families")?.clone();
    let own_lot = LotId(10);
    let own_part = LotId(11);
    let own_other = LotId(12);
    let victim_lot = LotId(999);
    let nonexistent = LotId(888);
    seed_origin_lot(w.engine_mut(), &families, own_lot, alice_a, "metals", 4, 1);
    seed_origin_lot(w.engine_mut(), &families, own_part, alice_a, "part", 1, 1);
    seed_origin_lot(
        w.engine_mut(),
        &families,
        own_other,
        alice_b,
        "metals",
        4,
        1,
    );
    seed_origin_lot(
        w.engine_mut(),
        &families,
        victim_lot,
        bob_v,
        "uranium",
        100,
        5,
    );

    // 1. Direct `inv_dissolve` on victim's lot vs nonexistent lot: byte-identical refusal,
    //    and `PoolCapacity::check` is NEVER invoked for the cross-endpoint lot.
    let dissolve_action = action_ref(&w.engine().registry, InventoryVerb::Dissolve).ok_or("act")?;
    let err_victim = submit_direct(
        &mut w,
        alice,
        dissolve_action.clone(),
        json!({
            "endpoint": alice_a.vault().0.to_string(),
            "lot": victim_lot.to_string(),
        }),
        1,
    )
    .expect_err("cross-endpoint dissolve must refuse");
    let err_absent = submit_direct(
        &mut w,
        alice,
        dissolve_action,
        json!({
            "endpoint": alice_a.vault().0.to_string(),
            "lot": nonexistent.to_string(),
        }),
        2,
    )
    .expect_err("absent dissolve must refuse");
    assert_eq!(err_victim.code(), err_absent.code());
    assert_eq!(err_victim.message(), err_absent.message());
    assert_eq!(
        checks.load(Ordering::SeqCst),
        0,
        "PoolCapacity::check must not run on a cross-endpoint lot"
    );

    // 2. Direct `inv_merge` / `InventoryFamilies::merge` with own_lot < victim_lot vs nonexistent:
    //    both must return `NoLot` ("no such lot"), never `MergeMismatch`.
    let merge_action = action_ref(&w.engine().registry, InventoryVerb::Merge).ok_or("act")?;
    let merge_victim = submit_direct(
        &mut w,
        alice,
        merge_action.clone(),
        json!({
            "endpoint": alice_a.vault().0.to_string(),
            "lots": format!("{own_lot},{victim_lot}"),
        }),
        3,
    )
    .expect_err("cross-endpoint merge must refuse");
    let merge_absent = submit_direct(
        &mut w,
        alice,
        merge_action,
        json!({
            "endpoint": alice_a.vault().0.to_string(),
            "lots": format!("{own_lot},{nonexistent}"),
        }),
        4,
    )
    .expect_err("absent merge must refuse");
    assert_eq!(merge_victim.code(), merge_absent.code());
    assert_eq!(merge_victim.message(), merge_absent.message());
    assert_eq!(merge_victim.message(), InventoryError::NoLot.to_string());

    // 3. Command-layer `@inv/merge`: own + victim vs own + absent (even when own is indivisible!)
    //    must both return byte-identical `NO_SUCH_LOT` and queue nothing.
    let cmd_victim = rendered(&w.run(alice, &format!("@inv/merge {own_lot},{victim_lot}")));
    assert!(!w.engine().has_authoritative_submit());
    let cmd_absent = rendered(&w.run(alice, &format!("@inv/merge {own_lot},{nonexistent}")));
    assert!(!w.engine().has_authoritative_submit());
    assert_eq!(cmd_victim, cmd_absent);
    assert!(
        cmd_victim.iter().any(|l| l.contains(NO_SUCH_LOT)),
        "{cmd_victim:?}"
    );

    let cmd_part_victim = rendered(&w.run(alice, &format!("@inv/merge {own_part},{victim_lot}")));
    let cmd_part_absent = rendered(&w.run(alice, &format!("@inv/merge {own_part},{nonexistent}")));
    assert_eq!(
        cmd_part_victim, cmd_part_absent,
        "indivisibility of first lot must not leak existence of second lot"
    );
    assert!(
        cmd_part_victim.iter().any(|l| l.contains(NO_SUCH_LOT)),
        "{cmd_part_victim:?}"
    );

    // 4. Merging two lots owned by Alice at two of Alice's own vaults returns MergeMismatch.
    let cmd_own_diff = rendered(&w.run(alice, &format!("@inv/merge {own_lot},{own_other}")));
    assert!(!w.engine().has_authoritative_submit());
    assert!(
        cmd_own_diff
            .iter()
            .any(|l| l.contains(&InventoryError::MergeMismatch.to_string())),
        "{cmd_own_diff:?}"
    );
    Ok(())
}

#[derive(Debug)]
struct SpyMovePolicy {
    calls: Arc<AtomicUsize>,
    escrow: EndpointRef,
    scrap: EndpointRef,
    denied_pair: (EndpointRef, EndpointRef),
}

impl MovePolicy for SpyMovePolicy {
    fn admit(
        &self,
        _w: &World,
        _actor: ObjId,
        from: EndpointRef,
        to: EndpointRef,
    ) -> Result<(), String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if from == self.escrow || to == self.escrow {
            return Err("That vault is a works escrow.".to_owned());
        }
        if to == self.scrap {
            return Err("That vault is a scrap vault.".to_owned());
        }
        if (from, to) == self.denied_pair {
            return Err("Move blocked by policy.".to_owned());
        }
        Ok(())
    }
}

/// `P2-MOD-51`: `@inv/move` must authorize both endpoints (and require live status) BEFORE
/// consulting `MovePolicy`, so an attacker cannot probe vault types or status on endpoints they
/// cannot access.
#[test]
fn p2_mod_51_inv_move_authorizes_endpoints_before_move_policy() -> TestResult {
    let calls = Arc::new(AtomicUsize::new(0));
    let policy = Arc::new(SpyMovePolicy {
        calls: Arc::clone(&calls),
        escrow: EndpointRef(VaultId(2)),
        scrap: EndpointRef(VaultId(3)),
        denied_pair: (EndpointRef(VaultId(0)), EndpointRef(VaultId(1))),
    });
    let mut w = TestWorld::with_modules(vec![
        Box::new(EconomyModule::new()),
        Box::new(InventoryModule::new().with_move_policy(policy)),
    ]);
    w.world_mut().set_now(T0);

    let alice = w.spawn_player("Alice");
    let bob = w.spawn_player("Bob");
    let alice_a = make_vault(&mut w, alice, "AliceA")?;
    let alice_b = make_vault(&mut w, alice, "AliceB")?;
    let bob_escrow = make_vault(&mut w, bob, "BobEscrow")?;
    let bob_scrap = make_vault(&mut w, bob, "BobScrap")?;
    let alice_frozen = make_vault(&mut w, alice, "AliceFrozen")?;
    assert_eq!(bob_escrow.vault().0, 2);
    assert_eq!(bob_scrap.vault().0, 3);
    w.run(GOD, &format!("@vault/freeze {}", alice_frozen.vault().0));

    // Probing Bob's escrow, Bob's scrap vault, an absent vault, or reversing from/to:
    // every refusal is byte-identical `NO_SUCH_ENDPOINT`, and `MovePolicy::admit` is NEVER called.
    let absent_out = rendered(&w.run(
        alice,
        &format!("@inv/move {}/metals:1=999", alice_a.vault().0),
    ));
    let escrow_out = rendered(&w.run(
        alice,
        &format!(
            "@inv/move {}/metals:1={}",
            alice_a.vault().0,
            bob_escrow.vault().0
        ),
    ));
    let scrap_out = rendered(&w.run(
        alice,
        &format!(
            "@inv/move {}/metals:1={}",
            alice_a.vault().0,
            bob_scrap.vault().0
        ),
    ));
    let reverse_out = rendered(&w.run(
        alice,
        &format!(
            "@inv/move {}/metals:1={}",
            bob_escrow.vault().0,
            alice_a.vault().0
        ),
    ));
    let frozen_out = rendered(&w.run(
        alice,
        &format!(
            "@inv/move {}/metals:1={}",
            alice_a.vault().0,
            alice_frozen.vault().0
        ),
    ));
    assert!(!w.engine().has_authoritative_submit());
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "MovePolicy::admit must never run for unauthorized or inactive endpoints"
    );
    for out in [
        &absent_out,
        &escrow_out,
        &scrap_out,
        &reverse_out,
        &frozen_out,
    ] {
        assert_eq!(out, &absent_out);
        assert!(out.iter().any(|l| l.contains(NO_SUCH_ENDPOINT)), "{out:?}");
    }

    // Moving between two of Alice's OWN live endpoints DOES consult MovePolicy.
    let denied = rendered(&w.run(
        alice,
        &format!(
            "@inv/move {}/metals:1={}",
            alice_a.vault().0,
            alice_b.vault().0
        ),
    ));
    assert!(!w.engine().has_authoritative_submit());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(
        denied.iter().any(|l| l.contains("Move blocked by policy.")),
        "{denied:?}"
    );

    // Admitted owned pair submits cleanly.
    let _ = w.run(
        alice,
        &format!(
            "@inv/move {}/metals:1={}",
            alice_b.vault().0,
            alice_a.vault().0
        ),
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(w.engine().has_authoritative_submit());
    let _ = w.engine_mut().take_authoritative_submit();

    // Staff bypasses MovePolicy as before.
    let _ = w.run(
        GOD,
        &format!(
            "@inv/move {}/metals:1={}",
            alice_a.vault().0,
            bob_escrow.vault().0
        ),
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(w.engine().has_authoritative_submit());
    Ok(())
}

/// `P2-MOD-52`: Boot/sweep reconciliation must not flip `InventoryStore::complete = false`
/// when pools exceed `MAX_BOOT_PAGES * INV_PAGE` (4,096) or when normal multi-endpoint
/// applied-marker churn exceeds `MAX_APPLIED_MARKERS` (4,096).
#[test]
fn p2_mod_52_boot_and_marker_churn_never_flip_complete_false() -> TestResult {
    let mut w = TestWorld::with_modules(vec![Box::new(InventoryModule::new())]);
    let families = w
        .world()
        .module_state::<InventoryHandles>()
        .ok_or("handles")?
        .0
        .clone();

    // 1. Seed 17 endpoints × 256 pools = 4,352 pools (> old 4,096 boot-scan cliff).
    let commodities: Vec<CanonicalId> = (0..MAX_POOLS_PER_ENDPOINT)
        .map(|i| cid(&format!("g{i}")))
        .collect();
    for good in &commodities {
        apply(w.engine_mut(), &families, |txn| {
            for ep_idx in 0..17_u64 {
                families.credit(txn, EndpointRef(VaultId(ep_idx + 1)), good, 1)?;
            }
            Ok(())
        })?;
    }
    w.modules_loaded(std::collections::HashMap::new());
    let store = w.world().module_state::<InventoryStore>().ok_or("store")?;
    assert!(
        store.is_complete(),
        "4,352 pools (17 endpoints × 256) must reconcile completely"
    );

    // 2. Drive 33 endpoints × 128 young marked transfers = 4,224 applied markers (> 4,096).
    let holds: Vec<EndpointRef> = (0..33_u64).map(|i| EndpointRef(VaultId(100 + i))).collect();
    let metals = cid("metals");
    apply(w.engine_mut(), &families, |txn| {
        for &ep in &holds {
            families.credit(txn, ep, &metals, 200)?;
        }
        Ok(())
    })?;
    let mut oldest_id = None;
    let mut newest_id = None;
    for step in 0..128_u64 {
        let stamp = now_ic(10_000 + i64::try_from(step)?);
        let wall = T0 + i64::try_from(step)?;
        apply(w.engine_mut(), &families, |txn| {
            for &ep in &holds {
                let to = EndpointRef(VaultId(10_000 + ep.vault().0));
                let id = derived_marker_id(stamp.at, &json!({ "ep": ep.vault().0, "step": step }))?;
                if oldest_id.is_none() {
                    oldest_id = Some(id.clone());
                }
                newest_id = Some(id.clone());
                families.transfer_marked(txn, ep, to, &metals, 1, &id, GOD, &stamp, wall)?;
            }
            Ok(())
        })?;
    }
    w.modules_loaded(std::collections::HashMap::new());
    let store = w.world().module_state::<InventoryStore>().ok_or("store")?;
    assert!(
        store.is_complete(),
        "4,224 applied markers across 33 endpoints must keep InventoryStore::complete = true"
    );
    assert!(store.markers_truncated());
    assert_eq!(store.markers(), MAX_APPLIED_MARKERS);
    let newest = newest_id.ok_or("newest")?;
    let oldest = oldest_id.ok_or("oldest")?;
    let resolved = InventoryFactProvider
        .resolve(w.world(), mush_inventory::FACT_TRANSFERRED, &newest, None)
        .ok_or("newest marker must resolve")?;
    assert!(resolved.satisfied);
    assert!(
        InventoryFactProvider
            .resolve(w.world(), mush_inventory::FACT_TRANSFERRED, &oldest, None)
            .is_none(),
        "evicted older marker must fail closed as None"
    );
    Ok(())
}

/// `P2-MOD-53`: Tranche generation rollovers in `dissolve` (depth cap and `MAX_TRANCHE_ABSORBED`)
/// and `carry_lineage` (`MAX_TRANCHE_INHERITED`) must carry forward unspent `remaining` credit
/// and drain the predecessor to 0 so `crystallize` and `settle_sale` are never stranded.
#[test]
fn p2_mod_53_tranche_rollover_preserves_remaining_credit() -> TestResult {
    let mut w = TestWorld::with_modules(vec![Box::new(InventoryModule::new())]);
    let families = w
        .world()
        .module_state::<InventoryHandles>()
        .ok_or("handles")?
        .0
        .clone();
    let metals = cid("metals");
    let grade = Grade::new(2)?;

    // Part A: depth-cap rollover in `dissolve` (a 1-unit depth-32 lot must not strand 1,000 units).
    let a = EndpointRef(VaultId(1));
    let buyer_ep = EndpointRef(VaultId(2));
    seed_origin_lot(w.engine_mut(), &families, LotId(100), a, "metals", 1_000, 2);
    apply(w.engine_mut(), &families, |txn| {
        families.dissolve(txn, a, LotId(100)).map(|_| ())
    })?;
    let deep_lot = Lot::new(
        LotId(101),
        a,
        metals.clone(),
        1,
        grade,
        Lineage::Split { parent: LotId(1) },
        MAX_LINEAGE_DEPTH,
    )?;
    apply(w.engine_mut(), &families, |txn| {
        txn.insert(&families.lots, &deep_lot.id(), deep_lot.clone())
            .map_err(InventoryError::from)
    })?;
    apply(w.engine_mut(), &families, |txn| {
        families.dissolve(txn, a, deep_lot.id()).map(|_| ())
    })?;

    let (gen1_id, gen1) = read(w.engine_mut(), &families, |txn| {
        families
            .tranche_for(txn, a, &metals)
            .expect("tranche_for")
            .expect("tranche exists")
    });
    assert_eq!(gen1.generation(), 1);
    assert_eq!(gen1.remaining(), 1_001);
    let gen0_id = gen1.predecessor().ok_or("predecessor")?;
    assert_ne!(gen0_id, gen1_id);
    let gen0 = read(w.engine_mut(), &families, |txn| {
        txn.get(&families.tranches, &gen0_id)
            .expect("get")
            .expect("gen0")
            .clone()
    });
    assert_eq!(
        gen0.remaining(),
        0,
        "predecessor must be drained to 0 so lineage credit is conserved"
    );

    // Both `crystallize` and `settle_sale` draw from the full carried-forward 1,001 credit.
    apply(w.engine_mut(), &families, |txn| {
        families
            .crystallize(txn, a, &metals, 500, grade, LotId(200))
            .map(|_| ())
    })?;
    let holder = ScopedIdempotencyKey::new(
        IdempotencyScope {
            principal: PrincipalId::Object { id: GOD },
            owner: mush_inventory::record::owner(),
            namespace: cid("p2_mod_53"),
        },
        1,
        IdempotencyKey::Request(RequestUid::from_bytes([7; 16])),
    );
    let stamp = now_ic(1_000);
    let res = apply(w.engine_mut(), &families, |txn| {
        families.reserve(txn, a, &metals, &holder, 500, &stamp)
    })?;
    apply(w.engine_mut(), &families, |txn| {
        families
            .settle_sale(
                txn,
                a,
                &metals,
                res,
                1,
                &stamp,
                CommitToken::from_bytes([9; 16]),
                LotSale {
                    destination: buyer_ep,
                    sold: 500,
                    grade,
                    buyer_lot: LotId(201),
                    remainder_lot: LotId(202),
                },
            )
            .map(|_| ())
    })?;
    let after = read(w.engine_mut(), &families, |txn| {
        families
            .tranche_for(txn, a, &metals)
            .expect("tranche_for")
            .expect("tranche")
            .1
    });
    assert_eq!(after.remaining(), 1);

    // Part B: `MAX_TRANCHE_ABSORBED` rollover in `dissolve`.
    let b = EndpointRef(VaultId(10));
    for i in 0..=MAX_TRANCHE_ABSORBED {
        let id = LotId(10_000 + u64::try_from(i)?);
        let qty = if i == MAX_TRANCHE_ABSORBED { 1 } else { 10 };
        seed_origin_lot(w.engine_mut(), &families, id, b, "metals", qty, 1);
        apply(w.engine_mut(), &families, |txn| {
            families.dissolve(txn, b, id).map(|_| ())
        })?;
    }
    let expected_b = u64::try_from(MAX_TRANCHE_ABSORBED)? * 10 + 1;
    apply(w.engine_mut(), &families, |txn| {
        families
            .crystallize(txn, b, &metals, expected_b, Grade::new(1)?, LotId(19_999))
            .map(|_| ())
    })?;

    // Part C: `MAX_TRANCHE_INHERITED` rollover in `carry_lineage`.
    let dst = EndpointRef(VaultId(20));
    for i in 0..=MAX_TRANCHE_INHERITED {
        let src = EndpointRef(VaultId(1_000 + u64::try_from(i)?));
        let id = LotId(20_000 + u64::try_from(i)?);
        let qty = if i == MAX_TRANCHE_INHERITED { 1 } else { 10 };
        seed_origin_lot(w.engine_mut(), &families, id, src, "metals", qty, 1);
        apply(w.engine_mut(), &families, |txn| {
            families.dissolve(txn, src, id).map(|_| ())
        })?;
        apply(w.engine_mut(), &families, |txn| {
            families.transfer(txn, src, dst, &metals, qty).map(|_| ())
        })?;
    }
    let expected_dst = u64::try_from(MAX_TRANCHE_INHERITED)? * 10 + 1;
    let dst_tranche = read(w.engine_mut(), &families, |txn| {
        families
            .tranche_for(txn, dst, &metals)
            .expect("tranche_for")
            .expect("tranche")
            .1
    });
    assert_eq!(dst_tranche.generation(), 1);
    assert_eq!(dst_tranche.remaining(), expected_dst);
    let dst_prev_id = dst_tranche.predecessor().ok_or("dst predecessor")?;
    let dst_prev = read(w.engine_mut(), &families, |txn| {
        txn.get(&families.tranches, &dst_prev_id)
            .expect("get")
            .expect("prev")
            .clone()
    });
    assert_eq!(dst_prev.remaining(), 0);
    apply(w.engine_mut(), &families, |txn| {
        families
            .crystallize(
                txn,
                dst,
                &metals,
                expected_dst,
                Grade::new(1)?,
                LotId(29_999),
            )
            .map(|_| ())
    })?;
    Ok(())
}

#[derive(Debug)]
struct StaticWreck {
    report: WreckReport,
}

impl SalvageSource for StaticWreck {
    fn wrecks(&self, _world: &World, _budget: usize) -> Vec<ObjId> {
        vec![self.report.root()]
    }

    fn wreck(&self, _world: &World, root: ObjId) -> Option<WreckReport> {
        (root == self.report.root()).then(|| self.report.clone())
    }
}

/// `P2-MOD-79`: `SalvageProvider` (`@salvage/recover`) must enforce `PoolCapacity` on the
/// destination endpoint before relocating a recovered lot, without leaking foreign lots.
#[test]
fn p2_mod_79_salvage_provider_enforces_pool_capacity() -> TestResult {
    let cell: InventoryHandleCell = Arc::default();
    let checks = Arc::new(AtomicUsize::new(0));
    // Predict vault IDs: 0 = wreck_ep, 1 = tight_dest (limit 10), 2 = other_ep.
    let wreck_root = ObjId(77);
    let wreck_ep = EndpointRef(VaultId(0));
    let tight_dest = EndpointRef(VaultId(1));
    let other_ep = EndpointRef(VaultId(2));
    let source = Arc::new(StaticWreck {
        report: WreckReport::new(wreck_root, None, ObjId(1), vec![wreck_ep]).ok_or("wreck")?,
    });
    let cap = Arc::new(CapPolicy {
        governed: vec![(tight_dest, 10)],
        forbidden: Some("uranium".to_owned()),
        checks: Arc::clone(&checks),
    });
    let inventory = InventoryModule::new()
        .with_handle_cell(Arc::clone(&cell))
        .with_salvage_source(source)
        .with_pool_capacity(cap);
    let mut w = TestWorld::with_modules(vec![Box::new(EconomyModule::new()), Box::new(inventory)]);
    set_offer_keys(&mut w);
    w.world_mut().set_now(T0);

    let pilot = w.spawn_player("Pilot");
    let v0 = make_vault(&mut w, pilot, "WreckHold")?;
    let v1 = make_vault(&mut w, pilot, "TightHold")?;
    let stranger = w.spawn_player("Stranger");
    let v2 = make_vault(&mut w, stranger, "StrangerHold")?;
    assert_eq!(v0, wreck_ep);
    assert_eq!(v1, tight_dest);
    assert_eq!(v2, other_ep);

    let families = cell.get().ok_or("families")?.clone();
    // Seed 5 pooled metals already in `tight_dest`, leaving room for only 5 more.
    apply(w.engine_mut(), &families, |txn| {
        families
            .credit(txn, tight_dest, &cid("metals"), 5)
            .map(|_| ())
    })?;
    let too_big = LotId(500);
    let fits = LotId(501);
    let foreign = LotId(502);
    seed_origin_lot(w.engine_mut(), &families, too_big, wreck_ep, "metals", 6, 1);
    seed_origin_lot(w.engine_mut(), &families, fits, wreck_ep, "metals", 5, 1);
    seed_origin_lot(
        w.engine_mut(),
        &families,
        foreign,
        other_ep,
        "uranium",
        100,
        1,
    );

    let salvage_action = mush_inventory::salvage::action_ref(&w.engine().registry).ok_or("act")?;
    let site = site_token(wreck_root, wreck_ep);

    // 1. Over-capacity recovery (5 pooled + 6 lot = 11 > 10) is refused by PoolCapacity.
    let err_full = submit_direct(
        &mut w,
        pilot,
        salvage_action.clone(),
        json!({
            "site": site,
            "to": tight_dest.vault().0.to_string(),
            "lot": too_big.to_string(),
        }),
        10,
    )
    .expect_err("over-capacity salvage must refuse");
    assert!(
        err_full.message().contains("Hold full"),
        "expected PoolCapacity refusal, got: {}",
        err_full.message()
    );
    let still_at_wreck = read(w.engine_mut(), &families, |txn| {
        txn.get(&families.lots, &too_big)
            .expect("get")
            .expect("lot")
            .endpoint()
    });
    assert_eq!(still_at_wreck, wreck_ep);

    // 2. Cross-endpoint foreign lot returns NO_SUCH_WRECK without invoking PoolCapacity.
    let before_checks = checks.load(Ordering::SeqCst);
    let err_foreign = submit_direct(
        &mut w,
        pilot,
        salvage_action.clone(),
        json!({
            "site": site_token(wreck_root, wreck_ep),
            "to": tight_dest.vault().0.to_string(),
            "lot": foreign.to_string(),
        }),
        11,
    )
    .expect_err("foreign lot salvage must refuse");
    assert_eq!(err_foreign.message(), NO_SUCH_WRECK);
    assert_eq!(checks.load(Ordering::SeqCst), before_checks);

    // 3. Recovery that fits within PoolCapacity (5 pooled + 5 lot = 10 <= 10) succeeds.
    submit_direct(
        &mut w,
        pilot,
        salvage_action,
        json!({
            "site": site_token(wreck_root, wreck_ep),
            "to": tight_dest.vault().0.to_string(),
            "lot": fits.to_string(),
        }),
        12,
    )?;
    let moved_ep = read(w.engine_mut(), &families, |txn| {
        txn.get(&families.lots, &fits)
            .expect("get")
            .expect("lot")
            .endpoint()
    });
    assert_eq!(moved_ep, tight_dest);
    Ok(())
}

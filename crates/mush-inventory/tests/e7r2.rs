//! E7 residue pass 2's inventory half: **P2 (salvage) and P4
//! (`inventory.salvage.recovered`)**, over the hazard axes 10b §15 names — **B**
//! provenance and **C** consent/custody, with **A**'s conservation clause (*nothing
//! mints*) asserted on every arm.
//!
//! The rig is `crates/mush-inventory/tests/e7c2.rs`'s: `apply()` opens the **same
//! owner-stamped transaction a provider gets** and installs it exactly as an exact
//! receipt does, so an `Err` installs nothing at all and every "writes nothing"
//! assertion means what it says. Every *authority* assertion goes through the real
//! `@salvage/*` command path against a `SalvageSource` standing in for `mush-server`'s
//! `SimSalvageSource` — the same trait the production composition wires, so what is
//! tested is the seam and not a mock of it.

use std::sync::Arc;

use mush_core::{ActionTxn, Audience, Engine, GOD, ObjId, ObjKind, World};
use mush_economy::{EconomyStore, ExternalRef, VaultId};
use mush_inventory::{
    AppliedMarker, EndpointRef, Grade, InventoryError, InventoryFamilies, InventoryHandles,
    InventoryModule, Lot, LotId, MAX_SALVAGE_ENDPOINTS, MAX_SALVAGE_SITES, MAX_SALVAGE_WRECKS,
    NO_SUCH_WRECK, SalvageSource, WreckReport, salvage_holder, salvage_holder_text,
};
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, text_of};

// ---- E7r2: the rig ---------------------------------------------------------

/// The composition-root oracle, stood in for. It is handed the wreck roots and the
/// endpoints that belong to them, which is exactly what
/// `mush_server::salvage_source::SimSalvageSource` derives from `JourneyBook`,
/// `SpatialState` and the economy's one-vault-per-backing index.
#[derive(Clone, Debug, Default)]
struct TestWrecks {
    /// `(root, captured_by, endpoints)`.
    wrecks: Vec<(ObjId, Option<ObjId>, Vec<EndpointRef>)>,
}

impl SalvageSource for TestWrecks {
    fn wrecks(&self, _world: &World, budget: usize) -> Vec<ObjId> {
        self.wrecks
            .iter()
            .map(|(root, _, _)| *root)
            .take(budget.min(MAX_SALVAGE_WRECKS))
            .collect()
    }
    fn wreck(&self, _world: &World, root: ObjId) -> Option<WreckReport> {
        self.wrecks
            .iter()
            .find(|(id, _, _)| *id == root)
            .and_then(|(id, by, endpoints)| WreckReport::new(*id, *by, ObjId(2), endpoints.clone()))
    }
}

fn boot(source: Option<Arc<dyn SalvageSource>>) -> (TestWorld, InventoryFamilies) {
    let mut inventory = InventoryModule::new();
    if let Some(source) = source {
        inventory = inventory.with_salvage_source(source);
    }
    let world = TestWorld::with_modules(vec![
        Box::new(mush_economy::EconomyModule::new()),
        Box::new(inventory),
    ]);
    let families = world
        .world()
        .module_state::<InventoryHandles>()
        .expect("inventory publishes its handles")
        .0
        .clone();
    (world, families)
}

fn texts(events: &[(Audience, OutputEvent)]) -> String {
    events
        .iter()
        .map(|(_, e)| text_of(e))
        .collect::<Vec<_>>()
        .join("\n")
}
fn has_error(events: &[(Audience, OutputEvent)]) -> bool {
    events.iter().any(|(_, e)| {
        matches!(e, OutputEvent::Sys(rt) if rt.spans().iter().any(|sp| sp.tag.as_deref() == Some("error")))
    })
}
fn ok(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    let ev = tw.run(actor, line);
    assert!(!has_error(&ev), "`{line}` was refused: {}", texts(&ev));
    texts(&ev)
}
fn err(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    let ev = tw.run(actor, line);
    assert!(
        has_error(&ev),
        "`{line}` should have been refused, got: {}",
        texts(&ev)
    );
    texts(&ev)
}

fn commodity(id: &str) -> mush_core::CanonicalId {
    mush_core::CanonicalId::new(id).expect("commodity id")
}

fn apply<T>(
    engine: &mut Engine,
    families: &InventoryFamilies,
    body: impl FnOnce(&mut ActionTxn<'_>) -> Result<T, InventoryError>,
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
    body: impl FnOnce(&mut ActionTxn<'_>) -> T,
) -> T {
    let mut txn = engine
        .action_transaction(&families.pools)
        .expect("inventory transaction");
    body(&mut txn)
}

/// The lot as it stands right now, read out of the family.
fn lot_of(tw: &mut TestWorld, families: &InventoryFamilies, id: LotId) -> Option<Lot> {
    let engine = tw.engine_mut();
    read(engine, families, |txn| {
        txn.get(&families.lots, &id).ok().flatten().cloned()
    })
}

/// The endpoint's lot census, which is what conservation over lots is counted on.
fn census(tw: &mut TestWorld, families: &InventoryFamilies, at: EndpointRef) -> u32 {
    let engine = tw.engine_mut();
    read(engine, families, |txn| {
        txn.get(&families.lot_census, &at)
            .ok()
            .flatten()
            .map_or(0, |c| c.get())
    })
}

/// One vault owned by `who`, backed by a fresh container object.
fn vault_for(tw: &mut TestWorld, who: ObjId, name: &str) -> EndpointRef {
    let object = tw
        .world_mut()
        .create(ObjKind::Thing, name, who, Some(mush_core::LIMBO))
        .expect("a backing object");
    let before: Vec<u64> = tw
        .world()
        .module_state::<EconomyStore>()
        .expect("economy")
        .snapshot()
        .vaults()
        .map(|v| v.id.0)
        .collect();
    ok(
        tw,
        GOD,
        &format!(
            "@vault/create {name}=container:#{} owner=#{}",
            object.0, who.0
        ),
    );
    let fresh = tw
        .world()
        .module_state::<EconomyStore>()
        .expect("economy")
        .snapshot()
        .vaults()
        .map(|v| v.id.0)
        .find(|id| !before.contains(id))
        .expect("a fresh vault");
    EndpointRef(VaultId(fresh))
}

/// Put one lot at `endpoint`, with a lineage the recovery must carry through.
///
/// `mint_origin` is the ONE verb that creates quantity, and this is the **fixture**:
/// it is what a mine did before the ship was lost. No salvage path calls it, which
/// `e7r2_p2_no_salvage_path_mints` probes at the source.
fn seed_lot(
    tw: &mut TestWorld,
    families: &InventoryFamilies,
    endpoint: EndpointRef,
    id: LotId,
    quantity: u64,
) {
    let engine = tw.engine_mut();
    apply(engine, families, |txn| {
        families
            .mint_origin(
                txn,
                endpoint,
                &commodity("iron"),
                quantity,
                Grade::new(2).expect("grade"),
                ExternalRef {
                    provider: "test".into(),
                    kind: "seed".into(),
                    id: "1".into(),
                    revision: None,
                },
                id,
            )
            .map(|_| ())
    })
    .expect("seed the stranded lot");
}

/// The whole fixture: Pat owns the wrecked hull's hold and a home vault; Robin is a
/// stranger; Captor took the ship.
struct Fx {
    pat: ObjId,
    robin: ObjId,
    captor: ObjId,
    wreck: ObjId,
    hold: EndpointRef,
    home: EndpointRef,
    lot: LotId,
}

/// `captured` names the captor the source reports, so the same fixture builds the
/// before- and after-capture worlds and the two refusals can be compared byte for
/// byte.
fn scene_with(captured: bool) -> (TestWorld, InventoryFamilies, Fx) {
    // The vault allocator is deterministic, so a throwaway world running the identical
    // opening sequence yields exactly the endpoints the real one will — the same trick
    // the recovery provider's pool account uses, and it asserts itself below.
    let (mut probe, _) = boot(None);
    let pat = probe.spawn_player("Pat");
    let _ = probe.spawn_player("Robin");
    let _ = probe.spawn_player("Captor");
    let hold = vault_for(&mut probe, pat, "hold");
    let home = vault_for(&mut probe, pat, "home");
    drop(probe);

    let (mut tw, families) = boot(None);
    let pat = tw.spawn_player("Pat");
    let robin = tw.spawn_player("Robin");
    let captor = tw.spawn_player("Captor");
    drop(tw);
    let source: Arc<dyn SalvageSource> = Arc::new(TestWrecks {
        wrecks: vec![(ObjId(7), captured.then_some(captor), vec![hold])],
    });
    let (mut tw, families2) = boot(Some(source));
    drop(families);
    let pat2 = tw.spawn_player("Pat");
    let robin2 = tw.spawn_player("Robin");
    let captor2 = tw.spawn_player("Captor");
    assert_eq!((pat2, robin2, captor2), (pat, robin, captor));
    let real_hold = vault_for(&mut tw, pat, "hold");
    let real_home = vault_for(&mut tw, pat, "home");
    assert_eq!(
        (real_hold, real_home),
        (hold, home),
        "the deterministic allocator gives the frozen source the real endpoints"
    );
    let lot = LotId(0x5a5a);
    seed_lot(&mut tw, &families2, hold, lot, 40);
    (
        tw,
        families2,
        Fx {
            pat,
            robin,
            captor,
            wreck: ObjId(7),
            hold,
            home,
            lot,
        },
    )
}

fn scene() -> (TestWorld, InventoryFamilies, Fx) {
    scene_with(false)
}

fn recover_line(fx: &Fx) -> String {
    format!(
        "@salvage/recover {}/{}/{}={}",
        fx.wreck.0,
        fx.hold.vault().0,
        fx.lot,
        fx.home.vault().0
    )
}

/// Run the recovery's own mechanic — the bundle `prepare_authoritative` stages —
/// against the same transaction the provider gets.
fn relocate(
    tw: &mut TestWorld,
    families: &InventoryFamilies,
    fx: &Fx,
) -> Result<Lot, InventoryError> {
    let engine = tw.engine_mut();
    apply(engine, families, |txn| {
        families.relocate(txn, fx.lot, fx.hold, fx.home)
    })
}

// ---- E7r2: P2 — hazard B, provenance ---------------------------------------

/// **A recovered lot is the SAME lot**: `id`, `commodity`, `quantity`, `grade`,
/// `lineage` and `depth` are byte-identical and only `endpoint` and `revision` moved
/// — and the two lot censuses move by one in opposite directions, so nothing was
/// created.
///
/// Asserted field by field rather than by a summary, because *"provenance intact"* is
/// exactly the sentence a coarsening bug would still pass.
#[test]
fn e7r2_p2_a_recovered_lot_is_the_same_lot_and_only_its_endpoint_and_revision_move() {
    let (mut tw, families, fx) = scene();
    let before = lot_of(&mut tw, &families, fx.lot).expect("the stranded lot");
    assert_eq!(before.endpoint(), fx.hold);
    let (hold_before, home_before) = (
        census(&mut tw, &families, fx.hold),
        census(&mut tw, &families, fx.home),
    );

    let moved = relocate(&mut tw, &families, &fx).expect("the custodian recovers it");

    assert_eq!(moved.id(), before.id(), "the SAME LotId");
    assert_eq!(moved.commodity(), before.commodity());
    assert_eq!(moved.quantity(), before.quantity(), "nothing was skimmed");
    assert_eq!(moved.grade(), before.grade());
    assert_eq!(moved.lineage(), before.lineage(), "provenance intact");
    assert_eq!(moved.depth(), before.depth(), "and not re-rooted");
    assert_eq!(moved.endpoint(), fx.home, "only the endpoint moved…");
    assert_eq!(moved.revision(), before.revision() + 1, "…and the revision");

    // The observable outcome: what the owner reads afterwards, and the conservation
    // probe over lots.
    let after = lot_of(&mut tw, &families, fx.lot).expect("still exactly one lot");
    assert_eq!(after, moved);
    assert_eq!(
        census(&mut tw, &families, fx.hold),
        hold_before - 1,
        "one lot left the wreck…"
    );
    assert_eq!(
        census(&mut tw, &families, fx.home),
        home_before + 1,
        "…and exactly one arrived: nothing was duplicated"
    );
    let shown = ok(&mut tw, fx.pat, &format!("@inv/lot {}", fx.lot));
    assert!(
        shown.contains(&fx.home.vault().0.to_string()),
        "and the owner reads it at its new endpoint: {shown}"
    );
}

/// **A second recovery of the same lot recovers nothing and duplicates nothing.**
///
/// Two independent guards and the test names both: `relocate` refuses a lot that is no
/// longer at `from` — which is what makes a *deliberate* second attempt a refusal
/// rather than a second move — and the applied marker refuses a replay of the
/// identical request, epoch-free so it survives a reboot.
#[test]
fn e7r2_p2_a_second_recovery_moves_nothing_and_duplicates_nothing() {
    let (mut tw, families, fx) = scene();
    let first = relocate(&mut tw, &families, &fx).expect("recovered");
    let censuses = (
        census(&mut tw, &families, fx.hold),
        census(&mut tw, &families, fx.home),
    );

    assert!(
        matches!(
            relocate(&mut tw, &families, &fx),
            Err(InventoryError::NoLot)
        ),
        "the lot is not at `from` any more"
    );
    assert_eq!(
        lot_of(&mut tw, &families, fx.lot).expect("still one lot"),
        first,
        "and the lot is byte-identical"
    );
    assert_eq!(
        (
            census(&mut tw, &families, fx.hold),
            census(&mut tw, &families, fx.home)
        ),
        censuses,
        "neither census moved"
    );
}

/// **The applied marker is the replay guard, and it survives a reboot.**
///
/// A `ScopedIdempotencyKey` carries a per-process random epoch and does **not** survive
/// a restart, so a retried recovery after one would be a genuinely new request. The
/// marker id is `{at}{request}` — a pure function of durable causal input — so an
/// existing one refuses, and it is still there after the module reloads.
#[test]
fn e7r2_p2_the_applied_marker_is_the_replay_guard_across_a_reboot() {
    let (mut tw, families, fx) = scene();
    let id = smol_str::SmolStr::new("8000000000000fa0deadbeefdeadbeef");
    let marker = AppliedMarker::new(
        mush_inventory::FACT_SALVAGE_RECOVERED,
        Some(fx.hold),
        Some(fx.home),
        commodity("iron"),
        40,
        fx.pat,
        1_000,
    )
    .expect("a salvage marker");
    let engine = tw.engine_mut();
    apply(engine, &families, |txn| {
        txn.insert(&families.applied, &id, marker.clone())
            .map_err(InventoryError::from)
    })
    .expect("write the marker");

    // A second write of the same id is refused, which is the branch the provider
    // takes before its one irreversible mutation.
    let engine = tw.engine_mut();
    assert!(
        apply(engine, &families, |txn| {
            txn.insert(&families.applied, &id, marker.clone())
                .map_err(InventoryError::from)
        })
        .is_err(),
        "an applied request cannot be applied twice"
    );

    // And it is still there after a reboot — the marker is typed state the kernel
    // hydrates, not a per-process cache.
    tw.modules_loaded(std::collections::HashMap::new());
    let engine = tw.engine_mut();
    let present = read(engine, &families, |txn| {
        txn.get(&families.applied, &id).ok().flatten().is_some()
    });
    assert!(present, "the guard survives the restart");
}

/// **No salvage path mints**, probed at the source: `mint_origin` — the one verb that
/// creates quantity rather than moving it — has no call site in `salvage.rs`, and
/// `relocate` touches no pool at all.
#[test]
fn e7r2_p2_no_salvage_path_mints() {
    let crate_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let salvage = std::fs::read_to_string(crate_dir.join("src/salvage.rs")).expect("salvage.rs");
    assert!(
        !salvage.contains("mint_origin("),
        "a recovered lot is MOVED, never minted"
    );
    assert!(
        salvage.contains(".relocate("),
        "and it moves through `InventoryFamilies::relocate`, which is \
         `Lot::checked_relocate`'s one caller"
    );
    let pool = std::fs::read_to_string(crate_dir.join("src/pool.rs")).expect("pool.rs");
    let at = pool.find("pub fn relocate(").expect("the relocate seam");
    let end = pool[at..]
        .find("    /// Merge several lots")
        .map_or(pool.len(), |offset| at + offset);
    let body = &pool[at..end];
    for forbidden in ["write_pair(", "mint_origin(", "add_pool(", "fresh_pair("] {
        assert!(
            !body.contains(forbidden),
            "`relocate` touches no pool: it must not call `{forbidden}`"
        );
    }
    // …and `checked_relocate` really is called exactly once in the whole crate.
    let record = std::fs::read_to_string(crate_dir.join("src/record.rs")).expect("record.rs");
    assert!(record.contains("pub fn checked_relocate("));
    assert_eq!(
        pool.matches("checked_relocate(").count(),
        1,
        "one caller, and it is `relocate`"
    );
}

// ---- E7r2: P2 — hazard C, consent and custody ------------------------------

/// **A captor, a stranger and a phantom wreck all read ONE line, and the captor's
/// refusal is byte-identical before and after the capture.**
///
/// Doc 10 T3: capture changes custody or possession, **never title**, and title moves
/// only through T3's closed list. Until custody moves through E3's own path the captor
/// is not `may_take_from_vault` over the cargo's vault — so what they read is not a
/// "denied", it is "there is no such wreck", the same line a stranger reads and the
/// same line anybody reads for a wreck that never existed.
#[test]
fn e7r2_p2_a_captor_a_stranger_and_a_phantom_wreck_all_read_one_line() {
    let (mut tw, families, fx) = scene_with(false);
    let captor_before = err(&mut tw, fx.captor, &recover_line(&fx));
    let stranger = err(&mut tw, fx.robin, &recover_line(&fx));
    let phantom = err(
        &mut tw,
        fx.captor,
        &format!(
            "@salvage/recover 4242/9999/{}={}",
            fx.lot,
            fx.home.vault().0
        ),
    );
    assert_eq!(
        captor_before, stranger,
        "a captor reads the stranger's line"
    );
    assert_eq!(
        captor_before, phantom,
        "…and both read the line a wreck that does not exist reads"
    );
    assert!(captor_before.contains(NO_SUCH_WRECK));
    assert!(
        tw.engine_mut().take_authoritative_submit().is_none(),
        "and none of the three staged anything at all"
    );
    assert_eq!(
        lot_of(&mut tw, &families, fx.lot)
            .expect("the lot")
            .endpoint(),
        fx.hold,
        "so nothing moved"
    );

    // After the capture: the source now names the captor, and NOTHING about the
    // refusal changes, because capture does not change `may_take_from_vault`'s answer.
    let (mut tw2, families2, fx2) = scene_with(true);
    let captor_after = err(&mut tw2, fx2.captor, &recover_line(&fx2));
    assert_eq!(
        captor_after, captor_before,
        "the refusal a captor reads is byte-identical before and after the capture"
    );
    // …while the custodian's own recovery still projects, so it is a custody answer
    // and not a broken lane.
    ok(&mut tw2, fx2.pat, &recover_line(&fx2));
    assert!(
        tw2.engine_mut().take_authoritative_submit().is_some(),
        "the custodian's own recovery stages its one submission"
    );
    assert!(relocate(&mut tw2, &families2, &fx2).is_ok());
}

/// **`@salvage/status` discloses no other principal's wreck**, and a captor's status
/// page is byte-identical to a stranger's and to an actor's on a world with no wrecks.
#[test]
fn e7r2_p2_status_discloses_no_other_principals_wreck() {
    let (mut tw, _families, fx) = scene_with(true);
    let captor = ok(&mut tw, fx.captor, "@salvage/status");
    let stranger = ok(&mut tw, fx.robin, "@salvage/status");
    assert_eq!(captor, stranger);
    assert!(captor.contains("(nothing to recover)"), "{captor}");
    assert!(
        !captor.contains(&format!("Wreck #{}", fx.wreck.0)),
        "no wreck is named to somebody who may not draw from it: {captor}"
    );

    // The custodian's own page names it, with the recommended command filled in.
    let custodian = ok(&mut tw, fx.pat, "@salvage/status");
    assert!(
        custodian.contains(&format!("Wreck #{}", fx.wreck.0)),
        "{custodian}"
    );
    assert!(custodian.contains("@salvage/recover"), "{custodian}");
}

/// **An unwired `SalvageSource` is fail-closed** and says nothing about the server.
///
/// No action is registered at all, so `@salvage/recover` on a real wreck reads exactly
/// what it reads for one that never existed — an unwired composition is not
/// announced, because that would tell a player about the server rather than the world.
#[test]
fn e7r2_p2_an_unwired_salvage_source_is_fail_closed_and_unannounced() {
    let (mut tw, families) = boot(None);
    let pat = tw.spawn_player("Pat");
    let hold = vault_for(&mut tw, pat, "hold");
    let home = vault_for(&mut tw, pat, "home");
    let lot = LotId(0x5a5a);
    seed_lot(&mut tw, &families, hold, lot, 40);
    assert!(
        mush_inventory::salvage::action_ref(tw.registry()).is_none(),
        "no salvage action is registered at all"
    );
    let status = ok(&mut tw, pat, "@salvage/status");
    assert!(status.contains("(nothing to recover)"), "{status}");
    let refusal = err(
        &mut tw,
        pat,
        &format!(
            "@salvage/recover 7/{}/{lot}={}",
            hold.vault().0,
            home.vault().0
        ),
    );
    assert!(refusal.contains(NO_SUCH_WRECK), "{refusal}");
    assert!(tw.engine_mut().take_authoritative_submit().is_none());
    assert_eq!(
        lot_of(&mut tw, &families, lot).expect("the lot").endpoint(),
        hold,
        "and nothing moved"
    );
}

/// **Title never moves on this lane** — not on the recovery, and the recovery does not
/// need it to.
///
/// `EconomicAsset.title_owner` is asserted across the whole sequence, because doc 10
/// T3's closed list is the only place title moves and a salvage **claim** (an E4b2
/// contract with a contest deadline and a named adjudicator) is explicitly not built
/// by this pass. What moved is matter, under custody.
#[test]
fn e7r2_p2_title_never_moves_on_the_salvage_lane() {
    let (mut tw, families, fx) = scene();
    let hull = tw
        .world_mut()
        .create(ObjKind::Thing, "Hull", fx.pat, Some(mush_core::LIMBO))
        .expect("a hull");
    ok(
        &mut tw,
        GOD,
        &format!("@asset/title #{}=#{}", hull.0, fx.pat.0),
    );
    let title_of = |tw: &TestWorld| {
        tw.world()
            .module_state::<EconomyStore>()
            .expect("economy")
            .snapshot()
            .asset(hull)
            .expect("the titled hull")
            .title_owner
            .clone()
    };
    let before = title_of(&tw);

    ok(&mut tw, fx.pat, &recover_line(&fx));
    relocate(&mut tw, &families, &fx).expect("recovered");

    assert_eq!(
        before,
        title_of(&tw),
        "recovery moved matter under custody, and no title at all"
    );
    assert_eq!(
        lot_of(&mut tw, &families, fx.lot)
            .expect("the lot")
            .endpoint(),
        fx.home,
        "…and it really did move the matter"
    );
}

/// **A destination the actor may not take from, and the wreck's own endpoint, are
/// both refused** — the two arms of the destination gate, named.
#[test]
fn e7r2_p2_a_strangers_vault_and_the_wrecks_own_endpoint_are_both_refused() {
    let (mut tw, families, fx) = scene();
    let theirs = vault_for(&mut tw, fx.robin, "theirs");
    let stranger_vault = err(
        &mut tw,
        fx.pat,
        &format!(
            "@salvage/recover {}/{}/{}={}",
            fx.wreck.0,
            fx.hold.vault().0,
            fx.lot,
            theirs.vault().0
        ),
    );
    assert!(stranger_vault.contains(NO_SUCH_WRECK), "{stranger_vault}");
    let self_move = err(
        &mut tw,
        fx.pat,
        &format!(
            "@salvage/recover {}/{}/{}={}",
            fx.wreck.0,
            fx.hold.vault().0,
            fx.lot,
            fx.hold.vault().0
        ),
    );
    assert!(
        self_move.contains(NO_SUCH_WRECK),
        "one endpoint's lot_census is one key: {self_move}"
    );
    assert!(tw.engine_mut().take_authoritative_submit().is_none());
    assert_eq!(
        lot_of(&mut tw, &families, fx.lot)
            .expect("the lot")
            .endpoint(),
        fx.hold,
        "neither wrote anything"
    );
}

// ---- E7r2: P2 — the seam, the key and the caps -----------------------------

/// **The salvage holder is deterministic, `Scoped`, and inside K's key cap** — the
/// third consumer of `wo_holder`'s pattern, copying it rather than inventing a third
/// key shape.
#[test]
fn e7r2_p2_the_salvage_holder_is_deterministic_scoped_and_inside_the_kernel_cap() {
    let a = salvage_holder(ObjId(7)).expect("a holder");
    let b = salvage_holder(ObjId(7)).expect("the same holder");
    assert_eq!(a, b, "a pure function of the root");
    assert_ne!(
        a,
        salvage_holder(ObjId(8)).expect("another"),
        "and one key per wreck"
    );
    assert!(
        salvage_holder_text(ObjId(u32::MAX)).len() <= mush_core::MAX_IDEMPOTENCY_KEY_BYTES,
        "inside K's key cap even at the largest root"
    );
    assert_eq!(mush_core::MAX_IDEMPOTENCY_KEY_BYTES, 128);
    // The `<wreck>/<endpoint>` token is one parameter carrying both halves, so a
    // request cannot name a wreck and an endpoint that disagree.
    let token = mush_inventory::site_token(ObjId(7), EndpointRef(VaultId(500)));
    assert_eq!(
        mush_inventory::salvage::parse_site(&token),
        Some((ObjId(7), EndpointRef(VaultId(500))))
    );
    assert!(mush_inventory::salvage::parse_site("7").is_none());
}

/// **The caps are encoded**, and a wreck reporting more endpoints than the cap is
/// **refused, never truncated** — a silently dropped endpoint is stock its owner is
/// told does not exist.
#[test]
fn e7r2_p2_the_caps_are_encoded_and_an_oversized_report_refuses() {
    assert_eq!(MAX_SALVAGE_WRECKS, 32);
    assert_eq!(MAX_SALVAGE_ENDPOINTS, 16);
    assert_eq!(MAX_SALVAGE_SITES, 64);
    let over: Vec<EndpointRef> = (0..=MAX_SALVAGE_ENDPOINTS as u64)
        .map(|id| EndpointRef(VaultId(id + 1)))
        .collect();
    assert_eq!(over.len(), MAX_SALVAGE_ENDPOINTS + 1);
    assert!(
        WreckReport::new(ObjId(7), None, ObjId(2), over).is_none(),
        "refused, not truncated"
    );
    let exact = (0..MAX_SALVAGE_ENDPOINTS as u64)
        .map(|id| EndpointRef(VaultId(id + 1)))
        .collect();
    assert!(WreckReport::new(ObjId(7), None, ObjId(2), exact).is_some());
    assert!(
        WreckReport::new(ObjId(0), None, ObjId(2), Vec::new()).is_none(),
        "and a zero root is no wreck"
    );
}

/// **`relocate` refuses every arm** — a lot that is not there, a lot that is somewhere
/// else, and a self-move — and writes nothing on any of them.
#[test]
fn e7r2_p2_relocate_refuses_a_missing_lot_a_moved_lot_and_a_self_move() {
    let (mut tw, families, fx) = scene();
    let engine = tw.engine_mut();
    assert!(matches!(
        apply(engine, &families, |txn| families.relocate(
            txn,
            LotId(0xeeee),
            fx.hold,
            fx.home
        )),
        Err(InventoryError::NoLot)
    ));
    let engine = tw.engine_mut();
    assert!(matches!(
        apply(engine, &families, |txn| families
            .relocate(txn, fx.lot, fx.home, fx.hold)),
        Err(InventoryError::NoLot)
    ));
    let engine = tw.engine_mut();
    assert!(matches!(
        apply(engine, &families, |txn| families
            .relocate(txn, fx.lot, fx.hold, fx.hold)),
        Err(InventoryError::MergeMismatch)
    ));
    assert_eq!(
        lot_of(&mut tw, &families, fx.lot)
            .expect("the lot")
            .endpoint(),
        fx.hold,
        "and after all three the lot is exactly where it was"
    );
}

/// **The five `InventoryVerb`s are still five**, and none of them is a relocate.
///
/// The absence is the lock (10b §15.5's constraint (3)): a player-reachable relocate
/// would let a seller walk a listed lot out from under the market's own hold. P2 adds
/// a separate action with its own gate, and this is the pin that keeps it separate.
#[test]
fn e7r2_p2_salvage_adds_no_inventory_verb() {
    let crate_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let provider = std::fs::read_to_string(crate_dir.join("src/provider.rs")).expect("provider.rs");
    let at = provider
        .find("pub enum InventoryVerb")
        .expect("the verb enum");
    let end = provider[at..].find('}').map_or(provider.len(), |o| at + o);
    let body = &provider[at..end];
    for variant in ["Move", "Split", "Merge", "Dissolve", "Crystallize"] {
        assert!(body.contains(variant), "the five stay five: {variant}");
    }
    for forbidden in ["Reserve", "Release", "Consume", "Relocate", "Salvage"] {
        assert!(
            !body.contains(forbidden),
            "no `{forbidden}` verb: the absence is the lock"
        );
    }
    // …and `@inv` still registers exactly the five submitting switches it always did.
    let commands = std::fs::read_to_string(crate_dir.join("src/commands.rs")).expect("commands.rs");
    assert!(
        !commands.contains("\"relocate\".into()"),
        "no player-reachable relocate switch either"
    );
}

// ---- E7r2: P4 — `inventory.salvage.recovered` ------------------------------

/// **The third `AppliedMarker` kind is a KIND and not a namespace**, its `from_parts`
/// refusal is still a closed list, and a fourth kind is refused on load.
#[test]
fn e7r2_p4_the_salvage_fact_is_a_third_kind_and_a_fourth_is_refused_on_load() {
    let marker = AppliedMarker::new(
        mush_inventory::FACT_SALVAGE_RECOVERED,
        Some(EndpointRef(VaultId(1))),
        Some(EndpointRef(VaultId(2))),
        commodity("iron"),
        40,
        ObjId(5),
        1_000,
    )
    .expect("the third kind is accepted");
    assert_eq!(marker.kind(), "inventory.salvage.recovered");
    let json = serde_json::to_value(&marker).expect("serialize");
    let back: AppliedMarker = serde_json::from_value(json).expect("re-enter through the wire");
    assert_eq!(back, marker);
    assert!(
        AppliedMarker::new(
            "tactics.salvage.recovered",
            None,
            None,
            commodity("iron"),
            1,
            ObjId(5),
            1
        )
        .is_err(),
        "`tactics.salvage.recovered` is decided never to exist (10b:1714)"
    );
    let bogus = serde_json::json!({
        "kind": "inventory.salvage.laundered",
        "from": null,
        "to": null,
        "commodity": "iron",
        "quantity": 1,
        "actor": 5,
        "at": 1
    });
    assert!(
        serde_json::from_value::<AppliedMarker>(bogus).is_err(),
        "a fourth kind is refused ON LOAD, never truncated"
    );
}

/// **The fact answers unsatisfied for a recovery that merely started, and satisfied
/// only from the applied marker** — and one marker answers exactly one kind.
#[test]
fn e7r2_p4_the_salvage_fact_is_answered_from_the_marker_and_never_from_a_status() {
    use mush_economy::FactProvider;
    let (mut tw, families, fx) = scene();
    let provider = mush_inventory::InventoryFactProvider;
    let id = smol_str::SmolStr::new("8000000000000fa0deadbeefdeadbeef");

    // The command alone writes no marker: the mechanic is the provider's, and a
    // recovery that merely STARTED satisfies nothing.
    ok(&mut tw, fx.pat, &recover_line(&fx));
    tw.modules_loaded(std::collections::HashMap::new());
    assert!(
        provider
            .resolve(
                tw.world(),
                mush_inventory::FACT_SALVAGE_RECOVERED,
                &id,
                None
            )
            .is_none(),
        "unknown is unsatisfied"
    );

    // The committed bundle's own marker is what satisfies it.
    let marker = AppliedMarker::new(
        mush_inventory::FACT_SALVAGE_RECOVERED,
        Some(fx.hold),
        Some(fx.home),
        commodity("iron"),
        40,
        fx.pat,
        1_000,
    )
    .expect("a salvage marker");
    let engine = tw.engine_mut();
    apply(engine, &families, |txn| {
        txn.insert(&families.applied, &id, marker)
            .map_err(InventoryError::from)
    })
    .expect("the bundle writes it");
    tw.modules_loaded(std::collections::HashMap::new());

    let state = provider
        .resolve(
            tw.world(),
            mush_inventory::FACT_SALVAGE_RECOVERED,
            &id,
            None,
        )
        .expect("the fact is satisfied from the marker");
    assert!(state.satisfied);
    assert_eq!(state.actor, Some(fx.pat), "attributed to the recoverer");
    assert!(
        provider
            .resolve(
                tw.world(),
                mush_inventory::FACT_SALVAGE_RECOVERED,
                &id,
                Some(2)
            )
            .is_none(),
        "a named revision other than 1 is a reference to a world this crate never had"
    );
    assert!(
        provider
            .resolve(tw.world(), mush_inventory::FACT_TRANSFERRED, &id, None)
            .is_none(),
        "one marker answers exactly one kind"
    );
    assert!(
        provider
            .resolve(tw.world(), "inventory.salvage.claimed", &id, None)
            .is_none(),
        "and an unknown kind answers nothing at all"
    );
}

/// **`with_facts` has six base namespaces and TacSim's two optional namespaces.**
/// Item 26 added a kind to `inventory.*`, never a separate salvage provider. Survey
/// and passenger receipts add providers only when the TacSim owner is composed.
#[test]
fn e7r2_p4_the_fact_provider_registers_no_separate_salvage_namespace() {
    // The full server composition root lives in `mush-composition`.
    let server = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .join("mush-composition/src/lib.rs");
    let src = std::fs::read_to_string(server).expect("mush-composition/src/lib.rs");
    let (_, facts) = src
        .split_once(".with_facts(")
        .expect("the facts composition");
    let (body, _) = facts
        .split_once(".with_observers(")
        .expect("the end of the facts composition");
    for provider in [
        "mush_jobs::JobsFactProvider",
        "mush_inventory::InventoryFactProvider",
        "mush_industry::IndustryFactProvider",
        "mush_campaign::CampaignFactProvider",
        "mush_sim::SimFactProvider",
        "mush_market::MarketFactProvider",
        "tacsim::surface::survey_facts::SurveyFactProvider",
        "tacsim::port::passenger::facts::PassengerFactProvider",
    ] {
        assert_eq!(
            body.matches(provider).count(),
            1,
            "one registration for {provider}: {body}"
        );
    }
    assert_eq!(
        body.matches("Arc::new(").count(),
        8,
        "six base providers and the owner-gated survey and passenger providers: {body}"
    );
    let (_, owner_gated) = body
        .split_once("if tacsim_delegate {")
        .expect("survey and passenger facts require the TacSim owner");
    let (owner_gated, _) = owner_gated.split_once('}').expect("the owner gate closes");
    assert!(
        owner_gated.contains("tacsim::surface::survey_facts::SurveyFactProvider"),
        "the survey provider is registered inside the owner gate"
    );
    assert!(
        owner_gated.contains("tacsim::port::passenger::facts::PassengerFactProvider"),
        "the passenger provider is registered inside the owner gate"
    );
    assert!(
        !body.contains("Salvage"),
        "P4 added a KIND on the existing `inventory.*` provider, never a namespace"
    );
}
